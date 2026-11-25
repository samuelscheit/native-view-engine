//! Accessible toast notification queue (`ToastProvider`, `use_toast`, `ToastController`,
//! `ToastViewport`) with `accessibilityRole="status"` and `ariaLive="polite" | "assertive"`.

use dioxus_core::{provide_context, try_consume_context, use_hook};
use dioxus_core_macro::Props;
use nve_dioxus::prelude::*;

use crate::style_buf::{StyleBuf, SurfaceStyle, TextStyle};
use crate::theme::use_theme;

/// Visual severity for a toast item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToastVariant {
    #[default]
    Info,
    Success,
    Warning,
    Danger,
}

/// ARIA live region politeness level for a toast item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToastLive {
    #[default]
    Polite,
    Assertive,
}

impl ToastLive {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Polite => "polite",
            Self::Assertive => "assertive",
        }
    }
}

/// Options used to enqueue a toast notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToastOptions {
    pub title: String,
    pub description: Option<String>,
    pub variant: ToastVariant,
    pub live: Option<ToastLive>,
    /// Auto-dismiss duration in milliseconds (`None` = sticky until dismissed).
    pub duration_ms: Option<u64>,
}

impl ToastOptions {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            description: None,
            variant: ToastVariant::Info,
            live: None,
            duration_ms: Some(4_000),
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_variant(mut self, variant: ToastVariant) -> Self {
        self.variant = variant;
        self
    }

    pub fn with_live(mut self, live: ToastLive) -> Self {
        self.live = Some(live);
        self
    }

    pub fn with_duration_ms(mut self, duration_ms: Option<u64>) -> Self {
        self.duration_ms = duration_ms;
        self
    }
}

/// Active toast entry stored in [`ToastQueue`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToastItem {
    pub id: u64,
    pub title: String,
    pub description: Option<String>,
    pub variant: ToastVariant,
    pub live: ToastLive,
    pub duration_ms: Option<u64>,
    pub elapsed_ms: u64,
}

/// Pure, deterministic toast queue state machine supporting enqueue, dismiss, clear, and tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToastQueue {
    next_id: u64,
    max_visible: usize,
    items: Vec<ToastItem>,
}

impl Default for ToastQueue {
    fn default() -> Self {
        Self::new(5)
    }
}

impl ToastQueue {
    pub fn new(max_visible: usize) -> Self {
        Self {
            next_id: 1,
            max_visible: max_visible.max(1),
            items: Vec::new(),
        }
    }

    pub fn items(&self) -> &[ToastItem] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Enqueues a toast and returns its monotonic ID. Drops oldest entries beyond `max_visible`.
    pub fn push(&mut self, opts: ToastOptions) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        let live = opts.live.unwrap_or(match opts.variant {
            ToastVariant::Danger => ToastLive::Assertive,
            ToastVariant::Info | ToastVariant::Success | ToastVariant::Warning => ToastLive::Polite,
        });
        self.items.push(ToastItem {
            id,
            title: opts.title,
            description: opts.description,
            variant: opts.variant,
            live,
            duration_ms: opts.duration_ms,
            elapsed_ms: 0,
        });
        while self.items.len() > self.max_visible {
            self.items.remove(0);
        }
        id
    }

    /// Dismisses a toast by ID, returning `true` if removed.
    pub fn dismiss(&mut self, id: u64) -> bool {
        let before = self.items.len();
        self.items.retain(|t| t.id != id);
        self.items.len() != before
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// Advances elapsed time by `delta_ms` and removes expired toasts, returning dismissed IDs.
    pub fn tick(&mut self, delta_ms: u64) -> Vec<u64> {
        let mut expired = Vec::new();
        self.items.retain_mut(|item| {
            item.elapsed_ms = item.elapsed_ms.saturating_add(delta_ms);
            if let Some(limit) = item.duration_ms
                && item.elapsed_ms >= limit
            {
                expired.push(item.id);
                false
            } else {
                true
            }
        });
        expired
    }
}

/// Imperative handle returned by [`use_toast()`] to push, dismiss, and tick notifications.
#[derive(Clone, Copy, PartialEq)]
pub struct ToastController {
    queue: Signal<ToastQueue>,
}

impl ToastController {
    pub fn push(&mut self, opts: ToastOptions) -> u64 {
        self.queue.write().push(opts)
    }

    pub fn info(&mut self, title: impl Into<String>) -> u64 {
        self.push(ToastOptions::new(title).with_variant(ToastVariant::Info))
    }

    pub fn success(&mut self, title: impl Into<String>) -> u64 {
        self.push(ToastOptions::new(title).with_variant(ToastVariant::Success))
    }

    pub fn warning(&mut self, title: impl Into<String>) -> u64 {
        self.push(ToastOptions::new(title).with_variant(ToastVariant::Warning))
    }

    pub fn danger(&mut self, title: impl Into<String>) -> u64 {
        self.push(ToastOptions::new(title).with_variant(ToastVariant::Danger))
    }

    pub fn dismiss(&mut self, id: u64) -> bool {
        self.queue.write().dismiss(id)
    }

    pub fn clear(&mut self) {
        self.queue.write().clear();
    }

    pub fn tick(&mut self, delta_ms: u64) -> Vec<u64> {
        self.queue.write().tick(delta_ms)
    }

