//! Integration tests for Dioxus `TypedStyle` & `StylePatch` layout-extension, CSS Grid,
//! RTL logical edges, sticky positioning, and `Transform2D` propagation into `RenderSink`.

use nve_abi::{ComponentKind, Dimension, FrameId, LayoutProps, NodeId, RawPropValueRef, StyleProps, SurfaceId};
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;
use nve_engine::{replay_command_buffer_into, BackendApplyStats, BackendError, RenderSink};

#[derive(Default)]
struct LayoutExtSink {
    layout_exts: Vec<(NodeId, LayoutExt)>,
    transforms: Vec<(NodeId, Transform2D)>,
    responsive_rules: Vec<(NodeId, Vec<ResponsiveRule>)>,
    state_rules: Vec<(NodeId, Vec<StateStyleRule>)>,
}

impl RenderSink for LayoutExtSink {
    fn begin_frame(&mut self, _: SurfaceId, _: FrameId) {}
    fn create_node(&mut self, _: NodeId, _: ComponentKind, _: Option<&str>) {}
    fn remove_node(&mut self, _: NodeId) {}
    fn set_children(&mut self, _: NodeId, _: &[NodeId]) {}
    fn set_text(&mut self, _: NodeId, _: &str) {}
    fn set_layout(&mut self, _: NodeId, _: &LayoutProps) {}
    fn set_style(&mut self, _: NodeId, _: &StyleProps) {}
    fn set_raw_prop(&mut self, _: NodeId, _: &str, _: RawPropValueRef<'_>) {}
    fn set_event_mask(&mut self, _: NodeId, _: u64) {}
    fn dispatch_command(&mut self, _: NodeId, _: &str, _: &[u8]) {}
    fn set_layout_ext(&mut self, node_id: NodeId, ext: Option<&LayoutExt>) {
        if let Some(e) = ext {
            self.layout_exts.push((node_id, e.clone()));
        }
    }
    fn set_transform(&mut self, node_id: NodeId, transform: Option<Transform2D>) {
        if let Some(tf) = transform {
            self.transforms.push((node_id, tf));
        }
    }
    fn set_responsive_rules(&mut self, node_id: NodeId, rules: &[ResponsiveRule]) {
        if !rules.is_empty() {
            self.responsive_rules.push((node_id, rules.to_vec()));
        }
    }
    fn set_state_rules(&mut self, node_id: NodeId, rules: &[StateStyleRule]) {
        if !rules.is_empty() {
            self.state_rules.push((node_id, rules.to_vec()));
        }
    }
    fn commit_frame(&mut self, _: bool) -> Result<BackendApplyStats, BackendError> {
        Ok(BackendApplyStats::default())
    }
}

fn layout_showcase_app() -> Element {
    let grid_style = TypedStyle::new()
        .display_grid()
        .grid_cols([GridTrack::repeat_minmax_zero_fr(1)])
        .grid_rows([GridTrack::px(48.0), GridTrack::auto()])
        .column_gap(16.0)
        .row_gap(12.0)
        .at(
            MediaQuery::expanded(),
            StylePatch::new().grid_cols([GridTrack::repeat_minmax_zero_fr(3)]),
        );

    let sticky_hdr = TypedStyle::new()
        .position_sticky()
        .inset_top_px(16.0)
        .grid_column_span(2);

    let card_style = TypedStyle::new()
        .flex_basis_pct(50.0)
        .aspect_ratio(16.0 / 9.0)
        .transform(Transform2D::translate(0.0, 4.0).with_origin(0.0, 0.5))
        .on_hover(StylePatch::new().translate(0.0, -4.0).scale(1.03).rotate(1.5));

    let rtl_bar = TypedStyle::new()
        .rtl()
        .flex_row()
        .padding_inline_start(24.0)
        .padding_inline_end(12.0)
        .margin_inline_start(8.0)
        .margin_inline_end(4.0);

    rsx! {
        view {
            nve_style: grid_style,
            view { nve_style: sticky_hdr, text { "Sticky Header" } }
            pressable { nve_style: card_style, text { "Interactive Card" } }
            view { nve_style: rtl_bar, text { "مرحبا" } }
        }
    }
}

fn assert_layout_ext_sink(sink: &LayoutExtSink) {
    assert_eq!(sink.layout_exts.len(), 4, "expected 4 nodes with LayoutExt metadata");

    let has_grid = sink.layout_exts.iter().any(|(_, e)| {
        e.display == Display::Grid
            && e.grid
                .as_ref()
                .is_some_and(|g| g.column_gap == Some(16.0) && g.row_gap == Some(12.0))
    });
    assert!(has_grid, "grid container LayoutExt missing");

    let has_sticky = sink.layout_exts.iter().any(|(_, e)| {
        e.position == PositionMode::Sticky
            && e.insets.top == Dimension::Points(16.0)
            && e.grid_item.column == GridTrackPlacement::span(2)
    });
    assert!(has_sticky, "sticky header LayoutExt missing");

    let has_card_ext = sink.layout_exts.iter().any(|(_, e)| {
        e.flex_basis == Dimension::Percent(50.0)
            && e.aspect_ratio
                .is_some_and(|r| (r - 16.0 / 9.0).abs() < 1e-5)
    });
    assert!(has_card_ext, "card flex_basis/aspect_ratio LayoutExt missing");

    let has_rtl = sink.layout_exts.iter().any(|(_, e)| {
        e.direction == Some(LayoutDirection::Rtl)
            && e.padding_inline.start == Some(24.0)
            && e.padding_inline.end == Some(12.0)
            && e.margin_inline.start == Some(8.0)
            && e.margin_inline.end == Some(4.0)
    });
    assert!(has_rtl, "RTL logical edges LayoutExt missing");

    assert_eq!(sink.transforms.len(), 1);
    assert_eq!(
        sink.transforms[0].1,
        Transform2D::translate(0.0, 4.0).with_origin(0.0, 0.5)
    );

    assert_eq!(sink.responsive_rules.len(), 1);
    assert!(sink.responsive_rules[0].1[0].patch.grid_template_columns.is_some());

    assert_eq!(sink.state_rules.len(), 1);
    let hover_tf = sink.state_rules[0].1[0].patch.transform.unwrap();
    assert_eq!(hover_tf.translate_y, -4.0);
    assert_eq!(hover_tf.scale_x, 1.03);
    assert_eq!(hover_tf.rotate_deg, 1.5);
}

#[test]
fn dioxus_surface_propagates_grid_sticky_rtl_and_transform_metadata() {
    let mut surface = DioxusSurface::new(1, layout_showcase_app);
    let mut sink = LayoutExtSink::default();
    surface.flush_into(1, &mut sink).unwrap();
    assert_layout_ext_sink(&sink);

    let mut wire_surface = DioxusSurface::new(1, layout_showcase_app);
    let frame = wire_surface.render_frame_slice(1);
    let mut replayed = LayoutExtSink::default();
    replay_command_buffer_into(frame, &mut replayed).unwrap();
    assert_layout_ext_sink(&replayed);
}
