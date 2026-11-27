//! Deep tests for `nve-ui` direct `TypedStyle` emission:
//! - `parse_color_rgba` and `UiPaletteRgba` parity (including mutated `UiPalette` like `nve-docs`)
//! - `StyleBuf::into_typed()` parity with `TypedStyleBuilder`
//! - `nve-ui` components merging user `style: Option<String>` overrides deterministically on top
//!   of `nve_style`, plus state-transition resets (`disabled`, `focused`, `checked`, `error`)
//!   with zero stale styles and zero leaked `"style"` raw props.

#[allow(dead_code)]
mod common;

use std::borrow::Cow;

use common::UiHarness;
use nve_abi::{ColorRgba, Dimension, EdgeInsets, FlexAlign, FlexDirection, FlexJustify};
use nve_dioxus::prelude::*;
use nve_ui::prelude::*;

#[test]
fn parse_color_rgba_and_palette_rgba_match_builtin_and_custom_palettes() {
    assert_eq!(parse_color_rgba("#0f172a"), Some(ColorRgba::rgb(0x0f, 0x17, 0x2a)));
    assert_eq!(parse_color_rgba("#0f172a80"), Some(ColorRgba::rgba(0x0f, 0x17, 0x2a, 0x80)));
    assert_eq!(parse_color_rgba("#fff"), Some(ColorRgba::WHITE));
    assert_eq!(parse_color_rgba("#0f8c"), Some(ColorRgba::rgba(0x00, 0xff, 0x88, 0xcc)));
    assert_eq!(parse_color_rgba("transparent"), Some(ColorRgba::TRANSPARENT));
    assert_eq!(parse_color_rgba("WHITE"), Some(ColorRgba::WHITE));
    assert_eq!(parse_color_rgba("black"), Some(ColorRgba::BLACK));
    assert_eq!(parse_color_rgba("not-a-color"), None);

    assert_eq!(UiTheme::light().palette_rgba(), UiPaletteRgba::LIGHT);
    assert_eq!(UiTheme::dark().palette_rgba(), UiPaletteRgba::DARK);
    assert_eq!(
        UiTheme::high_contrast_light().palette_rgba(),
        UiPaletteRgba::HIGH_CONTRAST_LIGHT
    );
    assert_eq!(
        UiTheme::high_contrast_dark().palette_rgba(),
        UiPaletteRgba::HIGH_CONTRAST_DARK
    );

    // Mutating `theme.palette` directly (as `nve-docs` does) produces matching `ColorRgba` tokens.
    let mut custom = UiTheme::dark();
    custom.palette.accent = Cow::Borrowed("#10b981");
    custom.palette.surface = Cow::Borrowed("#09090b");
    let custom_rgba = custom.palette_rgba();
    assert_eq!(custom_rgba.accent, ColorRgba::rgb(0x10, 0xb9, 0x81));
    assert_eq!(custom_rgba.surface, ColorRgba::rgb(0x09, 0x09, 0x0b));
}

#[test]
fn style_buf_into_typed_matches_typed_style_builder_identically() {
    let mut sb = StyleBuf::new();
    sb.width_pct(100.0)
        .height_px(44.0)
        .flex_direction("row")
        .justify_content("space-between")
        .align_items("center")
        .gap(8.0)
        .padding_xy(14.0, 6.0)
        .margin(4.0)
        .z_index(50)
        .overflow("hidden")
        .surface(SurfaceStyle {
            background: "#1e293b",
            border_width: 2.0,
            border_color: "#60a5fa",
            border_radius: 10.0,
            opacity: 0.75,
            box_shadow: "0 4px 12px rgba(0, 0, 0, 0.35)",
        })
        .text_style(TextStyle {
            color: "#f8fafc",
            font_size: 16.0,
            font_weight: "600",
            text_align: "center",
            opacity: 0.75,
        });

    let from_buf = sb.into_typed();
    let direct = TypedStyleBuilder::new()
        .width_pct(100.0)
        .height_px(44.0)
        .flex_row()
        .justify_content(FlexJustify::SpaceBetween)
        .align_items(FlexAlign::Center)
        .gap(8.0)
        .padding_xy(14.0, 6.0)
        .margin(4.0)
        .z_index(50)
        .overflow("hidden")
        .surface(TypedSurfaceStyle {
            background: ColorRgba::rgb(0x1e, 0x29, 0x3b),
            border_width: 2.0,
            border_color: ColorRgba::rgb(0x60, 0xa5, 0xfa),
            border_radius: 10.0,
            opacity: 0.75,
            box_shadow: Some(nve_tables::tables::BoxShadow::new(
                0.0,
                4.0,
                12.0,
                nve_store::Rgba::new(0, 0, 0, 89),
            )),
        })
        .text_style(TypedTextStyle {
            color: ColorRgba::rgb(0xf8, 0xfa, 0xfc),
            font_size: 16.0,
            font_weight: "600",
            text_align: "center",
            opacity: 0.75,
        })
        .finish();

    assert_eq!(from_buf, direct);
    assert_eq!(from_buf.layout.width, Dimension::Percent(100.0));
    assert_eq!(from_buf.layout.flex_direction, FlexDirection::Row);
    assert_eq!(from_buf.layout.padding, EdgeInsets::symmetric(6.0, 14.0));
}

