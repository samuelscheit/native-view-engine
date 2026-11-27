//! Embedding-host hook contracts: native text measurement, scene-guarded resource payloads, and
//! the optional app capabilities (props, outbound events, state snapshots, hot patches) through
//! both the Rust `SurfaceRuntime` API and the `NveRuntimeApi` C vtable.

use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use nve_abi::{
    NveBufferSlice, NveDirectEvent, NveEventBatch, NveEventBatchRef, NveMapEntry, NvePropsRef,
    NveRuntimeApi, NveRuntimeHandle, NveRuntimeSceneView, NveRuntimeStatus, NveStrSlice, NveValue,
    NVE_RUNTIME_ABI_VERSION,
};
use nve_components::tables::{Image, ImageSrc, Layout, TextContent};
use nve_engine::{HotPatchMsg, NveApp, Viewport};
use nve_layout::TextMeasurer;
use nve_runtime::{
    AppFrame, NveAppAdapter, RuntimeApp, RuntimeBuilder, RuntimeError, SurfaceRuntime,
    nve_runtime_get_interface,
};
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Resource, ResourceId};

const OK: i32 = NveRuntimeStatus::Ok as i32;
const INVALID: i32 = NveRuntimeStatus::InvalidArg as i32;
const UNSUPPORTED: i32 = NveRuntimeStatus::Unsupported as i32;
const BUSY: i32 = NveRuntimeStatus::Busy as i32;
const NOT_FOUND: i32 = NveRuntimeStatus::NotFound as i32;

fn api() -> NveRuntimeApi {
    let ptr = nve_runtime_get_interface(NVE_RUNTIME_ABI_VERSION);
    assert!(!ptr.is_null());
    // SAFETY: Static vtable returned for the matching ABI version.
    unsafe { *ptr }
}

/// RAII C-ABI surface for the duration of one test.
struct Surface {
    api: NveRuntimeApi,
    h: NveRuntimeHandle,
}

impl Surface {
    fn new() -> Self {
        let api = api();
        // SAFETY: NULL config selects defaults.
        let h = unsafe { (api.create)(std::ptr::null()) };
        assert!(!h.is_null());
        Self { api, h }
    }

    fn submit(&self, frame: &[u8]) {
        // SAFETY: Live handle; `frame` outlives the call.
        assert_eq!(unsafe { (self.api.submit_nve2)(self.h, NveBufferSlice::from_slice(frame)) }, OK);
        self.tick();
    }

    fn tick(&self) {
        // SAFETY: Live handle; null out slot allowed.
        assert_eq!(unsafe { (self.api.tick)(self.h, 0.0, std::ptr::null_mut()) }, OK);
    }

    fn acquire(&self) -> NveRuntimeSceneView {
        let mut view = NveRuntimeSceneView::default();
        // SAFETY: Live handle and valid out slot.
        assert_eq!(unsafe { (self.api.scene_acquire)(self.h, &mut view) }, OK);
        view
    }

    fn release(&self) {
        // SAFETY: Live handle.
        assert_eq!(unsafe { (self.api.scene_release)(self.h) }, OK);
    }

    fn rect_height(&self, node: NodeId) -> f32 {
        let view = self.acquire();
        // SAFETY: Reader lock held; index checked against `node_capacity`.
        let h = unsafe {
            assert!(node.index() < view.node_capacity as usize);
            (*view.rects.add(node.index())).height
        };
        self.release();
        h
    }

    fn resource(&self, id: u32) -> (i32, u64, Vec<u8>) {
        let (mut kind, mut payload) = (0u64, NveBufferSlice::EMPTY);
        // SAFETY: Live handle and valid out slots.
        let st = unsafe { (self.api.resource_payload)(self.h, id, &mut kind, &mut payload) };
        // SAFETY: On OK the payload is readable while the caller's scene reader is held.
        let bytes = unsafe { payload.try_as_slice() }.unwrap_or_default().to_vec();
        (st, kind, bytes)
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        // SAFETY: Handle created by `create` and destroyed once on the owner thread.
        unsafe { (self.api.destroy)(self.h) };
    }
}

