//! Deep end-to-end integration tests rendering all 15 `nve-ui` components through real
//! `DioxusSurface` + `HeadlessBackend`, verifying layout bounds, theme switching, explicit
//! style resets, interactive events, ARIA attributes, focus trapping, roving focus, toasts,
//! and 100,000-row virtualized list slicing.

mod common;

use std::collections::BTreeMap;

use common::UiHarness;
use nve_dioxus::prelude::*;
use nve_interop::{ColorScheme, ContrastLevel, ThemeContext};
use nve_ui::prelude::*;
use nve_ui::{Orientation, UiDensity};

fn island_theme() -> Theme {
    Theme::from_theme_context(
        &ThemeContext {
            color_scheme: None,
            contrast: ContrastLevel::Standard,
            accent_rgba: 0x10B9_81FF,
            tokens: BTreeMap::new(),
        },
        ColorScheme::Dark,
        AppBrand::Default,
    )
}

#[component]
fn LayoutActionsThemeApp() -> Element {
    let mut mode = use_signal(|| 0u8);
    let mut btn_disabled = use_signal(|| true);
    let mut btn_loading = use_signal(|| false);
    let mut presses = use_signal(|| 0u32);

    let theme = match mode() {
        0 => Theme::LIGHT,
        1 => Theme::DARK,
        2 => Theme::HIGH_CONTRAST_LIGHT,
        3 => Theme::HIGH_CONTRAST_DARK.with_density(UiDensity::Compact),
        _ => island_theme(),
    };

    rsx! {
        ThemeProvider {
            theme: theme,
            Col {
                test_id: "root-col", padding: 16.0, gap: 10.0,
                Row {
                    test_id: "controls-row", gap: 8.0,
                    Button {
                        test_id: "cycle-theme", variant: ButtonVariant::Secondary, size: ButtonSize::Sm,
                        on_press: move |_| mode.set((mode() + 1) % 5), "Theme"
                    }
                    Button {
                        test_id: "toggle-disabled", variant: ButtonVariant::Outline, size: ButtonSize::Sm,
                        on_press: move |_| btn_disabled.toggle(), "ToggleDisabled"
                    }
                    Button {
                        test_id: "toggle-loading", variant: ButtonVariant::Ghost, size: ButtonSize::Sm,
                        on_press: move |_| btn_loading.toggle(), "ToggleLoading"
                    }
                }
                Separator { test_id: "sep-h" }
                Stack {
                    test_id: "cards-stack", direction: Orientation::Horizontal,
                    Card {
                        test_id: "card-elevated", variant: CardVariant::Elevated,
                        Button {
                            test_id: "primary-btn", variant: ButtonVariant::Primary, size: ButtonSize::Lg,
                            disabled: btn_disabled(), loading: btn_loading(),
                            on_press: move |_| presses += 1, "Save"
                        }
                        IconButton {
                            test_id: "icon-btn", accessibility_label: "Close panel", icon: "x",
                            variant: ButtonVariant::Destructive, size: ButtonSize::Md,
                            on_press: move |_| presses += 10,
                        }
                    }
                    Card {
                        test_id: "card-outlined", variant: CardVariant::Outlined,
                        Badge { test_id: "badge-neutral", variant: BadgeVariant::Neutral, label: "Idle" }
                        Badge { test_id: "badge-accent", variant: BadgeVariant::Accent, label: "Beta" }
                        Badge { test_id: "badge-success", variant: BadgeVariant::Success, label: "OK" }
                        Badge { test_id: "badge-warning", variant: BadgeVariant::Warning, label: "Warn" }
                        Badge { test_id: "badge-danger", variant: BadgeVariant::Danger, label: "Err" }
                    }
                }
                text { test_id: "press-count", "{presses}" }
            }
        }
    }
}

