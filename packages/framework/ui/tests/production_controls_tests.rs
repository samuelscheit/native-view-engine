//! Deep state-machine and live `HeadlessBackend` E2E rendering tests for:
//! - Anchored `Popover`, `Tooltip`, and `DropdownMenu` collision-flipping integration
//! - `Combobox` / `ComboboxState` (single & multi-select, custom filter, IME guard, ARIA)
//! - `SplitPane` / `SplitPaneState` (horizontal/vertical, clamping, collapse/restore, keyboard)
//! - `BottomSheet` / `BottomSheetState` (multi-detent, flick velocity, inner-scroll handoff)

mod common;

use common::UiHarness;
use nve_dioxus::elements::backend::RenderBackend;
use nve_dioxus::prelude::*;
use nve_ui::prelude::*;
use nve_ui::Orientation;

// ---------------------------------------------------------------------------
// 1. Anchored Overlay Wiring E2E (`Popover`, `Tooltip`, `DropdownMenu`)
// ---------------------------------------------------------------------------

#[component]
fn AnchoredOverlaysApp() -> Element {
    let vp = Viewport::new(800.0, 600.0);
    let bottom_right_anchor = Rect::new(720.0, 540.0, 60.0, 36.0);
    let top_edge_anchor = Rect::new(350.0, 10.0, 80.0, 28.0);
    let items = vec![
        MenuItem::new("open", "Open"),
        MenuItem::new("delete", "Delete").destructive(true),
    ];

    rsx! {
        ThemeProvider {
            theme: UiTheme::light(),
            Col {
                padding: 8.0,
                gap: 8.0,
                Popover {
                    test_id: "anchored-pop",
                    open: true,
                    trigger_label: "Info",
                    anchor_rect: Some(bottom_right_anchor),
                    overlay_size: Some(Size::new(200.0, 120.0)),
                    viewport: Some(vp),
                    position_options: Some(
                        PositionOptions::new(Placement::BottomStart).with_auto_size(true)
                    ),
                    text { "Body" }
                }
                Tooltip {
                    test_id: "anchored-tip",
                    open: Some(true),
                    content: "Tip",
                    anchor_rect: Some(top_edge_anchor),
                    overlay_size: Some(Size::new(140.0, 36.0)),
                    viewport: Some(vp),
                    text { "Hover" }
                }
                DropdownMenu {
                    test_id: "anchored-menu",
                    open: true,
                    trigger_label: "Actions",
                    items: items,
                    anchor_rect: Some(bottom_right_anchor),
                    overlay_size: Some(Size::new(180.0, 100.0)),
                    viewport: Some(vp),
                }
            }
        }
    }
}

#[test]
fn anchored_overlays_flip_and_expose_computed_geometry() {
    let h = UiHarness::new(AnchoredOverlaysApp);

    // Popover at bottom-right with BottomStart double-flips to TopEnd.
    let pop = h.find("anchored-pop-content");
    assert_eq!(h.raw_str(pop, "data-placement"), Some("top-end"));
    assert_eq!(h.raw_str(pop, "data-flipped"), Some("true"));
    assert!(h.raw_str(pop, "data-arrow-x").is_some());

    // Tooltip defaults to Top; at y=10 it collides with top edge and flips to Bottom.
    let tip = h.find("anchored-tip-bubble");
    assert_eq!(h.raw_str(tip, "data-placement"), Some("bottom"));
    assert_eq!(h.raw_str(tip, "data-flipped"), Some("true"));

    // DropdownMenu defaults to BottomStart; at bottom-right it double-flips to TopEnd.
    let menu = h.find("anchored-menu-menu");
    assert_eq!(h.raw_str(menu, "data-placement"), Some("top-end"));
    assert_eq!(h.raw_str(menu, "data-flipped"), Some("true"));
}

// ---------------------------------------------------------------------------
// 2. Combobox State Machine + HeadlessBackend E2E
// ---------------------------------------------------------------------------