#[component]
fn OverrideAndStateResetApp() -> Element {
    let mut use_override = use_signal(|| true);
    let mut disabled = use_signal(|| false);
    let mut checked = use_signal(|| false);
    let mut has_error = use_signal(|| false);
    let mut dark = use_signal(|| false);

    let mut theme = if dark() {
        UiTheme::dark()
    } else {
        UiTheme::light()
    };
    if dark() {
        // Verify ThemeProvider picks up caller-mutated UiPalette cleanly into UiPaletteRgba.
        theme.palette.accent = Cow::Borrowed("#10b981");
    }

    let btn_override = if use_override() {
        Some("padding: 20px; border-radius: 22px;".to_owned())
    } else {
        None
    };

    rsx! {
        ThemeProvider {
            theme: theme,
            Col {
                test_id: "root",
                padding: 12.0,
                Button {
                    test_id: "styled-btn",
                    variant: ButtonVariant::Primary,
                    disabled: disabled(),
                    style: btn_override,
                    "Action"
                }
                Checkbox {
                    test_id: "state-cb",
                    checked: checked(),
                    disabled: disabled(),
                    label: "Agree",
                }
                Input {
                    id: "field",
                    test_id: "state-input",
                    error: if has_error() { Some("Bad value".into()) } else { None },
                    disabled: disabled(),
                }
                Button { test_id: "toggle-override", on_press: move |_| use_override.toggle(), "O" }
                Button { test_id: "toggle-disabled", on_press: move |_| disabled.toggle(), "D" }
                Button { test_id: "toggle-checked", on_press: move |_| checked.toggle(), "C" }
                Button { test_id: "toggle-error", on_press: move |_| has_error.toggle(), "E" }
                Button { test_id: "toggle-dark", on_press: move |_| dark.toggle(), "T" }
            }
        }
    }
}

#[test]
fn nve_ui_components_merge_style_overrides_and_reset_all_state_transitions_cleanly() {
    let mut h = UiHarness::new(OverrideAndStateResetApp);

    // 1. User `style` override merges onto `Button`'s `nve_style` cleanly.
    let btn = h.find("styled-btn");
    assert_eq!(h.padding(btn), EdgeInsets::all(20.0));
    assert_eq!(h.border_radius(btn), 22.0);
    assert_eq!(h.bg(btn), UiPaletteRgba::LIGHT.accent);
    assert_eq!(h.raw_str(btn, "style"), None);

    // 2. Focus -> border_width=2.0 while user override padding=20 & radius=22 stay intact!
    h.focus("styled-btn");
    assert_eq!(h.border_width(btn), 2.0);
    assert_eq!(h.border_color(btn), UiPaletteRgba::LIGHT.border_focus);
    assert_eq!(h.padding(btn), EdgeInsets::all(20.0));
    assert_eq!(h.border_radius(btn), 22.0);

    // Blur -> border_width resets cleanly to 0.0 and border_color to TRANSPARENT.
    h.blur("styled-btn");
    assert_eq!(h.border_width(btn), 0.0);
    assert_eq!(h.border_color(btn), ColorRgba::TRANSPARENT);

    // 3. Disabled -> opacity=0.5, re-enable -> opacity=1.0 (override stays intact throughout).
    h.press("toggle-disabled");
    assert!((h.opacity(btn) - 0.5).abs() < 1e-3);
    assert_eq!(h.border_radius(btn), 22.0);
    h.press("toggle-disabled");
    assert!((h.opacity(btn) - 1.0).abs() < 1e-3);

    // 4. Removing user `style` override restores exact theme radii & padding with zero raw "style" prop.
    h.press("toggle-override");
    assert_eq!(h.border_radius(btn), 8.0);
    assert_eq!(h.padding(btn), EdgeInsets::symmetric(8.0, 16.0));
    assert_eq!(h.raw_str(btn, "style"), None);

    // 5. Checkbox checked -> unchecked cleanly resets box fill & border.
    let cb = h.find("state-cb");
    let cb_box = h.children_of(cb)[0];
    assert_eq!(h.bg(cb_box), UiPaletteRgba::LIGHT.surface);
    h.press("toggle-checked");
    assert_eq!(h.bg(cb_box), UiPaletteRgba::LIGHT.accent);
    h.press("toggle-checked");
    assert_eq!(h.bg(cb_box), UiPaletteRgba::LIGHT.surface);

    // 6. Input error -> valid resets border_width (2 -> 1) and border_color (danger -> border).
    let inp = h.find("state-input");
    assert_eq!(h.border_width(inp), 1.0);
    assert_eq!(h.border_color(inp), UiPaletteRgba::LIGHT.border);
    h.press("toggle-error");
    assert_eq!(h.border_width(inp), 2.0);
    assert_eq!(h.border_color(inp), UiPaletteRgba::LIGHT.danger);
    h.press("toggle-error");
    assert_eq!(h.border_width(inp), 1.0);
    assert_eq!(h.border_color(inp), UiPaletteRgba::LIGHT.border);

    // 7. ThemeProvider syncs custom-mutated `theme.palette.accent = #10b981` into pre-parsed colors.
    h.press("toggle-dark");
    assert_eq!(h.bg(btn), ColorRgba::rgb(0x10, 0xb9, 0x81));
}
