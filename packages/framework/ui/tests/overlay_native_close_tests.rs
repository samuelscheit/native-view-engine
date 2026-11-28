//! Browser-initiated overlay closes: a bridged native `close` (`<dialog>` `close`, `[popover]`
//! `toggle` to `closed`, delivered target-only via `NVE_EVENT_FLAG_NO_BUBBLE`) drives `Dialog` /
//! `Popover` / `DropdownMenu` `on_open_change(false)` exactly once per open period, is a no-op
//! while already closed, and never reaches an enclosing overlay.

mod common;

use common::UiHarness;
use nve_abi::EventBatchBuf;
use nve_dioxus::prelude::*;
use nve_ui::prelude::*;

#[component]
fn OverlayCloseApp() -> Element {
    let mut dialog_open = use_signal(|| true);
    let mut popover_open = use_signal(|| true);
    let mut closes = use_signal(|| 0u32);
    let mut changes = use_signal(|| 0u32);
    let mut popover_changes = use_signal(|| 0u32);
    rsx! {
        ThemeProvider {
            theme: Theme::LIGHT,
            Col {
                Dialog {
                    open: dialog_open(),
                    keep_mounted: true,
                    test_id: "dlg",
                    title: "Settings",
                    on_open_change: move |open: bool| {
                        changes += 1;
                        dialog_open.set(open);
                    },
                    on_close: move |()| closes += 1,
                    text { "body" }
                }
                Popover {
                    open: popover_open(),
                    keep_mounted: true,
                    trigger_label: "More",
                    test_id: "pop",
                    on_open_change: move |open: bool| {
                        popover_changes += 1;
                        popover_open.set(open);
                    },
                    text { "menu" }
                }
                pressable {
                    test_id: "reopen",
                    onpress: move |_| {
                        dialog_open.set(true);
                        popover_open.set(true);
                    },
                    text { "reopen" }
                }
                text { test_id: "state", "{dialog_open}|{popover_open}|{changes}|{closes}" }
                text { test_id: "popover-changes", "{popover_changes}" }
            }
        }
    }
}

#[test]
fn native_close_drives_dialog_and_popover_state_exactly_once() {
    let mut h = UiHarness::new(OverlayCloseApp);
    assert_eq!(h.text_of("state"), "true|true|0|0");

    h.custom("dlg", "close", false);
    assert_eq!(
        h.text_of("state"),
        "false|true|1|1",
        "native dialog close reports once"
    );
    // A late duplicate `close` (app-driven `close()` echo) while closed is a no-op.
    h.custom("dlg", "close", false);
    assert_eq!(h.text_of("state"), "false|true|1|1");

    h.custom("pop-content", "close", false);
    assert_eq!(
        h.text_of("state"),
        "false|false|1|1",
        "popover light dismiss closes"
    );
    h.custom("pop-content", "close", false);
    assert_eq!(h.text_of("state"), "false|false|1|1");
    assert_eq!(h.text_of("popover-changes"), "1");

    // Reopened overlays close again through the same bridge.
    h.press("reopen");
    assert_eq!(h.text_of("state"), "true|true|1|1");
    h.custom("dlg", "close", false);
    h.custom("pop-content", "close", false);
    assert_eq!(h.text_of("state"), "false|false|2|2");
    assert_eq!(h.text_of("popover-changes"), "2");
}

/// `Escape` and the bridged `close` it triggers can land in ONE host batch: the popover reports
/// its close once (the latch, not the captured `open` prop, decides).
#[test]
fn popover_same_batch_escape_and_native_close_report_once() {
    let mut h = UiHarness::new(OverlayCloseApp);
    let mut batch = EventBatchBuf::new();
    h.push_keydown(&mut batch, "pop-content", "Escape", false);
    h.push_custom(&mut batch, "pop-content", "close", false);
    h.dispatch(&batch);
    assert_eq!(h.text_of("popover-changes"), "1");
    assert_eq!(h.text_of("state"), "true|false|0|0");

    // Trigger toggle + bridged close in one batch, after reopening.
    h.press("reopen");
    batch.clear();
    h.push_keydown(&mut batch, "pop-trigger", "Escape", false);
    h.push_custom(&mut batch, "pop-content", "close", false);
    h.dispatch(&batch);
    assert_eq!(h.text_of("popover-changes"), "2");
}

