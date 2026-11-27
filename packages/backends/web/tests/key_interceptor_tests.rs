//! Public key-interception API: interceptor ids / removal, the target-uniform
//! `WebKeyEventInfo::new`, and the per-role arrow ownership table that spatial navigation defers to.

use std::cell::RefCell;
use std::rc::Rc;

use nve_backend_web::{ArrowAxes, WebBackend, WebKeyDisposition, WebKeyEventInfo};

#[test]
fn removed_interceptors_stop_running_and_ids_are_single_use() {
    let mut backend = WebBackend::new();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let ids: Vec<_> = ["first", "second"]
        .into_iter()
        .map(|name| {
            let seen = Rc::clone(&seen);
            backend.add_key_interceptor(move |info| {
                seen.borrow_mut().push((name, info.key.to_owned()));
                WebKeyDisposition::PassThrough
            })
        })
        .collect();
    assert_ne!(ids[0], ids[1]);
    let info = WebKeyEventInfo::new("ArrowDown", "ArrowDown", 40);
    assert_eq!(backend.intercept_key(&info), WebKeyDisposition::PassThrough);
    assert!(backend.remove_key_interceptor(ids[0]));
    assert!(!backend.remove_key_interceptor(ids[0]));
    assert_eq!(backend.intercept_key(&info), WebKeyDisposition::PassThrough);
    assert_eq!(
        *seen.borrow(),
        [
            ("first", "ArrowDown".to_owned()),
            ("second", "ArrowDown".to_owned()),
            ("second", "ArrowDown".to_owned()),
        ]
    );
}

#[test]
fn new_event_info_is_a_plain_untargeted_press() {
    let info = WebKeyEventInfo {
        repeat: true,
        ..WebKeyEventInfo::new("Enter", "NumpadEnter", 13)
    };
    assert_eq!(
        (info.key, info.code, info.key_code),
        ("Enter", "NumpadEnter", 13)
    );
    assert!(info.repeat && !info.is_composing && !info.in_text_input);
    assert!(!info.has_command_modifier());
    assert_eq!(
        (info.target_node, info.owned_arrows),
        (None, ArrowAxes::NONE)
    );
}

#[test]
fn arrow_ownership_follows_the_wai_aria_keyboard_model_per_role() {
    use ArrowAxes as A;
    let role = |r: &str| ArrowAxes::of_widget("DIV", Some(r), None, None);
    for (r, axes) in [
        ("slider", A::HORIZONTAL),
        ("tablist", A::HORIZONTAL),
        ("menubar", A::HORIZONTAL),
        ("listbox", A::VERTICAL),
        ("menu", A::VERTICAL),
        ("combobox", A::VERTICAL),
        ("spinbutton", A::VERTICAL),
        ("grid", A::BOTH),
        ("treegrid", A::BOTH),
        ("tree", A::BOTH),
        ("button", A::NONE),
        ("tab", A::NONE),
        ("", A::NONE),
    ] {
        assert_eq!(role(r), axes, "{r}");
    }
    let oriented = |r: &str, o: &str| ArrowAxes::of_widget("DIV", Some(r), Some(o), None);
    assert_eq!(
        oriented("tablist", "vertical"),
        A::VERTICAL,
        "vertical tab rail"
    );
    assert_eq!(oriented("slider", "vertical"), A::VERTICAL);
    assert_eq!(oriented("listbox", "horizontal"), A::HORIZONTAL);
    assert_eq!(oriented("menu", "horizontal"), A::HORIZONTAL);
    assert_eq!(
        oriented("grid", "horizontal"),
        A::BOTH,
        "grids ignore orientation"
    );
    assert_eq!(oriented("combobox", "horizontal"), A::VERTICAL);
    // Native <select>: Up / Down change the value, Left / Right still leave it.
    let select = ArrowAxes::of_widget("SELECT", None, None, None);
    assert!(select.owns_vertical() && !select.owns_horizontal());
    assert_eq!(
        ArrowAxes::of_widget("select", None, None, None),
        A::VERTICAL
    );
    // Declarative opt-out / opt-in wins over role and tag.
    assert_eq!(
        ArrowAxes::of_widget("DIV", None, None, Some("off")),
        A::BOTH
    );
    assert_eq!(
        ArrowAxes::of_widget("DIV", None, None, Some("OFF")),
        A::BOTH
    );
    assert_eq!(
        ArrowAxes::of_widget("DIV", Some("grid"), None, Some("on")),
        A::NONE
    );
    assert_eq!(
        ArrowAxes::of_widget("SELECT", None, None, Some("")),
        A::NONE
    );
    assert!(A::BOTH.owns_horizontal() && A::BOTH.owns_vertical());
    assert!(!A::NONE.owns_horizontal() && !A::NONE.owns_vertical());
}

#[test]
fn arrow_owner_selector_lists_every_classified_widget() {
    let selector = nve_backend_web::ARROW_OWNER_SELECTOR;
    for part in [
        "[data-nve-spatial-nav]",
        "select",
        "[role=slider]",
        "[role=tablist]",
        "[role=menubar]",
        "[role=listbox]",
        "[role=menu]",
        "[role=combobox]",
        "[role=spinbutton]",
        "[role=grid]",
        "[role=treegrid]",
        "[role=tree]",
    ] {
        assert!(selector.split(',').any(|s| s == part), "{part}");
    }
    assert!(!selector.contains(":is("), "Chromium 56-79 TVs lack :is()");
}