#[test]
fn layout_actions_theme_switching_and_explicit_style_resets() {
    let mut h = UiHarness::new(LayoutActionsThemeApp);

    let sep = h.find("sep-h");
    assert_eq!(h.raw_str(sep, "accessibilityRole"), Some("separator"));
    assert_eq!(h.backend.layout_of(sep).unwrap().height, 1.0);

    let card_el = h.find("card-elevated");
    let card_out = h.find("card-outlined");
    assert!(h.backend.layout_of(card_el).unwrap().width > 0.0);
    assert!(h.backend.layout_of(card_el).unwrap().height > 0.0);
    assert_ne!(h.raw_str(card_el, "box-shadow"), Some("none"));
    assert_eq!(h.raw_str(card_out, "box-shadow"), Some("none"));

    let icon = h.find("icon-btn");
    assert_eq!(h.raw_str(icon, "accessibilityRole"), Some("button"));
    assert_eq!(h.raw_str(icon, "accessibilityLabel"), Some("Close panel"));
    assert_eq!(h.backend.layout_of(icon).unwrap().width, 36.0);
    assert_eq!(h.backend.layout_of(icon).unwrap().height, 36.0);

    // Primary button starts disabled: blocked press/Enter/Space, opacity=0.5, tabIndex=-1.
    let btn = h.find("primary-btn");
    assert_eq!(h.raw_str(btn, "accessibilityRole"), Some("button"));
    assert_eq!(h.raw_bool(btn, "disabled"), Some(true));
    assert_eq!(h.raw_str(btn, "tabIndex"), Some("-1"));
    assert!((h.opacity(btn) - 0.5).abs() < 1e-3);

    h.press("primary-btn");
    h.keydown("primary-btn", "Enter", false);
    h.keydown("primary-btn", " ", false);
    assert_eq!(h.text_of("press-count"), "0");

    // Re-enable button -> explicit style reset restores opacity=1.0 and tabIndex=0.
    h.press("toggle-disabled");
    let btn = h.find("primary-btn");
    assert_eq!(h.raw_bool(btn, "disabled"), Some(false));
    assert_eq!(h.raw_str(btn, "tabIndex"), Some("0"));
    assert!((h.opacity(btn) - 1.0).abs() < 1e-3);

    // Focus -> border_width=2.0, blur -> explicitly resets to the 1px resting boundary (no stale
    // focus ring).
    h.focus("primary-btn");
    assert_eq!(h.border_width(btn), 2.0);
    h.blur("primary-btn");
    assert_eq!(h.border_width(btn), 1.0);

    // Press, Enter, Space, and IconButton Enter activation.
    h.press("primary-btn");
    h.keydown("primary-btn", "Enter", false);
    h.keydown("primary-btn", "Space", false);
    h.keydown("icon-btn", "Enter", false);
    assert_eq!(h.text_of("press-count"), "13");

    // Loading blocks activation and sets opacity=0.5; clearing resets opacity=1.0.
    h.press("toggle-loading");
    h.press("primary-btn");
    assert_eq!(h.text_of("press-count"), "13");
    assert!((h.opacity(btn) - 0.5).abs() < 1e-3);
    h.press("toggle-loading");
    assert!((h.opacity(btn) - 1.0).abs() < 1e-3);

    // Theme switching: light -> dark -> high_contrast_light -> high_contrast_dark -> ThemeContext.
    assert_eq!(h.bg(card_el), Theme::LIGHT.surface.raised);
    h.press("cycle-theme");
    assert_eq!(h.bg(card_el), Theme::DARK.surface.raised);
    h.press("cycle-theme");
    assert_eq!(h.border_width(card_el), 2.0);
    assert_eq!(h.raw_str(card_el, "box-shadow"), Some("none"));
    h.press("cycle-theme");
    assert_eq!(h.bg(card_el), Theme::HIGH_CONTRAST_DARK.surface.raised);
    h.press("cycle-theme");
    // ThemeContext with `color_scheme: None` follows the OS (dark) and applies the island accent.
    assert_eq!(h.bg(card_el), Theme::DARK.surface.raised);
    assert_eq!(h.bg(btn), island_theme().status.accent.solid);
    assert_ne!(island_theme().status.accent.solid, Theme::DARK.status.accent.solid);
}