#[test]
fn combobox_state_keyboard_multi_select_custom_filter_and_ime_guard() {
    let opts = vec![
        ComboboxOption::new("rs", "Rust"),
        ComboboxOption::new("py", "Python").disabled(true),
        ComboboxOption::new("ts", "TypeScript"),
        ComboboxOption::new("rb", "Ruby"),
    ];

    // Single-select navigation skips disabled Python, wraps around, and selects on Enter.
    let mut single = ComboboxState::new(opts.clone(), false);
    assert_eq!(single.handle_key("ArrowDown"), ComboboxAction::Opened);
    assert_eq!(
        single.active_option_id("lang").as_deref(),
        Some("lang-option-rs")
    );
    assert_eq!(
        single.handle_key("ArrowDown"),
        ComboboxAction::Highlighted(Some("ts".into()))
    );
    assert_eq!(
        single.handle_key("End"),
        ComboboxAction::Highlighted(Some("rb".into()))
    );
    assert_eq!(
        single.handle_key("Home"),
        ComboboxAction::Highlighted(Some("rs".into()))
    );
    assert_eq!(
        single.handle_key("ArrowUp"),
        ComboboxAction::Highlighted(Some("rb".into()))
    );
    assert_eq!(
        single.handle_key("Enter"),
        ComboboxAction::Selected("rb".into())
    );
    assert!(!single.is_open());
    assert_eq!(single.query(), "Ruby");
    assert_eq!(single.selected(), &["rb"]);

    // Custom filter predicate (prefix-only match).
    single.set_query_with_filter("ru", |opt, q| {
        opt.label.to_ascii_lowercase().starts_with(&q.to_ascii_lowercase())
    });
    let labels: Vec<&str> = single
        .filtered_options()
        .iter()
        .map(|o| o.label.as_str())
        .collect();
    assert_eq!(labels, vec!["Rust", "Ruby"]);

    // Multi-select + Backspace removes last tag only when query is empty + IME composing guard.
    let mut multi = ComboboxState::new(opts, true);
    multi.open();
    assert_eq!(
        multi.handle_key("Enter"),
        ComboboxAction::Selected("rs".into())
    );
    assert!(multi.is_open()); // stays open in multi mode
    assert_eq!(
        multi.handle_key("ArrowDown"),
        ComboboxAction::Highlighted(Some("ts".into()))
    );
    assert_eq!(
        multi.handle_key("Enter"),
        ComboboxAction::Selected("ts".into())
    );
    assert_eq!(multi.selected(), &["rs", "ts"]);

    // IME composing active: Enter, Escape, Backspace, arrows MUST all be suppressed!
    multi.set_composing(true);
    assert_eq!(multi.handle_key("Enter"), ComboboxAction::None);
    assert_eq!(multi.handle_key("Escape"), ComboboxAction::None);
    assert_eq!(multi.handle_key("Backspace"), ComboboxAction::None);
    assert_eq!(multi.handle_key("ArrowDown"), ComboboxAction::None);
    assert!(multi.is_open());
    assert_eq!(multi.selected(), &["rs", "ts"]);

    // Ending composition restores Backspace-last-tag and Escape dismissal.
    multi.set_composing(false);
    assert_eq!(
        multi.handle_key("Backspace"),
        ComboboxAction::RemovedLastTag("ts".into())
    );
    assert_eq!(multi.selected(), &["rs"]);
    assert_eq!(multi.handle_key("Escape"), ComboboxAction::Closed);
    assert!(!multi.is_open());

    // Disabled option cannot be selected directly.
    assert_eq!(multi.select_value("py"), ComboboxAction::None);
}

