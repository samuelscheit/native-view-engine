//! Native top-layer runtime for [`HtmlSemantics`](nve_web_html::HtmlSemantics) overlays:
//! `<dialog>` `show()` / `showModal()` / `close()` (modality from `Accessibility::modal`), the
//! `closedby="any"` backdrop-click fallback for browsers without native `closedBy`,
//! `[popover]` `showPopover()` / `hidePopover()` driven by `data_nve_open`, and the native
//! close bridges (`<dialog>` `close`, `[popover]` `toggle` to `closed`) reported to the app as
//! `Custom { name: "close" }` events.

/// A dialog's border box in client (viewport) coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DialogRect {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

impl DialogRect {
    #[inline]
    pub fn contains(&self, x: f64, y: f64) -> bool {
        (self.left..=self.right).contains(&x) && (self.top..=self.bottom).contains(&y)
    }
}

/// Pure decision for the `<dialog closedby="any">` backdrop-click fallback: closes only when the
/// browser lacks native `closedBy`, the click targeted the dialog element itself (its `::backdrop`
/// hit-tests as the dialog), and the point lies outside the dialog box.
#[inline]
pub fn should_close_dialog_on_backdrop_click(
    closedby: Option<&str>,
    supports_native_closedby: bool,
    target_is_dialog: bool,
    (x, y): (f64, f64),
    rect: DialogRect,
) -> bool {
    !supports_native_closedby
        && target_is_dialog
        && closedby.is_some_and(|v| v.trim().eq_ignore_ascii_case("any"))
        && !rect.contains(x, y)
}

#[cfg(target_arch = "wasm32")]
pub use live::backdrop_fallback_close_count;
#[cfg(target_arch = "wasm32")]
pub(crate) use live::{CLOSE_BRIDGES, sync_overlay_state, try_dialog_backdrop_fallback};

#[cfg(target_arch = "wasm32")]
mod live {
    use std::sync::atomic::{AtomicU32, Ordering};

    use nve_backend_web::NativeEventBridge;
    use nve_tables::tables::{Accessibility, Overlay, OverlayFlags, OverlayLayer};
    use nve_store::{NodeId, NodeStore};
    use nve_web_html::HtmlSemantics;
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::wasm_bindgen;

    use super::{DialogRect, should_close_dialog_on_backdrop_click};

    #[wasm_bindgen(inline_js = "
export function nve_supports_dialog_closedby() {
  return typeof HTMLDialogElement !== 'undefined' && 'closedBy' in HTMLDialogElement.prototype;
}
export function nve_sync_popover_state(el, open) {
  if (!el || !el.isConnected) return;
  try {
    if (open && typeof el.showPopover === 'function' && !el.matches(':popover-open')) el.showPopover();
    else if (!open && typeof el.hidePopover === 'function' && el.matches(':popover-open')) el.hidePopover();
  } catch (_) {}
}
export function nve_toggle_event_closed(ev) {
  return ev.newState === 'closed';
}
")]
    extern "C" {
        fn nve_supports_dialog_closedby() -> bool;
        fn nve_sync_popover_state(el: &web_sys::Element, open: bool);
        fn nve_toggle_event_closed(ev: &web_sys::Event) -> bool;
    }

    /// Name of the `Custom` event a user/browser-initiated overlay close reports.
    const CLOSE_EVENT: &str = "close";

    /// Capture phase, delivered target-only: neither `close` nor `toggle` bubbles, so an
    /// `onclose`-less nested overlay never closes the enclosing `Dialog` / `Popover`.
    pub(crate) static CLOSE_BRIDGES: [NativeEventBridge; 2] = [
        NativeEventBridge {
            event: "close",
            capture: true,
            bubbles: false,
            map: dialog_closed,
        },
        NativeEventBridge {
            event: "toggle",
            capture: true,
            bubbles: false,
            map: popover_closed,
        },
    ];

