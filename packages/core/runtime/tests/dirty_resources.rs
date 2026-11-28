//! `NveSceneView::dirty_resources`: every frame lists exactly the live resource ids whose
//! bytes changed (created, recycled from the free list), so hosts can key caches by resource id.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use nve_abi::{FrameId, NveEventBatchRef};
use nve_components::tables::{Image, ImageSrc};
use nve_runtime::{AppFrame, RuntimeApp, RuntimeBuilder, SurfaceRuntime};
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, NodeId, NodeStore, ResourceId};

struct Session {
    store: NodeStore,
    encoder: WireEncoder,
    image: NodeId,
}

impl Session {
    fn new() -> Self {
        Self { store: NodeStore::new(), encoder: WireEncoder::default(), image: NodeId::ROOT }
    }

    /// One wire frame of `edit`, as a host-submitted `NVE2` delta.
    fn frame(&mut self, edit: impl FnOnce(&mut nve_store::Transaction<'_>, &mut NodeId)) -> Vec<u8> {
        let mut changes = ChangeSet::default();
        let mut tx = self.store.tx();
        edit(&mut tx, &mut self.image);
        tx.commit_into(&mut changes);
        let mut out = Vec::new();
        self.encoder.encode(&self.store, &changes, &mut out);
        out
    }

    fn set_src(&mut self, url: &str) -> Vec<u8> {
        let url = url.to_owned();
        self.frame(move |tx, image| {
            tx.edit::<Image>(*image).src(Some(ImageSrc::Url(url)));
        })
    }
}

/// `(dirty_resources, resource bytes of id)` of the published frame.
fn published(rt: &SurfaceRuntime, id: u32) -> (Vec<u32>, Vec<u8>) {
    let (dirty, bytes) = published_any(rt, Some(id));
    (dirty, bytes)
}

fn dirty_only(rt: &SurfaceRuntime) -> Vec<u32> {
    published_any(rt, None).0
}

fn published_any(rt: &SurfaceRuntime, id: Option<u32>) -> (Vec<u32>, Vec<u8>) {
    let guard = rt.acquire_scene().expect("acquire");
    let v = guard.view();
    let dirty = if v.dirty_resource_count == 0 {
        assert!(v.dirty_resources.is_null());
        Vec::new()
    } else {
        // SAFETY: the guard keeps the published slices alive.
        unsafe { std::slice::from_raw_parts(v.dirty_resources, v.dirty_resource_count as usize) }.to_vec()
    };
    let bytes = id
        .and_then(|id| guard.resource_payload(ResourceId(id)))
        .map(|(_, b)| b.to_vec())
        .unwrap_or_default();
    (dirty, bytes)
}

#[test]
fn recycled_and_new_resource_ids_are_listed_and_untouched_ones_are_not() {
    let mut rt = RuntimeBuilder::new().build();
    let mut s = Session::new();
    let first = s.frame(|tx, image| {
        let n = tx.create();
        tx.edit::<Image>(n).src(Some(ImageSrc::Url("asset://a.png".into())));
        tx.set_children(NodeId::ROOT, &[n]);
        *image = n;
    });
    rt.submit_nve2(&first).expect("keyframe");
    let created = dirty_only(&rt);
    assert_eq!(created.len(), 1, "a created resource is dirty");
    let a = created[0];
    let (_, bytes_a) = published(&rt, a);
    assert!(!bytes_a.is_empty());

    // A frame that touches no resource lists none.
    let touch = s.frame(|tx, _| {
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
    });
    rt.submit_nve2(&touch).expect("plain delta");
    assert_eq!(published(&rt, a).0, Vec::<u32>::new(), "untouched resources stay clean");

    // Swapping the source releases `a` (freed at frame end) and defines a new id.
    rt.submit_nve2(&s.set_src("asset://b.png")).expect("swap to b");
    let swapped = dirty_only(&rt);
    assert_eq!(swapped.len(), 1);
    let b = swapped[0];
    assert_ne!(a, b, "the released id is still pending free this frame");
    assert!(published(&rt, a).1.is_empty(), "the released id is no longer live");

    // The next new source recycles `a`'s slot: same id, different bytes, so it MUST be dirty.
    rt.submit_nve2(&s.set_src("asset://c.png")).expect("swap to c");
    let (dirty, bytes_c) = published(&rt, a);
    assert_eq!(dirty, vec![a], "the freed slot is recycled and reported dirty");
    assert_ne!(bytes_c, bytes_a, "the recycled id carries new bytes");
}

/// Direct-store app rewriting one ephemeral `Image::src` in place on every render: the same id
/// is (re)defined with new bytes. `pending` renders run as async work inside the NEXT tick, so
/// several renders merge into one published frame.
struct EphemeralApp {
    store: NodeStore,
    changes: ChangeSet,
    node: NodeId,
    generation: u32,
    pending: Arc<AtomicU32>,
}

impl EphemeralApp {
    fn new(pending: Arc<AtomicU32>) -> Self {
        let mut store = NodeStore::new();
        let mut changes = ChangeSet::default();
        let mut tx = store.tx();
        let node = tx.create();
        tx.edit::<Image>(node).src_ephemeral(ImageSrc::Url("gen-0".into()));
        tx.set_children(NodeId::ROOT, &[node]);
        tx.commit_into(&mut changes);
        Self { store, changes, node, generation: 0, pending }
    }
}

impl RuntimeApp for EphemeralApp {
    fn render_frame(&mut self, _frame_id: FrameId) -> AppFrame<'_> {
        if self.generation > 0 {
            let mut tx = self.store.tx();
            tx.edit::<Image>(self.node).src_ephemeral(ImageSrc::Url(format!("gen-{}", self.generation)));
            tx.commit_into(&mut self.changes);
        }
        self.generation += 1;
        AppFrame::Direct { changes: &self.changes }
    }

