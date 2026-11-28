//! Deep unit & edge-case tests for `Transform2D`, `LayoutExt`, CSS Grid tracks/placement,
//! RTL logical inline edges, `compute_sticky_offset`, and table-generic `RowPatch` overlays.

use nve_abi::{Dimension, EdgeInsets};
use nve_tables::layout_ext::{
    compute_sticky_offset, GridTrack,
    LayoutDirection, LayoutExt, LogicalInlineEdges, PositionInsets, PositionMode,
};
use nve_tables::snapshots::transform_from_patch;
use nve_tables::tables::{Display, GridLayout, GridTracks};
use nve_tables::{RowPatch, StylePatchExt};
use nve_store::NodeStore;
use nve_tables::transform::{affine_concat, Transform2D};

fn assert_close(actual: f32, expected: f32, eps: f32, label: &str) {
    assert!(
        (actual - expected).abs() <= eps,
        "{label}: expected {expected}, got {actual}"
    );
}

#[test]
fn transform2d_matrix_map_and_inverse_roundtrip_all_modes() {
    // 1. Identity
    assert!(Transform2D::IDENTITY.is_identity());
    assert!(!Transform2D::IDENTITY.has_custom_origin());
    let id_m = Transform2D::IDENTITY.to_matrix_2d(100.0, 50.0);
    assert_eq!(id_m, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    // 2. Pure translation
    let tr = Transform2D::translate(25.0, -10.0);
    let (mx, my) = tr.map_point(10.0, 20.0, 100.0, 50.0);
    assert_close(mx, 35.0, 1e-4, "tx");
    assert_close(my, 10.0, 1e-4, "ty");
    let (ix, iy) = tr.inverse_map_point(mx, my, 100.0, 50.0).unwrap();
    assert_close(ix, 10.0, 1e-4, "inv tx");
    assert_close(iy, 20.0, 1e-4, "inv ty");

    // 3. Scale around center origin (0.5, 0.5) on 100x100 box
    let sc = Transform2D::scale(2.0);
    assert_eq!(sc.map_point(50.0, 50.0, 100.0, 100.0), (50.0, 50.0));
    assert_eq!(sc.map_point(0.0, 0.0, 100.0, 100.0), (-50.0, -50.0));
    assert_eq!(sc.map_point(100.0, 100.0, 100.0, 100.0), (150.0, 150.0));

    // 4. 90deg clockwise rotation in Y-down screen coordinates around center of 100x100
    let rot = Transform2D::rotate(90.0);
    let (rx, ry) = rot.map_point(100.0, 50.0, 100.0, 100.0);
    assert_close(rx, 50.0, 1e-4, "rot90 x");
    assert_close(ry, 100.0, 1e-4, "rot90 y");
    let (irx, iry) = rot.inverse_map_point(rx, ry, 100.0, 100.0).unwrap();
    assert_close(irx, 100.0, 1e-4, "inv rot90 x");
    assert_close(iry, 50.0, 1e-4, "inv rot90 y");

    // 5. Combined translate + non-uniform scale + rotation + custom origin (0, 0)
    let combo = Transform2D::translate(40.0, -15.0)
        .with_scale_xy(1.5, 0.75)
        .with_rotate(37.0)
        .with_origin(0.0, 0.0);
    assert!(combo.has_custom_origin());
    let (fx, fy) = combo.map_surface_point(120.0, 80.0, 100.0, 50.0, 200.0, 100.0);
    let (bx, by) = combo
        .inverse_map_surface_point(fx, fy, 100.0, 50.0, 200.0, 100.0)
        .unwrap();
    assert_close(bx, 120.0, 1e-3, "combo inv x");
    assert_close(by, 80.0, 1e-3, "combo inv y");

    // 6. Singular scale(0) returns None on inverse
    let zero_scale = Transform2D::scale_xy(0.0, 1.0);
    assert_eq!(zero_scale.inverse_map_point(10.0, 10.0, 100.0, 100.0), None);

    // 7. Matrix concatenation matches sequential transform application
    let m_outer = Transform2D::translate(30.0, 10.0).to_matrix_2d(100.0, 100.0);
    let m_inner = Transform2D::scale(2.0).to_matrix_2d(100.0, 100.0);
    let m_cat = affine_concat(&m_outer, &m_inner);
    let p_seq = nve_tables::transform::affine_map_point(
        &m_outer,
        nve_tables::transform::affine_map_point(&m_inner, 25.0, 40.0).0,
        nve_tables::transform::affine_map_point(&m_inner, 25.0, 40.0).1,
    );
    let p_cat = nve_tables::transform::affine_map_point(&m_cat, 25.0, 40.0);
    assert_close(p_cat.0, p_seq.0, 1e-4, "concat x");
    assert_close(p_cat.1, p_seq.1, 1e-4, "concat y");
}

#[test]
fn rtl_logical_inline_edges_resolve_deterministically() {
    let physical = EdgeInsets {
        top: 4.0,
        right: 6.0,
        bottom: 8.0,
        left: 2.0,
    };
    let logical = LogicalInlineEdges {
        start: Some(20.0),
        end: Some(10.0),
    };

    let ltr = logical.resolve_into(physical, LayoutDirection::Ltr);
    assert_eq!(
        ltr,
        EdgeInsets {
            top: 4.0,
            right: 10.0,
            bottom: 8.0,
            left: 20.0,
        }
    );

    let rtl = logical.resolve_into(physical, LayoutDirection::Rtl);
    assert_eq!(
        rtl,
        EdgeInsets {
            top: 4.0,
            right: 20.0,
            bottom: 8.0,
            left: 10.0,
        }
    );
}

#[test]
fn sticky_offset_clamps_before_during_and_after_containing_block() {
    // Container y=[100..500] (h=400), sticky header y=120, h=40, scroll viewport y=[0..300]
    let item = (0.0, 120.0, 300.0, 40.0);
    let container = (0.0, 100.0, 300.0, 400.0);
    let viewport = (0.0, 0.0, 300.0, 300.0);
    let insets = PositionInsets {
        top: Dimension::Points(10.0),
        ..PositionInsets::AUTO
    };

    // Before threshold: scroll_y=0 and scroll_y=50
    assert_eq!(
        compute_sticky_offset(item, container, viewport, (0.0, 0.0), insets),
        (0.0, 0.0)
    );
    assert_eq!(
        compute_sticky_offset(item, container, viewport, (0.0, 50.0), insets),
        (0.0, 0.0)
    );

    // Clamped while scrolling: scroll_y=200 -> visual y = 120 - 200 + 90 = 10
    assert_eq!(
        compute_sticky_offset(item, container, viewport, (0.0, 200.0), insets),
        (0.0, 90.0)
    );

    // Pushed out at container bottom: scroll_y=480 -> cb_max_y=460 -> delta=340, visual y=-20
    assert_eq!(
        compute_sticky_offset(item, container, viewport, (0.0, 480.0), insets),
        (0.0, 340.0)
    );
}

#[test]
fn row_patch_merges_and_applies_extended_layout_and_transform() {
    let mut store = NodeStore::new();
    let (cols1, cols3) = {
        let mut tx = store.tx();
        let n1 = tx.create();
        let n3 = tx.create();
        tx.edit::<GridLayout>(n1)
            .template_columns(Some(GridTracks::new(vec![GridTrack::repeat_minmax_zero_fr(1)])));
        tx.edit::<GridLayout>(n3)
            .template_columns(Some(GridTracks::new(vec![GridTrack::repeat_minmax_zero_fr(3)])));
        let mut ch = nve_store::ChangeSet::default();
        tx.commit_into(&mut ch);
        (
            store.row::<GridLayout>(n1).template_columns(),
            store.row::<GridLayout>(n3).template_columns(),
        )
    };

    let p1 = RowPatch::new()
        .display_grid()
        .grid_cols_id(cols1)
        .flex_basis_px(120.0)
        .aspect_ratio(16.0 / 9.0)
        .position(PositionMode::Sticky)
        .inset_top(Dimension::Points(12.0))
        .direction(LayoutDirection::Rtl)
        .padding_inline_start(16.0)
        .translate(4.0, 8.0);
    let p2 = RowPatch::new().grid_cols_id(cols3).scale(1.1);
    let mut merged = p1;
    merged.merge(&p2);

    let mut ext = LayoutExt::default();
    merged.apply_layout_ext_to(&mut ext);
    assert_eq!(ext.display, Display::Grid);
    assert_eq!(merged.get_res_id(GridLayout::TEMPLATE_COLUMNS), Some(cols3));
    let view = store.resource::<GridTracks>(cols3).unwrap();
    assert_eq!(
        view.iter().collect::<Vec<_>>(),
        vec![GridTrack::repeat_minmax_zero_fr(3)]
    );
    assert_eq!(ext.flex_basis, Dimension::Points(120.0));
    assert_close(ext.aspect_ratio.unwrap(), 16.0 / 9.0, 1e-5, "aspect");
    assert_eq!(ext.position, PositionMode::Sticky);
    assert_eq!(ext.insets.top, Dimension::Points(12.0));
    assert_eq!(ext.direction, Some(LayoutDirection::Rtl));
    assert_eq!(ext.padding_inline.start, Some(16.0));
    let tf = transform_from_patch(&merged).unwrap();
    assert_eq!(tf.translate_x, 4.0);
    assert_eq!(tf.translate_y, 8.0);
    assert_eq!(tf.scale_x, 1.1);
    assert_eq!(tf.scale_y, 1.1);
}
