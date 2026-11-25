//! Minimal interactive Counter + Card app authored in Dioxus `rsx!` + signals.
//!
//! Implements the safe `nve_engine::NveApp` trait with zero `unsafe` and exports the modular
//! `nve_get_interface` C ABI via `nve_engine::export_nve_app!(CounterModule)` for Tier-2
//! hot-swapping, 0ms web snapshot state restore (`NWC1` binary codec), and embedding in RN,
//! Flutter, SwiftUI, and Jetpack Compose.

use std::cell::RefCell;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use nve_abi::{
    decode_events, encode_events, FrameId, NveEventBatchRef, NveGestureDisposition,
    NveIntrinsicSize, NvePropsRef, NveSizeConstraints, SurfaceId,
};
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;
use nve_engine::{HotPatchMsg, NveApp};

/// 4-byte ASCII magic identifying a binary Counter state payload (`NWC1`).
pub const COUNTER_STATE_BIN_MAGIC: [u8; 4] = *b"NWC1";

/// Schema version embedded in `NWC1` binary Counter snapshots.
pub const COUNTER_STATE_BIN_VERSION: u32 = 1;

thread_local! {
    static PENDING_COUNTER: RefCell<Option<Arc<AtomicI32>>> = const { RefCell::new(None) };
}

fn take_or_default_counter_store() -> Arc<AtomicI32> {
    PENDING_COUNTER
        .with(|slot| slot.borrow_mut().take())
        .unwrap_or_else(|| Arc::new(AtomicI32::new(0)))
}

/// Encodes `count` into a compact 10-byte zero-JSON little-endian `NWC1` binary payload.
#[inline]
pub fn encode_counter_state(count: i32) -> [u8; 10] {
    let mut out = [0u8; 10];
    out[0..4].copy_from_slice(&COUNTER_STATE_BIN_MAGIC);
    out[4..6].copy_from_slice(&(COUNTER_STATE_BIN_VERSION as u16).to_le_bytes());
    out[6..10].copy_from_slice(&count.to_le_bytes());
    out
}

/// Decodes a 10-byte `NWC1` binary payload back into `count`, rejecting corrupt or version-mismatched data.
#[inline]
pub fn decode_counter_state(bytes: &[u8]) -> Option<i32> {
    if bytes.len() != 10 || bytes[0..4] != COUNTER_STATE_BIN_MAGIC {
        return None;
    }
    let ver = u16::from_le_bytes([bytes[4], bytes[5]]) as u32;
    if ver != COUNTER_STATE_BIN_VERSION {
        return None;
    }
    Some(i32::from_le_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]))
}

#[component]
pub fn CounterApp() -> Element {
    let store = dioxus_core::use_hook(take_or_default_counter_store);
    let mut count = use_signal(|| store.load(Ordering::SeqCst));
    let inc_store = Arc::clone(&store);
    let reset_store = Arc::clone(&store);

    rsx! {
        view {
            style: "width: 100%; height: 100%; padding: 24px; gap: 16px; background-color: #0f172a;",
            view {
                style: "width: 100%; padding: 20px; gap: 8px; background-color: #1e293b; border-radius: 16px;",
                text {
                    style: "color: #38bdf8; font-size: 14px;",
                    "NATIVE VIEW ENGINE"
                }
                text {
                    style: "color: #ffffff; font-size: 28px;",
                    "Count: {count}"
                }
            }
            view {
                style: "width: 100%; flex-direction: row; gap: 12px;",
                pressable {
                    style: "width: 160px; height: 48px; justify-content: center; align-items: center; background-color: #2563eb; border-radius: 12px;",
                    onpress: move |_| {
                        let next = count() + 1;
                        inc_store.store(next, Ordering::SeqCst);
                        count.set(next);
                    },
                    text {
                        style: "color: #ffffff; font-size: 16px;",
                        "Increment +"
                    }
                }
                pressable {
                    style: "width: 120px; height: 48px; justify-content: center; align-items: center; background-color: #334155; border-radius: 12px;",
                    onpress: move |_| {
                        reset_store.store(0, Ordering::SeqCst);
                        count.set(0);
                    },
                    text {
                        style: "color: #e2e8f0; font-size: 16px;",
                        "Reset"
                    }
                }
            }
        }
    }
}