#[test]
fn unrelated_custom_events_do_not_close_overlays() {
    let mut h = UiHarness::new(OverlayCloseApp);
    h.custom("dlg", "toggle", false);
    h.custom("pop-content", "cancel", false);
    assert_eq!(h.text_of("state"), "true|true|0|0");
}

#[component]
fn NestedOverlayApp() -> Element {
    let mut dialog_open = use_signal(|| true);
    let mut popover_open = use_signal(|| true);
    let mut menu_open = use_signal(|| true);
    let mut menu_changes = use_signal(|| 0u32);
    rsx! {
        ThemeProvider {
            theme: Theme::LIGHT,
            Col {
                Dialog {
                    open: dialog_open(),
                    keep_mounted: true,
                    test_id: "outer",
                    on_open_change: move |open: bool| dialog_open.set(open),
                    Popover {
                        open: popover_open(),
                        keep_mounted: true,
                        trigger_label: "Menu",
                        test_id: "inner",
                        on_open_change: move |open: bool| popover_open.set(open),
                        text { "item" }
                    }
                    DropdownMenu {
                        open: menu_open(),
                        keep_mounted: true,
                        trigger_label: "Actions",
                        test_id: "dd",
                        items: vec![MenuItem::new("copy", "Copy"), MenuItem::new("paste", "Paste")],
                        on_open_change: move |open: bool| {
                            menu_changes += 1;
                            menu_open.set(open);
                        },
                    }
                    // An app overlay without `onclose` (e.g. a raw `view { popover }`) that
                    // listens for other events (so it is a dispatch target).
                    view { test_id: "bare", onpress: move |_| {}, text { "bare" } }
                }
                text {
                    test_id: "state",
                    "{dialog_open}|{popover_open}|{menu_open}|{menu_changes}"
                }
            }
        }
    }
}

/// `close` does not bubble natively: a nested popover's light dismiss closes only itself.
#[test]
fn nested_popover_close_is_target_only() {
    let mut h = UiHarness::new(NestedOverlayApp);
    assert_eq!(h.text_of("state"), "true|true|true|0");
    h.custom("inner-content", "close", false);
    assert_eq!(h.text_of("state"), "true|false|true|0");
    h.custom("outer", "close", false);
    assert_eq!(h.text_of("state"), "false|false|true|0");
}

/// `DropdownMenu { keep_mounted }` (`popover="auto"`) light dismiss / `Esc` syncs its `open` and
/// the trigger's `aria-expanded`, and leaves the enclosing `Dialog` open.
#[test]
fn dropdown_native_close_syncs_open_and_aria_expanded_inside_dialog() {
    let mut h = UiHarness::new(NestedOverlayApp);
    let trigger = h.find("dd-trigger");
    assert_eq!(h.raw_str(trigger, "ariaExpanded"), Some("true"));
    h.custom("dd-menu", "close", false);
    assert_eq!(h.text_of("state"), "true|true|false|1");
    assert_eq!(h.raw_str(trigger, "ariaExpanded"), Some("false"));
    // Escape + bridged close in one batch after reopening: one report.
    h.press("dd-trigger");
    assert_eq!(h.text_of("state"), "true|true|true|2");
    let mut batch = EventBatchBuf::new();
    h.push_keydown(&mut batch, "dd-menu", "Escape", false);
    h.push_custom(&mut batch, "dd-menu", "close", false);
    h.dispatch(&batch);
    assert_eq!(
        h.text_of("state"),
        "true|true|false|3",
        "Escape closes only the innermost menu, not the enclosing dialog"
    );
    // With the menu closed, Escape reaches the dialog's focus trap.
    h.keydown("dd-menu", "Escape", false);
    assert_eq!(h.text_of("state"), "false|true|false|3");
}