#[component]
fn ComboboxApp() -> Element {
    let mut chosen = use_signal(Vec::<String>::new);
    let mut loading = use_signal(|| false);
    let mut composing = use_signal(|| false);
    let opts = vec![
        ComboboxOption::new("rust", "Rust"),
        ComboboxOption::new("zig", "Zig").disabled(true),
        ComboboxOption::new("swift", "Swift"),
    ];

    rsx! {
        ThemeProvider {
            theme: UiTheme::light(),
            Col {
                padding: 12.0,
                gap: 8.0,
                Combobox {
                    test_id: "cb",
                    label: "Languages",
                    options: opts,
                    multi: true,
                    loading: loading(),
                    composing: composing(),
                    on_change: move |vals| chosen.set(vals),
                }
                Button { test_id: "toggle-loading", on_press: move |_| loading.toggle(), "Load" }
                Button { test_id: "toggle-ime", on_press: move |_| composing.toggle(), "IME" }
                text { test_id: "cb-selected", "{chosen().join(\",\")}" }
            }
        }
    }
}

#[test]
fn combobox_headless_e2e_aria_keyboard_tags_and_states() {
    let mut h = UiHarness::new(ComboboxApp);

    let input = h.find("cb-input");
    assert_eq!(h.raw_str(input, "accessibilityRole"), Some("combobox"));
    assert_eq!(h.raw_str(input, "ariaExpanded"), Some("false"));
    assert_eq!(h.raw_str(input, "ariaControls"), Some("cb-listbox"));
    assert_eq!(h.raw_str(input, "ariaAutoComplete"), Some("list"));
    assert!(h.maybe_find("cb-listbox").is_none());

    // Focus opens listbox with role="listbox", ariaMultiSelectable="true", and active descendant.
    h.focus("cb-input");
    let input = h.find("cb-input");
    assert_eq!(h.raw_str(input, "ariaExpanded"), Some("true"));
    assert_eq!(
        h.raw_str(input, "ariaActiveDescendant"),
        Some("cb-option-rust")
    );
    let lb = h.find("cb-listbox");
    assert_eq!(h.raw_str(lb, "accessibilityRole"), Some("listbox"));
    assert_eq!(h.raw_str(lb, "ariaMultiSelectable"), Some("true"));
    assert_eq!(h.raw_bool(h.find("cb-option-zig"), "disabled"), Some(true));

    // ArrowDown skips disabled "zig" and highlights "swift"; Enter selects "swift".
    h.keydown("cb-input", "ArrowDown", false);
    assert_eq!(
        h.raw_str(h.find("cb-input"), "ariaActiveDescendant"),
        Some("cb-option-swift")
    );
    h.keydown("cb-input", "Enter", false);
    assert_eq!(h.text_of("cb-selected"), "swift");
    assert_eq!(
        h.raw_str(h.find("cb-option-swift"), "ariaSelected"),
        Some("true")
    );
    assert!(h.maybe_find("cb-tag-swift").is_some());

    // IME composing blocks Backspace from deleting the selected tag.
    h.press("toggle-ime");
    h.keydown("cb-input", "Backspace", false);
    assert_eq!(h.text_of("cb-selected"), "swift");
    h.press("toggle-ime");

    // Backspace when query is empty removes last tag.
    h.keydown("cb-input", "Backspace", false);
    assert_eq!(h.text_of("cb-selected"), "");

    // Typing a query with no match shows empty message; loading flag shows loading state.
    h.text_change("cb-input", "zzzzzz");
    assert!(h.maybe_find("cb-empty").is_some());
    h.press("toggle-loading");
    assert!(h.maybe_find("cb-loading").is_some());

    // Escape closes listbox and blur clears focus ring.
    h.keydown("cb-input", "Escape", false);
    assert!(h.maybe_find("cb-listbox").is_none());
    h.blur("cb-input");
    assert_eq!(h.backend.node(h.find("cb-input")).unwrap().style.border_width, 1.0);
}

// ---------------------------------------------------------------------------
// 3. SplitPane State Machine + HeadlessBackend E2E
// ---------------------------------------------------------------------------