#[component]
fn FormsApp() -> Element {
    let mut name = use_signal(|| String::from("Ada"));
    let mut show_error = use_signal(|| false);
    let mut read_only = use_signal(|| false);
    let mut agreed = use_signal(|| false);
    let mut mixed = use_signal(|| true);
    let mut enabled = use_signal(|| false);
    let mut sw_disabled = use_signal(|| false);

    rsx! {
        ThemeProvider {
            theme: Theme::LIGHT,
            Col {
                padding: 12.0,
                Input {
                    id: "email", test_id: "email-input", label: "Email",
                    helper_text: "Work address",
                    error: if show_error() { Some("Invalid email".into()) } else { None },
                    value: name(), read_only: read_only(), secure: true, multiline: false,
                    on_change: move |v| name.set(v),
                }
                Checkbox {
                    test_id: "terms-cb", label: "Accept terms",
                    checked: agreed(), indeterminate: mixed(),
                    on_change: move |next| { mixed.set(false); agreed.set(next); },
                }
                Switch {
                    test_id: "alerts-sw", label: "Alerts",
                    checked: enabled(), disabled: sw_disabled(),
                    on_change: move |next| enabled.set(next),
                }
                Button { test_id: "toggle-err", on_press: move |_| show_error.toggle(), "Err" }
                Button { test_id: "toggle-ro", on_press: move |_| read_only.toggle(), "RO" }
                Button { test_id: "toggle-sw-dis", on_press: move |_| sw_disabled.toggle(), "Dis" }
                text { test_id: "form-state", "{name}|{agreed}|{enabled}" }
            }
        }
    }
}

#[test]
fn form_controls_aria_events_and_style_resets() {
    let mut h = UiHarness::new(FormsApp);

    let inp = h.find("email-input");
    assert_eq!(h.raw_str(inp, "accessibilityRole"), Some("textbox"));
    assert_eq!(h.raw_str(inp, "ariaLabelledBy"), Some("email-label"));
    assert_eq!(h.raw_str(inp, "ariaDescribedBy"), Some("email-helper"));
    assert_eq!(h.raw_bool(inp, "secureTextEntry"), Some(true));

    h.focus("email-input");
    assert_eq!(h.border_width(inp), 2.0);
    h.blur("email-input");
    assert_eq!(h.border_width(inp), 1.0);

    h.press("toggle-err");
    assert_eq!(h.raw_str(inp, "ariaDescribedBy"), Some("email-error"));
    assert_eq!(h.border_width(inp), 2.0);
    h.press("toggle-err");
    assert_eq!(h.raw_str(inp, "ariaDescribedBy"), Some("email-helper"));
    assert_eq!(h.border_width(inp), 1.0);

    h.text_change("email-input", "ada@nve.dev");
    assert_eq!(h.text_of("form-state"), "ada@nve.dev|false|false");
    h.press("toggle-ro");
    h.text_change("email-input", "blocked");
    assert_eq!(h.text_of("form-state"), "ada@nve.dev|false|false");

    let cb = h.find("terms-cb");
    assert_eq!(h.raw_str(cb, "accessibilityRole"), Some("checkbox"));
    assert_eq!(h.raw_str(cb, "ariaChecked"), Some("mixed"));
    h.keydown("terms-cb", " ", false);
    assert_eq!(h.raw_str(cb, "ariaChecked"), Some("true"));
    h.press("terms-cb");
    assert_eq!(h.raw_str(cb, "ariaChecked"), Some("false"));

    let sw = h.find("alerts-sw");
    assert_eq!(h.raw_str(sw, "accessibilityRole"), Some("switch"));
    assert_eq!(h.raw_str(sw, "ariaChecked"), Some("false"));
    h.keydown("alerts-sw", "Enter", false);
    assert_eq!(h.raw_str(sw, "ariaChecked"), Some("true"));
    h.keydown("alerts-sw", "Space", false);
    assert_eq!(h.raw_str(sw, "ariaChecked"), Some("false"));

    h.press("toggle-sw-dis");
    assert!((h.opacity(sw) - 0.5).abs() < 1e-3);
    h.press("alerts-sw");
    assert_eq!(h.raw_str(sw, "ariaChecked"), Some("false"));
    h.press("toggle-sw-dis");
    assert!((h.opacity(sw) - 1.0).abs() < 1e-3);
}

