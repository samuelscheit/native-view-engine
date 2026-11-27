//! Section 2: Inputs & Forms — `Input` (`AutofillHint` & `InputMode`), `Checkbox`, `Switch`,
//! live `FormState` validation (sync + cross-field password match + dirty/touched + submit/reset),
//! and `UndoRedoStack` live editor.

use nve_dioxus::prelude::*;
use nve_ui::{ColorHex, 
    use_theme, AutofillHint, Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Card,
    CardVariant, Checkbox, FormState, Input, InputMode, Row, Separator, Switch, UndoRedoStack,
};

use crate::i18n_copy::copy;
use crate::sections::{responsive_two_col, responsive_two_col_cell, SectionHeader};
use crate::shell::AppHandle;

fn build_demo_form() -> FormState {
    let mut form = FormState::new();
    form.register_field("email", String::new())
        .with_validator(|v: &String| {
            let trimmed = v.trim();
            if trimmed.is_empty() {
                Err("Email address is required".to_owned())
            } else if !trimmed.contains('@') || !trimmed.contains('.') {
                Err("Enter a valid email (e.g. dev@nve.rs)".to_owned())
            } else {
                Ok(())
            }
        });
    form.register_field("password", String::new())
        .with_validator(|v: &String| {
            if v.len() < 8 {
                Err("Password must be at least 8 characters".to_owned())
            } else {
                Ok(())
            }
        });
    form.register_field("confirm", String::new())
        .with_validator(|v: &String| {
            if v.is_empty() {
                Err("Please confirm your password".to_owned())
            } else {
                Ok(())
            }
        });
    form.register_field("otp", String::new())
        .with_validator(|v: &String| {
            if !v.is_empty() && (v.len() != 6 || !v.chars().all(|c| c.is_ascii_digit())) {
                Err("OTP code must be 6 digits".to_owned())
            } else {
                Ok(())
            }
        });
    form.add_cross_validator("confirm", |f| {
        let pw = f.value::<String>("password").map(String::as_str).unwrap_or("");
        let cf = f.value::<String>("confirm").map(String::as_str).unwrap_or("");
        if !cf.is_empty() && pw != cf {
            Err("Passwords do not match".to_owned())
        } else {
            Ok(())
        }
    });
    form
}

