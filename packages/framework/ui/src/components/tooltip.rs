//! Accessible [`Tooltip`] component with `accessibilityRole="tooltip"`, `aria-expanded`,
//! `aria-describedby`, `Escape` dismissal, and anchored collision-flipping via
//! [`crate::positioning::compute_overlay_position`].

use dioxus_core::EventHandler;
use dioxus_core_macro::Props;
use nve_abi::ColorRgba;
use nve_dioxus::prelude::*;

use crate::components::overlays::resolve_anchored_position;
use crate::positioning::{Placement, PositionOptions, Rect, Size, Viewport};
use crate::surface::{SurfaceStyleExt, TypedSurfaceStyle, TypedTextStyle};
use crate::theme::{use_theme, Layer};

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
    #[props(default)] nve_style: Option<TypedStyle>,
    children: Element,
) -> Element {
    let theme = use_theme();
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

    let wrap_style = TypedStyle::new()
        .flex_col()
        .gap(theme.spacing.xs)
        .merge_opt(nve_style.as_ref());

    let mut tip_style = TypedStyle::new()
        .padding_xy(theme.spacing.sm, theme.spacing.xs)
        .z_index(Layer::Tooltip.z())
        .surface(TypedSurfaceStyle {
            background: theme.text.primary,
            border_width: if theme.is_high_contrast() { 1.0 } else { 0.0 },
            border_color: if theme.is_high_contrast() {
                theme.border.default
            } else {
                ColorRgba::TRANSPARENT
            },
            border_radius: theme.radius.sm,
            opacity: 1.0,
            box_shadow: theme.elevation.overlay,
        });
    if let Some(pos) = computed {
        if let Some(mw) = pos.max_width {
            tip_style = tip_style.max_width_px(mw);
        }
        if let Some(mh) = pos.max_height {
            tip_style = tip_style.max_height_px(mh);
        }
    }

    let pos_placement = computed.map(|p| p.placement.as_str().to_owned());
    let pos_x = computed.map(|p| format!("{:.2}", p.x));
    let pos_y = computed.map(|p| format!("{:.2}", p.y));
    let pos_flipped = computed.map(|p| if p.flipped { "true" } else { "false" }.to_owned());
    let pos_shifted = computed.map(|p| if p.shifted { "true" } else { "false" }.to_owned());
    let pos_arrow_x = computed.and_then(|p| p.arrow_x).map(|v| format!("{v:.2}"));
    let pos_arrow_y = computed.and_then(|p| p.arrow_y).map(|v| format!("{v:.2}"));

    let tip_text_style = TypedStyle::new().text_style(TypedTextStyle {
        color: theme.surface.base,
        font_size: theme.typography.caption.size,
        font_weight: 500,
        text_align: TextAlign::Left,
        opacity: 1.0,
    });

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
                {
                    let mut tip_data = Vec::new();
                    if let Some(ref p) = pos_placement { tip_data.push(("placement".to_owned(), p.clone())); }
                    if let Some(ref x) = pos_x { tip_data.push(("x".to_owned(), x.clone())); }
                    if let Some(ref y) = pos_y { tip_data.push(("y".to_owned(), y.clone())); }
                    if let Some(ref f) = pos_flipped { tip_data.push(("flipped".to_owned(), f.clone())); }
                    if let Some(ref s) = pos_shifted { tip_data.push(("shifted".to_owned(), s.clone())); }
                    if let Some(ref ax) = pos_arrow_x { tip_data.push(("arrow-x".to_owned(), ax.clone())); }
                    if let Some(ref ay) = pos_arrow_y { tip_data.push(("arrow-y".to_owned(), ay.clone())); }
                    let tip_table = super::html_tables::html_data_attrs(tip_data);
                    rsx! {
                        view {
                            test_id: "{bubble_id}",
                            accessibility_role: "tooltip",
                            nve_table: tip_table,
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
    }
}