    fn dispatch_events(&mut self, _events: NveEventBatchRef<'_>) -> bool {
        false
    }

    fn poll_async_work(&mut self) -> bool {
        self.pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1))
            .is_ok()
    }

    fn direct_store(&self) -> Option<&NodeStore> {
        Some(&self.store)
    }

    fn direct_store_mut(&mut self) -> Option<&mut NodeStore> {
        Some(&mut self.store)
    }
}

fn mounted_ephemeral_app() -> (SurfaceRuntime, Arc<AtomicU32>, u32, Vec<u8>) {
    let pending = Arc::new(AtomicU32::new(0));
    let mut rt = RuntimeBuilder::new().app(Box::new(EphemeralApp::new(Arc::clone(&pending)))).build();
    let _ = rt.tick(0.0).expect("mount");
    let created = dirty_only(&rt);
    assert_eq!(created.len(), 1, "the created ephemeral resource is dirty: {created:?}");
    let (_, bytes) = published(&rt, created[0]);
    assert!(!bytes.is_empty());
    (rt, pending, created[0], bytes)
}

#[test]
fn direct_store_ephemeral_rewrites_in_place_are_dirty_every_frame() {
    let (mut rt, pending, id, mut prev) = mounted_ephemeral_app();
    for frame in 1..=3 {
        pending.store(1, Ordering::Release);
        assert!(rt.tick(0.0).expect("rewrite"), "frame {frame} changed");
        let (dirty, bytes) = published(&rt, id);
        assert_eq!(dirty, vec![id], "frame {frame}: an in-place rewrite keeps the id and MUST be dirty");
        assert_ne!(bytes, prev, "frame {frame}: the same id carries new bytes");
        prev = bytes;
    }
    assert!(!rt.tick(0.0).expect("idle"), "nothing pending: no new frame");
}

#[test]
fn rewrites_merged_across_pumps_of_one_tick_are_listed_once() {
    let (mut rt, pending, id, _) = mounted_ephemeral_app();
    pending.store(3, Ordering::Release);
    assert!(rt.tick(0.0).expect("three renders in one tick"));
    assert_eq!(pending.load(Ordering::Acquire), 0, "every pending render ran in this tick");
    let (dirty, bytes) = published(&rt, id);
    assert_eq!(dirty, vec![id], "one id rewritten by several pumps is listed once");
    assert!(bytes.ends_with(b"gen-3"), "the last rewrite wins: {bytes:?}");
}
