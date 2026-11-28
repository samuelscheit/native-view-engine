//! Platform-native Web `window.visualViewport` soft keyboard inset tracking.
//!
//! Uses `window.visualViewport`'s `"resize"` and `"scroll"` events on `wasm32` (zero polling,
//! zero JS shims) to derive [`KeyboardInsets`] via [`keyboard_insets_from_visual_viewport`],
//! notifies optional listeners, and emits compact 8-byte binary `"keyboard_insets"` custom events.

use std::rc::Rc;

use nve_abi::{TimestampUs, ROOT_NODE_ID};
use nve_codec::NveEncode;
pub use nve_tables::keyboard::{
    effective_bottom_inset, keyboard_insets_from_visual_viewport, KeyboardInsets,
    VISUAL_VIEWPORT_KEYBOARD_THRESHOLD_PX,
};

#[cfg(target_arch = "wasm32")]
use std::cell::{Cell, RefCell};
#[cfg(target_arch = "wasm32")]
use nve_abi::EventBatchBuf;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::closure::Closure;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;

#[cfg(target_arch = "wasm32")]
use crate::events::now_us;
use crate::WebBackend;

pub(crate) type KeyboardInsetsCallback = Rc<dyn Fn(KeyboardInsets)>;

/// Custom event name emitted on `ROOT_NODE_ID` (`NVE_TABLE_CUSTOM`) carrying an 8-byte binary
/// `NveCodec`-encoded [`KeyboardInsets`] payload whenever `window.visualViewport` keyboard
/// occlusion changes.
pub const KEYBOARD_INSETS_EVENT_NAME: &str = "keyboard_insets";

fn encode_keyboard_insets(insets: KeyboardInsets) -> Vec<u8> {
    insets
        .encode_to_vec()
        .expect("KeyboardInsets fixed 8-byte encoding is infallible")
}

/// Active `window.visualViewport` listener handle detached cleanly on [`WebBackend::drop`].
#[cfg(target_arch = "wasm32")]
pub(crate) struct VisualViewportTracker {
    visual_viewport: web_sys::VisualViewport,
    on_resize: Closure<dyn FnMut(web_sys::Event)>,
    on_scroll: Closure<dyn FnMut(web_sys::Event)>,
}

#[cfg(target_arch = "wasm32")]
impl Drop for VisualViewportTracker {
    fn drop(&mut self) {
        let _ = self.visual_viewport.remove_event_listener_with_callback(
            "resize",
            self.on_resize.as_ref().unchecked_ref(),
        );
        let _ = self.visual_viewport.remove_event_listener_with_callback(
            "scroll",
            self.on_scroll.as_ref().unchecked_ref(),
        );
    }
}

#[cfg(target_arch = "wasm32")]
fn sample_browser_keyboard_insets(
    window: &web_sys::Window,
    vv: &web_sys::VisualViewport,
) -> KeyboardInsets {
    let layout_height = window
        .inner_height()
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as f32;
    let visual_height = vv.height() as f32;
    let visual_offset_top = vv.offset_top() as f32;
    let visual_scale = vv.scale() as f32;
    keyboard_insets_from_visual_viewport(
        layout_height,
        visual_height,
        visual_offset_top,
        visual_scale,
    )
}

#[cfg(target_arch = "wasm32")]
fn commit_keyboard_insets(
    next: KeyboardInsets,
    slot: &Rc<Cell<KeyboardInsets>>,
    on_keyboard_insets: &Rc<RefCell<Option<KeyboardInsetsCallback>>>,
    events: &Rc<RefCell<EventBatchBuf>>,
    on_event: &Rc<RefCell<Option<crate::RenderCallback>>>,
    surface_id: &Rc<Cell<nve_abi::SurfaceId>>,
) {
    if slot.get() == next {
        return;
    }
    slot.set(next);
    if let Some(cb) = on_keyboard_insets.borrow().clone() {
        cb(next);
    }
    let data = encode_keyboard_insets(next);
    events.borrow_mut().push_custom(
        surface_id.get(),
        ROOT_NODE_ID,
        now_us(),
        KEYBOARD_INSETS_EVENT_NAME,
        &data,
    );
    if let Some(cb) = on_event.borrow().clone() {
        cb();
    }
}

impl WebBackend {
    /// Returns the most recently observed soft [`KeyboardInsets`] from `window.visualViewport`.
    #[inline]
    pub fn keyboard_insets(&self) -> KeyboardInsets {
        self.keyboard_insets.get()
    }

