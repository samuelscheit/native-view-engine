//! Minimal interactive Counter + Card app authored in Dioxus `rsx!` + signals.
//!
//! Compiles as BOTH:
//! - an `rlib` for instant headless inspection/testing (`cargo nve inspect`), and
//! - a Tier-2 hot-swappable `cdylib` exporting `nve_app_vtable` (for ~150-400ms live reloads
//!   inside macOS Skia, iOS Simulator, and Android host shells with preserved state!).

use std::sync::atomic::{AtomicI32, Ordering};

use nve_abi::{
    decode_events, NveAppHandle, NveAppVTable, NveBufferSlice, SurfaceId, NVE_APP_ABI_VERSION,
};
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;
use nve_engine::HotPatchMsg;

/// Initial count injected when restoring state across a Tier-2 dylib hot-swap.
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

/* -------------------------------------------------------------------------- */
/* Tier-2 Hot-Swappable C ABI (`nve_app_vtable`)                              */
/* -------------------------------------------------------------------------- */

struct ExportedApp {
    surface: DioxusSurface,
    last_frame_buf: Vec<u8>,
    last_state_buf: Vec<u8>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CounterSnapshot {
    count: i32,
}

unsafe extern "C" fn app_create(surface_id: u32) -> NveAppHandle {
    let app = Box::new(ExportedApp {
        surface: create_surface(surface_id),
        last_frame_buf: Vec::new(),
        last_state_buf: Vec::new(),
    });
    Box::into_raw(app) as NveAppHandle
}

unsafe extern "C" fn app_destroy(handle: NveAppHandle) {
    if !handle.is_null() {
        unsafe { drop(Box::from_raw(handle as *mut ExportedApp)) };
    }
}

unsafe extern "C" fn app_render(handle: NveAppHandle, frame_id: u64) -> NveBufferSlice {
    let app = unsafe { &mut *(handle as *mut ExportedApp) };
    let (_, _, bytes) = app.surface.render_frame(frame_id);
    app.last_frame_buf = bytes;
    NveBufferSlice::from_slice(&app.last_frame_buf)
}

unsafe extern "C" fn app_dispatch_events(handle: NveAppHandle, events: NveBufferSlice) -> u32 {
    let app = unsafe { &mut *(handle as *mut ExportedApp) };
    let raw = unsafe { events.as_slice() };
    let Ok(decoded) = decode_events(raw) else {
        return 0;
    };
    u32::from(app.surface.dispatch_events(&decoded))
}

unsafe extern "C" fn app_apply_hot_patch(handle: NveAppHandle, patch_json: NveBufferSlice) -> u32 {
    let app = unsafe { &mut *(handle as *mut ExportedApp) };
    let raw = unsafe { patch_json.as_slice() };
    let Ok(s) = std::str::from_utf8(raw) else {
        return 0;
    };
    let Ok(msg) = HotPatchMsg::from_json(s) else {
        return 0;
    };
    u32::from(app.surface.apply_hot_patch(&msg))
}

unsafe extern "C" fn app_snapshot_state(handle: NveAppHandle) -> NveBufferSlice {
    let app = unsafe { &mut *(handle as *mut ExportedApp) };
    let snap = CounterSnapshot {
        count: LATEST_COUNT.load(Ordering::SeqCst),
    };
    app.last_state_buf = serde_json::to_vec(&snap).unwrap_or_default();
    NveBufferSlice::from_slice(&app.last_state_buf)
}

unsafe extern "C" fn app_restore_state(handle: NveAppHandle, state: NveBufferSlice) {
    let app = unsafe { &mut *(handle as *mut ExportedApp) };
    let raw = unsafe { state.as_slice() };
    if let Ok(snap) = serde_json::from_slice::<CounterSnapshot>(raw) {
        INITIAL_COUNT.store(snap.count, Ordering::SeqCst);
        LATEST_COUNT.store(snap.count, Ordering::SeqCst);
        // Recreate the surface so the signal initializes from restored state.
        app.surface = create_surface(1);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn nve_app_vtable() -> NveAppVTable {
    NveAppVTable {
        abi_version: NVE_APP_ABI_VERSION,
        create: app_create,
        destroy: app_destroy,
        render: app_render,
        dispatch_events: app_dispatch_events,
        apply_hot_patch: app_apply_hot_patch,
        snapshot_state: app_snapshot_state,
        restore_state: app_restore_state,
    }
}