fn encode(store: &NodeStore, changes: &ChangeSet, enc: &mut WireEncoder) -> Vec<u8> {
    let mut out = Vec::new();
    enc.encode(store, changes, &mut out);
    out
}

/// Root column with one 300pt-wide text leaf (`id = 2`).
fn text_frame() -> (Vec<u8>, NodeId) {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut tx = store.tx();
    let text = tx.create();
    tx.edit::<Layout>(text).width(Length::points(300.0));
    tx.edit::<TextContent>(text).text("hello native text");
    tx.set_children(NodeId::ROOT, &[text]);
    tx.commit_into(&mut changes);
    (encode(&store, &changes, &mut WireEncoder::default()), text)
}

static MEASURE_CALLS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn fixed_measure(
    ctx: *mut c_void,
    text: NveStrSlice,
    _font_size: f32,
    font_weight: u32,
    max_width: f32,
    out_width: *mut f32,
    out_height: *mut f32,
) {
    // SAFETY: `ctx` is the `f32` height the test registered; out slots are runtime-owned.
    unsafe {
        assert_eq!(text.as_str(), Ok("hello native text"));
        assert!((100..=900).contains(&font_weight));
        assert!(max_width.is_infinite() || max_width >= 0.0);
        MEASURE_CALLS.fetch_add(1, Ordering::Relaxed);
        *out_width = 40.0;
        *out_height = *ctx.cast::<f32>();
    }
}

unsafe extern "C" fn nan_measure(
    _ctx: *mut c_void,
    _text: NveStrSlice,
    _font_size: f32,
    _font_weight: u32,
    _max_width: f32,
    out_width: *mut f32,
    out_height: *mut f32,
) {
    // SAFETY: Runtime-owned out slots.
    unsafe {
        *out_width = f32::NAN;
        *out_height = -5.0;
    }
}

#[test]
fn c_abi_text_measurer_drives_layout_sanitizes_and_restores_builtin() {
    let s = Surface::new();
    let (frame, text) = text_frame();
    s.submit(&frame);
    let builtin = s.rect_height(text);
    assert!(builtin > 0.0, "heuristic measurer sizes the text leaf");

    let mut native_height = 37.0f32;
    let ctx = (&raw mut native_height).cast::<c_void>();
    // SAFETY: Live handle; `ctx` outlives the measurer (cleared below).
    assert_eq!(unsafe { (s.api.set_text_measurer)(s.h, Some(fixed_measure), ctx) }, OK);
    s.tick();
    assert!(MEASURE_CALLS.load(Ordering::Relaxed) > 0, "native measurer invoked during layout");
    assert_eq!(s.rect_height(text), 37.0);

    // Hostile sizes collapse to zero instead of poisoning layout with NaN / negatives.
    // SAFETY: Live handle.
    assert_eq!(unsafe { (s.api.set_text_measurer)(s.h, Some(nan_measure), std::ptr::null_mut()) }, OK);
    s.tick();
    assert_eq!(s.rect_height(text), 0.0);

    // SAFETY: Live handle; NULL restores the built-in measurer.
    assert_eq!(unsafe { (s.api.set_text_measurer)(s.h, None, std::ptr::null_mut()) }, OK);
    s.tick();
    assert_eq!(s.rect_height(text), builtin);

    // Writers are refused while a host holds the scene.
    let _ = s.acquire();
    // SAFETY: Live handle.
    assert_eq!(unsafe { (s.api.set_text_measurer)(s.h, None, std::ptr::null_mut()) }, BUSY);
    s.release();
    // SAFETY: Null handle is rejected.
    assert_eq!(unsafe { (s.api.set_text_measurer)(std::ptr::null_mut(), None, std::ptr::null_mut()) }, INVALID);
}

struct HalfMeasurer;

impl TextMeasurer for HalfMeasurer {
    fn measure(&self, _text: &str, _font_size: f32, _max_width: Option<f32>) -> (f32, f32) {
        (10.0, 12.0)
    }
}