/// A target-only `close` on an `onclose`-less nested overlay reaches nobody; the same event
/// marked bubbling WOULD reach the enclosing `Dialog`, proving delivery honors the flag.
#[test]
fn onclose_less_nested_overlay_close_never_reaches_enclosing_dialog() {
    let mut h = UiHarness::new(NestedOverlayApp);
    h.custom("bare", "close", false);
    assert_eq!(h.text_of("state"), "true|true|true|0");
    h.custom("bare", "close", true);
    assert_eq!(
        h.text_of("state"),
        "false|true|true|0",
        "control: a bubbling close does reach the dialog"
    );
}

#[component]
fn VetoAndNoHandlerApp() -> Element {
    let mut bump = use_signal(|| 0u32);
    let mut outer_escapes = use_signal(|| 0u32);
    let mut dd_false = use_signal(|| 0u32);
    let mut dd_true = use_signal(|| 0u32);
    let mut pop_false = use_signal(|| 0u32);
    let mut pop_true = use_signal(|| 0u32);
    rsx! {
        ThemeProvider {
            theme: Theme::LIGHT,
            view {
                onkeydown: move |ev: KeyEvent| {
                    if matches!(ev.data().key.as_str(), "Escape" | "Esc") {
                        outer_escapes += 1;
                    }
                },
                // Controlled DropdownMenu whose parent ALWAYS vetoes `on_open_change(false)`.
                DropdownMenu {
                    open: true,
                    keep_mounted: true,
                    trigger_label: "VetoMenu",
                    test_id: "veto-dd",
                    items: vec![
                        MenuItem::new("copy", "Copy"),
                        MenuItem::new("paste", "Paste"),
                        MenuItem::new("delete", "Delete"),
                    ],
                    on_open_change: move |next: bool| {
                        if next {
                            dd_true += 1;
                        } else {
                            dd_false += 1;
                        }
                    },
                }
                // Controlled Popover whose parent ALWAYS vetoes `on_open_change(false)`.
                Popover {
                    open: true,
                    keep_mounted: true,
                    trigger_label: "VetoPop",
                    test_id: "veto-pop",
                    on_open_change: move |next: bool| {
                        if next {
                            pop_true += 1;
                        } else {
                            pop_false += 1;
                        }
                    },
                    text { "body" }
                }
                // Overlay with `on_open_change: None` must not permanently wedge on Escape.
                Popover {
                    open: true,
                    keep_mounted: true,
                    trigger_label: "NonePop",
                    test_id: "none-pop",
                    text { "unhandled" }
                }
                pressable {
                    test_id: "unrelated-rerender",
                    onpress: move |_| bump += 1,
                    text { "bump" }
                }
                text {
                    test_id: "veto-state",
                    "{bump}|{outer_escapes}|{dd_false}|{dd_true}|{pop_false}|{pop_true}"
                }
            }
        }
    }
}

/// A controlled parent that vetoes `on_open_change(false)` (or omits `on_open_change`) keeps
/// `latch.is_open() == true` after the self-healing render: subsequent `Escape` still stops
/// propagation, trigger click requests `false` (never `true`), and `RovingFocus` is never reset.
#[test]
fn close_latch_self_heals_when_parent_vetoes_or_omits_on_open_change() {
    let mut h = UiHarness::new(VetoAndNoHandlerApp);
    assert_eq!(h.raw_str(h.find("veto-dd-item-copy"), "tabIndex"), Some("0"));

    // Advance roving focus to the second item ("paste"), then Escape (vetoed by parent).
    h.keydown("veto-dd-item-copy", "ArrowDown", false);
    assert_eq!(h.raw_str(h.find("veto-dd-item-paste"), "tabIndex"), Some("0"));

    h.keydown("veto-dd-menu", "Escape", false);
    assert_eq!(h.text_of("veto-state"), "0|0|1|0|0|0");
    assert_eq!(
        h.raw_str(h.find("veto-dd-item-paste"), "tabIndex"),
        Some("0"),
        "vetoed close must not fire `opened` or reset DropdownMenu RovingFocus"
    );

    // An unrelated parent re-render must also preserve RovingFocus.
    h.press("unrelated-rerender");
    assert_eq!(h.raw_str(h.find("veto-dd-item-paste"), "tabIndex"), Some("0"));

    // Second Escape must still be handled and stopped (never leaks to outer onkeydown).
    h.keydown("veto-dd-menu", "Escape", false);
    assert_eq!(h.text_of("veto-state"), "1|0|2|0|0|0");

    // Clicking the trigger while still open must request `false`, never `true`.
    h.press("veto-dd-trigger");
    assert_eq!(h.text_of("veto-state"), "1|0|3|0|0|0");

    // Same veto self-healing on Popover.
    h.keydown("veto-pop-content", "Escape", false);
    h.keydown("veto-pop-content", "Escape", false);
    h.press("veto-pop-trigger");
    assert_eq!(h.text_of("veto-state"), "1|0|3|0|3|0");

    // `on_open_change: None` must not wedge the latch across consecutive Escapes.
    h.keydown("none-pop-content", "Escape", false);
    h.keydown("none-pop-content", "Escape", false);
    assert_eq!(
        h.text_of("veto-state"),
        "1|0|3|0|3|0",
        "on_open_change: None must keep consuming Escape with stop_propagation()"
    );
}

