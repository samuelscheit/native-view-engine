use std::cell::Cell;
use std::ffi::c_void;

use nve_abi::{
    encode_bin_value_to_vec, NveBufferSlice, NveComponentApi, NveDevApi, NveDirectEvent,
    NveEventBatch, NveGestureDisposition, NveHostApi, NveHostCallbacks, NveIntrinsicSize,
    NveLayoutApi, NveMapEntry, NvePropsRef, NveSizeConstraints, NveSurfaceApi, NveValue,
    NVE_IFACE_COMPONENT, NVE_IFACE_DEV, NVE_IFACE_HOST, NVE_IFACE_LAYOUT, NVE_IFACE_SURFACE,
};

use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum PanicTarget {
    None,
    Create,
    Render,
    DispatchEvents,
    DispatchEventsDirect,
    Measure,
    HitTest,
    SetProps,
    SetPropsDirect,
    DrainEmittedEvents,
    ApplyHotPatch,
    SnapshotState,
    RestoreState,
    BindHost,
    DispatchHostEvents,
    Destroy,
    NonStringPayload,
}

thread_local! {
    static PANIC_TARGET: Cell<PanicTarget> = const { Cell::new(PanicTarget::None) };
}

fn set_panic_target(target: PanicTarget) {
    PANIC_TARGET.with(|cell| cell.set(target));
}

fn current_panic_target() -> PanicTarget {
    PANIC_TARGET.with(Cell::get)
}

struct PanickingTestApp {
    surface_id: SurfaceId,
    frame_buf: Vec<u8>,
    emitted_buf: Vec<u8>,
    state_buf: Vec<u8>,
}

impl Drop for PanickingTestApp {
    fn drop(&mut self) {
        if current_panic_target() == PanicTarget::Destroy {
            panic!("boom in destroy for surface {}", self.surface_id);
        }
    }
}

impl NveApp for PanickingTestApp {
    fn create(surface_id: SurfaceId) -> Self {
        match current_panic_target() {
            PanicTarget::Create => panic!("boom in create"),
            PanicTarget::NonStringPayload => std::panic::panic_any(404u32),
            _ => Self {
                surface_id,
                frame_buf: vec![0xAA, 0xBB],
                emitted_buf: vec![0xCC],
                state_buf: vec![1, 2, 3],
            },
        }
    }

    fn render(&mut self, frame_id: FrameId) -> &[u8] {
        if current_panic_target() == PanicTarget::Render {
            panic!("boom in render frame {frame_id}");
        }
        &self.frame_buf
    }

    fn dispatch_events(&mut self, _encoded_events: &[u8]) -> bool {
        if current_panic_target() == PanicTarget::DispatchEvents {
            panic!("boom in dispatch_events");
        }
        true
    }

    fn dispatch_events_direct(&mut self, _events: NveEventBatchRef<'_>) -> bool {
        if current_panic_target() == PanicTarget::DispatchEventsDirect {
            panic!("boom in dispatch_events_direct");
        }
        true
    }

    fn measure(&mut self, _constraints: NveSizeConstraints) -> NveIntrinsicSize {
        if current_panic_target() == PanicTarget::Measure {
            panic!("boom in measure");
        }
        NveIntrinsicSize {
            width: 123.0,
            height: 45.0,
        }
    }

    fn hit_test(&mut self, _x: f32, _y: f32, _pointer_phase: u8) -> NveGestureDisposition {
        if current_panic_target() == PanicTarget::HitTest {
            panic!("boom in hit_test");
        }
        NveGestureDisposition::Claim
    }

    fn set_props(&mut self, _props_bytes: &[u8]) -> bool {
        if current_panic_target() == PanicTarget::SetProps {
            panic!("boom in set_props");
        }
        true
    }

    fn set_props_direct(&mut self, _props: NvePropsRef<'_>) -> bool {
        if current_panic_target() == PanicTarget::SetPropsDirect {
            panic!("boom in set_props_direct");
        }
        true
    }

    fn drain_emitted_events(&mut self) -> &[u8] {
        if current_panic_target() == PanicTarget::DrainEmittedEvents {
            panic!("boom in drain_emitted_events");
        }
        &self.emitted_buf
    }

    fn apply_hot_patch(&mut self, _patch: &crate::HotPatchMsg) -> bool {
        if current_panic_target() == PanicTarget::ApplyHotPatch {
            panic!("boom in apply_hot_patch");
        }
        true
    }

