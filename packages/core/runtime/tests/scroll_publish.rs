//! `NveSceneView::scroll_offsets`: per-container visual shifts (minus
//! `Scroll::virtual_top_spacer`), sorted by node, republished on the tick after `scroll_by`.

use nve_abi::NveScrollOffset;
use nve_components::tables::{Layout, Scroll};
use nve_engine::Viewport;
use nve_runtime::{RuntimeBuilder, SurfaceRuntime};
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, Length, NodeId, NodeStore};

/// Two 200x100 scroll containers (`plain = 2`, `spaced = 3` with a 30pt virtual top spacer),
/// each with six 50pt rows (max scroll 200).
fn scroll_frame() -> Vec<u8> {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut tx = store.tx();
    let plain = tx.create();
    let spaced = tx.create();
    for (container, spacer) in [(plain, 0.0), (spaced, 30.0)] {
        tx.edit::<Layout>(container)
            .width(Length::points(200.0))
            .height(Length::points(100.0))
            .flex_shrink(0.0);
        tx.edit::<Scroll>(container).virtual_top_spacer(spacer);
        let kids: Vec<NodeId> = (0..6)
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
    }
    tx.set_children(NodeId::ROOT, &[plain, spaced]);
    tx.commit_into(&mut changes);
    let mut out = Vec::new();
    WireEncoder::default().encode(&store, &changes, &mut out);
    out
}

fn published(rt: &SurfaceRuntime) -> (u64, Vec<NveScrollOffset>) {
    let guard = rt.acquire_scene().expect("acquire");
    let v = guard.view();
    let offsets = if v.scroll_count == 0 {
        assert!(v.scroll_offsets.is_null());
        Vec::new()
    } else {
        assert!(!v.scroll_offsets.is_null());
        // SAFETY: `guard` holds the reader lock; the runtime publishes `scroll_count` entries.
        unsafe { std::slice::from_raw_parts(v.scroll_offsets, v.scroll_count as usize) }.to_vec()
    };
    (v.frame_id, offsets)
}

fn shift(offsets: &[NveScrollOffset], node: u32) -> (f32, f32) {
    offsets
        .iter()
        .find(|s| s.node_id == node)
        .map_or((0.0, 0.0), |s| (s.x, s.y))
}

#[test]
fn scroll_offsets_publish_visual_shift_on_next_tick() {
    let mut rt = RuntimeBuilder::new()
        .viewport(Viewport {
            width: 400.0,
            height: 300.0,
            scale_factor: 1.0,
        })
        .build();
    rt.submit_nve2(&scroll_frame()).expect("submit");
    let (frame0, initial) = published(&rt);
    assert!(initial.iter().all(|s| (s.x, s.y) == (0.0, 0.0)), "{initial:?}");

    assert_eq!(rt.scroll_by(3, 0.0, 50.0).expect("scroll spaced"), (0.0, 50.0));
    assert_eq!(rt.scroll_by(2, 0.0, 40.0).expect("scroll plain"), (0.0, 40.0));
    let (frame_before_tick, stale) = published(&rt);
    assert_eq!((frame_before_tick, stale), (frame0, initial), "scroll_by never mutates a published frame");

    assert!(rt.tick(0.0).expect("tick"), "a scroll change republishes");
    let (frame1, offsets) = published(&rt);
    assert!(frame1 > frame0);
    assert!(offsets.windows(2).all(|w| w[0].node_id < w[1].node_id), "sorted by node: {offsets:?}");
    assert_eq!(shift(&offsets, 2), (0.0, 40.0));
    assert_eq!(shift(&offsets, 3), (0.0, 20.0), "virtual_top_spacer is subtracted");

    // Rects stay absolute and pre-scroll: the host applies the shift.
    let guard = rt.acquire_scene().expect("acquire");
    // SAFETY: reader lock held; node 4 (first row of `plain`) < node_capacity.
    let first_row = unsafe { *guard.view().rects.add(4) };
    assert_eq!(first_row.y, 0.0);
    drop(guard);

    // A no-op scroll does not republish.
    assert_eq!(rt.scroll_by(2, 0.0, 0.0).expect("noop"), (0.0, 40.0));
    assert!(!rt.tick(0.0).expect("idle tick"));
    assert_eq!(published(&rt).0, frame1);

    // Clamped to content: the published shift equals the clamped offset; the spacer floors at 0.
    let (_, y) = rt.scroll_by(2, 0.0, 10_000.0).expect("clamp");
    assert_eq!(y, 200.0);
    assert_eq!(rt.scroll_by(3, 0.0, -45.0).expect("into spacer"), (0.0, 5.0));
    assert!(rt.tick(0.0).expect("tick"));
    let (_, offsets) = published(&rt);
    assert_eq!(shift(&offsets, 2), (0.0, 200.0));
    assert_eq!(shift(&offsets, 3), (0.0, 0.0), "shift never goes negative inside the spacer");
}
