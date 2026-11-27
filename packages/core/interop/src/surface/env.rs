//! Iterative environment, theme, safe-area, and display-scale cascading for [`SurfaceOrchestrator`].

use nve_abi::SurfaceId;

use super::orchestrator::SurfaceOrchestrator;
use super::types::{
    EnvironmentOverrides, SafeAreaInsets, SurfaceEnvironment, SurfaceEvent, SurfaceRecord,
    ThemeContext,
};
use crate::error::InteropError;

impl SurfaceOrchestrator {
    /// Borrows distinct live `(parent, child)` surface records simultaneously via `split_at_mut`
    /// so comparisons and sealing never clone `parent.environment`.
    #[inline]
    fn parent_child_records_mut(
        &mut self,
        parent_id: SurfaceId,
        child_id: SurfaceId,
    ) -> Option<(&SurfaceRecord, &mut SurfaceRecord)> {
        let p_idx = parent_id.index();
        let c_idx = child_id.index();
        if p_idx == 0
            || c_idx == 0
            || p_idx == c_idx
            || p_idx >= self.slots.len()
            || c_idx >= self.slots.len()
        {
            return None;
        }
        let (parent_slot, child_slot) = if p_idx < c_idx {
            let (head, tail) = self.slots.split_at_mut(c_idx);
            (&head[p_idx], &mut tail[0])
        } else {
            let (head, tail) = self.slots.split_at_mut(p_idx);
            (&tail[0], &mut head[c_idx])
        };
        if parent_slot.generation != parent_id.generation()
            || child_slot.generation != child_id.generation()
        {
            return None;
        }
        Some((
            parent_slot.record.as_deref()?,
            child_slot.record.as_deref_mut()?,
        ))
    }

    /// Updates the environment at `surface_id` and propagates it down the subtree iteratively,
    /// respecting each descendant's [`super::types::EnvironmentOverrides`].
    ///
    /// When `surface_id` is a non-root child, seals `child.overrides` directly to `environment`
    /// against its parent's environment so `child.environment == child.overrides.apply(parent.environment)`
    /// holds across every surface in the tree.
    pub fn update_environment(
        &mut self,
        surface_id: SurfaceId,
        environment: SurfaceEnvironment,
    ) -> Result<Vec<SurfaceId>, InteropError> {
        environment.validate()?;
        let parent_id = self
            .get(surface_id)
            .ok_or(InteropError::SurfaceNotFound(surface_id))?
            .parent;

        if let Some(pid) = parent_id {
            if let Some((parent_rec, child_rec)) = self.parent_child_records_mut(pid, surface_id) {
                child_rec
                    .overrides
                    .seal_against_parent(&environment, &parent_rec.environment);
            }
        } else if let Some(rec) = self.get_mut(surface_id) {
            rec.overrides = EnvironmentOverrides::default();
        }

        let mut updated = Vec::new();
        self.propagate_env_iterative(surface_id, environment, Some(&mut updated));
        Ok(updated)
    }

