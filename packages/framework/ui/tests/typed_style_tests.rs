//! Deep tests for `nve-ui` direct `TypedStyle` emission:
//! - `SurfaceStyleExt` full-surface/full-text helpers & `TypedStyle::merge`/`merge_opt`
//! - `nve-ui` components merging user `nve_style: Option<TypedStyle>` overrides deterministically
//!   on top of base component styles, plus state-transition resets (`disabled`, `focused`,
//!   `checked`, `error`) with zero stale styles and zero leaked `"style"` raw props.

#[allow(dead_code)]
mod common;

use common::UiHarness;
use nve_abi::{ColorRgba, Dimension, EdgeInsets, FlexAlign, FlexDirection, FlexJustify};
use nve_dioxus::prelude::*;
use nve_ui::prelude::*;

#[test]
fn typed_style_surface_and_merge_preserve_unset_base_fields() {
    let base = TypedStyle::new()
        .width_pct(100.0)
        .height_px(44.0)
        .flex_row()
        .justify_content(FlexJustify::SpaceBetween)
        .align_items(FlexAlign::Center)
        .gap(8.0)
        .padding_xy(14.0, 6.0)
        .margin(4.0)
        .z_index(50)
        .overflow_mode(Overflow::Hidden)
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
            font_weight: 600,
            text_align: TextAlign::Center,
            opacity: 0.75,
        });

    let override_style = TypedStyle::new()
        .padding(20.0)
        .radius(22.0)
        .overflow_x(Overflow::Auto)
        .flex_wrap(FlexWrap::Wrap)
        .font_family("monospace")
        .line_height_ratio(1.5)
        .white_space(WhiteSpace::PreWrap)
        .overflow_wrap(OverflowWrap::BreakWord)
        .text_decoration(TextDecoration::UNDERLINE);

    let merged = base.clone().merge_opt(Some(&override_style));
    assert_eq!(merged.layout().width, Dimension::Percent(100.0));
    assert_eq!(merged.layout().flex_direction, FlexDirection::Row);
    assert_eq!(merged.layout().padding, EdgeInsets::all(20.0));
    assert_eq!(merged.style().border_radius, 22.0);
    assert_eq!(merged.style().background, ColorRgba::rgb(0x1e, 0x29, 0x3b));
    assert_eq!(merged.overflow_value(), Some((Overflow::Auto, Overflow::Hidden)));
    assert_eq!(merged.flex_wrap, Some(FlexWrap::Wrap));
    assert_eq!(merged.font_family.as_deref(), Some("monospace"));
    assert_eq!(merged.line_height, Some(Length::percent(150.0)));
    assert_eq!(merged.white_space, Some(WhiteSpace::PreWrap));
    assert_eq!(merged.overflow_wrap, Some(OverflowWrap::BreakWord));
    assert_eq!(merged.text_decoration, Some(TextDecoration::UNDERLINE));
    assert_eq!(base.clone().merge_opt(None), base);
}

fn emerald() -> Accent {
    Accent::from_srgb(ColorRgba::rgb(0x10, 0xb9, 0x81))
}

#[component]
fn OverrideAndStateResetApp() -> Element {
    let mut use_override = use_signal(|| true);
    let mut disabled = use_signal(|| false);
    let mut checked = use_signal(|| false);
    let mut has_error = use_signal(|| false);
    let mut dark = use_signal(|| false);

    let mut theme = if dark() {
        Theme::DARK
    } else {
        Theme::LIGHT
    };
    if dark() {
        // ThemeProvider propagates a runtime accent override to every component.
        theme = theme.with_accent(emerald());
    }

    let btn_override = if use_override() {
        Some(TypedStyle::new().padding(20.0).radius(22.0))
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
                    nve_style: btn_override,
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
    assert_eq!(h.bg(btn), Theme::LIGHT.status.accent.solid);
    assert_eq!(h.raw_str(btn, "style"), None);

    // 2. Focus -> border_width=2.0 while user override padding=20 & radius=22 stay intact!
    h.focus("styled-btn");
    assert_eq!(h.border_width(btn), 2.0);
    assert_eq!(h.border_color(btn), Theme::LIGHT.border.focus);
    assert_eq!(h.padding(btn), EdgeInsets::all(20.0));
    assert_eq!(h.border_radius(btn), 22.0);

    // Blur -> the border resets cleanly to the resting filled-control boundary.
    h.blur("styled-btn");
    let resting = nve_ui::button_colors(&Theme::LIGHT, ButtonVariant::Primary, false, false);
    assert_eq!(h.border_width(btn), resting.border_width);
    assert_eq!(h.border_color(btn), resting.border);

    // 3. Disabled -> opacity=0.5, re-enable -> opacity=1.0 (override stays intact throughout).
    h.press("toggle-disabled");
    assert!((h.opacity(btn) - 0.5).abs() < 1e-3);
    assert_eq!(h.border_radius(btn), 22.0);
    h.press("toggle-disabled");
    assert!((h.opacity(btn) - 1.0).abs() < 1e-3);

    // 4. Removing user `style` override restores exact theme radii & padding with zero raw "style" prop.
    h.press("toggle-override");
    assert_eq!(h.border_radius(btn), Theme::LIGHT.radius.md);
    assert_eq!(h.padding(btn), EdgeInsets::symmetric(8.0, 16.0));
    assert_eq!(h.raw_str(btn, "style"), None);

    // 5. Checkbox checked -> unchecked cleanly resets box fill & border.
    let cb = h.find("state-cb");
    let cb_box = h.children_of(cb)[0];
    assert_eq!(h.bg(cb_box), Theme::LIGHT.surface.base);
    h.press("toggle-checked");
    assert_eq!(h.bg(cb_box), Theme::LIGHT.status.accent.solid);
    h.press("toggle-checked");
    assert_eq!(h.bg(cb_box), Theme::LIGHT.surface.base);

    // 6. Input error -> valid resets border_width (2 -> 1) and border_color (danger -> border).
    let inp = h.find("state-input");
    assert_eq!(h.border_width(inp), 1.0);
    assert_eq!(h.border_color(inp), Theme::LIGHT.border.strong);
    h.press("toggle-error");
    assert_eq!(h.border_width(inp), 2.0);
    assert_eq!(h.border_color(inp), nve_ui::error_indicator(&Theme::LIGHT));
    h.press("toggle-error");
    assert_eq!(h.border_width(inp), 1.0);
    assert_eq!(h.border_color(inp), Theme::LIGHT.border.strong);

    // 7. ThemeProvider propagates the runtime emerald accent override.
    h.press("toggle-dark");
    assert_eq!(h.bg(btn), Theme::DARK.with_accent(emerald()).status.accent.solid);
}