#[test]
fn split_pane_state_clamping_collapse_drag_and_keyboard() {
    let mut sp = SplitPaneState::new(Orientation::Horizontal, 1006.0, 0.5)
        .with_divider_size(6.0)
        .with_min_primary(SplitBound::Pixels(200.0))
        .with_max_primary(SplitBound::Ratio(0.7))
        .with_min_secondary(SplitBound::Pixels(250.0));

    // Usable = 1000px; bounds = [200, min(700, 750) = 700].
    assert_eq!(sp.usable_size(), 1000.0);
    assert_eq!(sp.effective_bounds_px(), (200.0, 700.0));
    assert_eq!(sp.primary_px(), 500.0);
    assert_eq!(sp.secondary_px(), 500.0);

    // Drag clamped at max_primary (700) and min_primary (200).
    assert_eq!(sp.apply_drag_delta(400.0), 700.0);
    assert_eq!(sp.secondary_px(), 300.0);
    assert_eq!(sp.apply_drag_delta(-900.0), 200.0);
    assert_eq!(sp.secondary_px(), 800.0);

    // Keyboard End/Home/ArrowRight/ArrowLeft.
    assert!(sp.handle_key("End", 50.0));
    assert_eq!(sp.primary_px(), 700.0);
    assert!(sp.handle_key("Home", 50.0));
    assert_eq!(sp.primary_px(), 200.0);
    assert!(sp.handle_key("ArrowRight", 50.0));
    assert_eq!(sp.primary_px(), 250.0);
    assert!(sp.handle_key("ArrowLeft", 25.0));
    assert_eq!(sp.primary_px(), 225.0);

    // Collapse saves 225px ratio and restores cleanly.
    assert!(sp.collapse());
    assert!(sp.is_collapsed());
    assert_eq!(sp.primary_px(), 0.0);
    assert_eq!(sp.secondary_px(), 1000.0);
    assert!(sp.restore());
    assert!(!sp.is_collapsed());
    assert!((sp.primary_px() - 225.0).abs() < 1e-2);

    // Vertical orientation responds to ArrowUp/ArrowDown and ignores ArrowLeft/Right.
    let mut vert = SplitPaneState::new(Orientation::Vertical, 506.0, 0.4)
        .with_divider_size(6.0)
        .with_min_primary(SplitBound::Ratio(0.2))
        .with_min_secondary(SplitBound::Ratio(0.2));
    assert!(!vert.handle_key("ArrowRight", 20.0));
    assert!(vert.handle_key("ArrowDown", 50.0));
    assert!((vert.primary_px() - 250.0).abs() < 1e-2);
    assert!(vert.handle_key("ArrowUp", 100.0));
    assert!((vert.primary_px() - 150.0).abs() < 1e-2);
}

#[component]
fn SplitPaneApp() -> Element {
    let mut last_ratio = use_signal(|| 0.5f32);
    rsx! {
        ThemeProvider {
            theme: UiTheme::dark(),
            SplitPane {
                test_id: "sp",
                orientation: Orientation::Horizontal,
                container_size: 806.0,
                divider_size: 6.0,
                initial_ratio: 0.5,
                min_primary: SplitBound::Pixels(160.0),
                max_primary: Some(SplitBound::Pixels(600.0)),
                min_secondary: SplitBound::Pixels(160.0),
                keyboard_step_px: 40.0,
                on_change: move |r| last_ratio.set(r),
                primary: rsx! { text { test_id: "left-pane", "Left {last_ratio:.2}" } },
                secondary: rsx! { text { test_id: "right-pane", "Right" } },
            }
        }
    }
}