    /// Registers a callback invoked whenever soft [`KeyboardInsets`] change (e.g. to push straight
    /// into a Dioxus `ReactiveTraitStore::set::<KeyboardInsets>(insets)`).
    pub fn set_keyboard_insets_callback<F: Fn(KeyboardInsets) + 'static>(&mut self, callback: F) {
        *self.on_keyboard_insets.borrow_mut() = Some(Rc::new(callback));
    }

    /// Updates [`KeyboardInsets`] from raw `window.visualViewport` dimensions (testable on native
    /// targets and callable by custom web hosts). Returns `true` iff the effective insets changed.
    pub fn update_visual_viewport(
        &mut self,
        layout_height: f32,
        visual_height: f32,
        visual_offset_top: f32,
        visual_scale: f32,
    ) -> bool {
        let next = keyboard_insets_from_visual_viewport(
            layout_height,
            visual_height,
            visual_offset_top,
            visual_scale,
        );
        if self.keyboard_insets.get() == next {
            return false;
        }
        self.keyboard_insets.set(next);
        if let Some(cb) = self.on_keyboard_insets.borrow().clone() {
            cb(next);
        }
        let data = encode_keyboard_insets(next);
        self.pending_events.borrow_mut().push_custom(
            self.surface_id.get(),
            ROOT_NODE_ID,
            TimestampUs(0),
            KEYBOARD_INSETS_EVENT_NAME,
            &data,
        );
        true
    }

    /// Attaches native `window.visualViewport` `"resize"` and `"scroll"` listeners on `wasm32`.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn attach_visual_viewport_listener(&mut self) {
        let Some(window) = web_sys::window() else {
            return;
        };
        let Some(vv) = window.visual_viewport() else {
            return;
        };
        let initial = sample_browser_keyboard_insets(&window, &vv);
        self.keyboard_insets.set(initial);

        let make_handler = |window: web_sys::Window, vv: web_sys::VisualViewport| {
            let slot = Rc::clone(&self.keyboard_insets);
            let on_kb = Rc::clone(&self.on_keyboard_insets);
            let events = Rc::clone(&self.pending_events);
            let on_event = Rc::clone(&self.on_event);
            let surface_id = Rc::clone(&self.surface_id);
            Closure::<dyn FnMut(web_sys::Event)>::new(move |_ev: web_sys::Event| {
                let next = sample_browser_keyboard_insets(&window, &vv);
                commit_keyboard_insets(next, &slot, &on_kb, &events, &on_event, &surface_id);
            })
        };

        let on_resize = make_handler(window.clone(), vv.clone());
        let on_scroll = make_handler(window, vv.clone());

        let _ = vv.add_event_listener_with_callback("resize", on_resize.as_ref().unchecked_ref());
        let _ = vv.add_event_listener_with_callback("scroll", on_scroll.as_ref().unchecked_ref());

        self.visual_viewport_tracker = Some(VisualViewportTracker {
            visual_viewport: vv,
            on_resize,
            on_scroll,
        });
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use nve_engine::StoreBackend;

    use super::*;

    #[test]
    fn web_backend_tracks_visual_viewport_keyboard_insets_and_emits_binary_event() {
        let mut backend = WebBackend::new();
        let observed = Rc::new(RefCell::new(Vec::<KeyboardInsets>::new()));
        let obs_clone = Rc::clone(&observed);
        backend.set_keyboard_insets_callback(move |kb| obs_clone.borrow_mut().push(kb));

        // Initial state is hidden.
        assert_eq!(backend.keyboard_insets(), KeyboardInsets::HIDDEN);

        // Mobile Safari opens soft keyboard: layout=844, visual=508 -> 336px.
        assert!(backend.update_visual_viewport(844.0, 508.0, 0.0, 1.0));
        assert_eq!(backend.keyboard_insets().height, 336.0);
        assert!(backend.keyboard_insets().visible);

        // Identical viewport report is a no-op.
        assert!(!backend.update_visual_viewport(844.0, 508.0, 0.0, 1.0));

        // Keyboard dismisses back to full height.
        assert!(backend.update_visual_viewport(844.0, 844.0, 0.0, 1.0));
        assert_eq!(backend.keyboard_insets(), KeyboardInsets::HIDDEN);

        // Callback received exactly the 2 state transitions.
        let list = observed.borrow();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].height, 336.0);
        assert!(list[0].visible);
        assert_eq!(list[1], KeyboardInsets::HIDDEN);

        // Event queue received compact 8-byte binary payloads decodable back to KeyboardInsets.
        let mut events = nve_abi::EventBatchBuf::new();
        backend.drain_events(&mut events);
        let validated = events.as_validated();
        assert_eq!(validated.len(), 2);
        let ev0 = validated.get(0).unwrap();
        assert!(ev0.is_custom());
        assert_eq!(ev0.custom_name(), KEYBOARD_INSETS_EVENT_NAME);
        let data = ev0.custom_data();
        assert_eq!(data.len(), 8);
        let mut reader = nve_codec::ByteReader::new(data);
        assert_eq!(
            <KeyboardInsets as nve_codec::NveDecode>::decode(&mut reader).ok(),
            Some(list[0])
        );
    }
}
