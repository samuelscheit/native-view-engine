//! Browser-initiated overlay closes: a bridged native `close` (`<dialog>` `close`, `[popover]`
//! `toggle` to `closed`, delivered as `Custom { name: "close" }`) drives `Dialog` / `Popover`
//! `on_open_change(false)` + `on_close` exactly once, and is a no-op while already closed.

mod common;

use common::UiHarness;
use nve_dioxus::prelude::*;
use nve_ui::prelude::*;

#[component]
fn OverlayCloseApp() -> Element {
    let mut dialog_open = use_signal(|| true);
    let mut popover_open = use_signal(|| true);
    let mut closes = use_signal(|| 0u32);
    let mut changes = use_signal(|| 0u32);
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
                    on_open_change: move |open: bool| popover_open.set(open),
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
            }
        }
    }
}

#[test]
fn native_close_drives_dialog_and_popover_state_exactly_once() {
    let mut h = UiHarness::new(OverlayCloseApp);
    assert_eq!(h.text_of("state"), "true|true|0|0");

    h.custom("dlg", "close");
    assert_eq!(
        h.text_of("state"),
        "false|true|1|1",
        "native dialog close reports once"
    );
    // A late duplicate `close` (app-driven `close()` echo) while closed is a no-op.
    h.custom("dlg", "close");
    assert_eq!(h.text_of("state"), "false|true|1|1");

    h.custom("pop-content", "close");
    assert_eq!(
        h.text_of("state"),
        "false|false|1|1",
        "popover light dismiss closes"
    );
    h.custom("pop-content", "close");
    assert_eq!(h.text_of("state"), "false|false|1|1");

    // Reopened overlays close again through the same bridge.
    h.press("reopen");
    assert_eq!(h.text_of("state"), "true|true|1|1");
    h.custom("dlg", "close");
    h.custom("pop-content", "close");
    assert_eq!(h.text_of("state"), "false|false|2|2");
}

#[test]
fn unrelated_custom_events_do_not_close_overlays() {
    let mut h = UiHarness::new(OverlayCloseApp);
    h.custom("dlg", "toggle");
    h.custom("pop-content", "cancel");
    assert_eq!(h.text_of("state"), "true|true|0|0");
}

#[component]
fn NestedOverlayApp() -> Element {
    let mut dialog_open = use_signal(|| true);
    let mut popover_open = use_signal(|| true);
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
                }
                text { test_id: "state", "{dialog_open}|{popover_open}" }
            }
        }
    }
}

/// `close` does not bubble natively: a nested popover's light dismiss must not close the
/// enclosing dialog.
#[test]
fn nested_overlay_close_never_bubbles_into_the_enclosing_dialog() {
    let mut h = UiHarness::new(NestedOverlayApp);
    assert_eq!(h.text_of("state"), "true|true");
    h.custom("inner-content", "close");
    assert_eq!(h.text_of("state"), "true|false");
    h.custom("outer", "close");
    assert_eq!(h.text_of("state"), "false|false");
}
