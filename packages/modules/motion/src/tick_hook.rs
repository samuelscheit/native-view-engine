//! Opt-in [`nve_engine::FrameTickHook`] adapter ([`MotionRuntimeTickHook`]) advancing a
//! [`MotionClock`] inside `nve_runtime::SurfaceRuntime::tick` with zero idle allocations.

use std::sync::{Arc, Mutex};

use nve_engine::FrameTickHook;
use nve_store::{Length, Transaction};
use nve_tables::tables::{Layout, Visual};

use crate::clock::{MotionClock, MotionKey};
use crate::table::TransitionProperty;

/// Opt-in [`FrameTickHook`] adapter driving a shared [`MotionClock`] directly into a
/// `SurfaceRuntime`'s [`Transaction`] on each tick.
///
/// Hosts register this hook via `RuntimeBuilder::tick_hook` or `SurfaceRuntime::add_tick_hook`
/// while retaining a cloned `Arc<Mutex<MotionClock>>` (from [`Self::clock`]) to start, retarget,
/// or cancel tracks. When no tracks are active, [`FrameTickHook::tick`] returns `false` with zero
/// store writes and zero allocations so wake-driven hosts sleep at 0% idle CPU.
pub struct MotionRuntimeTickHook {
    clock: Arc<Mutex<MotionClock>>,
}

impl Default for MotionRuntimeTickHook {
    fn default() -> Self {
        Self::new()
    }
}

impl MotionRuntimeTickHook {
    /// Creates a new hook backed by a fresh [`MotionClock`].
    pub fn new() -> Self {
        Self::from_shared(Arc::new(Mutex::new(MotionClock::new())))
    }

    /// Creates a hook wrapping an existing shared [`MotionClock`].
    pub fn from_shared(clock: Arc<Mutex<MotionClock>>) -> Self {
        // Eagerly prime OS mutex storage so steady-state idle ticks allocate 0 bytes.
        drop(clock.lock());
        Self { clock }
    }

    /// Returns a clone of the shared [`MotionClock`] handle so app code can drive tracks.
    #[inline]
    pub fn clock(&self) -> Arc<Mutex<MotionClock>> {
        Arc::clone(&self.clock)
    }
}

#[inline]
fn is_driven_property(prop: TransitionProperty) -> bool {
    matches!(
        prop,
        TransitionProperty::Opacity
            | TransitionProperty::BorderRadius
            | TransitionProperty::Width
            | TransitionProperty::Height
            | TransitionProperty::Padding
            | TransitionProperty::Gap
    )
}

#[inline]
fn length_eq(a: Length, b: Length) -> bool {
    a.is_points() == b.is_points()
        && a.is_percent() == b.is_percent()
        && a.value().to_bits() == b.value().to_bits()
}

