//! [`CloseLatch`]: reports an overlay's close to `on_open_change(false)` exactly once per open
//! period, whichever path initiated it: the app (trigger, `Escape`, item selection) or the
//! browser (a bridged native `close`: light dismiss, `Esc`, `popovertargetaction=hide`).

use dioxus_core::EventHandler;
use nve_dioxus::prelude::*;

/// Latched open state of an overlay, synchronized with its `open` prop on every render.
#[derive(Clone, Copy)]
pub(crate) struct CloseLatch {
    open: Signal<bool>,
    /// `true` on the render where the overlay transitioned from closed to open.
    pub(crate) opened: bool,
}

/// Hook: latches `open` without subscribing the component (no render loop).
pub(crate) fn use_close_latch(open: bool) -> CloseLatch {
    let latch = use_signal(|| false);
    let was_open = *latch.peek();
    if was_open != open {
        *latch.write_unchecked() = open;
    }
    CloseLatch {
        open: latch,
        opened: open && !was_open,
    }
}

impl CloseLatch {
    /// Reports `on_open_change(false)` unless this open period already reported a close, so a
    /// same-batch app close (e.g. `Escape`) plus the bridged native `close` fires once.
    /// Returns `true` when this call closed the overlay.
    pub(crate) fn close(self, on_open_change: Option<EventHandler<bool>>) -> bool {
        let open = self.open;
        if !*open.peek() {
            return false;
        }
        *open.write_unchecked() = false;
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
        if *self.open.peek() {
            let _ = self.close(on_open_change);
        } else if let Some(handler) = on_open_change {
            handler.call(true);
        }
    }
}
