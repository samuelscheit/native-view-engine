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
