//! Subtree `ErrorBoundary` built directly on `dioxus_core::use_error_boundary_provider()`.
//!
//! Catches both structured `Result` render errors (`?` / [`throw_ui_error`] / [`throw_error`])
//! and panics inside a bounded component subtree (via `dioxus_core`'s `VProps::render`
//! `catch_unwind` on unwind-capable targets), delivers a structured [`UiErrorReport`] to the
//! fallback UI and optional `on_error` telemetry hook, and provides an [`ErrorBoundaryReset`]
//! handle that cleanly remounts the protected subtree without affecting sibling subtrees.

use core::fmt;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub use dioxus_core::{throw_error, CapturedError, ErrorContext};
use dioxus_core::{provide_context, use_hook, Element, EventHandler};
use crate::rsx;
use dioxus_core_macro::{component, Props};

use crate::dioxus_elements;

/// Structured UI error carrying a message and optional caller source location.
///
/// Implements `std::error::Error` so it converts directly into `dioxus_core::RenderError` via
/// `?` in components or into `CapturedError` via [`throw_ui_error`] / [`throw_error`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiError {
    pub message: String,
    pub source_location: Option<String>,
}

impl UiError {
    /// Creates a [`UiError`] capturing the caller's `file:line:column` location.
    #[track_caller]
    pub fn new(message: impl Into<String>) -> Self {
        let loc = core::panic::Location::caller();
        Self {
            message: message.into(),
            source_location: Some(format!("{}:{}:{}", loc.file(), loc.line(), loc.column())),
        }
    }

    /// Creates a [`UiError`] with an explicit source location override.
    pub fn with_location(
        message: impl Into<String>,
        source_location: Option<String>,
    ) -> Self {
        Self {
            message: message.into(),
            source_location,
        }
    }
}

impl fmt::Display for UiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for UiError {}

/// Convenience constructor for a [`UiError`] recording the caller's source location.
#[track_caller]
#[inline]
pub fn ui_error(message: impl Into<String>) -> UiError {
    UiError::new(message)
}

/// Throws a structured [`UiError`] into the current Dioxus scope, bubbling to the nearest
/// enclosing [`ErrorBoundary`].
#[track_caller]
#[inline]
pub fn throw_ui_error(message: impl Into<String>) {
    dioxus_core::throw_error(UiError::new(message));
}

/// Structured diagnostic report passed to [`ErrorBoundary`] fallback renderers and `on_error`
/// telemetry callbacks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiErrorReport {
    /// Human-readable error or panic description.
    pub message: String,
    /// Optional `file:line:column` origin when raised via [`UiError`] / [`ui_error`] /
    /// [`throw_ui_error`].
    pub source_location: Option<String>,
    /// Number of times this boundary has been reset prior to this report (`0` on first failure).
    pub reset_count: u32,
    /// `true` when the failure originated from a caught Rust panic inside a child component.
    pub is_panic: bool,
}

impl UiErrorReport {
    /// Extracts a structured [`UiErrorReport`] from a `dioxus_core::CapturedError`.
    pub fn from_captured(error: &CapturedError, reset_count: u32) -> Self {
        if let Some(ui_err) = error.downcast_ref::<UiError>() {
            return Self {
                message: ui_err.message.clone(),
                source_location: ui_err.source_location.clone(),
                reset_count,
                is_panic: false,
            };
        }
        let message = error.to_string();
        let is_panic = message.starts_with("Encountered panic:");
        Self {
            message,
            source_location: None,
            reset_count,
            is_panic,
        }
    }
}

/// Cloneable recovery handle delivered to [`ErrorBoundary`] fallback UIs.
#[derive(Clone)]
pub struct ErrorBoundaryReset {
    context: ErrorContext,
    reset_count: Rc<Cell<u32>>,
    on_reset: Option<EventHandler<u32>>,
}

impl PartialEq for ErrorBoundaryReset {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.context == other.context && Rc::ptr_eq(&self.reset_count, &other.reset_count)
    }
}

