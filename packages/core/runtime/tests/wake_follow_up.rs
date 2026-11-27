//! Wake-driven hosts tick only when woken: every tick hook that is still animating (asked for a
//! refresh or wrote this tick) must leave a follow-up wake, and an idle hook must not.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use nve_components::tables::Visual;
use nve_runtime::{FrameTickHook, RuntimeBuilder};
use nve_store::{NodeId, Rgba, Transaction};

/// Script per tick: `Some(true)` requests a refresh, `Some(false)` writes a store change without
/// requesting, `None` idles.
struct Scripted {
    script: Vec<Option<bool>>,
    tick: Arc<AtomicU32>,
    node: Option<NodeId>,
}

impl FrameTickHook for Scripted {
    fn tick(&mut self, tx: &mut Transaction<'_>, _dt_s: f32) -> bool {
        let i = self.tick.fetch_add(1, Ordering::Relaxed) as usize;
        match self.script.get(i).copied().flatten() {
            Some(true) => true,
            Some(false) => {
                let id = *self.node.get_or_insert_with(|| tx.create());
                tx.edit::<Visual>(id).background(Rgba::rgb(i as u8, 0, 0));
                false
            }
            None => false,
        }
    }
}

#[test]
fn animating_tick_hooks_keep_the_host_awake_and_idle_ones_let_it_sleep() {
    let ticks = Arc::new(AtomicU32::new(0));
    let hook = Scripted {
        script: vec![Some(true), Some(true), Some(false), Some(false), None, None],
        tick: Arc::clone(&ticks),
        node: None,
    };
    let mut rt = RuntimeBuilder::new().tick_hook(Box::new(hook)).build();
    let mut woken = Vec::new();
    for _ in 0..6 {
        rt.tick(0.016).unwrap();
        woken.push(rt.wake_bridge().is_pending());
    }
    assert_eq!(woken, [true, true, true, true, false, false], "request, request, write, write, idle, idle");
    assert_eq!(ticks.load(Ordering::Relaxed), 6);
}