    fn snapshot_state(&mut self) -> &[u8] {
        if current_panic_target() == PanicTarget::SnapshotState {
            panic!("boom in snapshot_state");
        }
        &self.state_buf
    }

    fn restore_state(&mut self, state: &[u8]) {
        if current_panic_target() == PanicTarget::RestoreState {
            panic!("boom in restore_state");
        }
        self.state_buf = state.to_vec();
    }

    fn bind_host(&mut self, _callbacks: Option<NveHostCallbacks>) {
        if current_panic_target() == PanicTarget::BindHost {
            panic!("boom in bind_host");
        }
    }

    fn dispatch_host_events(&mut self, _batch: &[u8]) -> bool {
        if current_panic_target() == PanicTarget::DispatchHostEvents {
            panic!("boom in dispatch_host_events");
        }
        true
    }
}

crate::export_nve_app!(PanickingTestApp, symbol = test_panicking_app_get_interface);

unsafe extern "C" {
    fn test_panicking_app_get_interface(interface_id: u32) -> *const c_void;
}

#[test]
fn export_nve_app_catches_panics_across_all_13_shims_and_null_handles() {
    clear_last_ffi_panic();
    set_panic_target(PanicTarget::None);

    let surface = unsafe {
        &*(test_panicking_app_get_interface(NVE_IFACE_SURFACE) as *const NveSurfaceApi)
    };
    let layout = unsafe {
        &*(test_panicking_app_get_interface(NVE_IFACE_LAYOUT) as *const NveLayoutApi)
    };
    let component = unsafe {
        &*(test_panicking_app_get_interface(NVE_IFACE_COMPONENT) as *const NveComponentApi)
    };
    let dev =
        unsafe { &*(test_panicking_app_get_interface(NVE_IFACE_DEV) as *const NveDevApi) };
    let host =
        unsafe { &*(test_panicking_app_get_interface(NVE_IFACE_HOST) as *const NveHostApi) };
    let dispatch_direct = surface
        .dispatch_events_direct
        .expect("dispatch_events_direct exported");
    let set_props_direct = component
        .set_props_direct
        .expect("set_props_direct exported");

    // 0. Null handle guards across all 14 handle-taking shims (no dereference, no panic).
    let null_h = std::ptr::null_mut();
    unsafe {
        assert_eq!((surface.render)(null_h, 1).len, 0);
        assert_eq!((surface.dispatch_events)(null_h, NveBufferSlice::EMPTY), 0);
        assert_eq!(dispatch_direct(null_h, NveEventBatch::EMPTY), 0);
        assert_eq!(
            (layout.measure)(null_h, NveSizeConstraints::loose(100.0, 100.0)),
            NveIntrinsicSize {
                width: 0.0,
                height: 0.0
            }
        );
        assert_eq!(
            (layout.hit_test)(null_h, 0.0, 0.0, 0),
            NveGestureDisposition::PassThrough as u8
        );
        assert_eq!((component.set_props)(null_h, NveBufferSlice::EMPTY), 0);
        assert_eq!(set_props_direct(null_h, std::ptr::null(), 0), 0);
        assert_eq!((component.drain_emitted_events)(null_h).len, 0);
        assert_eq!((dev.apply_hot_patch)(null_h, NveBufferSlice::EMPTY), 0);
        assert_eq!((dev.snapshot_state)(null_h).len, 0);
        (dev.restore_state)(null_h, NveBufferSlice::EMPTY);
        (host.bind_host)(null_h, std::ptr::null());
        assert_eq!((host.dispatch_host_events)(null_h, NveBufferSlice::EMPTY), 0);
        (surface.destroy)(null_h);
    }
    assert_eq!(last_ffi_panic(), None);

    // 1. Panicking `create` returns null and records message; non-string payloads also work.
    set_panic_target(PanicTarget::Create);
    let h_fail = unsafe { (surface.create)(7) };
    assert!(h_fail.is_null());
    let peeked = last_ffi_panic().expect("create panic recorded");
    assert!(peeked.contains("NveSurfaceApi::create"));
    assert!(peeked.contains("boom in create"));
    assert_eq!(take_last_ffi_panic(), Some(peeked));
    assert_eq!(last_ffi_panic(), None);

    set_panic_target(PanicTarget::NonStringPayload);
    let h_non_str = unsafe { (surface.create)(8) };
    assert!(h_non_str.is_null());
    let non_str_msg = take_last_ffi_panic().expect("non-string panic recorded");
    assert!(non_str_msg.contains("non-string panic payload"));

    // 2. Normal create + verify happy path does not record any panic.
    set_panic_target(PanicTarget::None);
    let handle = unsafe { (surface.create)(42) };
    assert!(!handle.is_null());
    assert_eq!(unsafe { (surface.render)(handle, 1).as_slice() }, &[0xAA, 0xBB]);
    assert_eq!(
        unsafe { (layout.measure)(handle, NveSizeConstraints::loose(300.0, 200.0)) },
        NveIntrinsicSize {
            width: 123.0,
            height: 45.0
        }
    );
    assert_eq!(
        unsafe { (layout.hit_test)(handle, 1.0, 2.0, 0) },
        NveGestureDisposition::Claim as u8
    );
    assert_eq!(last_ffi_panic(), None);

    // 3. Verify every remaining shim survives panics and returns its exact sentinel.
    set_panic_target(PanicTarget::Render);
    let r = unsafe { (surface.render)(handle, 99) };
    assert!(r.ptr.is_null() && r.len == 0);
    let msg = take_last_ffi_panic().unwrap();
    assert!(msg.contains("NveSurfaceApi::render") && msg.contains("boom in render frame 99"));

    set_panic_target(PanicTarget::DispatchEvents);
    assert_eq!(
        unsafe { (surface.dispatch_events)(handle, NveBufferSlice::from_slice(b"ev")) },
        0
    );
    let msg = take_last_ffi_panic().unwrap();
    assert!(msg.contains("NveSurfaceApi::dispatch_events") && msg.contains("boom in dispatch_events"));

    set_panic_target(PanicTarget::DispatchEventsDirect);
    let direct_ev = [NveDirectEvent::press(42, 2, 100, 1.0, 2.0)];
    assert_eq!(
        unsafe { dispatch_direct(handle, NveEventBatch::from_slice(&direct_ev)) },
        0
    );
    let msg = take_last_ffi_panic().unwrap();
    assert!(
        msg.contains("NveSurfaceApi::dispatch_events_direct")
            && msg.contains("boom in dispatch_events_direct")
    );

    set_panic_target(PanicTarget::Measure);
    assert_eq!(
        unsafe { (layout.measure)(handle, NveSizeConstraints::loose(100.0, 100.0)) },
        NveIntrinsicSize {
            width: 0.0,
            height: 0.0
        }
    );
    let msg = take_last_ffi_panic().unwrap();
    assert!(msg.contains("NveLayoutApi::measure") && msg.contains("boom in measure"));

    set_panic_target(PanicTarget::HitTest);
    assert_eq!(
        unsafe { (layout.hit_test)(handle, 5.0, 6.0, 1) },
        NveGestureDisposition::PassThrough as u8
    );
    let msg = take_last_ffi_panic().unwrap();
    assert!(msg.contains("NveLayoutApi::hit_test") && msg.contains("boom in hit_test"));

    set_panic_target(PanicTarget::SetProps);
    assert_eq!(
        unsafe { (component.set_props)(handle, NveBufferSlice::from_slice(b"props")) },
        0
    );
    let msg = take_last_ffi_panic().unwrap();
    assert!(msg.contains("NveComponentApi::set_props") && msg.contains("boom in set_props"));

    set_panic_target(PanicTarget::SetPropsDirect);
    let entries = [NveMapEntry::new("k", NveValue::i64(1))];
    assert_eq!(
        unsafe { set_props_direct(handle, entries.as_ptr(), entries.len() as u32) },
        0
    );
    let msg = take_last_ffi_panic().unwrap();
    assert!(
        msg.contains("NveComponentApi::set_props_direct")
            && msg.contains("boom in set_props_direct")
    );

    set_panic_target(PanicTarget::DrainEmittedEvents);
    let drained = unsafe { (component.drain_emitted_events)(handle) };
    assert!(drained.ptr.is_null() && drained.len == 0);
    let msg = take_last_ffi_panic().unwrap();
    assert!(
        msg.contains("NveComponentApi::drain_emitted_events")
            && msg.contains("boom in drain_emitted_events")
    );

    set_panic_target(PanicTarget::ApplyHotPatch);
    let patch = crate::HotPatchMsg::DevStatus { message: "x".into(), latency_ms: 0 }.to_bytes();
    assert_eq!(
        unsafe { (dev.apply_hot_patch)(handle, NveBufferSlice::from_slice(&patch)) },
        0
    );
    let msg = take_last_ffi_panic().unwrap();
    assert!(
        msg.contains("NveDevApi::apply_hot_patch") && msg.contains("boom in apply_hot_patch")
    );

    set_panic_target(PanicTarget::SnapshotState);
    let snap = unsafe { (dev.snapshot_state)(handle) };
    assert!(snap.ptr.is_null() && snap.len == 0);
    let msg = take_last_ffi_panic().unwrap();
    assert!(
        msg.contains("NveDevApi::snapshot_state") && msg.contains("boom in snapshot_state")
    );

    set_panic_target(PanicTarget::RestoreState);
    unsafe { (dev.restore_state)(handle, NveBufferSlice::from_slice(&[9, 8, 7])) };
    let msg = take_last_ffi_panic().unwrap();
    assert!(msg.contains("NveDevApi::restore_state") && msg.contains("boom in restore_state"));

    set_panic_target(PanicTarget::BindHost);
    let empty_cb = NveHostCallbacks::EMPTY;
    unsafe { (host.bind_host)(handle, &empty_cb) };
    let msg = take_last_ffi_panic().unwrap();
    assert!(msg.contains("NveHostApi::bind_host") && msg.contains("boom in bind_host"));

    set_panic_target(PanicTarget::DispatchHostEvents);
    assert_eq!(
        unsafe { (host.dispatch_host_events)(handle, NveBufferSlice::from_slice(b"host")) },
        0
    );
    let msg = take_last_ffi_panic().unwrap();
    assert!(
        msg.contains("NveHostApi::dispatch_host_events")
            && msg.contains("boom in dispatch_host_events")
    );

    // 4. Panicking `destroy` (Drop) survives without aborting and records the message.
    set_panic_target(PanicTarget::Destroy);
    unsafe { (surface.destroy)(handle) };
    let msg = take_last_ffi_panic().unwrap();
    assert!(
        msg.contains("NveSurfaceApi::destroy") && msg.contains("boom in destroy for surface 42")
    );
    set_panic_target(PanicTarget::None);
    assert_eq!(last_ffi_panic(), None);
}