#[test]
fn rust_text_measurer_swaps_and_resets() {
    let mut rt = SurfaceRuntime::new(1, Viewport::default());
    let (frame, text) = text_frame();
    rt.submit_nve2(&frame).unwrap();
    rt.tick(0.0).unwrap();
    rt.set_text_measurer(Some(Arc::new(HalfMeasurer))).unwrap();
    rt.tick(0.0).unwrap();
    let height = |rt: &SurfaceRuntime| {
        let g = rt.acquire_scene().unwrap();
        // SAFETY: Reader lock held by `g`; index within `node_capacity`.
        unsafe { (*g.view().rects.add(text.index())).height }
    };
    assert_eq!(height(&rt), 12.0);
    rt.set_text_measurer(None).unwrap();
    rt.tick(0.0).unwrap();
    assert_ne!(height(&rt), 12.0);
}

#[test]
fn resource_payload_borrows_published_bytes_only_under_scene_reader() {
    let s = Surface::new();
    let mut store = NodeStore::new();
    let mut enc = WireEncoder::default();
    let mut changes = ChangeSet::default();
    let mut tx = store.tx();
    let img = tx.create();
    tx.edit::<Image>(img).src(Some(ImageSrc::Url("https://cdn.example/cat.png".into())));
    tx.set_children(NodeId::ROOT, &[img]);
    tx.commit_into(&mut changes);
    s.submit(&encode(&store, &changes, &mut enc));
    let id = store.row::<Image>(img).src();
    let expected = store.resources().encoded(id).unwrap().to_vec();

    // No scene reader: the borrow would have no lifetime anchor.
    assert_eq!(s.resource(id.0).0, INVALID);

    let _ = s.acquire();
    let (st, kind, bytes) = s.resource(id.0);
    assert_eq!(st, OK);
    assert_eq!(kind, ImageSrc::KIND.0);
    assert_eq!(bytes, expected);
    assert_eq!(ImageSrc::decode(&bytes), Some(ImageSrc::Url("https://cdn.example/cat.png".into())));
    assert_eq!(s.resource(0).0, NOT_FOUND, "ResourceId::NONE");
    assert_eq!(s.resource(id.0 + 1000).0, NOT_FOUND, "out of range");
    // SAFETY: Live handle; a null payload slot is rejected, a null kind slot is optional.
    unsafe {
        assert_eq!((s.api.resource_payload)(s.h, id.0, std::ptr::null_mut(), std::ptr::null_mut()), INVALID);
        let mut payload = NveBufferSlice::EMPTY;
        assert_eq!((s.api.resource_payload)(s.h, id.0, std::ptr::null_mut(), &mut payload), OK);
        assert_eq!(payload.len, expected.len());
    }
    s.release();

    // Released resources become stale once the next frame publishes.
    let mut tx = store.tx();
    tx.edit::<Image>(img).src(None);
    tx.commit_into(&mut changes);
    s.submit(&encode(&store, &changes, &mut enc));
    s.tick();
    let _ = s.acquire();
    assert_eq!(s.resource(id.0).0, NOT_FOUND);
    s.release();
}

#[test]
fn rust_scene_guard_resource_payload_matches_store() {
    let mut rt = RuntimeBuilder::new().build();
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut tx = store.tx();
    let img = tx.create();
    tx.edit::<Image>(img).src(Some(ImageSrc::EncodedBytes(Arc::from(&[1u8, 2, 3][..]))));
    tx.set_children(NodeId::ROOT, &[img]);
    tx.commit_into(&mut changes);
    rt.submit_nve2(&encode(&store, &changes, &mut WireEncoder::default())).unwrap();
    rt.tick(0.0).unwrap();
    let id = store.row::<Image>(img).src();
    let g = rt.acquire_scene().unwrap();
    let (kind, bytes) = g.resource_payload(id).unwrap();
    assert_eq!(kind, ImageSrc::KIND);
    assert_eq!(bytes, store.resources().encoded(id).unwrap());
    assert!(g.resource_payload(ResourceId::NONE).is_none());
}

/// In-process app implementing every optional capability, exported through `export_nve_app!`.
#[derive(Default)]
struct CapableApp {
    props: Vec<(String, i64)>,
    emitted: Vec<NveDirectEvent>,
    drained: Vec<NveDirectEvent>,
    snapshot: Vec<u8>,
    restored: Vec<u8>,
    patches: u32,
}

