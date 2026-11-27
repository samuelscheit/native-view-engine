//! C ABI contracts of published resources (`NveRuntimeSceneView::resources` /
//! `dirty_resources`) and the host text measurer (`set_text_measurer`).

use std::ffi::c_void;
use std::sync::Mutex;

use nve_abi::{
    NveBufferSlice, NveResourceView, NveRuntimeApi, NveRuntimeHandle, NveRuntimeSceneView,
    NveRuntimeStatus, NveStrSlice, NVE_RUNTIME_ABI_VERSION,
};
use nve_components::tables::{AlignItems, BoxShadow, BoxShadows, Image, ImageSrc, Layout, TextContent, Visual};
use nve_runtime::nve_runtime_get_interface;
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Resource, ResourceId, Rgba, Transaction};

const OK: i32 = NveRuntimeStatus::Ok.as_i32();

fn api() -> NveRuntimeApi {
    let ptr = nve_runtime_get_interface(NVE_RUNTIME_ABI_VERSION);
    assert!(!ptr.is_null());
    // SAFETY: Static vtable for the matching ABI version.
    unsafe { *ptr }
}

/// Source store + encoder producing successive `NVE2` frames for the runtime under test.
struct Source {
    store: NodeStore,
    enc: WireEncoder,
}

impl Source {
    fn new() -> Self {
        Self { store: NodeStore::new(), enc: WireEncoder::default() }
    }