#[component]
fn DropdownKeyAuditApp() -> Element {
    let mut open = use_signal(|| true);
    let mut selected = use_signal(Vec::<String>::new);
    let mut opens = use_signal(|| 0u32);
    let mut closes = use_signal(|| 0u32);
    let selected_csv = selected.read().join(",");
    rsx! {
        ThemeProvider {
            theme: Theme::LIGHT,
            Col {
                DropdownMenu {
                    open: open(),
                    keep_mounted: true,
                    trigger_label: "Actions",
                    test_id: "dd",
                    items: vec![
                        MenuItem::new("copy", "Copy"),
                        MenuItem::new("paste", "Paste"),
                        MenuItem::new("delete", "Delete"),
                    ],
                    on_open_change: move |next: bool| {
                        if next {
                            opens += 1;
                        } else {
                            closes += 1;
                        }
                        open.set(next);
                    },
                    on_select: move |id: String| selected.write().push(id),
                }
                text {
                    test_id: "dd-log",
                    "{open}|{opens}|{closes}|{selected_csv}"
                }
            }
        }
    }
}

/// Focused menuitem keys must stop propagation (moving by 1 item, selecting once) and trigger
/// `onkeydown` must read `latch.is_open()` so same-batch native `close` + trigger key reopens
/// rather than selecting a hidden item.
#[test]
fn dropdown_item_keys_do_not_double_dispatch_and_trigger_keydown_uses_latch() {
    let mut h = UiHarness::new(DropdownKeyAuditApp);
    assert_eq!(h.raw_str(h.find("dd-item-copy"), "tabIndex"), Some("0"));

    // ArrowDown on a focused menuitem advances by 1 (copy -> paste, NOT copy -> delete).
    h.keydown("dd-item-copy", "ArrowDown", false);
    assert_eq!(h.raw_str(h.find("dd-item-paste"), "tabIndex"), Some("0"));
    assert_eq!(h.raw_str(h.find("dd-item-delete"), "tabIndex"), Some("-1"));

    // Enter on a focused menuitem fires `on_select` exactly ONCE.
    h.keydown("dd-item-paste", "Enter", false);
    assert_eq!(h.text_of("dd-log"), "false|0|1|paste");

    // Reopen and verify Space on a focused menuitem also fires `on_select` exactly ONCE.
    h.keydown("dd-trigger", "ArrowDown", false);
    assert_eq!(h.text_of("dd-log"), "true|1|1|paste");
    h.keydown("dd-item-copy", " ", false);
    assert_eq!(h.text_of("dd-log"), "false|1|2|paste,copy");

    // Reopen, then deliver native `close` + trigger `Enter` in ONE batch: trigger must see
    // `!latch.is_open()` and reopen instead of firing `on_select` on a hidden item.
    h.press("dd-trigger");
    assert_eq!(h.text_of("dd-log"), "true|2|2|paste,copy");
    let mut batch = EventBatchBuf::new();
    h.push_custom(&mut batch, "dd-menu", "close", false);
    h.push_keydown(&mut batch, "dd-trigger", "Enter", false);
    h.dispatch(&batch);
    assert_eq!(
        h.text_of("dd-log"),
        "true|3|3|paste,copy",
        "same-batch native close + trigger Enter must reopen without selecting hidden item"
    );
}

