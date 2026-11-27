//! Integration tests for CSS Grid, RTL logical edges, sticky positioning, overflow clipping,
//! and `Transform2D` inverse hit-testing in [`HeadlessBackend`].

use nve_abi::EventMask;
use nve_backend_headless::{GridTrack, HeadlessBackend};
use nve_components::tables::{
    Direction, Display, FlexDirection, GridLayout, GridTracks, Interaction, Layout, Overflow,
    Position, Transform, Visual,
};
use nve_engine::{StoreBackend, Viewport};
use nve_store::{ChangeSet, Length, NodeId, NodeStore};

#[test]
fn sticky_clamping_overflow_clipping_and_transform_hit_test() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = HeadlessBackend::new();
    backend.set_viewport(
        1,
        Viewport {
            width: 300.0,
            height: 200.0,
            scale_factor: 1.0,
        },
    );

    let mut tx = store.tx();
    let scroller = tx.create();
    let sticky_hdr = tx.create();
    let body = tx.create();
    let transformed_btn = tx.create();
    tx.set_children(NodeId::ROOT, &[scroller]);
    tx.set_children(scroller, &[sticky_hdr, body]);
    tx.set_children(body, &[transformed_btn]);

    tx.edit::<Layout>(scroller)
        .width(Length::points(300.0))
        .height(Length::points(200.0));
    tx.edit::<Visual>(scroller).overflow_y(Overflow::Scroll);

    tx.edit::<Layout>(sticky_hdr)
        .position(Position::Sticky)
        .inset_top(Length::points(0.0))
        .flex_shrink(0.0)
        .height(Length::points(40.0));
    // Paint order is DOM order plus `z-index` (position does not stack): the stuck header must
    // paint, and so hit-test, above the later `body` it overlaps once scrolled.
    tx.edit::<Visual>(sticky_hdr).z_index(1);
    tx.edit::<Interaction>(sticky_hdr)
        .event_mask(EventMask::PRESS as u32);

    tx.edit::<Layout>(body)
        .flex_shrink(0.0)
        .height(Length::points(600.0));
    tx.edit::<Layout>(transformed_btn)
        .width(Length::points(80.0))
        .height(Length::points(40.0));
    tx.edit::<Transform>(transformed_btn)
        .translate_x(100.0)
        .translate_y(20.0);
    tx.edit::<Interaction>(transformed_btn)
        .event_mask(EventMask::PRESS as u32);
    tx.commit_into(&mut changes);

    backend.commit(&store, &changes).unwrap();

    // Without scroll, transformed_btn sits at (100..180, 60..100).
    assert_eq!(backend.hit_test(&store, 140.0, 80.0), Some(transformed_btn.raw()));
    // Original untransformed rect (10, 60) should miss!
    assert_eq!(backend.hit_test(&store, 10.0, 60.0), None);

    // Scroll down by 120px: sticky header clamps to top of scroller (dy = +120px) so y=10 still hits sticky_hdr!
    backend.set_scroll_offset(scroller.raw(), 0.0, 120.0);
    let (dx, dy) = backend.compute_sticky_delta(&store, sticky_hdr);
    assert_eq!((dx, dy), (0.0, 120.0));
    assert_eq!(backend.hit_test(&store, 50.0, 10.0), Some(sticky_hdr.raw()));
}

#[test]
fn grid_and_rtl_layout_via_nodestore() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = HeadlessBackend::new();
    backend.set_viewport(
        1,
        Viewport {
            width: 300.0,
            height: 200.0,
            scale_factor: 1.0,
        },
    );

    let mut tx = store.tx();
    let grid = tx.create();
    let a = tx.create();
    let b = tx.create();
    let rtl = tx.create();
    let item = tx.create();
    tx.set_children(NodeId::ROOT, &[grid, rtl]);
    tx.set_children(grid, &[a, b]);
    tx.set_children(rtl, &[item]);

    tx.edit::<Layout>(grid)
        .display(Display::Grid)
        .height(Length::points(60.0))
        .column_gap(Length::points(20.0));
    tx.edit::<GridLayout>(grid)
        .template_columns(Some(GridTracks::new(vec![
            GridTrack::Fr(1.0),
            GridTrack::Fr(1.0),
        ])));

    tx.edit::<Layout>(rtl)
        .direction(Direction::Rtl)
        .flex_direction(FlexDirection::Row)
        .height(Length::points(40.0))
        .padding_left(Length::points(10.0))
        .padding_inline_logical(true);
    tx.edit::<Layout>(item)
        .width(Length::points(50.0))
        .height(Length::points(40.0));
    tx.commit_into(&mut changes);

    backend.commit(&store, &changes).unwrap();

    let ra = backend.layout_of(a.raw()).unwrap();
    let rb = backend.layout_of(b.raw()).unwrap();
    assert_eq!(ra.width, 140.0);
    assert_eq!(rb.x, 160.0);
    assert_eq!(rb.width, 140.0);

    let r_item = backend.layout_of(item.raw()).unwrap();
    assert_eq!(r_item.x, 240.0); // 300 - 10 (inline-start) - 50 (width)
}