pub fn create_surface(surface_id: SurfaceId) -> DioxusSurface {
    DioxusSurface::new(surface_id, CounterApp)
}

pub struct CounterModule {
    surface_id: SurfaceId,
    store: Arc<AtomicI32>,
    surface: DioxusSurface,
    mounted: bool,
    last_frame_buf: Vec<u8>,
    last_state_buf: [u8; 10],
    last_emitted_events_buf: Vec<u8>,
}

impl CounterModule {
    #[inline]
    pub fn count(&self) -> i32 {
        self.store.load(Ordering::SeqCst)
    }

    /// Attempts to restore counter state from `NWC1` binary bytes; returns `false` on corrupt data.
    pub fn try_restore_state(&mut self, state: &[u8]) -> bool {
        let Some(restored) = decode_counter_state(state) else {
            return false;
        };
        self.store.store(restored, Ordering::SeqCst);
        self.surface = create_surface(self.surface_id);
        self.mounted = false;
        true
    }
}

impl NveApp for CounterModule {
    fn create(surface_id: SurfaceId) -> Self {
        Self {
            surface_id,
            store: Arc::new(AtomicI32::new(0)),
            surface: create_surface(surface_id),
            mounted: false,
            last_frame_buf: Vec::new(),
            last_state_buf: encode_counter_state(0),
            last_emitted_events_buf: Vec::new(),
        }
    }

    fn render(&mut self, frame_id: FrameId) -> &[u8] {
        let bytes = if !self.mounted {
            PENDING_COUNTER.with(|slot| *slot.borrow_mut() = Some(Arc::clone(&self.store)));
            self.mounted = true;
            let (_, _, bytes) = self.surface.render_frame(frame_id);
            PENDING_COUNTER.with(|slot| *slot.borrow_mut() = None);
            bytes
        } else {
            let (_, _, bytes) = self.surface.render_frame(frame_id);
            bytes
        };
        self.last_frame_buf = bytes;
        &self.last_frame_buf
    }

    fn dispatch_events(&mut self, encoded_events: &[u8]) -> bool {
        let Ok(decoded) = decode_events(encoded_events) else {
            return false;
        };
        self.surface.dispatch_events(&decoded)
    }

    fn dispatch_events_direct(&mut self, events: NveEventBatchRef<'_>) -> bool {
        self.surface.dispatch_events_direct(events)
    }

    fn measure(&mut self, constraints: NveSizeConstraints) -> NveIntrinsicSize {
        let w = constraints.max_w.clamp(constraints.min_w.max(320.0), 640.0);
        let h = constraints.max_h.clamp(constraints.min_h.max(220.0), 480.0);
        NveIntrinsicSize {
            width: w,
            height: h,
        }
    }

    fn hit_test(&mut self, _x: f32, _y: f32, _pointer_phase: u8) -> NveGestureDisposition {
        NveGestureDisposition::Claim
    }

    fn set_props_direct(&mut self, props: NvePropsRef<'_>) -> bool {
        if let Some(c) = props
            .get("initialCount")
            .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
        {
            self.store.store(c as i32, Ordering::SeqCst);
            self.surface = create_surface(self.surface_id);
            self.mounted = false;
            return true;
        }
        false
    }

    fn drain_emitted_events(&mut self) -> &[u8] {
        self.last_emitted_events_buf = encode_events(&[]);
        &self.last_emitted_events_buf
    }

    fn apply_hot_patch(&mut self, patch: &HotPatchMsg) -> bool {
        self.surface.apply_hot_patch(patch)
    }

    fn snapshot_state(&mut self) -> &[u8] {
        self.last_state_buf = encode_counter_state(self.store.load(Ordering::SeqCst));
        &self.last_state_buf
    }

    fn restore_state(&mut self, state: &[u8]) {
        let _ = self.try_restore_state(state);
    }
}

// Keep the C ABI export in its own private module so `rustc` places `nve_get_interface` in a
// dedicated CGU. That prevents MSVC `link.exe` LNK2005 duplicate-symbol clashes when workspace
// crates (such as `nve-web-shell` tests under `cargo test --workspace`) link multiple example
// `rlib`s simultaneously while `cdylib` builds still export `nve_get_interface`.
#[cfg(all(feature = "cdylib-export", not(target_arch = "wasm32")))]
mod cdylib_export {
    nve_engine::export_nve_app!(super::CounterModule);
}