    fn frame(&mut self, f: impl FnOnce(&mut Transaction<'_>)) -> Vec<u8> {
        let mut changes = ChangeSet::default();
        let mut tx = self.store.tx();
        f(&mut tx);
        tx.commit_into(&mut changes);
        let mut out = Vec::new();
        self.enc.encode(&self.store, &changes, &mut out);
        out
    }
}

fn submit(api: &NveRuntimeApi, h: NveRuntimeHandle, frame: &[u8]) {
    // SAFETY: Live handle on its owner thread; `frame` outlives the call.
    assert_eq!(unsafe { (api.submit_nve2)(h, NveBufferSlice::from_slice(frame)) }, OK);
}

/// Runs `f` on the acquired scene, then releases it.
fn with_scene<R>(api: &NveRuntimeApi, h: NveRuntimeHandle, f: impl FnOnce(&NveRuntimeSceneView) -> R) -> R {
    let mut view = NveRuntimeSceneView::default();
    // SAFETY: Live handle; stack out pointer.
    assert_eq!(unsafe { (api.scene_acquire)(h, &mut view) }, OK);
    let r = f(&view);
    // SAFETY: Matches the acquire above.
    assert_eq!(unsafe { (api.scene_release)(h) }, OK);
    r
}

fn resource(view: &NveRuntimeSceneView, id: ResourceId) -> NveResourceView {
    assert!(id.0 < view.resource_capacity, "{id:?} past capacity {}", view.resource_capacity);
    // SAFETY: `resources` holds `resource_capacity` views while the scene is acquired.
    unsafe { *view.resources.add(id.0 as usize) }
}

fn bytes(view: &NveResourceView) -> Vec<u8> {
    if view.len == 0 {
        assert!(view.bytes.is_null());
        return Vec::new();
    }
    // SAFETY: `bytes` holds `len` bytes while the scene is acquired.
    unsafe { std::slice::from_raw_parts(view.bytes, view.len as usize) }.to_vec()
}

fn dirty(view: &NveRuntimeSceneView) -> Vec<u32> {
    if view.dirty_resource_count == 0 {
        return Vec::new();
    }
    // SAFETY: `dirty_resources` holds `dirty_resource_count` ids while the scene is acquired.
    unsafe { std::slice::from_raw_parts(view.dirty_resources, view.dirty_resource_count as usize) }.to_vec()
}

fn encoded<R: Resource>(r: &R) -> Vec<u8> {
    let mut out = Vec::new();
    r.encode(&mut out);
    out
}

#[test]
fn scene_publishes_live_resource_bytes_and_defined_ids() {
    let api = api();
    // SAFETY: Null config uses defaults.
    let h = unsafe { (api.create)(std::ptr::null()) };
    let mut src = Source::new();
    let logo = ImageSrc::Url("asset://logo.png".into());
    let shadows = BoxShadows::new(vec![BoxShadow { offset_x: 0.0, offset_y: 2.0, blur_radius: 4.0, spread_radius: 0.0, color: Rgba::rgb(0, 0, 0), inset: false }]);
    let (mut image, mut card) = (NodeId::ROOT, NodeId::ROOT);
    let frame = src.frame(|tx| {
        image = tx.create();
        card = tx.create();
        tx.edit::<Image>(image).src(Some(logo.clone()));
        tx.edit::<Visual>(card).box_shadows(Some(shadows.clone()));
        tx.set_children(NodeId::ROOT, &[image, card]);
    });
    submit(&api, h, &frame);
    let src_img = src.store.try_row::<Image>(image).unwrap().src();
    let src_shadow = src.store.try_row::<Visual>(card).unwrap().box_shadows();
    let first_ptr = with_scene(&api, h, |v| {
        let img = resource(v, src_img);
        assert_eq!((img.kind_lo, img.kind_hi), (ImageSrc::KIND.lo(), ImageSrc::KIND.hi()));
        assert_eq!(bytes(&img), encoded(&logo));
        assert_eq!(img.flags, 0);
        let sh = resource(v, src_shadow);
        assert_eq!((sh.kind_lo, sh.kind_hi), (BoxShadows::KIND.lo(), BoxShadows::KIND.hi()));
        assert_eq!(bytes(&sh), encoded(&shadows));
        let empty = resource(v, ResourceId::NONE);
        assert!(empty.len == 0 && empty.bytes.is_null() && empty.kind_lo == 0, "slot 0 stays empty");
        let mut ids = dirty(v);
        ids.sort_unstable();
        let mut want = vec![src_img.0, src_shadow.0];
        want.sort_unstable();
        assert_eq!(ids, want, "both resources were defined this frame");
        img.bytes
    });

    // A frame that touches no resource: bytes stay borrowed in place, nothing is dirty.
    let frame = src.frame(|tx| {
        tx.edit::<Layout>(card).width(Length::points(40.0));
    });
    submit(&api, h, &frame);
    with_scene(&api, h, |v| {
        assert!(dirty(v).is_empty());
        assert_eq!(resource(v, src_img).bytes, first_ptr, "unchanged resources are not copied");
    });

    // Swapping the image releases the old id and defines the new one.
    let hero = ImageSrc::EncodedBytes(vec![0x89, b'P', b'N', b'G'].into());
    let frame = src.frame(|tx| {
        tx.edit::<Image>(image).src(Some(hero.clone()));
    });
    submit(&api, h, &frame);
    let new_img = src.store.try_row::<Image>(image).unwrap().src();
    with_scene(&api, h, |v| {
        assert_eq!(bytes(&resource(v, new_img)), encoded(&hero));
        assert!(dirty(v).contains(&new_img.0));
        if new_img != src_img {
            let old = resource(v, src_img);
            assert!(old.len == 0 && old.bytes.is_null() && old.kind_lo == 0, "released slot is empty");
        }
    });
    // SAFETY: Live handle destroyed once.
    unsafe { (api.destroy)(h) };
}

#[derive(Default)]
struct MeasureLog {
    calls: Vec<(String, f32, u32, f32)>,
    fail: bool,
    nan: bool,
}

unsafe extern "C" fn host_measure(
    ctx: *mut c_void,
    text: NveStrSlice,
    font_size: f32,
    font_weight: u32,
    max_width: f32,
    out_w: *mut f32,
    out_h: *mut f32,
) -> i32 {
    // SAFETY: `ctx` is the test's `Mutex<MeasureLog>`, alive until the measurer is removed.
    let log = unsafe { &*ctx.cast::<Mutex<MeasureLog>>() };
    let mut log = log.lock().expect("measure log lock");
    // SAFETY: `text` borrows valid UTF-8 for the call.
    let s = unsafe { std::str::from_utf8(std::slice::from_raw_parts(text.ptr, text.len)) }.expect("measured text is UTF-8");
    log.calls.push((s.to_owned(), font_size, font_weight, max_width));
    if log.fail {
        return 1;
    }
    // SAFETY: Valid out pointers for the call.
    unsafe {
        *out_w = if log.nan { f32::NAN } else { 7.0 * s.len() as f32 };
        *out_h = 21.0;
    }
    0
}

fn rect_of(api: &NveRuntimeApi, h: NveRuntimeHandle, id: NodeId) -> (f32, f32) {
    with_scene(api, h, |v| {
        assert!(id.index() < v.node_capacity as usize);
        // SAFETY: `rects` holds `node_capacity` rects while the scene is acquired.
        let r = unsafe { *v.rects.add(id.index()) };
        (r.width, r.height)
    })
}

#[test]
fn host_text_measurer_sizes_text_and_falls_back_to_the_heuristic() {
    let api = api();
    // SAFETY: Null config uses defaults.
    let h = unsafe { (api.create)(std::ptr::null()) };
    let mut src = Source::new();
    let mut label = NodeId::ROOT;
    let frame = src.frame(|tx| {
        let row = tx.create();
        tx.edit::<Layout>(row).width(Length::points(300.0)).align_items(AlignItems::FlexStart);
        label = tx.create();
        tx.edit::<TextContent>(label).text("hello");
        tx.edit::<Visual>(label).background(Rgba::rgb(1, 2, 3));
        tx.set_children(row, &[label]);
        tx.set_children(NodeId::ROOT, &[row]);
    });
    submit(&api, h, &frame);
    let heuristic = rect_of(&api, h, label);

    let log = Box::new(Mutex::new(MeasureLog::default()));
    let ctx = std::ptr::from_ref::<Mutex<MeasureLog>>(&log).cast_mut().cast::<c_void>();
    // SAFETY: `log` outlives the measurer (removed below before it drops).
    assert_eq!(unsafe { (api.set_text_measurer)(h, Some(host_measure), ctx) }, OK);
    assert_eq!(rect_of(&api, h, label), (35.0, 21.0), "installing relayouts with the host sizes");
    {
        let log = log.lock().unwrap();
        let (text, size, weight, max_w) = log.calls.last().cloned().unwrap();
        assert_eq!(text, "hello");
        assert_eq!((size, weight), (16.0, 400), "inherited default typography");
        assert!(max_w == f32::INFINITY || max_w == 0.0 || max_w > 0.0, "contract-encoded width: {max_w}");
        assert!(log.calls.iter().all(|c| !c.3.is_nan() && c.3 >= 0.0), "max_width is never NaN / negative");
    }

    // A failing call or a non-finite size falls back to the heuristic.
    log.lock().unwrap().fail = true;
    // SAFETY: As above; reinstalling relayouts.
    assert_eq!(unsafe { (api.set_text_measurer)(h, Some(host_measure), ctx) }, OK);
    assert_eq!(rect_of(&api, h, label), heuristic);
    {
        let mut l = log.lock().unwrap();
        l.fail = false;
        l.nan = true;
    }
    // SAFETY: As above.
    assert_eq!(unsafe { (api.set_text_measurer)(h, Some(host_measure), ctx) }, OK);
    assert_eq!(rect_of(&api, h, label), heuristic);

    // Removing restores the heuristic and never calls the host again.
    // SAFETY: Clearing the measurer; `ctx` is ignored.
    assert_eq!(unsafe { (api.set_text_measurer)(h, None, std::ptr::null_mut()) }, OK);
    let calls = log.lock().unwrap().calls.len();
    assert_eq!(rect_of(&api, h, label), heuristic);
    let frame = src.frame(|tx| {
        tx.edit::<TextContent>(label).text("hello world");
    });
    submit(&api, h, &frame);
    assert_eq!(log.lock().unwrap().calls.len(), calls, "removed measurer is never called");

    // SAFETY: Null handle is rejected without dereferencing.
    let rc = unsafe { (api.set_text_measurer)(std::ptr::null_mut(), Some(host_measure), ctx) };
    assert_eq!(rc, NveRuntimeStatus::InvalidArg.as_i32());
    // SAFETY: Live handle destroyed once.
    unsafe { (api.destroy)(h) };
    drop(log);
}