impl FrameTickHook for MotionRuntimeTickHook {
    fn tick(&mut self, tx: &mut Transaction<'_>, dt_s: f32) -> bool {
        let mut clock = self.clock.lock().unwrap_or_else(|e| e.into_inner());
        if !clock.has_active_animations() {
            return false;
        }
        let mut any_live_driven_active = false;
        clock.step_and_prune_active_with(
            dt_s,
            tx,
            |tx, key| match key {
                MotionKey::Node(node_id, _) => tx.store().is_live(node_id),
                MotionKey::Named(_) => true,
            },
            |tx, key, val, settled| {
                let MotionKey::Node(node_id, prop) = key else {
                    return;
                };
                if !is_driven_property(prop) || !val.is_finite() {
                    return;
                }
                if !settled {
                    any_live_driven_active = true;
                }
                match prop {
                    TransitionProperty::Opacity => {
                        let next = val.clamp(0.0, 1.0);
                        if tx
                            .store()
                            .try_row::<Visual>(node_id)
                            .is_none_or(|v| v.opacity().to_bits() != next.to_bits())
                        {
                            tx.set(node_id, Visual::OPACITY, next);
                        }
                    }
                    TransitionProperty::BorderRadius => {
                        let next = val.max(0.0);
                        if tx
                            .store()
                            .try_row::<Visual>(node_id)
                            .is_none_or(|v| v.border_radius().to_bits() != next.to_bits())
                        {
                            tx.set(node_id, Visual::BORDER_RADIUS, next);
                        }
                    }
                    TransitionProperty::Width => {
                        let next = Length::points(val.max(0.0));
                        if tx
                            .store()
                            .try_row::<Layout>(node_id)
                            .is_none_or(|l| !length_eq(l.width(), next))
                        {
                            tx.set(node_id, Layout::WIDTH, next);
                        }
                    }
                    TransitionProperty::Height => {
                        let next = Length::points(val.max(0.0));
                        if tx
                            .store()
                            .try_row::<Layout>(node_id)
                            .is_none_or(|l| !length_eq(l.height(), next))
                        {
                            tx.set(node_id, Layout::HEIGHT, next);
                        }
                    }
                    TransitionProperty::Padding => {
                        let p = Length::points(val.max(0.0));
                        let same = tx.store().try_row::<Layout>(node_id).is_some_and(|l| {
                            length_eq(l.padding_top(), p)
                                && length_eq(l.padding_right(), p)
                                && length_eq(l.padding_bottom(), p)
                                && length_eq(l.padding_left(), p)
                        });
                        if !same {
                            tx.set(node_id, Layout::PADDING_TOP, p);
                            tx.set(node_id, Layout::PADDING_RIGHT, p);
                            tx.set(node_id, Layout::PADDING_BOTTOM, p);
                            tx.set(node_id, Layout::PADDING_LEFT, p);
                        }
                    }
                    TransitionProperty::Gap => {
                        let g = Length::points(val.max(0.0));
                        let same = tx.store().try_row::<Layout>(node_id).is_some_and(|l| {
                            length_eq(l.row_gap(), g) && length_eq(l.column_gap(), g)
                        });
                        if !same {
                            tx.set(node_id, Layout::ROW_GAP, g);
                            tx.set(node_id, Layout::COLUMN_GAP, g);
                        }
                    }
                    TransitionProperty::All
                    | TransitionProperty::BackgroundColor
                    | TransitionProperty::Color
                    | TransitionProperty::BorderColor
                    | TransitionProperty::Transform => {}
                }
            },
        );
        any_live_driven_active
    }
}

#[cfg(test)]
mod tests {
    use nve_store::{ChangeSet, NodeId, NodeStore};
    use nve_tables::tables::{register_core_tables, Layout, Visual};

    use super::*;
    use crate::SpringSpec;

    #[test]
    fn motion_runtime_tick_hook_drives_node_store_and_sleeps_when_settled() {
        register_core_tables();
        let mut store = NodeStore::new();
        let node = {
            let mut tx = store.tx();
            let child = tx.create();
            tx.set_children(NodeId::ROOT, &[child]);
            tx.set(child, Visual::OPACITY, 0.0);
            tx.set(child, Layout::WIDTH, nve_store::Length::points(10.0));
            let mut ch = ChangeSet::default();
            tx.commit_into(&mut ch);
            child
        };

        let mut hook = MotionRuntimeTickHook::new();
        let clock = hook.clock();

        // Idle hook writes nothing and returns false.
        let mut ch = ChangeSet::default();
        {
            let mut tx = store.tx();
            assert!(!hook.tick(&mut tx, 1.0 / 60.0));
            tx.commit_into(&mut ch);
        }
        assert!(ch.is_empty());

        // Start a spring on opacity and width.
        {
            let mut c = clock.lock().expect("clock lock");
            c.animate_spring((node, TransitionProperty::Opacity), SpringSpec::snappy(), 0.0, 1.0);
            c.animate_spring((node, TransitionProperty::Width), SpringSpec::snappy(), 10.0, 200.0);
        }

        // First step moves values and requests follow-up frame.
        {
            let mut tx = store.tx();
            assert!(hook.tick(&mut tx, 1.0 / 60.0));
            tx.commit_into(&mut ch);
        }
        assert!(!ch.is_empty());
        let mid_opacity = store.row::<Visual>(node).opacity();
        assert!(mid_opacity > 0.0 && mid_opacity < 1.0, "opacity={mid_opacity}");

        // Step until settled.
        let mut settled = false;
        for _ in 0..300 {
            let mut tx = store.tx();
            let more = hook.tick(&mut tx, 1.0 / 60.0);
            tx.commit_into(&mut ch);
            if !more {
                settled = true;
                break;
            }
        }
        assert!(settled, "spring must settle");
        assert!((store.row::<Visual>(node).opacity() - 1.0).abs() < 1e-3);
        assert!((store.row::<Layout>(node).width().value() - 200.0).abs() < 1e-1);

        // Subsequent idle tick does zero writes and returns false.
        {
            let mut tx = store.tx();
            assert!(!hook.tick(&mut tx, 1.0 / 60.0));
            tx.commit_into(&mut ch);
        }
        assert!(ch.is_empty());
    }

