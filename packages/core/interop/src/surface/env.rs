//! Iterative environment, theme, safe-area, and display-scale cascading for [`SurfaceOrchestrator`].

use nve_abi::SurfaceId;

use super::orchestrator::SurfaceOrchestrator;
use super::types::{SafeAreaInsets, SurfaceEnvironment, SurfaceEvent, ThemeContext};
use crate::error::InteropError;

impl SurfaceOrchestrator {
    /// Updates the environment at `surface_id` and propagates it down the subtree iteratively,
    /// respecting each descendant's [`super::types::EnvironmentOverrides`].
    pub fn update_environment(
        &mut self,
        surface_id: SurfaceId,
        environment: SurfaceEnvironment,
    ) -> Result<Vec<SurfaceId>, InteropError> {
        if self.get(surface_id).is_none() {
            return Err(InteropError::SurfaceNotFound(surface_id));
        }

        let mut updated = Vec::new();
        self.propagate_env_iterative(surface_id, environment, Some(&mut updated));
        Ok(updated)
    }

    /// Iteratively cascades `incoming` down `root_id`'s subtree in pre-order using an `O(depth)`
    /// cursor stack (zero recursion, zero per-child stack environment clones) and prunes any
    /// non-root subtree whose effective environment did not change.
    pub(super) fn propagate_env_iterative(
        &mut self,
        root_id: SurfaceId,
        mut incoming: SurfaceEnvironment,
        mut updated: Option<&mut Vec<SurfaceId>>,
    ) {
        let has_children = {
            let Some(rec) = self.get_mut(root_id) else {
                return;
            };
            rec.overrides.apply_to(&mut incoming);
            let hc = !rec.children.is_empty();
            let changed = rec.environment != incoming;
            if changed {
                rec.environment = incoming.clone();
            }
            if changed {
                if let Some(out) = updated.as_deref_mut() {
                    out.push(root_id);
                }
                self.events.push(SurfaceEvent::EnvironmentPropagated {
                    surface_id: root_id,
                    environment: incoming,
                });
            }
            hc
        };
        if !has_children {
            return;
        }

        // `O(depth)` DFS cursor frames: `(parent_surface_id, next_child_index)`.
        let mut stack: Vec<(SurfaceId, usize)> = vec![(root_id, 0)];
        while let Some((parent_id, next_child)) = stack.last_mut() {
            let parent_id = *parent_id;
            let child_idx_in_parent = *next_child;
            let child_id = match self.get(parent_id) {
                Some(parent_rec) if child_idx_in_parent < parent_rec.children.len() => {
                    *next_child += 1;
                    parent_rec.children[child_idx_in_parent]
                }
                _ => {
                    stack.pop();
                    continue;
                }
            };

            let p_slot_idx = parent_id.index();
            let c_slot_idx = child_id.index();
            if p_slot_idx == c_slot_idx
                || p_slot_idx >= self.slots.len()
                || c_slot_idx >= self.slots.len()
            {
                continue;
            }

            // Borrow distinct parent and child slots simultaneously without cloning parent's
            // environment when the child has no overrides and is already equal.
            let (parent_slot, child_slot) = if p_slot_idx < c_slot_idx {
                let (head, tail) = self.slots.split_at_mut(c_slot_idx);
                (&head[p_slot_idx], &mut tail[0])
            } else {
                let (head, tail) = self.slots.split_at_mut(p_slot_idx);
                (&tail[0], &mut head[c_slot_idx])
            };
            if parent_slot.generation != parent_id.generation()
                || child_slot.generation != child_id.generation()
            {
                continue;
            }
            let (Some(parent_rec), Some(child_rec)) =
                (parent_slot.record.as_deref(), child_slot.record.as_deref_mut())
            else {
                continue;
            };

            let (changed, child_has_children) = if child_rec.overrides.is_empty() {
                if child_rec.environment == parent_rec.environment {
                    (false, false)
                } else {
                    child_rec.environment = parent_rec.environment.clone();
                    let ev_env = child_rec.environment.clone();
                    let hc = !child_rec.children.is_empty();
                    if let Some(out) = updated.as_deref_mut() {
                        out.push(child_id);
                    }
                    self.events.push(SurfaceEvent::EnvironmentPropagated {
                        surface_id: child_id,
                        environment: ev_env,
                    });
                    (true, hc)
                }
            } else {
                let mut next = parent_rec.environment.clone();
                child_rec.overrides.apply_to(&mut next);
                if child_rec.environment == next {
                    (false, false)
                } else {
                    child_rec.environment = next.clone();
                    let hc = !child_rec.children.is_empty();
                    if let Some(out) = updated.as_deref_mut() {
                        out.push(child_id);
                    }
                    self.events.push(SurfaceEvent::EnvironmentPropagated {
                        surface_id: child_id,
                        environment: next,
                    });
                    (true, hc)
                }
            };

            if changed && child_has_children {
                stack.push((child_id, 0));
            }
        }
    }

    fn update_env_field(
        &mut self,
        surface_id: SurfaceId,
        mutate: impl FnOnce(&mut SurfaceEnvironment),
    ) -> Result<Vec<SurfaceId>, InteropError> {
        let mut env = self
            .get(surface_id)
            .ok_or(InteropError::SurfaceNotFound(surface_id))?
            .environment
            .clone();
        mutate(&mut env);
        self.update_environment(surface_id, env)
    }

    /// Convenience helper to update only the theme at `surface_id` and cascade to children.
    pub fn set_theme(
        &mut self,
        surface_id: SurfaceId,
        theme: ThemeContext,
    ) -> Result<Vec<SurfaceId>, InteropError> {
        self.update_env_field(surface_id, |env| env.theme = theme)
    }

    /// Convenience helper to update safe-area insets at `surface_id` and cascade to children.
    pub fn set_safe_area(
        &mut self,
        surface_id: SurfaceId,
        safe_area: SafeAreaInsets,
    ) -> Result<Vec<SurfaceId>, InteropError> {
        self.update_env_field(surface_id, |env| env.safe_area = safe_area)
    }

    /// Convenience helper to update display scale factor at `surface_id` and cascade to children.
    pub fn set_scale_factor(
        &mut self,
        surface_id: SurfaceId,
        scale_factor: f32,
    ) -> Result<Vec<SurfaceId>, InteropError> {
        self.update_env_field(surface_id, |env| env.scale_factor = scale_factor)
    }
}