impl NveApp for CapableApp {
    fn create(_surface_id: u32) -> Self {
        Self {
            snapshot: b"state-v1".to_vec(),
            ..Self::default()
        }
    }

    fn render(&mut self, _frame_id: u64) -> &[u8] {
        &[]
    }

    fn dispatch_events(&mut self, _events: NveEventBatchRef<'_>) -> bool {
        false
    }

    fn set_props(&mut self, props: NvePropsRef<'_>) -> bool {
        self.props = props.iter().map(|(k, v)| (k.to_owned(), v.as_i64().unwrap_or(-1))).collect();
        self.emitted = vec![
            NveDirectEvent::press(1, 7, 10, 1.0, 2.0),
            NveDirectEvent::press(1, 8, 11, 3.0, 4.0),
        ];
        true
    }

    fn drain_emitted_events(&mut self) -> &[NveDirectEvent] {
        // The drained batch stays alive until the next call (borrow contract).
        self.drained = std::mem::take(&mut self.emitted);
        &self.drained
    }

    fn apply_hot_patch(&mut self, _patch: &HotPatchMsg) -> bool {
        self.patches += 1;
        true
    }

    fn snapshot_state(&mut self) -> &[u8] {
        &self.snapshot
    }

    fn restore_state(&mut self, state: &[u8]) {
        self.restored = state.to_vec();
    }
}

nve_engine::export_nve_app!(CapableApp, symbol = nve_runtime_test_capable_app_get_interface);

unsafe extern "C" {
    fn nve_runtime_test_capable_app_get_interface(interface_id: u32) -> *const c_void;
}

fn hot_patch_bytes() -> Vec<u8> {
    HotPatchMsg::TemplatePatch {
        generation: 1,
        template_id: "t".into(),
        attr_updates: vec![],
        text_updates: vec![(0, "x".into())],
        target_test_id: None,
        node_path: vec![],
        text_replacements: vec![],
    }
    .to_bytes()
}

#[test]
fn c_abi_app_hooks_report_unsupported_without_app_and_reject_bad_args() {
    let s = Surface::new();
    let entries = [NveMapEntry::new("count", NveValue::i64(3))];
    let patch = hot_patch_bytes();
    let (mut changed, mut events, mut state) = (9u32, NveEventBatch::EMPTY, NveBufferSlice::EMPTY);
    // SAFETY: Live handle and valid buffers/out slots throughout.
    unsafe {
        assert_eq!((s.api.set_props)(s.h, entries.as_ptr(), 1, &mut changed), UNSUPPORTED);
        assert_eq!((s.api.drain_emitted_events)(s.h, &mut events), UNSUPPORTED);
        assert_eq!((s.api.snapshot_state)(s.h, &mut state), UNSUPPORTED);
        assert_eq!((s.api.restore_state)(s.h, NveBufferSlice::from_slice(b"x")), UNSUPPORTED);
        assert_eq!((s.api.apply_hot_patch)(s.h, NveBufferSlice::from_slice(&patch), &mut changed), UNSUPPORTED);
        assert_eq!(changed, 9, "out slots untouched on failure");
        // Malformed inputs fail validation before any app lookup.
        assert_eq!((s.api.apply_hot_patch)(s.h, NveBufferSlice::from_slice(b"NOPE"), &mut changed), INVALID);
        assert_eq!((s.api.set_props)(s.h, std::ptr::null(), 2, &mut changed), INVALID);
        assert_eq!((s.api.drain_emitted_events)(s.h, std::ptr::null_mut()), INVALID);
        assert_eq!((s.api.snapshot_state)(s.h, std::ptr::null_mut()), INVALID);
        let bad = NveBufferSlice { ptr: std::ptr::null(), len: 4 };
        assert_eq!((s.api.restore_state)(s.h, bad), INVALID);
    }
}

