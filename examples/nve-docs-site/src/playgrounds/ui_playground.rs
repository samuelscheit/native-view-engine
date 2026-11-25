//! Interactive `"ui-playground"` demo exercising `nve-ui` Button, Switch, Checkbox, Input,
//! Badge, Tabs, and Toast notifications inside `nve-docs`.

use nve_dioxus::prelude::*;
use nve_ui::{
    Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Checkbox, Input, Switch, TabList,
    TabPanel, TabTrigger, TabsRoot, ToastOptions, ToastProvider, ToastVariant, use_theme,
    use_toast,
};

pub const UI_PLAYGROUND_SOURCE: &str = r#"use nve_dioxus::prelude::*;
use nve_ui::{
    Badge, BadgeVariant, Button, ButtonVariant, Checkbox, Input,
    Switch, TabsRoot, TabList, TabTrigger, TabPanel, ToastProvider, use_toast,
};

#[component]
pub fn UiPlaygroundDemo() -> Element {
    let mut clicks = use_signal(|| 0u32);
    let mut high_perf = use_signal(|| true);
    let mut direct_ffi = use_signal(|| true);
    let mut name = use_signal(|| "Studio Surface".to_string());
    let mut toast = use_toast();

    rsx! {
        Button {
            variant: ButtonVariant::Primary,
            on_press: move |_| {
                clicks += 1;
                toast.success(format!("Frame committed ({})", clicks()));
            },
            "Primary ({clicks})"
        }
        Switch { checked: high_perf(), label: Some("Zero-Copy Pipeline".into()), on_change: move |v| high_perf.set(v) }
        Checkbox { checked: direct_ffi(), label: Some("Direct FFI".into()), on_change: move |v| direct_ffi.set(v) }
        Input { value: name(), on_change: move |v| name.set(v) }
    }
}"#;

#[component]
pub fn UiPlaygroundDemo() -> Element {
    rsx! {
        ToastProvider {
            max_visible: 3,
            test_id: Some("pg-ui-toast-viewport".to_string()),
            UiPlaygroundInner {}
        }
    }
}

