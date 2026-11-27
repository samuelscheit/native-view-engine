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
        self.propagate_env_iterative(surface_id, environment, &mut updated);
        Ok(updated)
    }

    /// Iteratively cascades `incoming` down `root_id`'s subtree (pre-order, zero recursion, zero
    /// per-node child-vec cloning).
    pub(super) fn propagate_env_iterative(
        &mut self,
        root_id: SurfaceId,
        incoming: SurfaceEnvironment,
        updated: &mut Vec<SurfaceId>,
    ) {
        let mut stack: Vec<(SurfaceId, SurfaceEnvironment)> = vec![(root_id, incoming)];
        while let Some((surface_id, env)) = stack.pop() {
            let idx = surface_id.index();
            let Some(slot) = self.slots.get_mut(idx) else {
                continue;
            };
            if slot.generation != surface_id.generation() {
                continue;
            }
            let Some(rec) = slot.record.as_deref_mut() else {
                continue;
            };

            let mut next = env;
            rec.overrides.apply_to(&mut next);
            let changed = rec.environment != next;
            if changed {
                rec.environment = next.clone();
                updated.push(surface_id);
                self.events.push(SurfaceEvent::EnvironmentPropagated {
                    surface_id,
                    environment: next,
                });
            }

            if !rec.children.is_empty() {
                let effective = rec.environment.clone();
                // Push children in reverse order so pop order preserves declaration pre-order.
                let last_idx = rec.children.len() - 1;
                for i in (1..=last_idx).rev() {
                    stack.push((rec.children[i], effective.clone()));
                }
                stack.push((rec.children[0], effective));
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
