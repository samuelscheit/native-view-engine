//! Deep integration tests for zero-serialization `TypedStyle` gradients (`linear_gradient`,
//! `radial_gradient`, responsive `@media` & `:hover`/`:active` `StylePatch` gradients) and
//! direct typed `<svg picture: SvgPictureAttr::from(...)>` attachment on `DioxusSurface`.

use nve_abi::ColorRgba;
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;

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

#[test]
fn dioxus_surface_stores_typed_gradients_and_svg_pictures_with_zero_serialization() {
    let mut surface = DioxusSurface::new(1, rich_graphics_app).with_in_process_mode(true);
    let (_, _, bytes) = surface.render_frame(1);
    assert!(!bytes.is_empty());

    let fills: Vec<_> = surface
        .iter_background_fills()
        .map(|(id, f)| (id, f.clone()))
        .collect();
    assert_eq!(fills.len(), 2, "expected linear card + radial badge fills");
    assert!(fills.iter().any(|(_, f)| matches!(f, BackgroundFill::Linear(lg) if (lg.angle_deg - 135.0).abs() < 1e-3 && lg.stops.len() == 2)));
    assert!(fills.iter().any(|(_, f)| matches!(f, BackgroundFill::Radial(rg) if (rg.center_x - 0.5).abs() < 1e-3 && rg.stops.len() == 2)));

    // Hover & responsive patches retain typed BackgroundFill without stringification.
    assert!(surface.iter_state_rules().any(|(_, rules)| {
        rules.iter().any(|r| {
            r.state == InteractionState::HOVER
                && matches!(r.patch.background_fill, Some(BackgroundFill::Linear(_)))
        })
    }));
    assert!(surface.iter_responsive_rules().any(|(_, rules)| {
        rules.iter().any(|r| {
            matches!(r.patch.background_fill, Some(BackgroundFill::Radial(_)))
        })
    }));

    // Typed SvgPicture attached directly on `<svg>` node without stringifying/re-parsing.
    let svgs: Vec<_> = surface
        .iter_svg_pictures()
        .map(|(id, pic)| (id, pic.clone()))
        .collect();
    assert_eq!(svgs.len(), 1);
    let (svg_id, pic) = &svgs[0];
    assert_eq!(pic.view_box, (0.0, 0.0, 24.0, 24.0));
    assert_eq!(pic.shapes.len(), 1);
    assert_eq!(surface.node_svg_picture(*svg_id), Some(pic));
}
