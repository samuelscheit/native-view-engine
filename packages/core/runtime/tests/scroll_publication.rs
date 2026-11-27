//! Published scroll offsets: `NveRuntimeSceneView::scroll_offsets` carries the visual child shift
//! of every scrolled container (host `scroll_by`, app-requested scroll, virtualized spacers) while
//! rects stay pre-scroll.

use nve_abi::NveScrollOffset;
use nve_components::tables::{Layout, Scroll};
use nve_engine::Viewport;
use nve_runtime::{RuntimeBuilder, SurfaceRuntime};
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, Length, NodeId, NodeStore};

/// Root → scroll container (id 2, 200×100) → `count` 50pt rows; `scroll` edits its `Scroll` row.
fn frame(count: usize, scroll: impl Fn(&mut nve_store::Transaction<'_>, NodeId)) -> Vec<u8> {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    {
        let mut tx = store.tx();
        let container = tx.create();
        tx.edit::<Layout>(container)
            .width(Length::points(200.0))
            .height(Length::points(100.0));
        scroll(&mut tx, container);
        let kids: Vec<NodeId> = (0..count)
            .map(|_| {
                let item = tx.create();
                tx.edit::<Layout>(item)
                    .width(Length::points(200.0))
                    .height(Length::points(50.0))
                    .flex_shrink(0.0);
                item
            })
            .collect();
        tx.set_children(container, &kids);
        tx.set_children(NodeId::ROOT, &[container]);
        tx.commit_into(&mut changes);
    }
    let mut out = Vec::new();
    WireEncoder::default().encode(&store, &changes, &mut out);
    out
}

fn runtime(bytes: &[u8]) -> SurfaceRuntime {
    let mut rt = RuntimeBuilder::new()
        .viewport(Viewport {
            width: 400.0,
            height: 300.0,
            scale_factor: 1.0,
        })
        .build();
    rt.submit_nve2(bytes).expect("submit");
    rt
}

fn published(rt: &SurfaceRuntime) -> (u64, Vec<NveScrollOffset>) {
    let guard = rt.acquire_scene().expect("acquire");
    let v = guard.view();
    let offsets = if v.scroll_count == 0 {
        assert!(v.scroll_offsets.is_null(), "empty scroll slab is null");
        Vec::new()
    } else {
        // SAFETY: `scroll_count` entries stay valid while the reader guard is held.
        unsafe { std::slice::from_raw_parts(v.scroll_offsets, v.scroll_count as usize) }.to_vec()
    };
    (v.frame_id, offsets)
}

fn at(node_id: u32, x: f32, y: f32) -> NveScrollOffset {
    NveScrollOffset { node_id, x, y }
}

#[test]
fn host_scroll_is_published_on_the_next_tick_and_rects_stay_pre_scroll() {
    let mut rt = runtime(&frame(6, |tx, c| {
        tx.edit::<Scroll>(c);
    }));
    let (frame0, none) = published(&rt);
    assert!(none.is_empty(), "nothing scrolled yet");
    let rect_before = {
        let g = rt.acquire_scene().expect("acquire");
        // SAFETY: index 3 (first row) < node_capacity while the guard is held.
        unsafe { *g.view().rects.add(3) }
    };

    assert_eq!(rt.scroll_by(2, 0.0, 75.0).expect("scroll"), (0.0, 75.0));
    assert_eq!(published(&rt), (frame0, Vec::new()), "scroll_by alone never mutates the view");
    assert!(rt.tick(0.0).expect("tick"), "scroll change republishes");
    let (frame1, offsets) = published(&rt);
    assert!(frame1 > frame0);
    assert_eq!(offsets, [at(2, 0.0, 75.0)]);
    let g = rt.acquire_scene().expect("acquire");
    // SAFETY: as above.
    assert_eq!(unsafe { *g.view().rects.add(3) }, rect_before, "rects are pre-scroll");
    drop(g);

    // Clamped to content (6 × 50 − 100 = 200); a no-op delta does not republish.
    assert_eq!(rt.scroll_by(2, 0.0, 1e6).expect("scroll"), (0.0, 200.0));
    assert!(rt.tick(0.0).expect("tick"));
    assert_eq!(published(&rt).1, [at(2, 0.0, 200.0)]);
    assert_eq!(rt.scroll_by(2, 0.0, 10.0).expect("scroll"), (0.0, 200.0));
    assert!(!rt.tick(0.0).expect("tick"), "unchanged offset needs no publish");

    // Back to the origin: zero shifts are omitted.
    assert_eq!(rt.scroll_by(2, 0.0, -1e6).expect("scroll"), (0.0, 0.0));
    assert!(rt.tick(0.0).expect("tick"));
    assert!(published(&rt).1.is_empty(), "zero shift is not published");
}

#[test]
fn app_requested_scroll_and_virtual_spacer_are_published_without_host_scroll() {
    let rt = runtime(&frame(10, |tx, c| {
        tx.edit::<Scroll>(c).requested_scroll_y(120.0);
    }));
    assert_eq!(published(&rt).1, [at(2, 0.0, 120.0)], "requested scroll applied by the runtime");

    let mut rt = runtime(&frame(10, |tx, c| {
        tx.edit::<Scroll>(c).virtual_top_spacer(30.0);
    }));
    rt.scroll_by(2, 0.0, 20.0).expect("scroll");
    assert!(rt.tick(0.0).expect("tick"));
    assert!(published(&rt).1.is_empty(), "shift inside the virtual spacer is zero");
    rt.scroll_by(2, 0.0, 80.0).expect("scroll");
    assert!(rt.tick(0.0).expect("tick"));
    assert_eq!(published(&rt).1, [at(2, 0.0, 70.0)], "spacer subtracted from the visual shift");
}

#[test]
fn removed_scroll_container_drops_its_offset() {
    let mut rt = runtime(&frame(6, |tx, c| {
        tx.edit::<Scroll>(c);
    }));
    rt.scroll_by(2, 0.0, 40.0).expect("scroll");
    assert!(rt.tick(0.0).expect("tick"));
    assert_eq!(published(&rt).1.len(), 1);

    // A fresh full frame (new store) replaces the replica: the old container's offset is gone.
    rt.submit_nve2(&frame(6, |_, _| {})).expect("resubmit");
    assert!(published(&rt).1.is_empty(), "no stale offsets after the container lost Scroll");
}