    /// `close` is queued as a task, so a programmatic `close()` immediately followed by
    /// `show()` / `showModal()` (modality changes) finds the dialog open again: not a close.
    fn dialog_closed(_ev: &web_sys::Event, el: &web_sys::Element) -> Option<&'static str> {
        el.dyn_ref::<web_sys::HtmlDialogElement>()
            .filter(|d| !d.open())
            .map(|_| CLOSE_EVENT)
    }

    /// `[popover]` `toggle` to `closed` (light dismiss, `Esc`, `popovertargetaction=hide`); a
    /// `<details>` `toggle` or a popover reopened before the task ran is ignored.
    fn popover_closed(ev: &web_sys::Event, el: &web_sys::Element) -> Option<&'static str> {
        (el.has_attribute("popover")
            && nve_toggle_event_closed(ev)
            && !el.matches(":popover-open").unwrap_or(false))
        .then_some(CLOSE_EVENT)
    }

    /// Applies `node`'s `HtmlSemantics` + `Overlay` open/modal state to its live `<dialog>` /
    /// `[popover]` element.
    pub(crate) fn sync_overlay_state(store: &NodeStore, node: NodeId, el: &web_sys::Element) {
        let Some(row) = store.try_row::<HtmlSemantics>(node) else {
            return;
        };
        let overlay = store.try_row::<Overlay>(node);
        let overlay_open = overlay
            .as_ref()
            .map(|ov| ov.options().contains(OverlayFlags::OPEN));
        if let Some(dialog) = el.dyn_ref::<web_sys::HtmlDialogElement>() {
            let modal = store
                .try_row::<Accessibility>(node)
                .is_some_and(|a| a.modal())
                || overlay
                    .as_ref()
                    .is_some_and(|ov| ov.layer() == OverlayLayer::Modal);
            sync_dialog_open_state(dialog, overlay_open.unwrap_or(row.open()), modal);
        } else if row.popover().is_some() {
            nve_sync_popover_state(el, overlay_open.unwrap_or(row.data_nve_open()));
        }
    }

    /// `showModal()` / `show()` / `close()`. A disconnected dialog is left untouched (an `open`
    /// attribute would show it non-modally once inserted; `showModal()` throws): the commit
    /// re-syncs it in Step 6 once it is connected, moved, or retagged.
    fn sync_dialog_open_state(dialog: &web_sys::HtmlDialogElement, open: bool, modal: bool) {
        if !dialog.is_connected() {
            return;
        }
        if !open {
            if dialog.open() {
                dialog.close();
            }
            return;
        }
        let is_modal = dialog.matches(":modal").unwrap_or(false);
        if modal && !is_modal {
            // A non-modal (or SSR-adopted `<dialog open>`) dialog reopens in the top layer.
            if dialog.open() {
                dialog.close();
            }
            let _ = dialog.show_modal();
        } else if !modal && is_modal {
            dialog.close();
            dialog.show();
        } else if !dialog.open() {
            dialog.show();
        }
    }

    static BACKDROP_FALLBACK_CLOSES: AtomicU32 = AtomicU32::new(0);

    /// `<dialog>`s closed by the `closedby="any"` backdrop fallback so far: lets live probes
    /// prove the fallback (not the browser's native light dismiss) closed a dialog.
    pub fn backdrop_fallback_close_count() -> u32 {
        BACKDROP_FALLBACK_CLOSES.load(Ordering::Relaxed)
    }

    /// Root-delegator click hook: closes an open `<dialog closedby="any">` on a backdrop hit when
    /// the browser lacks native `closedBy`. Returns `true` when it consumed the click.
    pub(crate) fn try_dialog_backdrop_fallback(ev: &web_sys::MouseEvent) -> bool {
        let Some(dialog) = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlDialogElement>().ok())
            .filter(web_sys::HtmlDialogElement::open)
        else {
            return false;
        };
        // Cheap gates first: no layout flush unless the fallback can actually apply.
        let closedby = dialog.get_attribute("closedby");
        if closedby.is_none() || nve_supports_dialog_closedby() {
            return false;
        }
        let r = dialog.get_bounding_client_rect();
        let close = should_close_dialog_on_backdrop_click(
            closedby.as_deref(),
            false,
            true,
            (f64::from(ev.client_x()), f64::from(ev.client_y())),
            DialogRect {
                left: r.left(),
                top: r.top(),
                right: r.right(),
                bottom: r.bottom(),
            },
        );
        if close {
            dialog.close();
            BACKDROP_FALLBACK_CLOSES.fetch_add(1, Ordering::Relaxed);
        }
        close
    }
}

#[cfg(test)]
mod tests {
    use super::{DialogRect, should_close_dialog_on_backdrop_click as close};

    const RECT: DialogRect = DialogRect {
        left: 100.0,
        top: 100.0,
        right: 400.0,
        bottom: 300.0,
    };

    #[test]
    fn dialog_backdrop_click_fallback_respects_closedby_and_native_support() {
        // Native `closedBy` present -> the fallback stays passive.
        assert!(!close(Some("any"), true, true, (5.0, 5.0), RECT));
        // Outside the box on the dialog target with closedby="any" (any case/whitespace) -> close.
        assert!(close(Some("any"), false, true, (12.0, 150.0), RECT));
        assert!(close(Some(" ANY "), false, true, (200.0, 350.0), RECT));
        // Inside the box, including its exact edges -> keep open.
        assert!(!close(Some("any"), false, true, (200.0, 200.0), RECT));
        assert!(!close(Some("any"), false, true, (100.0, 300.0), RECT));
        // A click on a child element (target is not the dialog) never closes.
        assert!(!close(Some("any"), false, false, (12.0, 150.0), RECT));
        // closedby="closerequest" / "none" / absent never close on a backdrop click.
        for closedby in [Some("closerequest"), Some("none"), None] {
            assert!(
                !close(closedby, false, true, (12.0, 150.0), RECT),
                "{closedby:?}"
            );
        }
    }
}