#[derive(Default)]
struct DirectOnlyPropsApp {
    count: i64,
    route: String,
    nested_flag: bool,
    tag_count: usize,
}

impl NveApp for DirectOnlyPropsApp {
    fn create(_surface_id: SurfaceId) -> Self {
        Self::default()
    }

    fn render(&mut self, _frame_id: FrameId) -> &[u8] {
        &[]
    }

    fn dispatch_events(&mut self, _encoded_events: &[u8]) -> bool {
        false
    }

    fn set_props_direct(&mut self, props: NvePropsRef<'_>) -> bool {
        let mut changed = false;
        if let Some(c) = props.get("count").and_then(|v| v.as_i64()) {
            self.count = c;
            changed = true;
        }
        if let Some(r) = props.get("route").and_then(|v| v.as_str()) {
            self.route = r.to_owned();
            changed = true;
        }
        if let Some(flag) = props
            .get("meta")
            .and_then(|m| m.get("active"))
            .and_then(|v| v.as_bool())
        {
            self.nested_flag = flag;
            changed = true;
        }
        if let Some(tags) = props.get("tags").and_then(|v| v.as_array()) {
            self.tag_count = tags.len();
            changed = true;
        }
        changed
    }
}

#[test]
fn default_set_props_bridges_bin_value_maps_to_set_props_direct() {
    let mut app = DirectOnlyPropsApp::create(1);
    let nested_entries = [NveMapEntry::new("active", NveValue::bool(true))];
    let tag_elems = [NveValue::from_str("a"), NveValue::from_str("b")];
    let entries = [
        NveMapEntry::new("count", NveValue::i64(77)),
        NveMapEntry::new("route", NveValue::from_str("/docs/abi")),
        NveMapEntry::new("meta", NveValue::map(&nested_entries)),
        NveMapEntry::new("tags", NveValue::array(&tag_elems)),
    ];
    let map_val = NveValue::map(&entries);
    let bin_bytes = encode_bin_value_to_vec(unsafe { map_val.as_ref() }).unwrap();

    assert!(app.set_props(&bin_bytes));
    assert_eq!(app.count, 77);
    assert_eq!(app.route, "/docs/abi");
    assert!(app.nested_flag);
    assert_eq!(app.tag_count, 2);

    // Malformed / non-map bytes return false cleanly.
    assert!(!app.set_props(b"not-valid-bin"));
    let scalar_val = NveValue::i64(1);
    let scalar_bin = encode_bin_value_to_vec(unsafe { scalar_val.as_ref() }).unwrap();
    assert!(!app.set_props(&scalar_bin));
}