#[component]
fn NavigationAndOverlaysApp() -> Element {
    let mut dialog_open = use_signal(|| false);
    let mut popover_open = use_signal(|| false);
    let mut menu_open = use_signal(|| false);
    let mut last_focus = use_signal(|| String::from("none"));
    let mut last_menu_item = use_signal(|| String::from("none"));

    let menu_items = vec![
        MenuItem::new("edit", "Edit"),
        MenuItem::new("archive", "Archive").disabled(true),
        MenuItem::new("delete", "Delete").destructive(true),
    ];

    rsx! {
        ThemeProvider {
            theme: Theme::DARK,
            ToastProvider {
                test_id: "toasts",
                Col {
                    padding: 12.0, gap: 12.0,
                    TabsRoot {
                        default_value: "one", id_prefix: "demo",
                        TabList {
                            test_id: "demo-tablist",
                            TabTrigger { value: "one", "One" }
                            TabTrigger { value: "two", disabled: true, "Two" }
                            TabTrigger { value: "three", "Three" }
                            TabTrigger { value: "four", "Four" }
                        }
                        TabPanel { value: "one", text { "Panel One" } }
                        TabPanel { value: "three", text { "Panel Three" } }
                        TabPanel { value: "four", text { "Panel Four" } }
                    }
                    Row {
                        Button { test_id: "open-dialog", on_press: move |_| dialog_open.set(true), "Open" }
                        Popover {
                            test_id: "info-pop", open: popover_open(), trigger_label: "Info",
                            on_open_change: move |v| popover_open.set(v),
                            text { test_id: "pop-body", "Popover content" }
                        }
                        Tooltip { test_id: "help-tip", content: "Helpful hint", text { "Hover me" } }
                        DropdownMenu {
                            test_id: "actions-dd", open: menu_open(), trigger_label: "Actions",
                            items: menu_items,
                            on_open_change: move |v| menu_open.set(v),
                            on_select: move |id| last_menu_item.set(id),
                        }
                    }
                    ToastControls {}
                    Modal {
                        test_id: "confirm-dlg", open: dialog_open(),
                        title: "Confirm Action", description: "Are you sure?",
                        focusable_ids: vec!["dlg-cancel".into(), "dlg-ok".into()],
                        previous_focus: "open-dialog",
                        on_focus_change: move |f: Option<String>| {
                            last_focus.set(f.unwrap_or_else(|| "none".into()));
                        },
                        on_close: move |_| dialog_open.set(false),
                        text { "Dialog Body" }
                    }
                    text { test_id: "overlay-log", "{last_focus}|{last_menu_item}" }
                }
            }
        }
    }
}

#[component]
fn ToastControls() -> Element {
    let mut toast = use_toast();
    rsx! {
        Row {
            Button {
                test_id: "push-info-toast",
                on_press: move |_| {
                    toast.push(
                        ToastOptions::new("Saved")
                            .with_description("Draft synced")
                            .with_variant(ToastVariant::Info)
                            .with_duration_ms(Some(1000)),
                    );
                },
                "InfoToast"
            }
            Button {
                test_id: "push-danger-toast",
                on_press: move |_| {
                    toast.push(
                        ToastOptions::new("Failed")
                            .with_variant(ToastVariant::Danger)
                            .with_duration_ms(None),
                    );
                },
                "DangerToast"
            }
            Button { test_id: "tick-toasts", on_press: move |_| { toast.tick(1200); }, "Tick" }
        }
    }
}

