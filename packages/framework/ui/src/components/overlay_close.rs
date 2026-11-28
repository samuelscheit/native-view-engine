//! [`CloseLatch`]: reports an overlay's close to `on_open_change(false)` exactly once per open
//! period, whichever path initiated it: the app (trigger, `Escape`, item selection) or the
//! browser (a bridged native `close`: light dismiss, `Esc`, `popovertargetaction=hide`).

use dioxus_core::{use_hook, EventHandler};
use dioxus_signals::CopyValue;
use nve_dioxus::prelude::*;

/// Latched open state of an overlay, synchronized with its `open` prop on every render.
///
/// Prop transitions (`was_open` -> `opened` / `closed`) are tracked separately from same-batch
/// close deduplication (`close_pending`) so a controlled parent that vetoes `on_open_change(false)`
/// (or omits `on_open_change`) self-heals on the next render without permanently wedging the latch
/// or spuriously firing `opened`.
#[derive(Clone, Copy)]
pub(crate) struct CloseLatch {
    was_open: CopyValue<bool>,
    close_pending: CopyValue<bool>,
    reconcile_tick: Signal<u32>,
    /// `true` on the render where the `open` prop transitioned from `false` to `true`.
    pub(crate) opened: bool,
    /// `true` on the render where the `open` prop transitioned from `true` to `false`.
    pub(crate) closed: bool,
}

/// Hook: tracks prop transitions (`opened` / `closed`) and deduplicates same-batch close requests.
///
/// Subscribes only to `reconcile_tick` (zero extra renders when the parent accepts and flips
/// `open` to `false`; one self-healing re-render clearing `close_pending` when the parent vetoes
/// `open == true` or omits `on_open_change`).
pub(crate) fn use_close_latch(open: bool) -> CloseLatch {
    let mut was_open = use_hook(|| CopyValue::new(false));
    let mut close_pending = use_hook(|| CopyValue::new(false));
    let reconcile_tick = use_signal(|| 0u32);
    let _ = reconcile_tick();

    let prev_open = *was_open.peek();
    if prev_open != open {
        was_open.set(open);
    }
    if *close_pending.peek() {
        close_pending.set(false);
    }
    CloseLatch {
        was_open,
        close_pending,
        reconcile_tick,
        opened: open && !prev_open,
        closed: !open && prev_open,
    }
}

impl CloseLatch {
    /// `true` while the overlay is open and has not already emitted a close in the current batch.
    #[inline]
    pub(crate) fn is_open(self) -> bool {
        *self.was_open.peek() && !*self.close_pending.peek()
    }

    /// Reports `on_open_change(false)` unless this open period already reported a close in the
    /// current batch, so a same-batch app close (e.g. `Escape`) plus the bridged native `close`
    /// fires once. Returns `true` when this call closed the overlay.
    pub(crate) fn close(self, on_open_change: Option<EventHandler<bool>>) -> bool {
        if !self.is_open() {
            return false;
        }
        let mut close_pending = self.close_pending;
        let mut reconcile_tick = self.reconcile_tick;
        close_pending.set(true);
        let next = reconcile_tick.peek().wrapping_add(1);
        reconcile_tick.set(next);
        if let Some(handler) = on_open_change {
            handler.call(false);
        }
        true
    }

    /// `Escape` closes the innermost open overlay only: a consumed key stops propagation so an
    /// enclosing `Dialog`'s focus trap does not dismiss too.
    pub(crate) fn close_on_escape(self, ev: &KeyEvent, on_open_change: Option<EventHandler<bool>>) {
        if matches!(ev.data().key.as_str(), "Escape" | "Esc") && self.close(on_open_change) {
            ev.stop_propagation();
        }
    }

    /// Trigger toggle: closes through the latch, or requests opening.
    pub(crate) fn toggle(self, on_open_change: Option<EventHandler<bool>>) {
        if self.is_open() {
            let _ = self.close(on_open_change);
        } else if let Some(handler) = on_open_change {
            handler.call(true);
        }
    }
}