    pub fn items(&self) -> Vec<ToastItem> {
        self.queue.read().items().to_vec()
    }

    pub fn len(&self) -> usize {
        self.queue.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.read().is_empty()
    }
}

/// Provides a [`ToastController`] context and optionally renders [`ToastViewport`].
#[component]
pub fn ToastProvider(
    #[props(default = 5)] max_visible: usize,
    #[props(default = true)] render_viewport: bool,
    #[props(default)] test_id: Option<String>,
    children: Element,
) -> Element {
    let _controller = use_hook(|| {
        let queue = Signal::new(ToastQueue::new(max_visible));
        provide_context(ToastController { queue })
    });

    rsx! {
        {children}
        if render_viewport {
            ToastViewport { test_id: test_id }
        }
    }
}

/// Returns the [`ToastController`] from the nearest [`ToastProvider`].
///
/// Falls back to a component-local controller when called outside a provider.
pub fn use_toast() -> ToastController {
    if let Some(ctrl) = try_consume_context::<ToastController>() {
        ctrl
    } else {
        let queue = use_signal(ToastQueue::default);
        ToastController { queue }
    }
}

/// Renders active toasts from the nearest [`ToastProvider`] with live-region semantics.
#[component]
pub fn ToastViewport(#[props(default)] test_id: Option<String>) -> Element {
    let theme = use_theme();
    let mut ctrl = use_toast();
    let items = ctrl.items();

    if items.is_empty() {
        return rsx! {};
    }

    let base_id = test_id.as_deref().unwrap_or("nve-toast-viewport");

    let mut stack_sb = StyleBuf::new();
    stack_sb
        .flex_direction("column")
        .gap(theme.spacing.sm)
        .padding(theme.spacing.md)
        .z_index(1100);
    let stack_style = stack_sb.finish();

    let mut rendered = Vec::with_capacity(items.len());
    for item in items {
        let accent = match item.variant {
            ToastVariant::Info => theme.palette.accent.as_ref(),
            ToastVariant::Success => theme.palette.success.as_ref(),
            ToastVariant::Warning => theme.palette.warning.as_ref(),
            ToastVariant::Danger => theme.palette.danger.as_ref(),
        };

        let mut card_sb = StyleBuf::new();
        card_sb
            .flex_direction("row")
            .justify_content("space-between")
            .align_items("center")
            .padding(theme.spacing.md)
            .gap(theme.spacing.md)
            .surface(SurfaceStyle {
                background: theme.palette.surface_elevated.as_ref(),
                border_width: 2.0,
                border_color: accent,
                border_radius: theme.radii.md,
                opacity: 1.0,
                box_shadow: theme.elevation_shadow(),
            });
        let card_style = card_sb.finish();

        let mut title_sb = StyleBuf::new();
        title_sb.text_style(TextStyle {
            color: theme.palette.text_primary.as_ref(),
            font_size: theme.typography.sm,
            font_weight: "600",
            text_align: "left",
            opacity: 1.0,
        });
        let title_style = title_sb.finish();

        let mut desc_sb = StyleBuf::new();
        desc_sb.text_style(TextStyle {
            color: theme.palette.text_secondary.as_ref(),
            font_size: theme.typography.xs,
            font_weight: "400",
            text_align: "left",
            opacity: 1.0,
        });
        let desc_style = desc_sb.finish();

        let mut dismiss_sb = StyleBuf::new();
        dismiss_sb
            .padding_xy(theme.spacing.sm, theme.spacing.xs)
            .surface(SurfaceStyle {
                background: theme.palette.surface_subtle.as_ref(),
                border_width: 1.0,
                border_color: theme.palette.border.as_ref(),
                border_radius: theme.radii.sm,
                opacity: 1.0,
                box_shadow: "none",
            });
        let dismiss_style = dismiss_sb.finish();

        let mut dismiss_ts = StyleBuf::new();
        dismiss_ts.text_style(TextStyle {
            color: theme.palette.text_primary.as_ref(),
            font_size: theme.typography.xs,
            font_weight: "600",
            text_align: "center",
            opacity: 1.0,
        });
        let dismiss_text_style = dismiss_ts.finish();

        let id = item.id;
        let live_str = item.live.as_str();
        let toast_tid = format!("{base_id}-item-{id}");
        let dismiss_tid = format!("{base_id}-dismiss-{id}");
        let title = item.title;
        let desc = item.description;

        rendered.push(rsx! {
            view {
                key: "{id}",
                test_id: "{toast_tid}",
                accessibility_role: "status",
                aria_live: "{live_str}",
                style: "{card_style}",
                view {
                    style: "flex-direction: column; gap: 2px;",
                    text {
                        style: "{title_style}",
                        "{title}"
                    }
                    if let Some(d) = desc {
                        text {
                            style: "{desc_style}",
                            "{d}"
                        }
                    }
                }
                pressable {
                    test_id: "{dismiss_tid}",
                    accessibility_role: "button",
                    accessibility_label: "Dismiss notification",
                    tab_index: 0,
                    style: "{dismiss_style}",
                    onpress: move |_| {
                        ctrl.dismiss(id);
                    },
                    text {
                        style: "{dismiss_text_style}",
                        "Dismiss"
                    }
                }
            }
        });
    }

    rsx! {
        view {
            test_id: "{base_id}",
            style: "{stack_style}",
            {rendered.into_iter()}
        }
    }
}