#[test]
fn tabs_overlays_dropdown_and_toasts_end_to_end() {
    let mut h = UiHarness::new(NavigationAndOverlaysApp);

    // 1. Tabs: ARIA roles, roving tabindex, skipping disabled tab "two".
    let t1 = h.find("demo-tab-one");
    assert_eq!(h.raw_str(t1, "accessibilityRole"), Some("tab"));
    assert_eq!(h.raw_str(t1, "ariaSelected"), Some("true"));
    assert_eq!(h.raw_str(t1, "ariaControls"), Some("demo-panel-one"));
    assert_eq!(h.raw_str(t1, "tabIndex"), Some("0"));
    assert!(h.maybe_find("demo-panel-one").is_some());
    assert!(h.maybe_find("demo-panel-three").is_none());

    h.keydown("demo-tab-one", "ArrowRight", false);
    assert_eq!(h.raw_str(h.find("demo-tab-three"), "ariaSelected"), Some("true"));
    assert_eq!(h.raw_str(h.find("demo-tab-three"), "tabIndex"), Some("0"));
    assert_eq!(h.raw_str(h.find("demo-tab-one"), "tabIndex"), Some("-1"));
    assert_eq!(
        h.raw_str(h.find("demo-panel-three"), "ariaLabelledBy"),
        Some("demo-tab-three")
    );
    h.keydown("demo-tab-three", "End", false);
    assert_eq!(h.raw_str(h.find("demo-tab-four"), "ariaSelected"), Some("true"));
    h.keydown("demo-tab-four", "ArrowRight", false);
    assert_eq!(h.raw_str(h.find("demo-tab-one"), "ariaSelected"), Some("true"));

    // 2. Dialog/Modal: FocusTrap wrap-around, Escape restore, backdrop dismiss.
    assert!(h.maybe_find("confirm-dlg").is_none());
    h.press("open-dialog");
    let dlg = h.find("confirm-dlg");
    assert_eq!(h.raw_str(dlg, "accessibilityRole"), Some("dialog"));
    assert_eq!(h.raw_str(dlg, "ariaModal"), Some("true"));
    assert_eq!(h.raw_str(dlg, "ariaLabelledBy"), Some("confirm-dlg-title"));
    assert_eq!(h.raw_str(dlg, "ariaDescribedBy"), Some("confirm-dlg-description"));
    assert_eq!(h.raw_str(dlg, "ariaControls"), Some("dlg-cancel"));
    h.render();
    assert_eq!(h.text_of("overlay-log"), "dlg-cancel|none");

    h.keydown("confirm-dlg", "Tab", false);
    assert_eq!(h.text_of("overlay-log"), "dlg-ok|none");
    h.keydown("confirm-dlg", "Tab", false);
    assert_eq!(h.text_of("overlay-log"), "dlg-cancel|none");
    h.keydown("confirm-dlg", "Tab", true);
    assert_eq!(h.text_of("overlay-log"), "dlg-ok|none");

    h.keydown("confirm-dlg", "Escape", false);
    assert!(h.maybe_find("confirm-dlg").is_none());
    assert_eq!(h.text_of("overlay-log"), "open-dialog|none");

    h.press("open-dialog");
    // A press inside the panel (e.g. on its text field or padding) must not reach the backdrop.
    h.press("confirm-dlg");
    assert!(h.maybe_find("confirm-dlg").is_some(), "panel press dismissed the dialog");
    assert!(h.maybe_find("confirm-dlg-backdrop").is_some());
    h.press("confirm-dlg-backdrop");
    assert!(h.maybe_find("confirm-dlg").is_none());

    // 3. Popover & Tooltip: ariaExpanded + Escape dismiss.
    assert_eq!(h.raw_str(h.find("info-pop-trigger"), "ariaExpanded"), Some("false"));
    h.press("info-pop-trigger");
    assert_eq!(h.raw_str(h.find("info-pop-trigger"), "ariaExpanded"), Some("true"));
    assert!(h.maybe_find("info-pop-content").is_some());
    h.keydown("info-pop-content", "Escape", false);
    assert!(h.maybe_find("info-pop-content").is_none());

    h.focus("help-tip-trigger");
    assert_eq!(h.raw_str(h.find("help-tip-bubble"), "accessibilityRole"), Some("tooltip"));
    assert_eq!(h.raw_str(h.find("help-tip-trigger"), "ariaExpanded"), Some("true"));
    h.keydown("help-tip-trigger", "Escape", false);
    assert!(h.maybe_find("help-tip-bubble").is_none());

    // 4. DropdownMenu: ArrowDown skips disabled item -> Enter selects & closes.
    h.press("actions-dd-trigger");
    assert_eq!(h.raw_str(h.find("actions-dd-menu"), "accessibilityRole"), Some("menu"));
    assert_eq!(
        h.raw_str(h.find("actions-dd-item-edit"), "accessibilityRole"),
        Some("menuitem")
    );
    assert_eq!(h.raw_str(h.find("actions-dd-item-edit"), "tabIndex"), Some("0"));
    h.keydown("actions-dd-menu", "ArrowDown", false);
    assert_eq!(h.raw_str(h.find("actions-dd-item-delete"), "tabIndex"), Some("0"));
    h.keydown("actions-dd-menu", "Enter", false);
    assert!(h.maybe_find("actions-dd-menu").is_none());
    assert_eq!(h.text_of("overlay-log"), "open-dialog|delete");

    // 5. Toasts: polite + assertive ariaLive, tick auto-expire, manual dismiss.
    h.press("push-info-toast");
    h.press("push-danger-toast");
    assert_eq!(h.raw_str(h.find("toasts-item-1"), "accessibilityRole"), Some("status"));
    assert_eq!(h.raw_str(h.find("toasts-item-1"), "ariaLive"), Some("polite"));
    assert_eq!(h.raw_str(h.find("toasts-item-2"), "ariaLive"), Some("assertive"));

    h.press("tick-toasts");
    assert!(h.maybe_find("toasts-item-1").is_none());
    assert!(h.maybe_find("toasts-item-2").is_some());
    h.press("toasts-dismiss-2");
    assert!(h.maybe_find("toasts-item-2").is_none());
}

