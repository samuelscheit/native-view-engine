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
    /// cursor stack (zero recursion, zero `SurfaceEnvironment` clones on unchanged nodes).
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
            if rec.environment != incoming {
                rec.environment = incoming.clone();
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

            let child_has_children = !child_rec.children.is_empty();
            if child_rec.overrides.is_empty() {
                if child_rec.environment != parent_rec.environment {
                    child_rec.environment = parent_rec.environment.clone();
                    let ev_env = child_rec.environment.clone();
                    if let Some(out) = updated.as_deref_mut() {
                        out.push(child_id);
                    }
                    self.events.push(SurfaceEvent::EnvironmentPropagated {
                        surface_id: child_id,
                        environment: ev_env,
                    });
                }
            } else {
                let mut next = parent_rec.environment.clone();
                child_rec.overrides.apply_to(&mut next);
                if child_rec.environment != next {
                    child_rec.environment = next.clone();
                    if let Some(out) = updated.as_deref_mut() {
                        out.push(child_id);
                    }
                    self.events.push(SurfaceEvent::EnvironmentPropagated {
                        surface_id: child_id,
                        environment: next,
                    });
                }
            }

            if child_has_children {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::{EnvironmentOverrides, FrameworkKind, SurfaceDescriptor};

    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            self.0 >> 32
        }
    }

    #[test]
    fn propagate_env_iterative_matches_recursive_model_across_overrides_reparent_and_diverged_descendants() {
        // Explicit regression: depth >= 2 diverged descendant under an unchanged mid node!
        let mut orch = SurfaceOrchestrator::new();
        let root_env = SurfaceEnvironment {
            scale_factor: 1.0,
            ..Default::default()
        };
        let root = orch
            .register_root(FrameworkKind::Nve, "root", root_env.clone())
            .unwrap();
        let mid = orch
            .register_surface(
                SurfaceDescriptor::new(FrameworkKind::Nve, "mid").with_parent(root),
            )
            .unwrap();
        let diverged_leaf = orch
            .register_surface(
                SurfaceDescriptor::new(FrameworkKind::Nve, "leaf")
                    .with_parent(mid)
                    .with_environment(SurfaceEnvironment {
                        scale_factor: 9.0,
                        ..Default::default()
                    }),
            )
            .unwrap();
        let _ = orch.drain_events();

        // Re-propagating `root_env` leaves `root` and `mid` unchanged (0 clones) yet MUST still
        // reconcile `diverged_leaf` back to `scale_factor = 1.0`!
        let updated = orch.update_environment(root, root_env.clone()).unwrap();
        assert_eq!(updated, vec![diverged_leaf]);
        assert_eq!(orch.get(diverged_leaf).unwrap().environment.scale_factor, 1.0);
        let events = orch.drain_events();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            SurfaceEvent::EnvironmentPropagated { surface_id, environment }
                if *surface_id == diverged_leaf && environment.scale_factor == 1.0
        ));

        // Seeded property test covering overrides, sibling pre-order, `p > c` reparented slots,
        // `updated` list, and `EnvironmentPropagated` events.
        let mut rng = Lcg(0xC1_0004_E117);
        let mut prop_orch = SurfaceOrchestrator::new();
        let mut ids = Vec::new();
        let r0 = prop_orch
            .register_root(FrameworkKind::Flutter, "r0", SurfaceEnvironment::default())
            .unwrap();
        ids.push(r0);

        for step in 1..28usize {
            let parent = ids[(rng.next() as usize) % ids.len()];
            let mut desc = SurfaceDescriptor::new(FrameworkKind::Nve, format!("n{step}"))
                .with_parent(parent);
            if rng.next().is_multiple_of(3) {
                desc = desc.with_overrides(EnvironmentOverrides {
                    theme: rng.next().is_multiple_of(2).then(ThemeContext::dark),
                    scale_factor: rng.next().is_multiple_of(2).then_some(2.0),
                    ..Default::default()
                });
            }
            if rng.next().is_multiple_of(4) {
                desc = desc.with_environment(SurfaceEnvironment {
                    scale_factor: 3.5,
                    locale: "ja-JP".into(),
                    ..Default::default()
                });
            }
            ids.push(prop_orch.register_surface(desc).unwrap());
        }

        // Force `p > c` slot ordering by reparenting an early child (`ids[1]`) under a newer
        // non-descendant slot so both arms of `split_at_mut` execute during propagation.
        let low_child = ids[1];
        let high_parent = *ids
            .iter()
            .rev()
            .find(|&&cand| {
                cand != low_child
                    && !prop_orch.ancestor_chain(cand).unwrap().contains(&low_child)
            })
            .unwrap();
        assert!(high_parent.index() > low_child.index());
        prop_orch.reparent(low_child, Some(high_parent)).unwrap();
        let _ = prop_orch.drain_events();

        // Snapshot pre-propagation tree to run an independent recursive reference model.
        fn recursive_model(
            orch: &SurfaceOrchestrator,
            id: SurfaceId,
            mut incoming: SurfaceEnvironment,
            out: &mut Vec<(SurfaceId, SurfaceEnvironment)>,
        ) {
            let rec = orch.get(id).unwrap();
            rec.overrides.apply_to(&mut incoming);
            if rec.environment != incoming {
                out.push((id, incoming.clone()));
            }
            for &child in &rec.children {
                recursive_model(orch, child, incoming.clone(), out);
            }
        }

        for round in 0..8u32 {
            let next_env = SurfaceEnvironment {
                theme: if round.is_multiple_of(2) {
                    ThemeContext::dark()
                } else {
                    ThemeContext::default()
                },
                scale_factor: 1.0 + (round % 3) as f32 * 0.5,
                locale: if round.is_multiple_of(3) { "de-DE".into() } else { "en-US".into() },
                ..Default::default()
            };
            let mut expected = Vec::new();
            recursive_model(&prop_orch, r0, next_env.clone(), &mut expected);

            let updated = prop_orch.update_environment(r0, next_env).unwrap();
            let evs: Vec<(SurfaceId, SurfaceEnvironment)> = prop_orch
                .drain_events()
                .into_iter()
                .map(|ev| match ev {
                    SurfaceEvent::EnvironmentPropagated { surface_id, environment } => {
                        (surface_id, environment)
                    }
                    other => panic!("unexpected event during env propagation: {other:?}"),
                })
                .collect();

            let expected_ids: Vec<SurfaceId> = expected.iter().map(|(id, _)| *id).collect();
            assert_eq!(updated, expected_ids, "round {round} updated order mismatch");
            assert_eq!(evs, expected, "round {round} emitted events mismatch");
            for (sid, env) in &expected {
                assert_eq!(&prop_orch.get(*sid).unwrap().environment, env);
            }
        }
    }
}
