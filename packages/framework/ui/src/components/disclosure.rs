//! Accessible [`Disclosure`] and [`Accordion`] components emitting native HTML5
//! `<details>` / `<summary>` (working with 0 bytes of JavaScript on Web & static GitHub Pages)
//! paired with a [`SpringSpec`]-driven [`DisclosureSpring`] controller for smooth native motion.

use dioxus_core::EventHandler;
use dioxus_core_macro::Props;
use nve_abi::{ColorRgba, FlexAlign, FlexJustify};
use nve_dioxus::prelude::*;

use crate::style_buf::{TypedStyleBuilder, TypedSurfaceStyle, TypedTextStyle};
use crate::theme::use_theme_rgba;

/// Physics-backed disclosure animation state using [`SpringSpec`] for native hosts while Web
/// delegates zero-JS expand/collapse directly to `<details>` / `<summary>`.
#[derive(Debug, Clone, PartialEq)]
pub struct DisclosureSpring {
    spec: SpringSpec,
    state: SpringState,
    open: bool,
    motion_preference: MotionPreference,
}

impl DisclosureSpring {
    pub fn new(open: bool) -> Self {
        let initial = if open { 1.0 } else { 0.0 };
        Self {
            spec: SpringSpec::snappy(),
            state: SpringState::settled(initial),
            open,
            motion_preference: MotionPreference::Full,
        }
    }

    pub fn with_spec(mut self, spec: SpringSpec) -> Self {
        self.spec = spec;
        self
    }

    pub fn with_motion_preference(mut self, pref: MotionPreference) -> Self {
        self.motion_preference = pref;
        self
    }

    #[inline]
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Updates open target; under `MotionPreference::Reduce`, snaps immediately without oscillation.
    pub fn set_open(&mut self, open: bool) {
        self.open = open;
        let target = if open { 1.0 } else { 0.0 };
        self.state.retarget(target);
        if self.motion_preference == MotionPreference::Reduce {
            self.state.snap_to_target();
        }
    }

    /// Advances the disclosure spring by `dt_secs`, returning the clamped `[0.0, 1.0]` progress.
    pub fn step(&mut self, dt_secs: f32) -> f32 {
        self.state.step(&self.spec, dt_secs, self.motion_preference);
        self.progress()
    }

    #[inline]
    pub fn progress(&self) -> f32 {
        self.state.value.clamp(0.0, 1.0)
    }

    #[inline]
    pub fn is_settled(&self) -> bool {
        self.state.is_settled(&self.spec)
    }
}

/// Accessible single disclosure emitting native HTML5 `<details>` and `<summary>`.
///
/// - On Web (even with JavaScript completely disabled): `<details>`/`<summary>` expands/collapses
///   natively with 0 B JS, and optional `group_name` emits `<details name="...">` for native
///   exclusive accordion grouping.
/// - On Native (Skia/Headless) and hydrated Web: tracks open state, drives [`DisclosureSpring`],
///   supports `Enter`/`Space` keyboard toggling, and notifies `on_open_change`.
#[component]
pub fn Disclosure(
    summary: String,
    #[props(default)] open: Option<bool>,
    #[props(default)] default_open: bool,
    #[props(default)] group_name: Option<String>,
    #[props(default)] disabled: bool,
    #[props(default)] on_open_change: Option<EventHandler<bool>>,
    #[props(default)] id: Option<String>,
    #[props(default)] test_id: Option<String>,
    #[props(default)] style: Option<String>,
    children: Element,
) -> Element {
    let (theme, colors) = use_theme_rgba();
    let mut internal_open = use_signal(|| default_open);
    let mut spring = use_signal(|| DisclosureSpring::new(open.unwrap_or(default_open)));
    let mut focused = use_signal(|| false);

    let is_open = match open {
        Some(v) => v,
        None => internal_open(),
    };
    if spring.peek().is_open() != is_open {
        spring.write_unchecked().set_open(is_open);
    }

    let base_id = test_id.as_deref().unwrap_or("nve-disclosure");
    let details_id = id.as_deref().unwrap_or(base_id).to_owned();
    let summary_id = format!("{base_id}-summary");
    let content_id = format!("{base_id}-content");
    let aria_expanded = if is_open { "true" } else { "false" };

    let container_style = TypedStyleBuilder::new()
        .flex_col()
        .width_pct(100.0)
        .surface(TypedSurfaceStyle {
            background: colors.surface,
            border_width: if theme.is_high_contrast() { 2.0 } else { 1.0 },
            border_color: colors.border,
            border_radius: theme.radii.md,
            opacity: if disabled { 0.55 } else { 1.0 },
            box_shadow: "none",
        })
        .finish();

    let summary_style = TypedStyleBuilder::new()
        .flex_row()
        .justify_content(FlexJustify::SpaceBetween)
        .align_items(FlexAlign::Center)
        .padding_xy(theme.spacing.md, theme.spacing.sm)
        .surface(TypedSurfaceStyle {
            background: ColorRgba::TRANSPARENT,
            border_width: if focused() && !disabled { 2.0 } else { 0.0 },
            border_color: if focused() && !disabled {
                colors.border_focus
            } else {
                ColorRgba::TRANSPARENT
            },
            border_radius: theme.radii.md,
            opacity: 1.0,
            box_shadow: "none",
        })
        .finish();

    let summary_text_style = TypedStyleBuilder::new()
        .text_style(TypedTextStyle {
            color: colors.text_primary,
            font_size: theme.typography.md,
            font_weight: "600",
            text_align: "left",
            opacity: 1.0,
        })
        .finish();

    let content_style = TypedStyleBuilder::new()
        .flex_col()
        .padding_xy(theme.spacing.md, theme.spacing.sm)
        .gap(theme.spacing.xs)
        .finish();

    let is_controlled = open.is_some();
    let tab_index = if disabled { -1 } else { 0 };

    rsx! {
        details {
            id: "{details_id}",
            test_id: "{base_id}",
            open: is_open,
            name: group_name,
            nve_style: container_style,
            style: style,
            summary {
                test_id: "{summary_id}",
                accessibility_role: "button",
                aria_expanded: "{aria_expanded}",
                aria_controls: "{content_id}",
                disabled: disabled,
                tab_index: tab_index,
                nve_style: summary_style,
                onfocus: move |_| focused.set(true),
                onblur: move |_| focused.set(false),
                onpress: move |_| {
                    if !disabled {
                        let next = !is_open;
                        if !is_controlled {
                            internal_open.set(next);
                        }
                        spring.write().set_open(next);
                        if let Some(handler) = on_open_change {
                            handler.call(next);
                        }
                    }
                },
                onkeydown: move |ev: KeyEvent| {
                    let data = ev.data();
                    let key = data.key.as_str();
                    if !disabled && matches!(key, "Enter" | " " | "Space") {
                        let next = !is_open;
                        if !is_controlled {
                            internal_open.set(next);
                        }
                        spring.write().set_open(next);
                        if let Some(handler) = on_open_change {
                            handler.call(next);
                        }
                    }
                },
                text {
                    nve_style: summary_text_style,
                    "{summary}"
                }
            }
            view {
                id: "{content_id}",
                test_id: "{content_id}",
                "data-nve-open": "{aria_expanded}",
                nve_style: content_style,
                {children}
            }
        }
    }
}