#[component]
fn NestedAndSameBatchDialogApp() -> Element {
    let mut outer_open = use_signal(|| true);
    let mut inner_open = use_signal(|| true);
    let mut backdrop_open = use_signal(|| true);
    let mut outer_changes = use_signal(|| 0u32);
    let mut outer_closes = use_signal(|| 0u32);
    let mut inner_changes = use_signal(|| 0u32);
    let mut inner_closes = use_signal(|| 0u32);
    let mut backdrop_changes = use_signal(|| 0u32);
    let mut backdrop_closes = use_signal(|| 0u32);
    rsx! {
        ThemeProvider {
            theme: Theme::LIGHT,
            Col {
                Dialog {
                    open: outer_open(),
                    keep_mounted: true,
                    test_id: "outer-dlg",
                    close_label: "Close Outer",
                    on_open_change: move |next: bool| {
                        outer_changes += 1;
                        outer_open.set(next);
                    },
                    on_close: move |()| outer_closes += 1,
                    Dialog {
                        open: inner_open(),
                        keep_mounted: true,
                        test_id: "inner-dlg",
                        close_label: "Close Inner",
                        on_open_change: move |next: bool| {
                            inner_changes += 1;
                            inner_open.set(next);
                        },
                        on_close: move |()| inner_closes += 1,
                        text { "nested body" }
                    }
                }
                Dialog {
                    open: backdrop_open(),
                    test_id: "scrim-dlg",
                    on_open_change: move |next: bool| {
                        backdrop_changes += 1;
                        backdrop_open.set(next);
                    },
                    on_close: move |()| backdrop_closes += 1,
                    text { "scrim body" }
                }
                pressable {
                    test_id: "reopen-all",
                    onpress: move |_| {
                        outer_open.set(true);
                        inner_open.set(true);
                        backdrop_open.set(true);
                    },
                    text { "reopen" }
                }
                text {
                    test_id: "dlg-state",
                    "{outer_open}:{outer_changes}/{outer_closes}|{inner_open}:{inner_changes}/{inner_closes}|{backdrop_open}:{backdrop_changes}/{backdrop_closes}"
                }
            }
        }
    }
}

/// `Dialog` uses `CloseLatch`: same-batch native `close` + close button (or backdrop) press fires
/// `on_open_change(false)` and `on_close` once, and `Escape` in a nested `Dialog` stops
/// propagation so only the inner dialog dismisses.
#[test]
fn dialog_close_latch_dedupes_same_batch_and_nested_escape_dismisses_inner_only() {
    let mut h = UiHarness::new(NestedAndSameBatchDialogApp);
    assert_eq!(h.text_of("dlg-state"), "true:0/0|true:0/0|true:0/0");

    // Escape inside nested Dialog dismisses ONLY the inner dialog (outer stays open).
    h.keydown("inner-dlg", "Escape", false);
    assert_eq!(h.text_of("dlg-state"), "true:0/0|false:1/1|true:0/0");

    // With inner closed, Escape on inner-dlg bubbles to outer-dlg and dismisses outer.
    h.keydown("inner-dlg", "Escape", false);
    assert_eq!(h.text_of("dlg-state"), "false:1/1|false:1/1|true:0/0");

    // Reopen and deliver native `close` + close-button click in ONE batch: fires once.
    h.press("reopen-all");
    let mut batch = EventBatchBuf::new();
    h.push_custom(&mut batch, "inner-dlg", "close", false);
    h.push_press(&mut batch, "inner-dlg-close");
    h.dispatch(&batch);
    assert_eq!(h.text_of("dlg-state"), "true:1/1|false:2/2|true:0/0");

    // Same-batch backdrop press + native `close` on scrim Dialog: fires once.
    batch.clear();
    h.push_press(&mut batch, "scrim-dlg-backdrop");
    h.push_custom(&mut batch, "scrim-dlg", "close", false);
    h.dispatch(&batch);
    assert_eq!(h.text_of("dlg-state"), "true:1/1|false:2/2|false:1/1");
}
