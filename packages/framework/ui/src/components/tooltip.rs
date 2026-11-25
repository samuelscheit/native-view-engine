//! Accessible [`Tooltip`] component with `accessibilityRole="tooltip"`, `aria-expanded`,
//! `aria-describedby`, `Escape` dismissal, and anchored collision-flipping via
//! [`crate::positioning::compute_overlay_position`].

use dioxus_core::EventHandler;
use dioxus_core_macro::Props;
use nve_abi::ColorRgba;
use nve_dioxus::prelude::*;

use crate::components::overlays::resolve_anchored_position;
use crate::positioning::{Placement, PositionOptions, Rect, Size, Viewport};
use crate::style_buf::{TypedStyleBuilder, TypedSurfaceStyle, TypedTextStyle};
use crate::theme::use_theme_rgba;

/// Accessible tooltip (`accessibilityRole="tooltip"`, `aria-expanded`, `aria-describedby`,
/// `Escape` dismiss) with optional anchored collision-flipping via [`crate::positioning::compute_overlay_position`].
/// Works in both controlled (`open: Some(bool)`) and uncontrolled modes.
#[component]
pub fn Tooltip(
    content: String,
    #[props(default)] open: Option<bool>,
    #[props(default)] on_open_change: Option<EventHandler<bool>>,
    #[props(default)] anchor_rect: Option<Rect>,
    #[props(default)] overlay_size: Option<Size>,
    #[props(default)] viewport: Option<Viewport>,
    #[props(default)] position_options: Option<PositionOptions>,
    #[props(default)] test_id: Option<String>,
    #[props(default)] style: Option<String>,
    children: Element,
) -> Element {
    let (theme, colors) = use_theme_rgba();
    let mut internal_open = use_signal(|| false);
    let is_open = match open {
        Some(v) => v,
        None => internal_open(),
    };

    let base_id = test_id.as_deref().unwrap_or("nve-tooltip");
    let trigger_id = format!("{base_id}-trigger");
    let bubble_id = format!("{base_id}-bubble");

    let default_tip_opts = position_options
        .unwrap_or_else(|| PositionOptions::new(Placement::Top).with_offset(6.0));
    let computed = resolve_anchored_position(
        anchor_rect,
        overlay_size,
        viewport,
        Some(default_tip_opts),
        Size::new(160.0, 32.0),
    );

    let wrap_style = TypedStyleBuilder::new()
        .flex_col()
        .gap(theme.spacing.xs)
        .finish();

    let mut tip_builder = TypedStyleBuilder::new()
        .padding_xy(theme.spacing.sm, theme.spacing.xs)
        .z_index(950)
        .surface(TypedSurfaceStyle {
            background: colors.text_primary,
            border_width: if theme.is_high_contrast() { 1.0 } else { 0.0 },
            border_color: if theme.is_high_contrast() {
                colors.border
            } else {
                ColorRgba::TRANSPARENT
            },
            border_radius: theme.radii.sm,
            opacity: 1.0,
            box_shadow: theme.elevation_shadow(),
        });
    if let Some(pos) = computed {
        if let Some(mw) = pos.max_width {
            tip_builder = tip_builder.max_width_px(mw);
        }
        if let Some(mh) = pos.max_height {
            tip_builder = tip_builder.max_height_px(mh);
        }
    }
    let tip_style = tip_builder.finish();

    let pos_placement = computed.map(|p| p.placement.as_str().to_owned());
    let pos_x = computed.map(|p| format!("{:.2}", p.x));
    let pos_y = computed.map(|p| format!("{:.2}", p.y));
    let pos_flipped = computed.map(|p| if p.flipped { "true" } else { "false" }.to_owned());
    let pos_shifted = computed.map(|p| if p.shifted { "true" } else { "false" }.to_owned());
    let pos_arrow_x = computed.and_then(|p| p.arrow_x).map(|v| format!("{v:.2}"));
    let pos_arrow_y = computed.and_then(|p| p.arrow_y).map(|v| format!("{v:.2}"));

    let tip_text_style = TypedStyleBuilder::new()
        .text_style(TypedTextStyle {
            color: colors.surface,
            font_size: theme.typography.xs,
            font_weight: "500",
            text_align: "left",
            opacity: 1.0,
        })
        .finish();

    let aria_expanded = if is_open { "true" } else { "false" };
    let described_by = if is_open {
        Some(bubble_id.clone())
    } else {
        None
    };

    rsx! {
        view {
            test_id: "{base_id}",
            nve_style: wrap_style,
            style: style,
            pressable {
                test_id: "{trigger_id}",
                tab_index: 0,
                aria_expanded: "{aria_expanded}",
                aria_describedby: described_by,
                onfocus: move |_| {
                    internal_open.set(true);
                    if let Some(handler) = on_open_change {
                        handler.call(true);
                    }
                },
                onblur: move |_| {
                    internal_open.set(false);
                    if let Some(handler) = on_open_change {
                        handler.call(false);
                    }
                },
                onkeydown: move |ev: KeyEvent| {
                    let data = ev.data();
                    let key = data.key.as_str();
                    if is_open && matches!(key, "Escape" | "Esc") {
                        internal_open.set(false);
                        if let Some(handler) = on_open_change {
                            handler.call(false);
                        }
                    }
                },
                {children}
            }
            if is_open {
                view {
                    test_id: "{bubble_id}",
                    accessibility_role: "tooltip",
                    "data-placement": pos_placement,
                    "data-x": pos_x,
                    "data-y": pos_y,
                    "data-flipped": pos_flipped,
                    "data-shifted": pos_shifted,
                    "data-arrow-x": pos_arrow_x,
                    "data-arrow-y": pos_arrow_y,
                    nve_style: tip_style,
                    text {
                        nve_style: tip_text_style,
                        "{content}"
                    }
                }
            }
        }
    }
}
