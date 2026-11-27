//! Theme-aware [`ErrorBoundary`] wrapper built directly on `nve_dioxus::ErrorBoundary`.

use dioxus_core::EventHandler;
use dioxus_core_macro::Props;
use nve_dioxus::prelude::*;
pub use nve_dioxus::{
    throw_error, throw_ui_error, ui_error, CapturedError, ErrorBoundaryReset, ErrorContext,
    ErrorFallback, UiError, UiErrorReport,
};

use crate::components::{Button, ButtonSize, ButtonVariant, Card, CardVariant};
use crate::style_buf::{TypedStyleBuilder, TypedTextStyle};
use crate::components::roles::error_text;
use crate::theme::use_theme;

/// Theme-aware subtree [`ErrorBoundary`] isolating render `Result` errors and panics.
///
/// Delegates directly to `nve_dioxus::ErrorBoundary` (backed by `dioxus_core::ErrorContext`),
/// rendering a themed [`Card`] + Retry [`Button`] fallback by default when `fallback` is `None`.
#[component]
pub fn ErrorBoundary(
    #[props(into, default)] fallback: ErrorFallback,
    #[props(default)] on_error: Option<EventHandler<UiErrorReport>>,
    #[props(default)] on_reset: Option<EventHandler<u32>>,
    #[props(default)] title: Option<String>,
    #[props(default)] retry_label: Option<String>,
    #[props(default)] test_id: Option<String>,
    children: Element,
) -> Element {
    let resolved_fallback = if fallback.is_custom() {
        fallback
    } else {
        let fallback_title = title
            .clone()
            .unwrap_or_else(|| "Something went wrong".to_owned());
        let fallback_retry = retry_label.clone().unwrap_or_else(|| "Retry".to_owned());
        let fallback_tid = test_id.clone();
        ErrorFallback::from(move |report: UiErrorReport, reset: ErrorBoundaryReset| {
            let title_str = fallback_title.clone();
            let retry_str = fallback_retry.clone();
            let tid = fallback_tid.clone();
            rsx! {
                DefaultErrorFallbackView {
                    title: title_str,
                    retry_label: retry_str,
                    report,
                    reset,
                    test_id: tid,
                }
            }
        })
    };

    rsx! {
        nve_dioxus::ErrorBoundary {
            fallback: resolved_fallback,
            on_error: on_error,
            on_reset: on_reset,
            test_id: test_id,
            {children}
        }
    }
}

#[component]
fn DefaultErrorFallbackView(
    title: String,
    retry_label: String,
    report: UiErrorReport,
    reset: ErrorBoundaryReset,
    #[props(default)] test_id: Option<String>,
) -> Element {
    let theme = use_theme();
    let base_id = test_id.unwrap_or_else(|| "nve-error-boundary".to_owned());
    let title_id = format!("{base_id}-title");
    let msg_id = format!("{base_id}-message");
    let retry_id = format!("{base_id}-retry");

    let title_style = TypedStyleBuilder::new()
        .text_style(TypedTextStyle {
            color: error_text(&theme),
            font_size: theme.typography.body.size,
            font_weight: "600",
            text_align: "left",
            opacity: 1.0,
        })
        .finish();

    let message_style = TypedStyleBuilder::new()
        .text_style(TypedTextStyle {
            color: theme.text.secondary,
            font_size: theme.typography.ui.size,
            font_weight: "400",
            text_align: "left",
            opacity: 1.0,
        })
        .finish();

    rsx! {
        view {
            test_id: base_id,
            accessibility_role: "alert",
            aria_live: "assertive",
            Card {
                variant: CardVariant::Outlined,
                text {
                    test_id: title_id,
                    nve_style: title_style,
                    "{title}"
                }
                text {
                    test_id: msg_id,
                    nve_style: message_style,
                    "{report.message}"
                }
                Button {
                    test_id: retry_id,
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    on_press: move |_| reset.reset(),
                    text { "{retry_label}" }
                }
            }
        }
    }
}
