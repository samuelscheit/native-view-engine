//! Keyboard and press parity tests for `DropdownMenu` and `Popover`: roving keys stop
//! propagation, menuitem `onpress` is guarded on `latch.is_open()` (same-batch native `close` +
//! press and closed `keep_mounted` menus), and `Popover` trigger `Enter`/`Space` stop propagation.

mod common;

use common::UiHarness;
use nve_abi::EventBatchBuf;
use nve_dioxus::prelude::*;
use nve_ui::prelude::*;

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

/// Menuitem `onpress` is guarded on `latch.is_open()`: same-batch `[native close, item press]`
/// and presses on a `keep_mounted` closed menu never invoke `on_select`.
#[test]
fn dropdown_item_press_ignored_when_closed_or_same_batch_native_close() {
    let mut h = UiHarness::new(DropdownKeyAuditApp);
    let mut batch = EventBatchBuf::new();
    h.push_custom(&mut batch, "dd-menu", "close", false);
    h.push_press(&mut batch, "dd-item-copy");
    h.dispatch(&batch);
    assert_eq!(h.text_of("dd-log"), "false|0|1|");

    // Press on a closed `keep_mounted` item must also be ignored.
    h.press("dd-item-paste");
    assert_eq!(h.text_of("dd-log"), "false|0|1|");

    // Control: reopening and pressing an item selects and closes.
    h.press("dd-trigger");
    h.press("dd-item-paste");
    assert_eq!(h.text_of("dd-log"), "false|1|2|paste");
}

#[component]
fn PopoverTriggerStopPropApp() -> Element {
    let mut open = use_signal(|| false);
    let mut parent_keys = use_signal(Vec::<String>::new);
    let keys_csv = parent_keys.read().join(",");
    rsx! {
        ThemeProvider {
            theme: Theme::LIGHT,
            view {
                onkeydown: move |ev: KeyEvent| parent_keys.write().push(ev.data().key.clone()),
                Popover {
                    open: open(),
                    keep_mounted: true,
                    trigger_label: "Info",
                    test_id: "sp-pop",
                    on_open_change: move |next: bool| open.set(next),
                    text { "body" }
                }
                text { test_id: "sp-log", "{open}|{keys_csv}" }
            }
        }
    }
}

/// `Popover` trigger `Enter` / `" "` / `"Space"` stop propagation (matching `DropdownMenu`).
#[test]
fn popover_trigger_enter_and_space_stop_propagation() {
    let mut h = UiHarness::new(PopoverTriggerStopPropApp);
    for (key, expected_open) in [("Enter", "true"), (" ", "false"), ("Space", "true")] {
        h.keydown("sp-pop-trigger", key, false);
        assert_eq!(h.text_of("sp-log"), format!("{expected_open}|"));
    }
    // Control: unhandled keys still bubble to the enclosing view.
    h.keydown("sp-pop-trigger", "a", false);
    assert_eq!(h.text_of("sp-log"), "true|a");
}