impl ErrorBoundaryReset {
    /// Clears the captured error, increments `reset_count`, notifies `on_reset` (if registered),
    /// and marks the enclosing [`ErrorBoundary`] dirty so its child subtree cleanly remounts.
    pub fn reset(&self) {
        let next = self.reset_count.get().saturating_add(1);
        self.reset_count.set(next);
        if let Some(cb) = self.on_reset {
            cb.call(next);
        }
        self.context.clear_errors();
    }

    /// Returns how many times this boundary has been reset so far.
    #[inline]
    pub fn reset_count(&self) -> u32 {
        self.reset_count.get()
    }
}

/// Optional custom fallback renderer passed directly as a closure to [`ErrorBoundary`].
///
/// Defaults to the built-in accessible alert + Retry button fallback when omitted.
#[derive(Clone, Default)]
pub struct ErrorFallback(Option<Rc<dyn Fn(UiErrorReport, ErrorBoundaryReset) -> Element>>);

impl PartialEq for ErrorFallback {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (None, None) => true,
            (Some(a), Some(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl<F> From<F> for ErrorFallback
where
    F: Fn(UiErrorReport, ErrorBoundaryReset) -> Element + 'static,
{
    #[inline]
    fn from(f: F) -> Self {
        Self(Some(Rc::new(f)))
    }
}

impl ErrorFallback {
    /// Returns `true` when a custom fallback closure was supplied.
    #[inline]
    pub fn is_custom(&self) -> bool {
        self.0.is_some()
    }

    /// Invokes the custom fallback renderer if present.
    #[inline]
    pub fn render(&self, report: UiErrorReport, reset: ErrorBoundaryReset) -> Option<Element> {
        self.0.as_ref().map(|f| f(report, reset))
    }
}

/// Subtree panic & `Result` error boundary backed directly by `dioxus_core::ErrorContext`.
///
/// Isolates failures inside `children` so sibling subtrees remain mounted and interactive,
/// reports each distinct failure once to `on_error`, and renders either `fallback` or an
/// accessible built-in alert with a working Retry button (`reset.reset()`).
#[component]
pub fn ErrorBoundary(
    #[props(into, default)] fallback: ErrorFallback,
    #[props(default)] on_error: Option<EventHandler<UiErrorReport>>,
    #[props(default)] on_reset: Option<EventHandler<u32>>,
    #[props(default)] test_id: Option<String>,
    children: Element,
) -> Element {
    let error_context = use_hook(|| provide_context(ErrorContext::new(None)));
    let reset_count = use_hook(|| Rc::new(Cell::new(0u32)));
    let last_reported = use_hook(|| Rc::new(RefCell::new(None::<CapturedError>)));

    let captured = error_context.error();

    if let Some(err) = captured {
        let report = UiErrorReport::from_captured(&err, reset_count.get());
        let should_notify = {
            let mut slot = last_reported.borrow_mut();
            if slot.as_ref() != Some(&err) {
                *slot = Some(err);
                true
            } else {
                false
            }
        };
        if should_notify && let Some(handler) = on_error {
            handler.call(report.clone());
        }

        let reset_handle = ErrorBoundaryReset {
            context: error_context,
            reset_count,
            on_reset,
        };

        if let Some(custom_el) = fallback.render(report.clone(), reset_handle.clone()) {
            return custom_el;
        }
        return render_default_fallback(report, reset_handle, test_id);
    }

    last_reported.borrow_mut().take();
    rsx! { {children} }
}

fn render_default_fallback(
    report: UiErrorReport,
    reset: ErrorBoundaryReset,
    test_id: Option<String>,
) -> Element {
    let container_id = test_id.unwrap_or_else(|| "nve-error-boundary".to_owned());
    let message_id = format!("{container_id}-message");
    let retry_id = format!("{container_id}-retry");
    let msg = report.message;

    rsx! {
        view {
            test_id: container_id,
            accessibility_role: "alert",
            aria_live: "assertive",
            text {
                test_id: message_id,
                "{msg}"
            }
            pressable {
                test_id: retry_id,
                accessibility_role: "button",
                onpress: move |_| reset.reset(),
                text { "Retry" }
            }
        }
    }
}
