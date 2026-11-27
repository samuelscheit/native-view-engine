//! Pure DOM decision rules behind the `wasm32` controller: focusability, focus-trap activity,
//! Select activation targets, and multi-pad slot resets.

use nve_tv_web::rules::{
    ActivationFacts, FocusableFacts, MAX_GAMEPADS, TrapFacts, absent_pad_slots, is_editable,
    is_hidden_visibility, parse_tabindex,
};

fn tag(tag: &str) -> FocusableFacts<'_> {
    FocusableFacts {
        tag,
        ..FocusableFacts::default()
    }
}

#[test]
fn natively_focusable_tags_and_tabindex() {
    for t in ["BUTTON", "SELECT", "TEXTAREA", "SUMMARY", "INPUT"] {
        assert!(tag(t).is_focusable(), "{t}");
    }
    assert!(!tag("A").is_focusable(), "<a> without href");
    assert!(
        FocusableFacts {
            has_href: true,
            ..tag("A")
        }
        .is_focusable()
    );
    assert!(
        !FocusableFacts {
            input_type: "HIDDEN",
            ..tag("INPUT")
        }
        .is_focusable()
    );
    assert!(!tag("DIV").is_focusable(), "plain div needs a tabindex");
    for (tabindex, focusable) in [
        ("0", true),
        (" 3 ", true),
        ("-1", false),
        ("-2", false),
        (" -1 ", false),
        ("garbage", false),
    ] {
        let div = FocusableFacts {
            tabindex: Some(tabindex),
            ..tag("DIV")
        };
        assert_eq!(div.is_focusable(), focusable, "div tabindex={tabindex:?}");
    }
    let button = FocusableFacts {
        tabindex: Some("-1"),
        ..tag("BUTTON")
    };
    assert!(
        !button.is_focusable(),
        "negative tabindex removes native focusability"
    );
    let garbage = FocusableFacts {
        tabindex: Some("x"),
        ..tag("BUTTON")
    };
    assert!(garbage.is_focusable(), "unparsable tabindex is ignored");
    assert_eq!(parse_tabindex(Some(" -7")), Some(-7));
    assert_eq!(parse_tabindex(None), None);
}

#[test]
fn contenteditable_disabled_aria_disabled_and_inert_are_excluded() {
    for (value, focusable) in [
        ("", true),
        ("true", true),
        ("plaintext-only", true),
        ("false", false),
        (" FALSE ", false),
    ] {
        let div = FocusableFacts {
            contenteditable: Some(value),
            ..tag("DIV")
        };
        assert_eq!(div.is_focusable(), focusable, "contenteditable={value:?}");
        assert_eq!(is_editable(Some(value)), focusable);
    }
    let editable_off_but_tabbable = FocusableFacts {
        contenteditable: Some("false"),
        tabindex: Some("0"),
        ..tag("DIV")
    };
    assert!(
        editable_off_but_tabbable.is_focusable(),
        "tabindex still qualifies it"
    );
    for facts in [
        FocusableFacts {
            disabled: true,
            ..tag("BUTTON")
        },
        FocusableFacts {
            aria_disabled: true,
            ..tag("BUTTON")
        },
        FocusableFacts {
            inert_or_hidden: true,
            ..tag("BUTTON")
        },
        FocusableFacts {
            disabled: true,
            tabindex: Some("0"),
            ..tag("INPUT")
        },
    ] {
        assert!(!facts.is_focusable(), "{facts:?}");
    }
    assert!(is_hidden_visibility("hidden") && is_hidden_visibility("collapse"));
    assert!(!is_hidden_visibility("visible"));
}

fn rendered() -> TrapFacts<'static> {
    TrapFacts {
        rendered: true,
        ..TrapFacts::default()
    }
}

#[test]
fn traps_must_be_rendered_non_inert_and_modal_or_explicit() {
    let explicit = TrapFacts {
        focus_trap: Some(""),
        ..rendered()
    };
    assert!(explicit.is_active());
    assert!(
        !TrapFacts {
            focus_trap: Some("false"),
            ..rendered()
        }
        .is_active()
    );
    assert!(
        !TrapFacts {
            rendered: false,
            ..explicit
        }
        .is_active(),
        "display:none trap"
    );
    assert!(
        !TrapFacts {
            inert_or_hidden: true,
            ..explicit
        }
        .is_active(),
        "[hidden] / [inert]"
    );

    let dialog = TrapFacts {
        open_dialog: true,
        ..rendered()
    };
    assert!(
        TrapFacts {
            modal: Some(true),
            ..dialog
        }
        .is_active()
    );
    assert!(
        !TrapFacts {
            modal: Some(false),
            ..dialog
        }
        .is_active(),
        "non-modal show()"
    );
    // Legacy engines without `:modal`: `aria-modal` (written by NVE for modal dialogs).
    assert!(
        TrapFacts {
            modal: None,
            aria_modal: Some("true"),
            ..dialog
        }
        .is_active()
    );
    assert!(
        !TrapFacts {
            modal: None,
            aria_modal: Some("false"),
            ..dialog
        }
        .is_active()
    );
    assert!(
        !TrapFacts {
            modal: None,
            ..dialog
        }
        .is_active()
    );
    // Modern engine reporting `:modal` false still honors an explicit aria-modal dialog.
    assert!(
        TrapFacts {
            modal: Some(false),
            aria_modal: Some("true"),
            ..dialog
        }
        .is_active()
    );
    assert!(
        !TrapFacts {
            aria_modal: Some("true"),
            ..rendered()
        }
        .is_active(),
        "closed dialog"
    );
    let opted_out_modal = TrapFacts {
        modal: Some(true),
        focus_trap: Some("false"),
        ..dialog
    };
    assert!(
        opted_out_modal.is_active(),
        "a modal dialog makes the rest inert anyway"
    );
}

#[test]
fn select_never_activates_dialogs_disabled_inert_or_programmatic_targets() {
    let button = ActivationFacts {
        tag: "BUTTON",
        ..ActivationFacts::default()
    };
    assert!(button.is_activatable());
    assert!(
        ActivationFacts {
            tabindex: Some("0"),
            ..button
        }
        .is_activatable()
    );
    for facts in [
        ActivationFacts {
            tag: "DIALOG",
            ..button
        },
        ActivationFacts {
            tag: "dialog",
            ..button
        },
        ActivationFacts {
            tabindex: Some("-1"),
            ..button
        },
        ActivationFacts {
            disabled: true,
            ..button
        },
        ActivationFacts {
            inert: true,
            ..button
        },
    ] {
        assert!(!facts.is_activatable(), "{facts:?}");
    }
}

#[test]
fn absent_gamepads_release_their_slots() {
    let stale: Vec<usize> = absent_pad_slots(0b101, 4).collect();
    assert_eq!(stale, [1, 3]);
    assert_eq!(absent_pad_slots(0, 0).count(), 0);
    assert_eq!(
        absent_pad_slots(u64::MAX, MAX_GAMEPADS + 8).count(),
        0,
        "capped at 64 pads"
    );
}
