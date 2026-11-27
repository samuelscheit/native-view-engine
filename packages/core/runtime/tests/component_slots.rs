//! C ABI contracts of the embedded-component, dev-loop, text-measurer, and resource slots of
//! `NveRuntimeApi` (`set_props`, `drain_emitted_events`, `measure`, `apply_hot_patch`,
//! `snapshot_state`, `restore_state`, `set_text_measurer`, `resource_bytes`).

use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

use nve_abi::{
    NveBufferSlice, NveDirectEvent, NveEventBatch, NveEventBatchRef, NveIntrinsicSize,
    NveMapEntry, NvePropsRef, NveRuntimeApi, NveRuntimeConfig, NveRuntimeHandle,
    NveRuntimeSceneView, NveRuntimeStatus, NveSizeConstraints, NveStrSlice, NveValue,
    NVE_RUNTIME_ABI_VERSION,
};
use nve_components::tables::{Image, ImageSrc, Layout, TextContent};
use nve_engine::{HotPatchMsg, NveApp};
use nve_runtime::nve_runtime_get_interface;
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, Length, NodeId, NodeStore, NveTable, Resource};

const OK: i32 = NveRuntimeStatus::Ok.as_i32();
const IMAGE_URL: &str = "https://cdn.example.com/hero.png";

fn api() -> NveRuntimeApi {
    let ptr = nve_runtime_get_interface(NVE_RUNTIME_ABI_VERSION);
    assert!(!ptr.is_null());
    // SAFETY: Static vtable returned for the matching ABI version.
    unsafe { *ptr }
}

/// App exercising every optional interface: props, outbound events, measure, dev snapshot.
struct ComponentApp {
    store: NodeStore,
    changes: ChangeSet,
    enc: WireEncoder,
    buf: Vec<u8>,
    dirty: bool,
    label: String,
    emitted: Vec<NveDirectEvent>,
    drained: Vec<NveDirectEvent>,
    state: Vec<u8>,
}

impl NveApp for ComponentApp {
    fn create(_surface_id: u32) -> Self {
        Self {
            store: NodeStore::new(),
            changes: ChangeSet::default(),
            enc: WireEncoder::default(),
            buf: Vec::new(),
            dirty: true,
            label: "hello".to_owned(),
            emitted: Vec::new(),
            drained: Vec::new(),
            state: Vec::new(),
        }
    }

    fn render(&mut self, _frame_id: u64) -> &[u8] {
        if !self.dirty {
            return &[];
        }
        self.dirty = false;
        {
            let mut tx = self.store.tx();
            let (text, image) = match (tx.store().id_at(2), tx.store().id_at(3)) {
                (Some(t), Some(i)) => (t, i),
                _ => {
                    let t = tx.create();
                    let i = tx.create();
                    tx.edit::<Layout>(i)
                        .width(Length::points(64.0))
                        .height(Length::points(32.0));
                    tx.edit::<Image>(i).src(Some(ImageSrc::Url(IMAGE_URL.into())));
                    tx.set_children(NodeId::ROOT, &[t, i]);
                    (t, i)
                }
            };
            let _ = image;
            tx.edit::<TextContent>(text).text(&self.label);
            tx.commit_into(&mut self.changes);
        }
        self.buf.clear();
        self.enc.encode(&self.store, &self.changes, &mut self.buf);
        &self.buf
    }

    fn dispatch_events(&mut self, _events: NveEventBatchRef<'_>) -> bool {
        false
    }

    fn measure(&mut self, c: NveSizeConstraints) -> NveIntrinsicSize {
        NveIntrinsicSize { width: c.max_w.min(300.0), height: 42.0 }
    }

    fn set_props(&mut self, props: NvePropsRef<'_>) -> bool {
        let Some(label) = props.get("label").and_then(|v| v.as_str()) else {
            return false;
        };
        if label == self.label {
            return false;
        }
        self.label = label.to_owned();
        self.dirty = true;
        self.emitted.push(NveDirectEvent::press(1, 2, 7, 1.0, 2.0));
        true
    }

