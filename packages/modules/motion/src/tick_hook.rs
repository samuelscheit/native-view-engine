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

impl FrameTickHook for MotionRuntimeTickHook {
    fn tick(&mut self, tx: &mut Transaction<'_>, dt_s: f32) -> bool {
        let mut clock = self.clock.lock().unwrap_or_else(|e| e.into_inner());
        if !clock.has_active_animations() {
            return false;
        }
        clock.step_active_with(dt_s, |key, val, _settled| {
            let MotionKey::Node(node_id, prop) = key else {
                return;
            };
            if !tx.store().is_live(node_id) || !val.is_finite() {
                return;
            }
            match prop {
                TransitionProperty::Opacity => {
                    tx.set(node_id, Visual::OPACITY, val.clamp(0.0, 1.0));
                }
                TransitionProperty::BorderRadius => {
                    tx.set(node_id, Visual::BORDER_RADIUS, val.max(0.0));
                }
                TransitionProperty::Width => {
                    tx.set(node_id, Layout::WIDTH, Length::points(val.max(0.0)));
                }
                TransitionProperty::Height => {
                    tx.set(node_id, Layout::HEIGHT, Length::points(val.max(0.0)));
                }
                TransitionProperty::Padding => {
                    let p = Length::points(val.max(0.0));
                    tx.set(node_id, Layout::PADDING_TOP, p);
                    tx.set(node_id, Layout::PADDING_RIGHT, p);
                    tx.set(node_id, Layout::PADDING_BOTTOM, p);
                    tx.set(node_id, Layout::PADDING_LEFT, p);
                }
                TransitionProperty::Gap => {
                    let g = Length::points(val.max(0.0));
                    tx.set(node_id, Layout::ROW_GAP, g);
                    tx.set(node_id, Layout::COLUMN_GAP, g);
                }
                TransitionProperty::All
                | TransitionProperty::BackgroundColor
                | TransitionProperty::Color
                | TransitionProperty::BorderColor
                | TransitionProperty::Transform => {}
            }
        });
        clock.has_active_animations()
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
}