#[test]
fn split_pane_headless_e2e_separator_aria_and_keyboard_resize() {
    let mut h = UiHarness::new(SplitPaneApp);

    let div = h.find("sp-divider");
    assert_eq!(h.raw_str(div, "accessibilityRole"), Some("separator"));
    assert_eq!(h.raw_str(div, "ariaOrientation"), Some("vertical"));
    assert_eq!(h.raw_str(div, "ariaValueNow"), Some("50"));
    assert_eq!(h.raw_str(div, "ariaValueMin"), Some("20"));
    assert_eq!(h.raw_str(div, "ariaValueMax"), Some("75"));

    let primary_w0 = h.backend.layout_of(h.find("sp-primary")).unwrap().width;
    assert!((primary_w0 - 400.0).abs() < 1.0);

    h.keydown("sp-divider", "ArrowRight", false);
    assert_eq!(h.raw_str(h.find("sp-divider"), "ariaValueNow"), Some("55"));
    let primary_w1 = h.backend.layout_of(h.find("sp-primary")).unwrap().width;
    assert!((primary_w1 - 440.0).abs() < 1.0);

    // Enter collapses primary to 0px; Enter again restores to 440px.
    h.keydown("sp-divider", "Enter", false);
    assert_eq!(h.raw_str(h.find("sp-primary"), "data-collapsed"), Some("true"));
    assert_eq!(h.raw_str(h.find("sp-divider"), "ariaValueNow"), Some("0"));
    h.keydown("sp-divider", "Enter", false);
    assert_eq!(h.raw_str(h.find("sp-primary"), "data-collapsed"), Some("false"));
    assert_eq!(h.raw_str(h.find("sp-divider"), "ariaValueNow"), Some("55"));
}

// ---------------------------------------------------------------------------
// 4. BottomSheet State Machine + HeadlessBackend E2E
// ---------------------------------------------------------------------------

#[test]
fn bottom_sheet_snap_points_flick_velocity_and_inner_scroll_handoff() {
    let mut sheet = BottomSheetState::new(
        vec![SheetDetent::Full, SheetDetent::Peek, SheetDetent::Half],
        1, // Half (0.50) after sorting -> [Peek(0.25), Half(0.50), Full(0.92)]
        1000.0,
    );
    assert_eq!(sheet.active_detent(), SheetDetent::Half);
    assert!((sheet.sheet_height_px() - 500.0).abs() < 1e-2);

    // Upward drag expands sheet up to Full (920px), passing excess upward delta to inner scroll!
    let res = sheet.handle_drag_delta(-500.0, 0.0);
    match res {
        DragHandoff::SheetMoved {
            height_fraction,
            consumed_dy,
            remaining_scroll_dy,
        } => {
            assert!((height_fraction - 0.92).abs() < 1e-3);
            assert!((consumed_dy - (-420.0)).abs() < 1e-2);
            assert!((remaining_scroll_dy - (-80.0)).abs() < 1e-2);
        }
        other => panic!("expected SheetMoved, got {other:?}"),
    }

    // Further upward drag while fully expanded hands off 100% to inner scroll.
    assert_eq!(
        sheet.handle_drag_delta(-120.0, 80.0),
        DragHandoff::InnerScroll { delta_y: -120.0 }
    );

    // Downward drag while inner scroll_top > 0 scrolls inner content first!
    assert_eq!(
        sheet.handle_drag_delta(50.0, 100.0),
        DragHandoff::InnerScroll { delta_y: 50.0 }
    );

    // Downward drag exceeding inner_scroll_top consumes scroll_top and drags sheet with remainder.
    let handoff = sheet.handle_drag_delta(150.0, 50.0);
    match handoff {
        DragHandoff::SheetMoved {
            height_fraction,
            consumed_dy,
            remaining_scroll_dy,
        } => {
            assert!((consumed_dy - 100.0).abs() < 1e-2);
            assert!((remaining_scroll_dy - 50.0).abs() < 1e-2);
            assert!((height_fraction - 0.82).abs() < 1e-3);
        }
        other => panic!("expected SheetMoved, got {other:?}"),
    }

    // Fast downward flick snaps from 0.82 down to Half (0.50).
    assert_eq!(
        sheet.end_drag(800.0),
        SheetSnapOutcome::Snapped {
            index: 1,
            detent: SheetDetent::Half,
            fraction: 0.50,
        }
    );

    // Fast upward flick snaps from Half up to Full.
    assert_eq!(
        sheet.end_drag(-900.0),
        SheetSnapOutcome::Snapped {
            index: 2,
            detent: SheetDetent::Full,
            fraction: 0.92,
        }
    );

    // Slow drag below half of Peek (0.25 * 0.5 = 0.125) dismisses cleanly.
    sheet.open_at(0); // Peek = 0.25
    let _ = sheet.handle_drag_delta(180.0, 0.0); // 0.25 - 0.18 = 0.07
    assert_eq!(sheet.end_drag(20.0), SheetSnapOutcome::Dismissed);
    assert!(!sheet.is_open());

    // Fast downward flick from Peek dismisses when dismissible, stays at Peek when non-dismissible.
    sheet.open_at(0);
    assert_eq!(sheet.end_drag(750.0), SheetSnapOutcome::Dismissed);

    let mut persistent = BottomSheetState::new(vec![SheetDetent::Custom(0.3), SheetDetent::Full], 0, 800.0)
        .with_dismissible(false);
    assert_eq!(
        persistent.end_drag(1200.0),
        SheetSnapOutcome::Snapped {
            index: 0,
            detent: SheetDetent::Custom(0.3),
            fraction: 0.3,
        }
    );
    assert!(persistent.is_open());
}