/// Data item rendered inside an [`Accordion`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccordionItem {
    pub id: String,
    pub summary: String,
    pub content: String,
    pub disabled: bool,
}

impl AccordionItem {
    pub fn new(
        id: impl Into<String>,
        summary: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            summary: summary.into(),
            content: content.into(),
            disabled: false,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

/// Accessible multi-item [`Accordion`] backed by native HTML5 `<details>` / `<summary>`.
///
/// When `exclusive` is `true` (default), every `<details>` emits `name="{group_name}"` so browsers
/// enforce single-open mutual exclusion natively with 0 bytes of JS, while live WASM/native hosts
/// synchronize the exact same exclusive open set in Rust.
#[component]
pub fn Accordion(
    items: Vec<AccordionItem>,
    #[props(default = true)] exclusive: bool,
    #[props(default)] default_open_id: Option<String>,
    #[props(default)] group_name: Option<String>,
    #[props(default)] on_toggle: Option<EventHandler<(String, bool)>>,
    #[props(default)] test_id: Option<String>,
    #[props(default)] style: Option<String>,
) -> Element {
    let (theme, colors) = use_theme_rgba();
    let mut open_ids = use_signal(|| {
        default_open_id
            .clone()
            .map(|id| vec![id])
            .unwrap_or_default()
    });

    let base_id = test_id.as_deref().unwrap_or("nve-accordion");
    let resolved_group = if exclusive {
        Some(group_name.unwrap_or_else(|| format!("{base_id}-group")))
    } else {
        None
    };

    let wrap_style = TypedStyleBuilder::new()
        .flex_col()
        .gap(theme.spacing.xs)
        .width_pct(100.0)
        .finish();

    let body_text_style = TypedStyleBuilder::new()
        .text_style(TypedTextStyle {
            color: colors.text_secondary,
            font_size: theme.typography.sm,
            font_weight: "400",
            text_align: "left",
            opacity: 1.0,
        })
        .finish();

    let mut rendered = Vec::with_capacity(items.len());
    for item in items {
        let item_id = item.id.clone();
        let item_test_id = format!("{base_id}-item-{}", item.id);
        let is_open = open_ids.read().iter().any(|id| id == &item_id);
        let group = resolved_group.clone();
        let summary = item.summary.clone();
        let body = item.content.clone();
        let body_style = body_text_style.clone();

        rendered.push(rsx! {
            Disclosure {
                key: "{item_id}",
                id: "{item_test_id}",
                test_id: "{item_test_id}",
                summary: summary,
                open: Some(is_open),
                group_name: group,
                disabled: item.disabled,
                on_open_change: move |next| {
                    let mut ids = open_ids.write();
                    if next {
                        if exclusive {
                            ids.clear();
                        }
                        if !ids.iter().any(|x| x == &item_id) {
                            ids.push(item_id.clone());
                        }
                    } else {
                        ids.retain(|x| x != &item_id);
                    }
                    if let Some(handler) = on_toggle {
                        handler.call((item_id.clone(), next));
                    }
                },
                text {
                    nve_style: body_style,
                    "{body}"
                }
            }
        });
    }

    rsx! {
        view {
            test_id: "{base_id}",
            accessibility_role: "region",
            nve_style: wrap_style,
            style: style,
            {rendered.into_iter()}
        }
    }
}
