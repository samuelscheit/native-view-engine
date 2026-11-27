//! Native top-layer runtime for [`HtmlSemantics`](nve_web_html::HtmlSemantics) overlays:
//! `<dialog>` `show()` / `showModal()` / `close()` (modality from `Accessibility::modal`), the
//! `closedby="any"` backdrop-click fallback for browsers without native `closedBy`, and
//! `[popover]` `showPopover()` / `hidePopover()` driven by `data_nve_open`.

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
pub(crate) use live::{is_native_dialog, sync_overlay_state, try_dialog_backdrop_fallback};

#[cfg(target_arch = "wasm32")]
mod live {
    use nve_components::tables::Accessibility;
    use nve_store::{NodeId, NodeStore};
    use nve_web_html::{HtmlSemantics, PopoverMode};
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
")]
    extern "C" {
        fn nve_supports_dialog_closedby() -> bool;
        fn nve_sync_popover_state(el: &web_sys::Element, open: bool);
    }

    #[inline]
    pub(crate) fn is_native_dialog(el: &web_sys::Element) -> bool {
        el.is_instance_of::<web_sys::HtmlDialogElement>()
    }

    /// Applies `node`'s `HtmlSemantics` open state to its live `<dialog>` / `[popover]` element.
    pub(crate) fn sync_overlay_state(store: &NodeStore, node: NodeId, el: &web_sys::Element) {
        let Some(row) = store.try_row::<HtmlSemantics>(node) else {
            return;
        };
        if let Some(dialog) = el.dyn_ref::<web_sys::HtmlDialogElement>() {
            let modal = store
                .try_row::<Accessibility>(node)
                .is_some_and(|a| a.modal());
            sync_dialog_open_state(dialog, row.open(), modal);
        } else if row.popover() != PopoverMode::None {
            nve_sync_popover_state(el, row.data_nve_open());
        }
    }

    /// `showModal()` / `show()` / `close()`; a detached dialog only mirrors the `open` attribute
    /// (`showModal()` throws on disconnected elements).
    fn sync_dialog_open_state(dialog: &web_sys::HtmlDialogElement, open: bool, modal: bool) {
        if !dialog.is_connected() {
            let _ = if open {
                dialog.set_attribute("open", "")
            } else {
                dialog.remove_attribute("open")
            };
            return;
        }
        if !open {
            if dialog.open() {
                dialog.close();
            }
        } else if modal {
            if !dialog.matches(":modal").unwrap_or(false) {
                // An SSR-adopted `<dialog open>` is non-modal; reopen it in the top layer.
                if dialog.open() {
                    dialog.close();
                }
                let _ = dialog.show_modal();
            }
        } else if !dialog.open() {
            dialog.show();
        }
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
        let r = dialog.get_bounding_client_rect();
        let close = should_close_dialog_on_backdrop_click(
            dialog.get_attribute("closedby").as_deref(),
            nve_supports_dialog_closedby(),
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