#[component]
fn UiPlaygroundInner() -> Element {
    let theme = use_theme();
    let mut press_count = use_signal(|| 0u32);
    let mut zero_copy = use_signal(|| true);
    let mut direct_ffi = use_signal(|| true);
    let mut surface_name = use_signal(|| "Aether Surface".to_string());
    let mut active_tab = use_signal(|| "controls".to_string());
    let mut toast = use_toast();

    let status_badge = if zero_copy() && direct_ffi() {
        ("Zero-Copy Active", BadgeVariant::Success)
    } else if zero_copy() || direct_ffi() {
        ("Hybrid Mode", BadgeVariant::Accent)
    } else {
        ("Buffered Fallback", BadgeVariant::Warning)
    };

    rsx! {
        view {
            test_id: "pg-ui-root",
            style: "width: 100%; flex-direction: column; gap: 14px;",

            // Header telemetry row
            view {
                style: "width: 100%; flex-direction: row; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 8px;",
                view {
                    style: "flex-direction: row; align-items: center; gap: 8px; flex-wrap: wrap;",
                    Badge {
                        test_id: Some("pg-ui-status-badge".to_string()),
                        label: status_badge.0.to_string(),
                        variant: status_badge.1,
                    }
                    Badge {
                        test_id: Some("pg-ui-count-badge".to_string()),
                        label: format!("Actions: {}", press_count()),
                        variant: BadgeVariant::Neutral,
                    }
                }
                text {
                    test_id: "pg-ui-surface-echo",
                    style: "font-size: 12px; color: {theme.palette.text_secondary};",
                    "Target: {surface_name}"
                }
            }

            // Button variants row
            view {
                style: "width: 100%; flex-direction: row; flex-wrap: wrap; gap: 8px; align-items: center;",
                Button {
                    test_id: Some("pg-ui-btn-primary".to_string()),
                    variant: ButtonVariant::Primary,
                    size: ButtonSize::Sm,
                    on_press: move |_| {
                        let next = press_count() + 1;
                        press_count.set(next);
                        toast.push(
                            ToastOptions::new(format!("Primary action #{next}"))
                                .with_description(format!("Dispatched to {}", surface_name()))
                                .with_variant(ToastVariant::Success),
                        );
                    },
                    "Primary"
                }
                Button {
                    test_id: Some("pg-ui-btn-secondary".to_string()),
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    on_press: move |_| press_count += 1,
                    "Secondary"
                }
                Button {
                    test_id: Some("pg-ui-btn-outline".to_string()),
                    variant: ButtonVariant::Outline,
                    size: ButtonSize::Sm,
                    on_press: move |_| press_count += 1,
                    "Outline"
                }
                Button {
                    test_id: Some("pg-ui-btn-ghost".to_string()),
                    variant: ButtonVariant::Ghost,
                    size: ButtonSize::Sm,
                    on_press: move |_| press_count += 1,
                    "Ghost"
                }
                Button {
                    test_id: Some("pg-ui-btn-destructive".to_string()),
                    variant: ButtonVariant::Destructive,
                    size: ButtonSize::Sm,
                    on_press: move |_| {
                        press_count.set(0);
                        toast.push(
                            ToastOptions::new("Action counter reset")
                                .with_variant(ToastVariant::Warning),
                        );
                    },
                    "Reset"
                }
            }

            // Tabs compound component showcasing Controls vs Form vs Accessibility
            TabsRoot {
                test_id: Some("pg-ui-tabs".to_string()),
                id_prefix: Some("pg-ui".to_string()),
                value: Some(active_tab()),
                default_value: "controls".to_string(),
                on_change: move |v| active_tab.set(v),

                TabList {
                    accessibility_label: Some("UI Playground sections".to_string()),
                    TabTrigger {
                        value: "controls".to_string(),
                        test_id: Some("pg-ui-tab-controls".to_string()),
                        "Toggles & Input"
                    }
                    TabTrigger {
                        value: "badges".to_string(),
                        test_id: Some("pg-ui-tab-badges".to_string()),
                        "Semantic Badges"
                    }
                    TabTrigger {
                        value: "toasts".to_string(),
                        test_id: Some("pg-ui-tab-toasts".to_string()),
                        "Live Toasts"
                    }
                }

                TabPanel {
                    value: "controls".to_string(),
                    test_id: Some("pg-ui-panel-controls".to_string()),
                    view {
                        style: "width: 100%; flex-direction: column; gap: 10px;",
                        view {
                            style: "flex-direction: row; flex-wrap: wrap; gap: 16px; align-items: center;",
                            Switch {
                                test_id: Some("pg-ui-switch".to_string()),
                                checked: zero_copy(),
                                label: Some("Zero-Copy SceneView".to_string()),
                                on_change: move |v| zero_copy.set(v),
                            }
                            Checkbox {
                                test_id: Some("pg-ui-checkbox".to_string()),
                                checked: direct_ffi(),
                                label: Some("Direct Event FFI".to_string()),
                                on_change: move |v| direct_ffi.set(v),
                            }
                        }
                        Input {
                            test_id: Some("pg-ui-input".to_string()),
                            label: Some("Surface Label".to_string()),
                            placeholder: Some("Enter a surface name...".to_string()),
                            helper_text: Some("Updates the live header telemetry across all backends.".to_string()),
                            value: surface_name(),
                            on_change: move |v| surface_name.set(v),
                        }
                    }
                }

                TabPanel {
                    value: "badges".to_string(),
                    test_id: Some("pg-ui-panel-badges".to_string()),
                    view {
                        style: "flex-direction: row; flex-wrap: wrap; gap: 8px; align-items: center;",
                        Badge { label: "Neutral".to_string(), variant: BadgeVariant::Neutral }
                        Badge { label: "Accent".to_string(), variant: BadgeVariant::Accent }
                        Badge { label: "Success".to_string(), variant: BadgeVariant::Success }
                        Badge { label: "Warning".to_string(), variant: BadgeVariant::Warning }
                        Badge { label: "Danger".to_string(), variant: BadgeVariant::Danger }
                    }
                }

                TabPanel {
                    value: "toasts".to_string(),
                    test_id: Some("pg-ui-panel-toasts".to_string()),
                    view {
                        style: "flex-direction: row; flex-wrap: wrap; gap: 8px; align-items: center;",
                        Button {
                            test_id: Some("pg-ui-toast-success".to_string()),
                            variant: ButtonVariant::Primary,
                            size: ButtonSize::Sm,
                            on_press: move |_| {
                                toast.push(
                                    ToastOptions::new("Snapshot saved")
                                        .with_description("Binary NVE_SIG1 archive written in 0.12ms.")
                                        .with_variant(ToastVariant::Success),
                                );
                            },
                            "Success Toast"
                        }
                        Button {
                            test_id: Some("pg-ui-toast-danger".to_string()),
                            variant: ButtonVariant::Destructive,
                            size: ButtonSize::Sm,
                            on_press: move |_| {
                                toast.push(
                                    ToastOptions::new("Checksum mismatch rejected")
                                        .with_description("Corrupt frame quarantined safely.")
                                        .with_variant(ToastVariant::Danger),
                                );
                            },
                            "Assertive Alert"
                        }
                    }
                }
            }
        }
    }
}