    fn drain_emitted_events(&mut self) -> &[NveDirectEvent] {
        // Valid until the next call; the next drain starts empty.
        self.drained.clear();
        std::mem::swap(&mut self.drained, &mut self.emitted);
        &self.drained
    }

    fn apply_hot_patch(&mut self, patch: &HotPatchMsg) -> bool {
        matches!(patch, HotPatchMsg::DevStatus { .. })
    }

    fn snapshot_state(&mut self) -> &[u8] {
        self.state = self.label.as_bytes().to_vec();
        &self.state
    }

    fn restore_state(&mut self, state: &[u8]) {
        self.label = String::from_utf8_lossy(state).into_owned();
        self.dirty = true;
    }
}

nve_engine::export_nve_app!(ComponentApp, symbol = nve_runtime_test_component_app_get_interface);

/// The component app's interface table filtered down to the required surface interface.
unsafe extern "C" fn surface_only_get_interface(id: u32) -> *const c_void {
    // SAFETY: Delegates to the exported interface table, filtering out every optional interface.
    if id == nve_abi::NVE_IFACE_SURFACE { unsafe { nve_runtime_test_component_app_get_interface(id) } } else { std::ptr::null() }
}

unsafe extern "C" {
    fn nve_runtime_test_component_app_get_interface(id: u32) -> *const c_void;
}

struct Surface {
    api: NveRuntimeApi,
    handle: NveRuntimeHandle,
}

impl Surface {
    fn new() -> Self {
        let api = api();
        let cfg = NveRuntimeConfig { width: 320.0, height: 240.0, ..NveRuntimeConfig::default() };
        // SAFETY: Valid config pointer.
        let handle = unsafe { (api.create)(&cfg) };
        assert!(!handle.is_null());
        Self { api, handle }
    }

    fn attach(&self, get_iface: unsafe extern "C" fn(u32) -> *const c_void) {
        // SAFETY: Live handle; static interface table.
        assert_eq!(unsafe { (self.api.attach_app_interface)(self.handle, Some(get_iface)) }, OK);
    }

    fn tick(&self) -> bool {
        let mut changed = 0u32;
        // SAFETY: Live handle; valid out pointer.
        assert_eq!(unsafe { (self.api.tick)(self.handle, 0.016, &mut changed) }, OK);
        changed != 0
    }

    fn with_scene<R>(&self, f: impl FnOnce(&NveRuntimeSceneView) -> R) -> R {
        let mut view = NveRuntimeSceneView::default();
        // SAFETY: Live handle; valid out pointer; released below.
        assert_eq!(unsafe { (self.api.scene_acquire)(self.handle, &mut view) }, OK);
        let out = f(&view);
        // SAFETY: Matches the acquire above.
        assert_eq!(unsafe { (self.api.scene_release)(self.handle) }, OK);
        out
    }

    /// Current `nve.text` string of the label node (published sparse row 0).
    fn label(&self) -> String {
        let slot = u32::from(TextContent::slot().0);
        self.with_scene(|v| {
            // SAFETY: Tables and the text slab are valid until release.
            unsafe {
                let tables = std::slice::from_raw_parts(v.tables, v.table_count as usize);
                let t = tables.iter().find(|t| t.slot == slot).expect("nve.text published");
                let (off, len) = (*t.rows as usize, *t.rows.add(1) as usize);
                let slab = std::slice::from_raw_parts(v.text_slab, v.text_slab_len as usize);
                String::from_utf8_lossy(&slab[off..off + len]).into_owned()
            }
        })
    }

    fn rect_height(&self, index: usize) -> f32 {
        self.with_scene(|v| {
            assert!(index < v.node_capacity as usize);
            // SAFETY: `index < node_capacity` asserted; rects valid until release.
            unsafe { (*v.rects.add(index)).height }
        })
    }

    fn set_label(&self, label: &str) -> (i32, u32) {
        let entries = [NveMapEntry::new("label", NveValue::str(NveStrSlice::from_str(label)))];
        let mut changed = 7u32;
        // SAFETY: `entries` lives for the call; valid out pointer.
        let st = unsafe { (self.api.set_props)(self.handle, entries.as_ptr(), 1, &mut changed) };
        (st, changed)
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        // SAFETY: Handle created in `new` and destroyed once.
        unsafe { (self.api.destroy)(self.handle) };
    }
}

