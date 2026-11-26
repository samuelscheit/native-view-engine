//! Deep integration tests for zero-serialization `TypedStyle` gradients (`linear_gradient`,
//! `radial_gradient`, responsive `@media` & `:hover`/`:active` `StylePatch` gradients) and
//! direct typed `<svg picture: SvgPictureAttr::from(...)>` streaming into `RenderSink` and
//! round-tripping through `BatchSink` + `replay_command_buffer_into`.

use nve_abi::{ColorRgba, ComponentKind, FrameId, LayoutProps, NodeId, RawPropValueRef, StyleProps, SurfaceId};
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;
use nve_engine::{replay_command_buffer_into, BackendApplyStats, BackendError, RenderSink};

#[derive(Default)]
struct GraphicsSink {
    fills: Vec<(NodeId, BackgroundFill)>,
    state_rules: Vec<(NodeId, Vec<StateStyleRule>)>,
    responsive_rules: Vec<(NodeId, Vec<ResponsiveRule>)>,
    svgs: Vec<(NodeId, SvgPicture)>,
}

impl RenderSink for GraphicsSink {
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
    fn set_background_fill(&mut self, node_id: NodeId, fill: Option<&BackgroundFill>) {
        if let Some(f) = fill {
            self.fills.push((node_id, f.clone()));
        }
    }
    fn set_state_rules(&mut self, node_id: NodeId, rules: &[StateStyleRule]) {
        if !rules.is_empty() {
            self.state_rules.push((node_id, rules.to_vec()));
        }
    }
    fn set_responsive_rules(&mut self, node_id: NodeId, rules: &[ResponsiveRule]) {
        if !rules.is_empty() {
            self.responsive_rules.push((node_id, rules.to_vec()));
        }
    }
    fn set_svg(&mut self, node_id: NodeId, svg: Option<&SvgPicture>) {
        if let Some(p) = svg {
            self.svgs.push((node_id, p.clone()));
        }
    }
    fn commit_frame(&mut self, _: bool) -> Result<BackendApplyStats, BackendError> {
        Ok(BackendApplyStats::default())
    }
}

fn rich_graphics_app() -> Element {
    let icon = SvgPicture::from_paths(24.0, 24.0, &["M4 12 L20 12 M14 6 L20 12 L14 18"]);
    let card_style = TypedStyle::new()
        .width_px(320.0)
        .height_px(180.0)
        .radius(16.0)
        .linear_gradient(
            135.0,
            [
                GradientStop::new(0.0, ColorRgba::rgb(37, 99, 235)),
                GradientStop::new(1.0, ColorRgba::rgb(124, 58, 237)),
            ],
        )
        .on_hover_with(|p| {
            p.linear_gradient(
                90.0,
                [
                    GradientStop::new(0.0, ColorRgba::rgb(14, 165, 233)),
                    GradientStop::new(1.0, ColorRgba::rgb(99, 102, 241)),
                ],
            )
        })
        .at_with(MediaQuery::min_width(768), |p| {
            p.radial_gradient(
                0.5,
                0.25,
                1.0,
                [
                    GradientStop::new(0.0, ColorRgba::rgb(56, 189, 248)),
                    GradientStop::new(1.0, ColorRgba::rgb(15, 23, 42)),
                ],
            )
        });

    let badge_style = TypedStyle::new()
        .width_px(64.0)
        .height_px(64.0)
        .radial_gradient(
            0.5,
            0.5,
            1.0,
            [
                GradientStop::new(0.0, ColorRgba::WHITE),
                GradientStop::new(1.0, ColorRgba::TRANSPARENT),
            ],
        );

    let icon_style = TypedStyle::new()
        .width_px(24.0)
        .height_px(24.0)
        .rtl()
        .color(ColorRgba::rgb(248, 250, 252));

    rsx! {
        view {
            nve_style: card_style,
            view { nve_style: badge_style }
            svg {
                nve_style: icon_style,
                picture: SvgPictureAttr::from(icon),
                mirror: "directional",
            }
        }
    }
}

fn assert_graphics_sink(sink: &GraphicsSink) {
    assert_eq!(sink.fills.len(), 2, "expected linear card + radial badge fills");
    assert!(sink.fills.iter().any(|(_, f)| matches!(f, BackgroundFill::Linear(lg) if (lg.angle_deg - 135.0).abs() < 1e-3 && lg.stops.len() == 2)));
    assert!(sink.fills.iter().any(|(_, f)| matches!(f, BackgroundFill::Radial(rg) if (rg.center_x - 0.5).abs() < 1e-3 && rg.stops.len() == 2)));

    assert!(sink.state_rules.iter().any(|(_, rules)| {
        rules.iter().any(|r| {
            r.state == InteractionState::HOVER
                && matches!(r.patch.background_fill, Some(BackgroundFill::Linear(_)))
        })
    }));
    assert!(sink.responsive_rules.iter().any(|(_, rules)| {
        rules.iter().any(|r| {
            matches!(r.patch.background_fill, Some(BackgroundFill::Radial(_)))
        })
    }));

    assert_eq!(sink.svgs.len(), 1);
    let (_, pic) = &sink.svgs[0];
    assert_eq!(pic.view_box, (0.0, 0.0, 24.0, 24.0));
    assert_eq!(pic.shapes.len(), 1);
}

#[test]
fn dioxus_surface_streams_typed_gradients_and_svg_directly_and_across_batch_sink() {
    // 1. Direct in-process zero-serialization flush_into.
    let mut surface = DioxusSurface::new(1, rich_graphics_app);
    let mut direct_sink = GraphicsSink::default();
    surface.flush_into(1, &mut direct_sink).unwrap();
    assert_graphics_sink(&direct_sink);
    let (svg_id, pic) = &direct_sink.svgs[0];
    assert_eq!(surface.node_svg_picture(*svg_id), Some(pic));

    // 2. Cross-boundary BatchSink + replay_command_buffer_into preserves every extended prop!
    let mut batch_surface = DioxusSurface::new(1, rich_graphics_app);
    let (_, _, bytes) = batch_surface.render_frame(1);
    assert!(!bytes.is_empty());
    let mut replayed_sink = GraphicsSink::default();
    replay_command_buffer_into(&bytes, &mut replayed_sink).unwrap();
    assert_graphics_sink(&replayed_sink);
}