    /// Iteratively cascades `incoming` down `root_id`'s subtree in pre-order using a reusable
    /// `O(depth)` cursor stack with `O(changed subtree)` pruning (safe because every child seals
    /// `child.environment == child.overrides.apply(parent.environment)` at ingress) and zero
    /// `SurfaceEnvironment` / `String` / `BTreeMap` clones on unchanged nodes (including nodes
    /// with active [`super::types::EnvironmentOverrides`]).
    pub(super) fn propagate_env_iterative(
        &mut self,
        root_id: SurfaceId,
        mut incoming: SurfaceEnvironment,
        mut updated: Option<&mut Vec<SurfaceId>>,
    ) {
        let should_descend = {
            let Some(rec) = self.get_mut(root_id) else {
                return;
            };
            if rec.overrides.matches_resolved(&rec.environment, &incoming) {
                return;
            }
            rec.overrides.apply_to(&mut incoming);
            let hc = !rec.children.is_empty();
            rec.environment = incoming.clone();
            if let Some(out) = updated.as_deref_mut() {
                out.push(root_id);
            }
            self.events.push(SurfaceEvent::EnvironmentPropagated {
                surface_id: root_id,
                environment: incoming,
            });
            hc
        };
        if !should_descend {
            return;
        }

        // Reusable `O(depth)` DFS cursor frames: `(parent_surface_id, next_child_index)`.
        let mut stack = core::mem::take(&mut self.env_stack);
        stack.clear();
        stack.push((root_id, 0));
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

            let Some((parent_rec, child_rec)) =
                self.parent_child_records_mut(parent_id, child_id)
            else {
                continue;
            };

            // Zero-alloc check across both plain and override children; unchanged subtrees prune in O(1).
            if child_rec
                .overrides
                .matches_resolved(&child_rec.environment, &parent_rec.environment)
            {
                continue;
            }

            let child_has_children = !child_rec.children.is_empty();
            let mut next = parent_rec.environment.clone();
            child_rec.overrides.apply_to(&mut next);
            child_rec.environment = next.clone();
            if let Some(out) = updated.as_deref_mut() {
                out.push(child_id);
            }
            self.events.push(SurfaceEvent::EnvironmentPropagated {
                surface_id: child_id,
                environment: next,
            });

            if child_has_children {
                stack.push((child_id, 0));
            }
        }
        self.env_stack = stack;
    }

    fn update_env_field<T>(
        &mut self,
        surface_id: SurfaceId,
        value: T,
        is_same: impl FnOnce(&SurfaceEnvironment, &T) -> bool,
        pin_override: impl FnOnce(&mut EnvironmentOverrides, &T),
        build_next: impl FnOnce(&SurfaceEnvironment, T) -> SurfaceEnvironment,
    ) -> Result<Vec<SurfaceId>, InteropError> {
        let next_env = {
            let rec = self
                .get_mut(surface_id)
                .ok_or(InteropError::SurfaceNotFound(surface_id))?;
            if rec.parent.is_some() {
                pin_override(&mut rec.overrides, &value);
            } else {
                rec.overrides = EnvironmentOverrides::default();
            }
            if is_same(&rec.environment, &value) {
                return Ok(Vec::new());
            }
            build_next(&rec.environment, value)
        };
        let mut updated = Vec::new();
        self.propagate_env_iterative(surface_id, next_env, Some(&mut updated));
        Ok(updated)
    }

    /// Convenience helper to update only the theme at `surface_id` and cascade to children.
    pub fn set_theme(
        &mut self,
        surface_id: SurfaceId,
        theme: ThemeContext,
    ) -> Result<Vec<SurfaceId>, InteropError> {
        self.update_env_field(
            surface_id,
            theme,
            |env, t| env.theme == *t,
            |ov, t| {
                if ov.theme.as_ref() != Some(t) {
                    ov.theme = Some(t.clone());
                }
            },
            |env, theme| SurfaceEnvironment {
                theme,
                safe_area: env.safe_area,
                scale_factor: env.scale_factor,
                text_scale_factor: env.text_scale_factor,
                locale: env.locale.clone(),
                reduced_motion: env.reduced_motion,
            },
        )
    }

    /// Convenience helper to update safe-area insets at `surface_id` and cascade to children.
    pub fn set_safe_area(
        &mut self,
        surface_id: SurfaceId,
        safe_area: SafeAreaInsets,
    ) -> Result<Vec<SurfaceId>, InteropError> {
        safe_area.validate()?;
        self.update_env_field(
            surface_id,
            safe_area,
            |env, sa| env.safe_area == *sa,
            |ov, &sa| ov.safe_area = Some(sa),
            |env, sa| {
                let mut next = env.clone();
                next.safe_area = sa;
                next
            },
        )
    }

    /// Convenience helper to update display scale factor at `surface_id` and cascade to children.
    pub fn set_scale_factor(
        &mut self,
        surface_id: SurfaceId,
        scale_factor: f32,
    ) -> Result<Vec<SurfaceId>, InteropError> {
        if !scale_factor.is_finite() {
            return Err(InteropError::InvalidEnvironment(
                "scale_factor must be finite".into(),
            ));
        }
        self.update_env_field(
            surface_id,
            scale_factor,
            |env, sf| env.scale_factor == *sf,
            |ov, &sf| ov.scale_factor = Some(sf),
            |env, sf| {
                let mut next = env.clone();
                next.scale_factor = sf;
                next
            },
        )
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

    fn assert_tree_env_invariant(orch: &SurfaceOrchestrator, ids: &[SurfaceId]) {
        for &sid in ids {
            let rec = orch.get(sid).unwrap();
            if let Some(pid) = rec.parent {
                let p_rec = orch.get(pid).unwrap();
                let mut expected = p_rec.environment.clone();
                rec.overrides.apply_to(&mut expected);
                assert_eq!(
                    rec.environment, expected,
                    "child {sid:?} violated `child.env == overrides.apply(parent.env)`"
                );
                assert!(
                    rec.overrides.matches_resolved(&rec.environment, &p_rec.environment),
                    "matches_resolved disagreed with apply_to for child {sid:?}"
                );
            } else {
                assert_eq!(
                    rec.overrides,
                    EnvironmentOverrides::default(),
                    "root {sid:?} must keep empty overrides so direct updates never freeze"
                );
            }
        }
    }

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

    #[test]
    fn propagate_env_iterative_seals_invariant_prunes_unchanged_and_matches_model_multi_seed() {
        // 1. Sealed child-ingress + unchanged-root pruning + zero-alloc override match.
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
        let pinned_leaf = orch
            .register_surface(
                SurfaceDescriptor::new(FrameworkKind::Nve, "leaf")
                    .with_parent(mid)
                    .with_environment(SurfaceEnvironment {
                        scale_factor: 9.0,
                        ..Default::default()
                    }),
            )
            .unwrap();
        assert_tree_env_invariant(&orch, &[root, mid, pinned_leaf]);
        let _ = orch.drain_events();

        // Unchanged root update must prune in O(1) with 0 updates and 0 events!
        let updated = orch.update_environment(root, root_env.clone()).unwrap();
        assert!(updated.is_empty());
        assert!(orch.drain_events().is_empty());
        assert_eq!(orch.get(pinned_leaf).unwrap().environment.scale_factor, 9.0);

        // Changing root theme propagates through `mid` to `pinned_leaf` while preserving its 9.0 scale override.
        let upd_theme = orch.set_theme(root, ThemeContext::dark()).unwrap();
        assert_eq!(upd_theme, vec![root, mid, pinned_leaf]);
        assert_eq!(orch.get(pinned_leaf).unwrap().environment.theme, ThemeContext::dark());
        assert_eq!(orch.get(pinned_leaf).unwrap().environment.scale_factor, 9.0);
        assert_tree_env_invariant(&orch, &[root, mid, pinned_leaf]);

        // 2. Multi-seed property test across overrides, explicit child envs, reparent (`p > c`),
        //    unchanged-root rounds, and child `set_theme`/`set_scale_factor`/`update_environment`.
        for seed in [0xC1_0004_E117u64, 0xA11C_E551_0002u64, 0xBEEF_CAFE_0003u64] {
            let mut rng = Lcg(seed);
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
                        reduced_motion: rng.next().is_multiple_of(3).then_some(true),
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
            assert_tree_env_invariant(&prop_orch, &ids);

            // Force `p > c` slot ordering so both arms of `split_at_mut` execute.
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
            assert_tree_env_invariant(&prop_orch, &ids);
            let _ = prop_orch.drain_events();

            for round in 0..10u32 {
                // Every 3rd round repeats the exact current root env (unchanged-root O(1) prune!).
                let next_env = if round.is_multiple_of(3) {
                    prop_orch.get(r0).unwrap().environment.clone()
                } else {
                    SurfaceEnvironment {
                        theme: if round.is_multiple_of(2) {
                            ThemeContext::dark()
                        } else {
                            ThemeContext::default()
                        },
                        scale_factor: 1.0 + (round % 3) as f32 * 0.5,
                        text_scale_factor: 1.0 + (round % 2) as f32 * 0.25,
                        locale: if round.is_multiple_of(4) {
                            "de-DE".into()
                        } else {
                            "en-US".into()
                        },
                        reduced_motion: round.is_multiple_of(5),
                        ..Default::default()
                    }
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
                assert_eq!(updated, expected_ids, "seed {seed:#x} round {round} updated mismatch");
                assert_eq!(evs, expected, "seed {seed:#x} round {round} events mismatch");
                assert_tree_env_invariant(&prop_orch, &ids);

                // Exercise child-level mutations (`set_theme` / `set_scale_factor` / `update_environment`)
                // and verify the sealed invariant still holds across the entire tree.
                let target = ids[1 + ((rng.next() as usize) % (ids.len() - 1))];
                match round % 3 {
                    0 => {
                        let _ = prop_orch.set_theme(target, ThemeContext::dark()).unwrap();
                        assert_eq!(
                            prop_orch.get(target).unwrap().environment.theme,
                            ThemeContext::dark()
                        );
                    }
                    1 => {
                        let _ = prop_orch.set_scale_factor(target, 2.5).unwrap();
                        assert_eq!(prop_orch.get(target).unwrap().environment.scale_factor, 2.5);
                    }
                    _ => {
                        let req = SurfaceEnvironment {
                            locale: "fr-FR".into(),
                            scale_factor: 1.75,
                            ..Default::default()
                        };
                        let _ = prop_orch.update_environment(target, req.clone()).unwrap();
                        assert_eq!(
                            prop_orch.get(target).unwrap().environment,
                            req,
                            "direct update_environment on overridden child must apply requested env"
                        );
                    }
                }
                let _ = prop_orch.drain_events();
                assert_tree_env_invariant(&prop_orch, &ids);
            }

            // Promote an overridden child to root and verify direct root updates never freeze!
            let promoted = ids[1];
            prop_orch.set_scale_factor(promoted, 3.0).unwrap();
            prop_orch.reparent(promoted, None).unwrap();
            assert_tree_env_invariant(&prop_orch, &ids);
            let _ = prop_orch.drain_events();
            let promoted_upd = prop_orch.set_scale_factor(promoted, 1.25).unwrap();
            assert!(promoted_upd.contains(&promoted));
            assert_eq!(prop_orch.get(promoted).unwrap().environment.scale_factor, 1.25);
            let new_root_env = SurfaceEnvironment {
                theme: ThemeContext::dark(),
                scale_factor: 2.25,
                locale: "es-ES".into(),
                ..Default::default()
            };
            let _ = prop_orch.update_environment(promoted, new_root_env.clone()).unwrap();
            assert_eq!(prop_orch.get(promoted).unwrap().environment, new_root_env);
            assert_tree_env_invariant(&prop_orch, &ids);
        }
    }

    #[test]
    fn env_validation_root_overrides_and_live_destroyed_transition_guards() {
        use crate::surface::LifecycleState;

        let mut orch = SurfaceOrchestrator::new();

        // Registering a root with `.with_overrides(...)` applies initial values without freezing future updates.
        let root = orch
            .register_surface(
                SurfaceDescriptor::new(FrameworkKind::Nve, "root_ov").with_overrides(
                    EnvironmentOverrides {
                        scale_factor: Some(2.0),
                        theme: Some(ThemeContext::dark()),
                        ..Default::default()
                    },
                ),
            )
            .unwrap();
        assert_eq!(orch.get(root).unwrap().environment.scale_factor, 2.0);
        assert_eq!(orch.get(root).unwrap().overrides, EnvironmentOverrides::default());
        let upd = orch.set_scale_factor(root, 1.5).unwrap();
        assert_eq!(upd, vec![root]);
        assert_eq!(orch.get(root).unwrap().environment.scale_factor, 1.5);

        // Non-finite scale / text_scale / safe_area rejected at every ingress.
        assert!(matches!(
            orch.set_scale_factor(root, f32::NAN),
            Err(InteropError::InvalidEnvironment(_))
        ));
        assert!(matches!(
            orch.set_scale_factor(root, f32::INFINITY),
            Err(InteropError::InvalidEnvironment(_))
        ));
        assert!(matches!(
            orch.set_safe_area(root, SafeAreaInsets::new(f32::NAN, 0.0, 0.0, 0.0)),
            Err(InteropError::InvalidEnvironment(_))
        ));
        assert!(matches!(
            orch.update_environment(
                root,
                SurfaceEnvironment {
                    text_scale_factor: f32::NEG_INFINITY,
                    ..Default::default()
                }
            ),
            Err(InteropError::InvalidEnvironment(_))
        ));
        assert!(matches!(
            orch.register_surface(
                SurfaceDescriptor::new(FrameworkKind::Nve, "bad").with_overrides(
                    EnvironmentOverrides {
                        scale_factor: Some(f32::NAN),
                        ..Default::default()
                    }
                )
            ),
            Err(InteropError::InvalidEnvironment(_))
        ));

        // Direct update_environment on an overridden child must replace stale pinned overrides.
        let child = orch
            .register_surface(
                SurfaceDescriptor::new(FrameworkKind::Nve, "child")
                    .with_parent(root)
                    .with_overrides(EnvironmentOverrides {
                        theme: Some(ThemeContext::dark()),
                        scale_factor: Some(3.0),
                        ..Default::default()
                    }),
            )
            .unwrap();
        let replacement = SurfaceEnvironment {
            theme: ThemeContext::default(),
            scale_factor: 1.25,
            locale: "de-DE".into(),
            ..Default::default()
        };
        let changed = orch.update_environment(child, replacement.clone()).unwrap();
        assert_eq!(changed, vec![child]);
        assert_eq!(orch.get(child).unwrap().environment, replacement);

        // Live-surface transition_state and propagate_lifecycle MUST reject LifecycleState::Destroyed.
        assert_eq!(
            orch.transition_state(root, LifecycleState::Destroyed),
            Err(InteropError::InvalidLifecycleTransition {
                surface: root,
                from: LifecycleState::Mounted,
                to: LifecycleState::Destroyed,
            })
        );
        assert_eq!(
            orch.propagate_lifecycle(root, LifecycleState::Destroyed),
            Err(InteropError::InvalidLifecycleTransition {
                surface: root,
                from: LifecycleState::Mounted,
                to: LifecycleState::Destroyed,
            })
        );
        assert_eq!(orch.active_count(), 2);
        assert_eq!(orch.get(root).unwrap().state, LifecycleState::Mounted);
        assert_eq!(orch.get(child).unwrap().state, LifecycleState::Mounted);
    }
}