#[component]
pub fn FormsSection(handle: AppHandle) -> Element {
    let theme = use_theme();
    let snap = handle.read_state();
    let pseudo = snap.pseudo_locale;

    let mut form = use_signal(build_demo_form);
    let mut submit_status = use_signal(|| Option::<String>::None);
    let mut history = use_signal(|| UndoRedoStack::new(snap.notes.clone(), 32));

    let text_primary = theme.text.primary.hex();
    let text_muted = theme.text.tertiary.hex();

    let (email_val, pw_val, confirm_val, otp_val, email_err, pw_err, confirm_err, otp_err, dirty_count, is_dirty, is_valid) = {
        let f = form.read();
        (
            f.value::<String>("email").cloned().unwrap_or_default(),
            f.value::<String>("password").cloned().unwrap_or_default(),
            f.value::<String>("confirm").cloned().unwrap_or_default(),
            f.value::<String>("otp").cloned().unwrap_or_default(),
            f.visible_error("email").map(str::to_owned),
            f.visible_error("password").map(str::to_owned),
            f.visible_error("confirm").map(str::to_owned),
            f.visible_error("otp").map(str::to_owned),
            f.dirty_fields().len(),
            f.is_dirty(),
            f.is_valid(),
        )
    };

    let (hist_current, undo_count, redo_count, can_undo, can_redo) = {
        let h = history.read();
        (
            h.current().clone(),
            h.undo_count(),
            h.redo_count(),
            h.can_undo(),
            h.can_redo(),
        )
    };

    rsx! {
        view {
            test_id: "section-forms",
            style: "width: 100%; flex-direction: column; gap: 16px;",

            SectionHeader {
                title: copy("Inputs & Forms", pseudo),
                subtitle: copy("Typed AutofillHint + InputMode text fields, Checkbox/Switch toggles, live FormState sync + cross-field validation, and UndoRedoStack history.", pseudo),
                tag: "5 Controls".to_owned(),
            }

            view {
                nve_style: responsive_two_col(),

                // Live FormState Validation Card
                view {
                    nve_style: responsive_two_col_cell(),
                    Card {
                        test_id: Some("ks-form-card".to_owned()),
                        variant: CardVariant::Outlined,
                        padding: Some(16.0),
                        gap: Some(12.0),

                        view {
                            style: "flex-direction: row; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 8px;",
                            text {
                                style: "font-size: 15px; font-weight: 700; color: {text_primary};",
                                "{copy(\"FormState Validation\", pseudo)}"
                            }
                            view {
                                style: "flex-direction: row; gap: 6px; flex-wrap: wrap;",
                                Badge {
                                    label: if is_dirty { format!("Dirty ({dirty_count})") } else { "Clean".to_owned() },
                                    variant: if is_dirty { BadgeVariant::Warning } else { BadgeVariant::Neutral },
                                }
                                Badge {
                                    label: if is_valid { "Valid".to_owned() } else { "Needs Fix".to_owned() },
                                    variant: if is_valid { BadgeVariant::Success } else { BadgeVariant::Danger },
                                }
                            }
                        }

                        Input {
                            test_id: Some("ks-input-email".to_owned()),
                            label: Some(copy("Email (AutofillHint::Email / InputMode::Email)", pseudo)),
                            placeholder: Some("engineer@nve.rs".to_owned()),
                            value: email_val,
                            error: email_err,
                            autocomplete: Some(AutofillHint::Email),
                            input_mode: Some(InputMode::Email),
                            on_change: move |v: String| {
                                let mut f = form.write();
                                f.set_value("email", v);
                                f.touch("email");
                                f.validate_field("email");
                            },
                        }

                        Input {
                            test_id: Some("ks-input-password".to_owned()),
                            label: Some(copy("New Password (AutofillHint::NewPassword)", pseudo)),
                            placeholder: Some("At least 8 characters".to_owned()),
                            value: pw_val,
                            error: pw_err,
                            secure: true,
                            autocomplete: Some(AutofillHint::NewPassword),
                            on_change: move |v: String| {
                                let mut f = form.write();
                                f.set_value("password", v);
                                f.touch("password");
                                f.validate_all();
                            },
                        }

                        Input {
                            test_id: Some("ks-input-confirm".to_owned()),
                            label: Some(copy("Confirm Password (Cross-Field Validator)", pseudo)),
                            placeholder: Some("Repeat password".to_owned()),
                            value: confirm_val,
                            error: confirm_err,
                            secure: true,
                            autocomplete: Some(AutofillHint::NewPassword),
                            on_change: move |v: String| {
                                let mut f = form.write();
                                f.set_value("confirm", v);
                                f.touch("confirm");
                                f.validate_all();
                            },
                        }

                        Input {
                            test_id: Some("ks-input-otp".to_owned()),
                            label: Some(copy("One-Time Code (AutofillHint::OneTimeCode / Numeric)", pseudo)),
                            placeholder: Some("6-digit SMS code (optional)".to_owned()),
                            value: otp_val,
                            error: otp_err,
                            autocomplete: Some(AutofillHint::OneTimeCode),
                            input_mode: Some(InputMode::Numeric),
                            on_change: move |v: String| {
                                let mut f = form.write();
                                f.set_value("otp", v);
                                f.touch("otp");
                                f.validate_field("otp");
                            },
                        }

                        view {
                            style: "flex-direction: row; flex-wrap: wrap; gap: 8px; align-items: center;",
                            Button {
                                test_id: Some("ks-form-submit".to_owned()),
                                variant: ButtonVariant::Primary,
                                size: ButtonSize::Sm,
                                on_press: move |_| {
                                    let ok = form.write().submit();
                                    submit_status.set(Some(if ok {
                                        "Form validated and submitted cleanly!".to_owned()
                                    } else {
                                        "Validation failed — review highlighted fields.".to_owned()
                                    }));
                                },
                                text { "{copy(\"Submit Form\", pseudo)}" }
                            }
                            Button {
                                test_id: Some("ks-form-fill-valid".to_owned()),
                                variant: ButtonVariant::Secondary,
                                size: ButtonSize::Sm,
                                on_press: move |_| {
                                    let mut f = form.write();
                                    f.set_value("email", "architect@nve.rs".to_owned());
                                    f.set_value("password", "native-view-2026".to_owned());
                                    f.set_value("confirm", "native-view-2026".to_owned());
                                    f.set_value("otp", "482910".to_owned());
                                    f.validate_all();
                                    submit_status.set(Some("Populated valid sample credentials.".to_owned()));
                                },
                                text { "{copy(\"Fill Sample\", pseudo)}" }
                            }
                            Button {
                                test_id: Some("ks-form-reset".to_owned()),
                                variant: ButtonVariant::Outline,
                                size: ButtonSize::Sm,
                                on_press: move |_| {
                                    form.write().reset_to_clean();
                                    submit_status.set(None);
                                },
                                text { "{copy(\"Reset\", pseudo)}" }
                            }
                        }

                        if let Some(msg) = submit_status() {
                            text {
                                test_id: "ks-form-status",
                                style: "font-size: 12px; color: {text_muted};",
                                "{msg}"
                            }
                        }
                    }
                }

                // Right Column: Checkbox + Switch + UndoRedoStack
                view {
                    nve_style: responsive_two_col_cell(),
                    style: "flex-direction: column; gap: 14px;",

                    Card {
                        test_id: Some("ks-toggles-card".to_owned()),
                        variant: CardVariant::Outlined,
                        padding: Some(16.0),
                        gap: Some(10.0),
                        text {
                            style: "font-size: 15px; font-weight: 700; color: {text_primary};",
                            "{copy(\"Checkbox & Switch\", pseudo)}"
                        }
                        Checkbox {
                            test_id: Some("ks-checkbox-terms".to_owned()),
                            checked: snap.checkbox_terms,
                            label: Some(copy("Accept binary ABI contract", pseudo)),
                            on_change: move |v: bool| {
                                handle.update(|st| st.checkbox_terms = v);
                            },
                        }
                        Checkbox {
                            test_id: Some("ks-checkbox-indeterminate".to_owned()),
                            checked: false,
                            indeterminate: snap.checkbox_indeterminate,
                            label: Some(copy("Indeterminate partial selection", pseudo)),
                            on_change: move |_| {
                                handle.update(|st| st.checkbox_indeterminate = !st.checkbox_indeterminate);
                            },
                        }
                        Checkbox {
                            checked: true,
                            disabled: true,
                            label: Some(copy("Disabled checked option", pseudo)),
                        }
                        Separator {}
                        Switch {
                            test_id: Some("ks-switch-notifications".to_owned()),
                            checked: snap.switch_notifications,
                            label: Some(copy("Live hot-reload notifications", pseudo)),
                            on_change: move |v: bool| {
                                handle.update(|st| st.switch_notifications = v);
                            },
                        }
                        Switch {
                            test_id: Some("ks-switch-telemetry".to_owned()),
                            checked: snap.switch_telemetry,
                            label: Some(copy("Frame timing telemetry", pseudo)),
                            on_change: move |v: bool| {
                                handle.update(|st| st.switch_telemetry = v);
                            },
                        }
                    }

                    Card {
                        test_id: Some("ks-history-card".to_owned()),
                        variant: CardVariant::Elevated,
                        padding: Some(16.0),
                        gap: Some(10.0),
                        Row {
                            justify: Some("space-between".to_owned()),
                            align: Some("center".to_owned()),
                            text {
                                style: "font-size: 15px; font-weight: 700; color: {text_primary};",
                                "{copy(\"UndoRedoStack Editor\", pseudo)}"
                            }
                            Badge {
                                label: format!("Undo {undo_count} · Redo {redo_count}"),
                                variant: BadgeVariant::Accent,
                            }
                        }

                        Input {
                            test_id: Some("ks-notes-input".to_owned()),
                            label: Some(copy("Persisted Notes (pushed to UndoRedoStack)", pseudo)),
                            value: hist_current.clone(),
                            multiline: true,
                            on_change: move |v: String| {
                                history.write().push(v.clone());
                                handle.update(|st| st.notes = v);
                            },
                        }

                        view {
                            style: "flex-direction: row; flex-wrap: wrap; gap: 6px;",
                            Button {
                                test_id: Some("ks-history-append".to_owned()),
                                variant: ButtonVariant::Secondary,
                                size: ButtonSize::Sm,
                                on_press: move |_| {
                                    let next = format!("{} [step {}]", history.read().current(), history.read().undo_count() + 1);
                                    history.write().push(next.clone());
                                    handle.update(|st| st.notes = next);
                                },
                                text { "{copy(\"+ Push Step\", pseudo)}" }
                            }
                            Button {
                                test_id: Some("ks-history-undo".to_owned()),
                                variant: ButtonVariant::Outline,
                                size: ButtonSize::Sm,
                                disabled: !can_undo,
                                on_press: move |_| {
                                    let maybe = history.write().undo().cloned();
                                    if let Some(prev) = maybe {
                                        handle.update(|st| st.notes = prev);
                                    }
                                },
                                text { "{copy(\"Undo\", pseudo)}" }
                            }
                            Button {
                                test_id: Some("ks-history-redo".to_owned()),
                                variant: ButtonVariant::Outline,
                                size: ButtonSize::Sm,
                                disabled: !can_redo,
                                on_press: move |_| {
                                    let maybe = history.write().redo().cloned();
                                    if let Some(next) = maybe {
                                        handle.update(|st| st.notes = next);
                                    }
                                },
                                text { "{copy(\"Redo\", pseudo)}" }
                            }
                        }
                    }
                }
            }
        }
    }
}
