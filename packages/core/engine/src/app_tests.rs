use std::cell::Cell;
use std::ffi::c_void;

use nve_abi::{
    NodeId, NveBufferSlice, NveComponentApi, NveDevApi, NveDirectEvent, NveEventBatch,
    NveGestureDisposition, NveHostApi, NveHostCallbacks, NveIntrinsicSize, NveLayoutApi,
    InterfaceId, NveMapEntry, NvePropsRef, NveSizeConstraints, NveSurfaceApi, NveValue,
    NVE_IFACE_COMPONENT, NVE_IFACE_DEV, NVE_IFACE_HOST, NVE_IFACE_LAYOUT, NVE_IFACE_SURFACE,
};

use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum PanicTarget {
    None,
    Create,
    Render,
    DispatchEvents,
    Measure,
    HitTest,
    SetProps,
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
    emitted_buf: Vec<NveDirectEvent>,
    state_buf: Vec<u8>,
}

impl Drop for PanickingTestApp {
    fn drop(&mut self) {
        if current_panic_target() == PanicTarget::Destroy {
            panic!("boom in destroy for {}", self.surface_id);
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
                emitted_buf: vec![NveDirectEvent::press(surface_id, NodeId::ROOT, 10, 1.0, 2.0)],
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

    fn dispatch_events(&mut self, _events: NveEventBatchRef<'_>) -> bool {
        if current_panic_target() == PanicTarget::DispatchEvents {
            panic!("boom in dispatch_events");
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

    fn set_props(&mut self, _props: NvePropsRef<'_>) -> bool {
        if current_panic_target() == PanicTarget::SetProps {
            panic!("boom in set_props");
        }
        true
    }

    fn drain_emitted_events(&mut self) -> &[NveDirectEvent] {
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
    fn test_panicking_app_get_interface(interface_id: InterfaceId) -> *const c_void;
}

#[test]
fn export_nve_app_catches_panics_across_all_12_shims_and_null_handles() {
    clear_last_ffi_panic();
    set_panic_target(PanicTarget::None);

    // SAFETY: Tests the locally exported `export_nve_app!` C ABI tables, panic catch-unwind
    // boundaries, null-handle guards, and balanced create/destroy lifecycle.
    unsafe {
        let surface =
            &*(test_panicking_app_get_interface(NVE_IFACE_SURFACE) as *const NveSurfaceApi);
        let layout =
            &*(test_panicking_app_get_interface(NVE_IFACE_LAYOUT) as *const NveLayoutApi);
        let component =
            &*(test_panicking_app_get_interface(NVE_IFACE_COMPONENT) as *const NveComponentApi);
        let dev = &*(test_panicking_app_get_interface(NVE_IFACE_DEV) as *const NveDevApi);
        let host = &*(test_panicking_app_get_interface(NVE_IFACE_HOST) as *const NveHostApi);

        // 0. Null handle guards across all 12 handle-taking shims (no dereference, no panic).
        let null_h = std::ptr::null_mut();
        assert_eq!((surface.render)(null_h, FrameId(1)).len, 0);
        assert_eq!((surface.dispatch_events)(null_h, NveEventBatch::EMPTY), 0);
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
        assert_eq!((component.set_props)(null_h, std::ptr::null(), 0), 0);
        assert_eq!((component.drain_emitted_events)(null_h).len, 0);
        assert_eq!((dev.apply_hot_patch)(null_h, NveBufferSlice::EMPTY), 0);
        assert_eq!((dev.snapshot_state)(null_h).len, 0);
        (dev.restore_state)(null_h, NveBufferSlice::EMPTY);
        (host.bind_host)(null_h, std::ptr::null());
        assert_eq!((host.dispatch_host_events)(null_h, NveBufferSlice::EMPTY), 0);
        (surface.destroy)(null_h);
        assert_eq!(last_ffi_panic(), None);

        // 1. Panicking `create` returns null and records message; non-string payloads also work.
        set_panic_target(PanicTarget::Create);
        let h_fail = (surface.create)(SurfaceId::new(7, 0));
        assert!(h_fail.is_null());
        let peeked = last_ffi_panic().expect("create panic recorded");
        assert!(peeked.contains("NveSurfaceApi::create"));
        assert!(peeked.contains("boom in create"));
        assert_eq!(take_last_ffi_panic(), Some(peeked));
        assert_eq!(last_ffi_panic(), None);

        set_panic_target(PanicTarget::NonStringPayload);
        let h_non_str = (surface.create)(SurfaceId::new(8, 0));
        assert!(h_non_str.is_null());
        let non_str_msg = take_last_ffi_panic().expect("non-string panic recorded");
        assert!(non_str_msg.contains("non-string panic payload"));

        // 2. Normal create + verify happy path does not record any panic.
        set_panic_target(PanicTarget::None);
        let handle = (surface.create)(SurfaceId::new(42, 0));
        assert!(!handle.is_null());
        assert_eq!((surface.render)(handle, FrameId(1)).as_slice(), &[0xAA, 0xBB]);
        assert_eq!(
            (layout.measure)(handle, NveSizeConstraints::loose(300.0, 200.0)),
            NveIntrinsicSize {
                width: 123.0,
                height: 45.0
            }
        );
        assert_eq!(
            (layout.hit_test)(handle, 1.0, 2.0, 0),
            NveGestureDisposition::Claim as u8
        );
        assert_eq!(last_ffi_panic(), None);

        // 3. Verify every remaining shim survives panics and returns its exact sentinel.
        set_panic_target(PanicTarget::Render);
        let r = (surface.render)(handle, FrameId(99));
        assert!(r.ptr.is_null() && r.len == 0);
        let msg = take_last_ffi_panic().unwrap();
        assert!(msg.contains("NveSurfaceApi::render") && msg.contains("boom in render frame 99"));

        set_panic_target(PanicTarget::DispatchEvents);
        let direct_ev = [NveDirectEvent::press(SurfaceId::new(42, 0), NodeId::new(2, 0), 100, 1.0, 2.0)];
        assert_eq!(
            (surface.dispatch_events)(handle, NveEventBatch::from_slice(&direct_ev)),
            0
        );
        let msg = take_last_ffi_panic().unwrap();
        assert!(
            msg.contains("NveSurfaceApi::dispatch_events")
                && msg.contains("boom in dispatch_events")
        );

        set_panic_target(PanicTarget::Measure);
        assert_eq!(
            (layout.measure)(handle, NveSizeConstraints::loose(100.0, 100.0)),
            NveIntrinsicSize {
                width: 0.0,
                height: 0.0
            }
        );
        let msg = take_last_ffi_panic().unwrap();
        assert!(msg.contains("NveLayoutApi::measure") && msg.contains("boom in measure"));

        set_panic_target(PanicTarget::HitTest);
        assert_eq!(
            (layout.hit_test)(handle, 5.0, 6.0, 1),
            NveGestureDisposition::PassThrough as u8
        );
        let msg = take_last_ffi_panic().unwrap();
        assert!(msg.contains("NveLayoutApi::hit_test") && msg.contains("boom in hit_test"));

        set_panic_target(PanicTarget::SetProps);
        let entries = [NveMapEntry::new("k", NveValue::i64(1))];
        assert_eq!(
            (component.set_props)(handle, entries.as_ptr(), entries.len() as u32),
            0
        );
        let msg = take_last_ffi_panic().unwrap();
        assert!(msg.contains("NveComponentApi::set_props") && msg.contains("boom in set_props"));

        set_panic_target(PanicTarget::DrainEmittedEvents);
        let drained = (component.drain_emitted_events)(handle);
        assert!(drained.ptr.is_null() && drained.len == 0);
        let msg = take_last_ffi_panic().unwrap();
        assert!(
            msg.contains("NveComponentApi::drain_emitted_events")
                && msg.contains("boom in drain_emitted_events")
        );

        set_panic_target(PanicTarget::ApplyHotPatch);
        let patch = crate::HotPatchMsg::DevStatus {
            message: "x".into(),
            latency_ms: 0,
        }
        .to_bytes()
        .unwrap();
        assert_eq!(
            (dev.apply_hot_patch)(handle, NveBufferSlice::from_slice(&patch)),
            0
        );
        let msg = take_last_ffi_panic().unwrap();
        assert!(
            msg.contains("NveDevApi::apply_hot_patch") && msg.contains("boom in apply_hot_patch")
        );

        set_panic_target(PanicTarget::SnapshotState);
        let snap = (dev.snapshot_state)(handle);
        assert!(snap.ptr.is_null() && snap.len == 0);
        let msg = take_last_ffi_panic().unwrap();
        assert!(
            msg.contains("NveDevApi::snapshot_state") && msg.contains("boom in snapshot_state")
        );

        set_panic_target(PanicTarget::RestoreState);
        (dev.restore_state)(handle, NveBufferSlice::from_slice(&[9, 8, 7]));
        let msg = take_last_ffi_panic().unwrap();
        assert!(msg.contains("NveDevApi::restore_state") && msg.contains("boom in restore_state"));

        set_panic_target(PanicTarget::BindHost);
        let empty_cb = NveHostCallbacks::EMPTY;
        (host.bind_host)(handle, &empty_cb);
        let msg = take_last_ffi_panic().unwrap();
        assert!(msg.contains("NveHostApi::bind_host") && msg.contains("boom in bind_host"));

        set_panic_target(PanicTarget::DispatchHostEvents);
        assert_eq!(
            (host.dispatch_host_events)(handle, NveBufferSlice::from_slice(b"host")),
            0
        );
        let msg = take_last_ffi_panic().unwrap();
        assert!(
            msg.contains("NveHostApi::dispatch_host_events")
                && msg.contains("boom in dispatch_host_events")
        );

        // 4. Panicking `destroy` (Drop) survives without aborting and records the message.
        set_panic_target(PanicTarget::Destroy);
        (surface.destroy)(handle);
        let msg = take_last_ffi_panic().unwrap();
        assert!(
            msg.contains("NveSurfaceApi::destroy")
                && msg.contains("boom in destroy for surface(42,0)")
        );
        set_panic_target(PanicTarget::None);
        assert_eq!(last_ffi_panic(), None);
    }
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

    fn dispatch_events(&mut self, _events: NveEventBatchRef<'_>) -> bool {
        false
    }

    fn set_props(&mut self, props: NvePropsRef<'_>) -> bool {
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
fn direct_set_props_reads_nested_maps_and_arrays_zero_copy() {
    let mut app = DirectOnlyPropsApp::create(SurfaceId::ROOT);
    let nested_entries = [NveMapEntry::new("active", NveValue::bool(true))];
    let tag_elems = [NveValue::from_str("a"), NveValue::from_str("b")];
    let entries = [
        NveMapEntry::new("count", NveValue::i64(77)),
        NveMapEntry::new("route", NveValue::from_str("/docs/abi")),
        NveMapEntry::new("meta", NveValue::map(&nested_entries)),
        NveMapEntry::new("tags", NveValue::array(&tag_elems)),
    ];

    // SAFETY: `entries` and its nested backing slices outlive the synchronous `set_props` call.
    assert!(app.set_props(unsafe { NvePropsRef::from_slice(&entries) }));
    assert_eq!(app.count, 77);
    assert_eq!(app.route, "/docs/abi");
    assert!(app.nested_flag);
    assert_eq!(app.tag_count, 2);
    assert!(!app.set_props(NvePropsRef::EMPTY));
}