#[test]
fn c_abi_app_hooks_forward_to_attached_app() {
    let s = Surface::new();
    // SAFETY: Live handle and exported `nve_get_interface` symbol.
    assert_eq!(unsafe { (s.api.attach_app_interface)(s.h, Some(nve_runtime_test_capable_app_get_interface)) }, OK);
    s.tick();

    let entries = [NveMapEntry::new("count", NveValue::i64(3)), NveMapEntry::new("max", NveValue::i64(9))];
    let mut changed = 0u32;
    // SAFETY: Live handle; `entries` outlives the call.
    assert_eq!(unsafe { (s.api.set_props)(s.h, entries.as_ptr(), 2, &mut changed) }, OK);
    assert_eq!(changed, 1);

    let mut events = NveEventBatch::EMPTY;
    // SAFETY: Live handle; the batch is read before the next runtime call.
    unsafe {
        assert_eq!((s.api.drain_emitted_events)(s.h, &mut events), OK);
        let drained = events.try_as_slice().unwrap();
        assert_eq!(drained.iter().map(|e| e.node_id).collect::<Vec<_>>(), [7, 8]);
        assert_eq!((s.api.drain_emitted_events)(s.h, &mut events), OK);
        assert_eq!(events.len, 0, "draining is destructive");
    }

    let mut state = NveBufferSlice::EMPTY;
    // SAFETY: Live handle and valid out slot; slice read before the next runtime call.
    unsafe {
        assert_eq!((s.api.snapshot_state)(s.h, &mut state), OK);
        assert_eq!(state.try_as_slice(), Some(&b"state-v1"[..]));
        assert_eq!((s.api.restore_state)(s.h, NveBufferSlice::from_slice(b"state-v2")), OK);
        let patch = hot_patch_bytes();
        assert_eq!((s.api.apply_hot_patch)(s.h, NveBufferSlice::from_slice(&patch), &mut changed), OK);
        assert_eq!(changed, 1);
    }

    // Writers are refused while the scene is held.
    let _ = s.acquire();
    // SAFETY: Live handle.
    assert_eq!(unsafe { (s.api.restore_state)(s.h, NveBufferSlice::EMPTY) }, BUSY);
    s.release();
}

#[test]
fn rust_app_hooks_forward_through_nve_app_adapter() {
    let mut rt = RuntimeBuilder::new()
        .app(Box::new(NveAppAdapter::<CapableApp>::create(1)))
        .build();
    rt.tick(0.0).unwrap();
    let entries = [NveMapEntry::new("count", NveValue::i64(3))];
    // SAFETY: `entries` outlives the call.
    let props = unsafe { NvePropsRef::from_slice(&entries) };
    assert!(rt.set_props(props).unwrap());
    assert_eq!(rt.drain_emitted_events().unwrap().len(), 2);
    assert!(rt.drain_emitted_events().unwrap().is_empty());
    assert_eq!(rt.snapshot_state().unwrap(), b"state-v1");
    rt.restore_state(b"state-v2").unwrap();
    assert!(rt.apply_hot_patch(&HotPatchMsg::from_bytes(&hot_patch_bytes()).unwrap()).unwrap());
}

/// Minimal app with no optional capabilities.
struct BareApp;

impl RuntimeApp for BareApp {
    fn render_frame(&mut self, _frame_id: u64) -> AppFrame<'_> {
        AppFrame::Clean
    }

    fn dispatch_events(&mut self, _events: NveEventBatchRef<'_>) -> bool {
        false
    }
}

#[test]
fn rust_app_hooks_map_missing_capabilities_to_unsupported() {
    let mut rt = RuntimeBuilder::new().app(Box::new(BareApp)).build();
    let unsupported = |e: RuntimeError| matches!(e, RuntimeError::Unsupported(_));
    assert!(unsupported(rt.set_props(NvePropsRef::EMPTY).unwrap_err()));
    assert!(unsupported(rt.snapshot_state().unwrap_err()));
    assert!(unsupported(rt.restore_state(b"").unwrap_err()));
    assert!(rt.drain_emitted_events().is_err_and(unsupported));
    let patch = HotPatchMsg::from_bytes(&hot_patch_bytes()).unwrap();
    assert!(unsupported(rt.apply_hot_patch(&patch).unwrap_err()));
}
