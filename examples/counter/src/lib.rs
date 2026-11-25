//! Minimal interactive Counter + Card app authored in Dioxus `rsx!` + signals.
//!
//! Implements the safe `nve_engine::NveApp` trait with zero `unsafe` and exports the modular
//! `nve_get_interface` C ABI via `nve_engine::export_nve_app!(CounterModule)` for Tier-2
//! hot-swapping and embedding in RN, Flutter, SwiftUI, and Jetpack Compose.

use std::sync::atomic::{AtomicI32, Ordering};

use nve_abi::{
    decode_events, encode_events, FrameId, NveEventBatchRef, NveGestureDisposition,
    NveIntrinsicSize, NvePropsRef, NveSizeConstraints, SurfaceId,
};
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;
use nve_engine::{HotPatchMsg, NveApp};

static INITIAL_COUNT: AtomicI32 = AtomicI32::new(0);
static LATEST_COUNT: AtomicI32 = AtomicI32::new(0);

#[component]
pub fn CounterApp() -> Element {
    let mut count = use_signal(|| INITIAL_COUNT.load(Ordering::SeqCst));
    LATEST_COUNT.store(count(), Ordering::SeqCst);

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
                    onpress: move |_| count += 1,
                    text {
                        style: "color: #ffffff; font-size: 16px;",
                        "Increment +"
                    }
                }
                pressable {
                    style: "width: 120px; height: 48px; justify-content: center; align-items: center; background-color: #334155; border-radius: 12px;",
                    onpress: move |_| count.set(0),
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

#[derive(serde::Serialize, serde::Deserialize)]
struct CounterSnapshot {
    count: i32,
}

pub struct CounterModule {
    surface_id: SurfaceId,
    surface: DioxusSurface,
    last_frame_buf: Vec<u8>,
    last_state_buf: Vec<u8>,
    last_emitted_events_buf: Vec<u8>,
}

impl NveApp for CounterModule {
    fn create(surface_id: SurfaceId) -> Self {
        Self {
            surface_id,
            surface: create_surface(surface_id),
            last_frame_buf: Vec::new(),
            last_state_buf: Vec::new(),
            last_emitted_events_buf: Vec::new(),
        }
    }

    fn render(&mut self, frame_id: FrameId) -> &[u8] {
        let (_, _, bytes) = self.surface.render_frame(frame_id);
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
            INITIAL_COUNT.store(c as i32, Ordering::SeqCst);
            LATEST_COUNT.store(c as i32, Ordering::SeqCst);
            self.surface = create_surface(self.surface_id);
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
        let snap = CounterSnapshot {
            count: LATEST_COUNT.load(Ordering::SeqCst),
        };
        self.last_state_buf = serde_json::to_vec(&snap).unwrap_or_default();
        &self.last_state_buf
    }

    fn restore_state(&mut self, state: &[u8]) {
        if let Ok(snap) = serde_json::from_slice::<CounterSnapshot>(state) {
            INITIAL_COUNT.store(snap.count, Ordering::SeqCst);
            LATEST_COUNT.store(snap.count, Ordering::SeqCst);
            self.surface = create_surface(self.surface_id);
        }
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