#[component]
fn BottomSheetApp() -> Element {
    let mut open = use_signal(|| true);
    let mut last_detent = use_signal(|| String::from("half"));

    rsx! {
        ThemeProvider {
            theme: UiTheme::light(),
            Col {
                BottomSheet {
                    test_id: "sheet",
                    open: open(),
                    title: "Details",
                    viewport_height: 600.0,
                    initial_detent: 1,
                    on_dismiss: move |_| open.set(false),
                    on_detent_change: move |d: SheetDetent| {
                        last_detent.set(format!("{:.2}", d.fraction()));
                    },
                    text { "Scrollable content" }
                }
                text { test_id: "sheet-status", "{open}|{last_detent}" }
            }
        }
    }
}

#[test]
fn bottom_sheet_headless_e2e_detents_keyboard_and_dismiss() {
    let mut h = UiHarness::new(BottomSheetApp);

    let sheet = h.find("sheet");
    assert_eq!(h.raw_str(sheet, "accessibilityRole"), Some("dialog"));
    assert_eq!(h.raw_str(sheet, "ariaModal"), Some("true"));
    assert_eq!(h.raw_str(sheet, "data-detent-fraction"), Some("0.50"));

    let handle = h.find("sheet-handle");
    assert_eq!(h.raw_str(handle, "accessibilityRole"), Some("slider"));
    assert_eq!(h.raw_str(handle, "ariaValueNow"), Some("50"));
    assert_eq!(h.raw_str(handle, "ariaValueMin"), Some("25"));
    assert_eq!(h.raw_str(handle, "ariaValueMax"), Some("92"));

    // ArrowUp snaps Half -> Full (92%).
    h.keydown("sheet-handle", "ArrowUp", false);
    assert_eq!(h.raw_str(h.find("sheet-handle"), "ariaValueNow"), Some("92"));
    assert_eq!(h.text_of("sheet-status"), "true|0.92");

    // Home snaps to Peek (25%); inner scrollview handles scroll events cleanly.
    #[cfg(feature = "virtual-list")]
    h.scroll("sheet-scroll", 40.0);
    h.keydown("sheet-handle", "Home", false);
    assert_eq!(h.raw_str(h.find("sheet-handle"), "ariaValueNow"), Some("25"));
    assert_eq!(h.text_of("sheet-status"), "true|0.25");

    // ArrowDown from Peek dismisses the sheet cleanly.
    h.keydown("sheet-handle", "ArrowDown", false);
    assert!(h.maybe_find("sheet").is_none());
    assert_eq!(h.text_of("sheet-status"), "false|0.25");
}