    #[test]
    fn motion_runtime_tick_hook_prunes_dead_nodes_ignores_unsupported_and_avoids_zero_dt_churn() {
        register_core_tables();
        let mut store = NodeStore::new();
        let (live_node, dead_node) = {
            let mut tx = store.tx();
            let a = tx.create();
            let b = tx.create();
            tx.set_children(NodeId::ROOT, &[a, b]);
            tx.set(a, Visual::OPACITY, 0.25);
            tx.set(b, Visual::OPACITY, 0.5);
            let mut ch = ChangeSet::default();
            tx.commit_into(&mut ch);
            (a, b)
        };

        let mut hook = MotionRuntimeTickHook::new();
        let clock = hook.clock();

        // 1. Named & unsupported TransitionProperty tracks must NOT spin (`tick` returns `false`,
        //    zero store writes).
        {
            let mut c = clock.lock().expect("clock lock");
            c.animate_spring(MotionKey::named("custom_ui_knob"), SpringSpec::snappy(), 0.0, 1.0);
            c.animate_spring(
                (live_node, TransitionProperty::BackgroundColor),
                SpringSpec::snappy(),
                0.0,
                1.0,
            );
            c.animate_spring(
                (live_node, TransitionProperty::Transform),
                SpringSpec::snappy(),
                0.0,
                100.0,
            );
        }
        let mut ch = ChangeSet::default();
        {
            let mut tx = store.tx();
            assert!(
                !hook.tick(&mut tx, 1.0 / 60.0),
                "named & unsupported properties must not force follow-up frames"
            );
            tx.commit_into(&mut ch);
        }
        assert!(ch.is_empty());

        // 2. Dead-node track is pruned immediately when `!store.is_live(dead_node)`.
        {
            let mut c = clock.lock().expect("clock lock");
            c.clear();
            c.animate_spring((dead_node, TransitionProperty::Opacity), SpringSpec::snappy(), 0.5, 1.0);
        }
        {
            let mut tx = store.tx();
            tx.set_children(NodeId::ROOT, &[live_node]);
            tx.remove(dead_node);
            tx.commit_into(&mut ch);
        }
        assert!(!store.is_live(dead_node));
        {
            let mut tx = store.tx();
            assert!(!hook.tick(&mut tx, 1.0 / 60.0), "dead node track must be pruned");
            tx.commit_into(&mut ch);
        }
        assert!(ch.is_empty());
        assert!(
            !clock.lock().expect("clock lock").has_active_animations(),
            "dead node track must be removed from MotionClock"
        );

        // 3. `dt_s == 0.0` when initial value already equals current column -> zero dirty writes!
        {
            let mut c = clock.lock().expect("clock lock");
            c.animate_spring((live_node, TransitionProperty::Opacity), SpringSpec::snappy(), 0.25, 1.0);
        }
        {
            let mut tx = store.tx();
            assert!(
                hook.tick(&mut tx, 0.0),
                "unsettled live track still requests next frame"
            );
            tx.commit_into(&mut ch);
        }
        assert!(
            ch.is_empty(),
            "dt_s == 0.0 with unchanged column value must not dirty NodeStore"
        );
    }
}