#[test]
fn props_events_measure_snapshot_and_hot_patch_reach_the_app() {
    let s = Surface::new();
    s.attach(nve_runtime_test_component_app_get_interface);
    assert!(s.tick());
    assert!(s.label() == "hello");

    // Props: changed → next tick renders; identical props → unchanged.
    assert_eq!(s.set_label("world"), (OK, 1));
    assert!(s.tick());
    assert_eq!(s.label(), "world");
    assert_eq!(s.set_label("world"), (OK, 0));
    // Null out_changed is allowed; a null entries pointer with count > 0 is rejected.
    // SAFETY: Live handle; invalid args are validated.
    unsafe {
        assert_eq!((s.api.set_props)(s.handle, std::ptr::null(), 0, std::ptr::null_mut()), OK);
        assert_eq!(
            (s.api.set_props)(s.handle, std::ptr::null(), 3, std::ptr::null_mut()),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
    }

    // Outbound events: one press per effective prop change, drained once.
    let mut batch = NveEventBatch::from_slice(&[]);
    // SAFETY: Live handle; valid out pointer.
    assert_eq!(unsafe { (s.api.drain_emitted_events)(s.handle, &mut batch) }, OK);
    // SAFETY: Batch borrowed until the next owner-thread call.
    let events = unsafe { batch.as_slice() };
    assert_eq!(events.len(), 1);
    assert_eq!((events[0].node_id, events[0].x, events[0].y), (2, 1.0, 2.0));
    // SAFETY: Live handle; valid out pointer.
    assert_eq!(unsafe { (s.api.drain_emitted_events)(s.handle, &mut batch) }, OK);
    assert_eq!(batch.len, 0);
    // SAFETY: Null out pointer is rejected, not written.
    let null_out = unsafe { (s.api.drain_emitted_events)(s.handle, std::ptr::null_mut()) };
    assert_eq!(null_out, NveRuntimeStatus::InvalidArg.as_i32());

    // Measure forwards to `NveLayoutApi::measure`.
    let mut size = NveIntrinsicSize::default();
    // SAFETY: Live handle; valid out pointer.
    assert_eq!(unsafe { (s.api.measure)(s.handle, NveSizeConstraints::loose(250.0, 900.0), &mut size) }, OK);
    assert_eq!((size.width, size.height), (250.0, 42.0));

    // Snapshot → restore round trip re-renders the restored state.
    let mut state = NveBufferSlice::from_slice(&[]);
    // SAFETY: Live handle; valid out pointer.
    assert_eq!(unsafe { (s.api.snapshot_state)(s.handle, &mut state) }, OK);
    // SAFETY: Borrowed until the next owner-thread call; copied immediately.
    let saved = unsafe { state.as_slice() }.to_vec();
    assert_eq!(saved, b"world");
    assert_eq!(s.set_label("transient"), (OK, 1));
    assert!(s.tick());
    // SAFETY: Live handle; `saved` outlives the call.
    assert_eq!(unsafe { (s.api.restore_state)(s.handle, NveBufferSlice::from_slice(&saved)) }, OK);
    assert!(s.tick());
    assert!(s.label() == "world");

    // Hot patch: decoded NVP1 frames reach `NveDevApi::apply_hot_patch`; malformed ones are rejected.
    let patch = HotPatchMsg::DevStatus { message: "ok".into(), latency_ms: 1 }.to_bytes();
    let mut changed = 0u32;
    // SAFETY: Live handle; `patch` outlives the call.
    unsafe {
        assert_eq!((s.api.apply_hot_patch)(s.handle, NveBufferSlice::from_slice(&patch), &mut changed), OK);
        assert_eq!(changed, 1);
        assert_eq!(
            (s.api.apply_hot_patch)(s.handle, NveBufferSlice::from_slice(b"junk"), &mut changed),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
    }
}

#[test]
fn optional_interfaces_report_unsupported_and_missing_app_reports_not_found() {
    let s = Surface::new();
    let mut size = NveIntrinsicSize::default();
    let mut batch = NveEventBatch::from_slice(&[]);
    let not_found = NveRuntimeStatus::NotFound.as_i32();
    // SAFETY: Live handle; valid out pointers.
    unsafe {
        assert_eq!((s.api.measure)(s.handle, NveSizeConstraints::loose(1.0, 1.0), &mut size), not_found);
        assert_eq!((s.api.drain_emitted_events)(s.handle, &mut batch), not_found);
    }
    assert_eq!(s.set_label("x").0, not_found);

    s.attach(surface_only_get_interface);
    let unsupported = NveRuntimeStatus::Unsupported.as_i32();
    assert_eq!(s.set_label("x").0, unsupported);
    // SAFETY: Live handle; valid out pointers.
    unsafe {
        assert_eq!((s.api.measure)(s.handle, NveSizeConstraints::loose(1.0, 1.0), &mut size), unsupported);
        assert_eq!((s.api.restore_state)(s.handle, NveBufferSlice::from_slice(b"s")), unsupported);
        // Draining / snapshotting an app without the interface is an empty success.
        assert_eq!((s.api.drain_emitted_events)(s.handle, &mut batch), OK);
        assert_eq!(batch.len, 0);
        let mut state = NveBufferSlice::from_slice(b"x");
        assert_eq!((s.api.snapshot_state)(s.handle, &mut state), OK);
        assert_eq!(state.len, 0);
    }
}

static MEASURE_CALLS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn tall_measurer(
    ctx: *mut c_void,
    text: NveStrSlice,
    font_size: f32,
    font_weight: u16,
    max_width: f32,
) -> NveIntrinsicSize {
    // SAFETY: `ctx` is the `&AtomicUsize` installed below.
    unsafe { &*ctx.cast::<AtomicUsize>() }.fetch_add(1, Ordering::Relaxed);
    // SAFETY: `text` borrows live UTF-8 for the call.
    assert!(!unsafe { text.as_bytes() }.is_empty());
    assert!(font_size > 0.0 && (100..=900).contains(&font_weight));
    assert!(max_width.is_nan() || max_width > 0.0);
    NveIntrinsicSize { width: 40.0, height: 77.0 }
}

#[test]
fn host_text_measurer_drives_layout_and_clearing_restores_the_heuristic() {
    let s = Surface::new();
    s.attach(nve_runtime_test_component_app_get_interface);
    assert!(s.tick());
    let heuristic = s.rect_height(2);
    assert!(heuristic > 0.0 && heuristic < 77.0);

    let ctx = std::ptr::from_ref(&MEASURE_CALLS).cast_mut().cast::<c_void>();
    // SAFETY: Live handle; `MEASURE_CALLS` is static.
    assert_eq!(unsafe { (s.api.set_text_measurer)(s.handle, Some(tall_measurer), ctx) }, OK);
    assert!(s.tick(), "installing a measurer forces a relayout + publish");
    assert!(MEASURE_CALLS.load(Ordering::Relaxed) > 0);
    assert_eq!(s.rect_height(2), 77.0);

    // SAFETY: Live handle; clearing restores the built-in heuristic.
    assert_eq!(unsafe { (s.api.set_text_measurer)(s.handle, None, std::ptr::null_mut()) }, OK);
    assert!(s.tick());
    assert_eq!(s.rect_height(2), heuristic);
}

#[test]
fn resource_bytes_copies_published_image_sources() {
    let s = Surface::new();
    s.attach(nve_runtime_test_component_app_get_interface);
    assert!(s.tick());
    let image_slot = Image::slot().0;
    let src_word = Image::DESC.fields.iter().find(|f| f.name == "src").unwrap().word as usize;
    let resource_id = s.with_scene(|v| {
        // SAFETY: Tables valid until release.
        let tables = unsafe { std::slice::from_raw_parts(v.tables, v.table_count as usize) };
        let t = tables.iter().find(|t| t.slot == u32::from(image_slot)).expect("nve.image published");
        assert_eq!(t.row_count, 1);
        // SAFETY: One row of `words` words.
        unsafe { *t.rows.add(src_word) }
    });

    let mut len = 0usize;
    // SAFETY: Live handle; `cap = 0` only reports the length.
    assert_eq!(unsafe { (s.api.resource_bytes)(s.handle, resource_id, std::ptr::null_mut(), 0, &mut len) }, OK);
    let mut buf = vec![0u8; len];
    let mut small = vec![0u8; len - 1];
    // SAFETY: Buffers valid for their lengths.
    unsafe {
        assert_eq!(
            (s.api.resource_bytes)(s.handle, resource_id, small.as_mut_ptr(), small.len(), &mut len),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
        assert_eq!((s.api.resource_bytes)(s.handle, resource_id, buf.as_mut_ptr(), buf.len(), &mut len), OK);
    }
    assert_eq!(ImageSrc::decode(&buf), Some(ImageSrc::Url(IMAGE_URL.into())));

    // Readable while a scene reader is held (shared lock), and unknown ids are NotFound.
    s.with_scene(|_| {
        let mut again = vec![0u8; len];
        // SAFETY: Buffer valid for `len` bytes.
        unsafe {
            assert_eq!((s.api.resource_bytes)(s.handle, resource_id, again.as_mut_ptr(), len, &mut len), OK);
            assert_eq!(
                (s.api.resource_bytes)(s.handle, 0xFFFF_FF00, again.as_mut_ptr(), len, &mut len),
                NveRuntimeStatus::NotFound.as_i32()
            );
        }
        assert_eq!(again, buf);
    });
    // SAFETY: Null `out_len` is rejected.
    let null_len =
        unsafe { (s.api.resource_bytes)(s.handle, resource_id, std::ptr::null_mut(), 0, std::ptr::null_mut()) };
    assert_eq!(null_len, NveRuntimeStatus::InvalidArg.as_i32());
}

#[test]
fn encode_scene_replays_as_instant_boot_frame0_into_a_fresh_runtime() {
    let live = Surface::new();
    live.attach(nve_runtime_test_component_app_get_interface);
    assert!(live.tick());
    assert_eq!(live.set_label("persisted"), (OK, 1));
    assert!(live.tick());

    let mut len = 0usize;
    // SAFETY: Live handle; `cap = 0` only sizes.
    assert_eq!(unsafe { (live.api.encode_scene)(live.handle, std::ptr::null_mut(), 0, &mut len) }, OK);
    assert!(len > 24, "a keyframe carries the NVE2 header plus ops");
    let mut frame0 = vec![0u8; len];
    // SAFETY: Buffer valid for `len` bytes.
    assert_eq!(unsafe { (live.api.encode_scene)(live.handle, frame0.as_mut_ptr(), len, &mut len) }, OK);
    assert_eq!(&frame0[..4], b"NVE2");
    let mut small = vec![0u8; 8];
    // SAFETY: Buffer valid for 8 bytes; rejected as too small.
    let too_small = unsafe { (live.api.encode_scene)(live.handle, small.as_mut_ptr(), small.len(), &mut len) };
    assert_eq!(too_small, NveRuntimeStatus::InvalidArg.as_i32());

    // Cold start: replay Frame-0 before any app attaches; the scene is complete immediately.
    let cold = Surface::new();
    // SAFETY: Live handle; `frame0` outlives the call.
    assert_eq!(unsafe { (cold.api.submit_nve2)(cold.handle, NveBufferSlice::from_slice(&frame0)) }, OK);
    cold.tick(); // `submit_nve2` already published the replayed frame
    assert_eq!(cold.label(), "persisted");
    assert_eq!(cold.rect_height(3), live.rect_height(3), "layout state replays too");
    // Encoding is a pure read: available while a reader holds the scene.
    cold.with_scene(|_| {
        // SAFETY: Live handle; sizing only.
        assert_eq!(unsafe { (cold.api.encode_scene)(cold.handle, std::ptr::null_mut(), 0, &mut len) }, OK);
    });
}