#[cfg(feature = "virtual-list")]
#[component]
fn VirtualList100kApp() -> Element {
    let mut selected = use_signal(|| 0usize);
    let heights = use_signal(|| {
        RowHeights::variable_from_fn(100_000, |i| 32 + ((i % 4) as u32) * 8)
    });

    rsx! {
        ThemeProvider {
            theme: Theme::LIGHT,
            Col {
                VirtualList {
                    test_id: "vlist", heights: heights, viewport_height: 300.0,
                    overscan: 2, initial_selected: Some(0),
                    on_select: move |idx| selected.set(idx),
                    render_row: move |(idx, _sel)| rsx! { text { "Row #{idx}" } },
                }
                text { test_id: "vlist-selected", "{selected}" }
            }
        }
    }
}

#[cfg(feature = "virtual-list")]
#[test]
fn virtual_list_100k_rows_renders_o_viewport_with_scroll_and_keyboard() {
    let mut h = UiHarness::new(VirtualList100kApp);

    assert!(
        h.live_node_count() < 80,
        "100k VirtualList must render O(viewport) nodes, got {}",
        h.live_node_count()
    );
    assert!(h.maybe_find("vlist-row-0").is_some());
    assert!(h.maybe_find("vlist-row-50000").is_none());

    h.scroll("vlist", 2_200_000.0);
    assert!(h.maybe_find("vlist-row-0").is_none());
    assert!(h.maybe_find("vlist-row-50000").is_some());
    assert!(h.live_node_count() < 80);

    let top_h = h.backend.layout_of(h.find("vlist-top-spacer")).unwrap().height;
    let bot_h = h.backend.layout_of(h.find("vlist-bottom-spacer")).unwrap().height;
    assert!(top_h > 2_000_000.0);
    assert!(bot_h > 2_000_000.0);

    h.keydown("vlist", "ArrowDown", false);
    assert_eq!(h.text_of("vlist-selected"), "1");
    h.keydown("vlist", "PageDown", false);
    let after_pgdn: usize = h.text_of("vlist-selected").parse().unwrap();
    assert!(after_pgdn > 1);
    h.keydown("vlist", "ArrowUp", false);
    assert_eq!(h.text_of("vlist-selected").parse::<usize>().unwrap(), after_pgdn - 1);
    h.keydown("vlist", "End", false);
    assert_eq!(h.text_of("vlist-selected"), "99999");
    assert_eq!(h.raw_str(h.find("vlist-row-99999"), "ariaSelected"), Some("true"));
    h.keydown("vlist", "Home", false);
    assert_eq!(h.text_of("vlist-selected"), "0");
}

#[component]
fn WrappedVerticalSeparatorApp() -> Element {
    rsx! {
        view {
            test_id: "wrap-row",
            nve_style: TypedStyle::row()
                .width_px(100.0)
                .flex_wrap(FlexWrap::Wrap)
                .align(FlexAlign::Center),
            view { nve_style: TypedStyle::new().width_px(80.0).height_px(40.0) }
            Separator { test_id: "sep-v", orientation: Orientation::Vertical }
            view { nve_style: TypedStyle::new().width_px(80.0).height_px(20.0) }
        }
    }
}

/// A vertical rule spans its own flex line (`align-self: stretch`), never a percentage of the
/// whole wrapping row: that feedback loop inflated the line and overflowed the row.
#[test]
fn vertical_separator_spans_its_flex_line_in_a_wrapping_centered_row() {
    let h = UiHarness::new(WrappedVerticalSeparatorApp);
    let sep = h.backend.layout_of(h.find("sep-v")).expect("separator layout");
    let row = h.backend.layout_of(h.find("wrap-row")).expect("row layout");
    assert_eq!((sep.width, sep.height), (1.0, 40.0), "separator spans line 1 only: {sep:?}");
    assert_eq!(row.height, 60.0, "two lines (40 + 20) with no inflated line: {row:?}");
}
