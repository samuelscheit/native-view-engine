//! Resizable horizontal and vertical [`SplitPane`] component and pure headless
//! [`SplitPaneState`] engine with pixel/ratio bounds clamping, collapse/restore memory,
//! pointer drag deltas, and accessible keyboard (`Arrow`/`Home`/`End`/`Enter`/`Space`)
//! separator resizing.

use dioxus_core::EventHandler;
use dioxus_core_macro::Props;
use nve_abi::{FlexAlign, FlexJustify};
use nve_dioxus::prelude::*;

use crate::focus::Orientation;
use crate::style_buf::{TypedStyleBuilder, TypedSurfaceStyle};
use crate::theme::use_theme_rgba;

/// Size constraint expressed either in logical pixels or as a fraction (`0.0..=1.0`) of the
/// usable split axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SplitBound {
    Pixels(f32),
    Ratio(f32),
}

impl SplitBound {
    #[inline]
    pub fn resolve(self, usable_px: f32) -> f32 {
        let usable = if usable_px.is_finite() {
            usable_px.max(0.0)
        } else {
            0.0
        };
        match self {
            Self::Pixels(px) => {
                if px.is_finite() {
                    px.max(0.0)
                } else {
                    0.0
                }
            }
            Self::Ratio(r) => {
                if r.is_finite() {
                    r.clamp(0.0, 1.0) * usable
                } else {
                    0.0
                }
            }
        }
    }
}

/// Headless resizable split-pane state machine supporting pixel or ratio constraints,
/// collapse/restore toggle, pointer drag delta updates, and keyboard resizing.
#[derive(Debug, Clone, PartialEq)]
pub struct SplitPaneState {
    orientation: Orientation,
    container_size: f32,
    divider_size: f32,
    primary_ratio: f32,
    min_primary: SplitBound,
    max_primary: Option<SplitBound>,
    min_secondary: SplitBound,
    collapsible: bool,
    collapsed: bool,
    pre_collapse_ratio: f32,
}

impl SplitPaneState {
    pub fn new(orientation: Orientation, container_size: f32, initial_ratio: f32) -> Self {
        let size = if container_size.is_finite() {
            container_size.max(0.0)
        } else {
            0.0
        };
        let ratio = if initial_ratio.is_finite() {
            initial_ratio.clamp(0.0, 1.0)
        } else {
            0.5
        };
        Self {
            orientation,
            container_size: size,
            divider_size: 6.0,
            primary_ratio: ratio,
            min_primary: SplitBound::Pixels(0.0),
            max_primary: None,
            min_secondary: SplitBound::Pixels(0.0),
            collapsible: true,
            collapsed: false,
            pre_collapse_ratio: ratio,
        }
    }

    pub fn with_divider_size(mut self, divider_size: f32) -> Self {
        self.divider_size = if divider_size.is_finite() {
            divider_size.max(0.0)
        } else {
            0.0
        };
        self.reclamp();
        self
    }

    pub fn with_min_primary(mut self, bound: SplitBound) -> Self {
        self.min_primary = bound;
        self.reclamp();
        self
    }

    pub fn with_max_primary(mut self, bound: SplitBound) -> Self {
        self.max_primary = Some(bound);
        self.reclamp();
        self
    }

    pub fn with_min_secondary(mut self, bound: SplitBound) -> Self {
        self.min_secondary = bound;
        self.reclamp();
        self
    }

    pub fn with_collapsible(mut self, collapsible: bool) -> Self {
        self.collapsible = collapsible;
        if !collapsible && self.collapsed {
            self.collapsed = false;
            self.reclamp();
        }
        self
    }

    #[inline]
    pub fn orientation(&self) -> Orientation {
        self.orientation
    }

    #[inline]
    pub fn container_size(&self) -> f32 {
        self.container_size
    }

    #[inline]
    pub fn divider_size(&self) -> f32 {
        self.divider_size
    }

    #[inline]
    pub fn is_collapsed(&self) -> bool {
        self.collapsed
    }

    #[inline]
    pub fn usable_size(&self) -> f32 {
        (self.container_size - self.divider_size).max(0.0)
    }

    /// Computes effective `(min_primary_px, max_primary_px)` after resolving `min_primary`,
    /// `max_primary`, and `min_secondary` against the current container size.
    pub fn effective_bounds_px(&self) -> (f32, f32) {
        let usable = self.usable_size();
        let min_p = self.min_primary.resolve(usable).clamp(0.0, usable);
        let min_s = self.min_secondary.resolve(usable).clamp(0.0, usable);
        let max_from_secondary = (usable - min_s).max(0.0);
        let max_p = match self.max_primary {
            Some(bound) => bound
                .resolve(usable)
                .clamp(0.0, usable)
                .min(max_from_secondary),
            None => max_from_secondary,
        };
        let eff_min = min_p.min(max_p);
        (eff_min, max_p)
    }

    /// Updates container extent and re-clamps the primary ratio cleanly.
    pub fn set_container_size(&mut self, container_size: f32) {
        self.container_size = if container_size.is_finite() {
            container_size.max(0.0)
        } else {
            0.0
        };
        self.reclamp();
    }

    #[inline]
    pub fn primary_px(&self) -> f32 {
        if self.collapsed {
            return 0.0;
        }
        let usable = self.usable_size();
        let (min_p, max_p) = self.effective_bounds_px();
        (self.primary_ratio * usable).clamp(min_p, max_p)
    }

    #[inline]
    pub fn secondary_px(&self) -> f32 {
        (self.usable_size() - self.primary_px()).max(0.0)
    }

    #[inline]
    pub fn primary_ratio(&self) -> f32 {
        let usable = self.usable_size();
        if usable <= 0.0 {
            0.0
        } else {
            (self.primary_px() / usable).clamp(0.0, 1.0)
        }
    }

    /// Sets primary pane size in logical pixels, uncollapsing if needed and clamping to bounds.
    pub fn set_primary_px(&mut self, px: f32) -> f32 {
        let usable = self.usable_size();
        if usable <= 0.0 {
            self.primary_ratio = 0.0;
            return 0.0;
        }
        self.collapsed = false;
        let (min_p, max_p) = self.effective_bounds_px();
        let clamped = if px.is_finite() { px } else { min_p }.clamp(min_p, max_p);
        self.primary_ratio = (clamped / usable).clamp(0.0, 1.0);
        self.pre_collapse_ratio = self.primary_ratio;
        clamped
    }

    /// Sets primary pane ratio (`0.0..=1.0`), uncollapsing if needed and clamping to bounds.
    pub fn set_primary_ratio(&mut self, ratio: f32) -> f32 {
        let r = if ratio.is_finite() {
            ratio.clamp(0.0, 1.0)
        } else {
            0.5
        };
        self.set_primary_px(r * self.usable_size())
    }

    /// Applies an incremental pointer drag delta along the split axis (`+delta` grows primary).
    pub fn apply_drag_delta(&mut self, delta_px: f32) -> f32 {
        if !delta_px.is_finite() || delta_px == 0.0 {
            return self.primary_px();
        }
        if self.collapsed {
            if delta_px > 0.0 {
                return self.set_primary_px(delta_px);
            }
            return 0.0;
        }
        let next = self.primary_px() + delta_px;
        self.set_primary_px(next)
    }

    /// Collapses the primary pane to `0px`, remembering the previous ratio for [`Self::restore`].
    pub fn collapse(&mut self) -> bool {
        if !self.collapsible || self.collapsed {
            return false;
        }
        self.pre_collapse_ratio = self.primary_ratio;
        self.collapsed = true;
        true
    }

    /// Restores the primary pane from its collapsed state to the saved ratio (clamped to bounds).
    pub fn restore(&mut self) -> bool {
        if !self.collapsed {
            return false;
        }
        self.collapsed = false;
        self.primary_ratio = self.pre_collapse_ratio;
        self.reclamp();
        true
    }

    /// Toggles collapsed state; returns the new `is_collapsed()` value.
    pub fn toggle_collapse(&mut self) -> bool {
        if self.collapsed {
            self.restore();
        } else {
            self.collapse();
        }
        self.collapsed
    }

    /// Handles keyboard separator resizing:
    /// - Horizontal: `ArrowLeft` / `ArrowRight`
    /// - Vertical: `ArrowUp` / `ArrowDown`
    /// - `Home` -> jump to minimum primary size
    /// - `End` -> jump to maximum primary size
    /// - `Enter` / `Space` -> toggle collapse (when collapsible)
    pub fn handle_key(&mut self, key: &str, step_px: f32) -> bool {
        let step = if step_px.is_finite() && step_px > 0.0 {
            step_px
        } else {
            16.0
        };
        match (self.orientation, key) {
            (Orientation::Horizontal, "ArrowLeft") | (Orientation::Vertical, "ArrowUp") => {
                let prev = self.primary_px();
                let next = self.apply_drag_delta(-step);
                (next - prev).abs() > 1e-4
            }
            (Orientation::Horizontal, "ArrowRight") | (Orientation::Vertical, "ArrowDown") => {
                let prev = self.primary_px();
                let next = self.apply_drag_delta(step);
                (next - prev).abs() > 1e-4
            }
            (_, "Home") => {
                let (min_p, _) = self.effective_bounds_px();
                let prev = self.primary_px();
                let was_collapsed = self.collapsed;
                let next = self.set_primary_px(min_p);
                was_collapsed || (next - prev).abs() > 1e-4
            }
            (_, "End") => {
                let (_, max_p) = self.effective_bounds_px();
                let prev = self.primary_px();
                let was_collapsed = self.collapsed;
                let next = self.set_primary_px(max_p);
                was_collapsed || (next - prev).abs() > 1e-4
            }
            (_, "Enter" | " " | "Space") if self.collapsible => {
                let prev = self.collapsed;
                self.toggle_collapse() != prev
            }
            _ => false,
        }
    }

    /// Integer percentage (`0..=100`) for `aria-valuenow`.
    pub fn aria_value_now(&self) -> i32 {
        (self.primary_ratio() * 100.0).round() as i32
    }

    /// Integer percentage (`0..=100`) for `aria-valuemin`.
    pub fn aria_value_min(&self) -> i32 {
        let usable = self.usable_size();
        if usable <= 0.0 {
            0
        } else {
            let (min_p, _) = self.effective_bounds_px();
            ((min_p / usable).clamp(0.0, 1.0) * 100.0).round() as i32
        }
    }

    /// Integer percentage (`0..=100`) for `aria-valuemax`.
    pub fn aria_value_max(&self) -> i32 {
        let usable = self.usable_size();
        if usable <= 0.0 {
            100
        } else {
            let (_, max_p) = self.effective_bounds_px();
            ((max_p / usable).clamp(0.0, 1.0) * 100.0).round() as i32
        }
    }

    fn reclamp(&mut self) {
        let usable = self.usable_size();
        if usable <= 0.0 {
            return;
        }
        let (min_p, max_p) = self.effective_bounds_px();
        let clamped = (self.primary_ratio * usable).clamp(min_p, max_p);
        self.primary_ratio = (clamped / usable).clamp(0.0, 1.0);
    }
}

/// Accessible resizable horizontal or vertical split-pane component with a keyboard-focusable
/// `separator` divider (`ArrowLeft`/`ArrowRight` or `ArrowUp`/`ArrowDown`, `Home`, `End`,
/// `Enter`/`Space` collapse toggle).
#[component]
pub fn SplitPane(
    #[props(default = Orientation::Horizontal)] orientation: Orientation,
    #[props(default = 0.5)] initial_ratio: f32,
    #[props(default = 800.0)] container_size: f32,
    #[props(default = 6.0)] divider_size: f32,
    #[props(default = SplitBound::Pixels(80.0))] min_primary: SplitBound,
    #[props(default)] max_primary: Option<SplitBound>,
    #[props(default = SplitBound::Pixels(80.0))] min_secondary: SplitBound,
    #[props(default = 24.0)] keyboard_step_px: f32,
    #[props(default = true)] collapsible: bool,
    #[props(default)] on_change: Option<EventHandler<f32>>,
    #[props(default)] accessibility_label: Option<String>,
    #[props(default)] test_id: Option<String>,
    #[props(default)] style: Option<String>,
    primary: Element,
    secondary: Element,
) -> Element {
    let (_theme, colors) = use_theme_rgba();
    let mut divider_focused = use_signal(|| false);
    let mut state = use_signal(|| {
        let mut s = SplitPaneState::new(orientation, container_size, initial_ratio)
            .with_divider_size(divider_size)
            .with_min_primary(min_primary)
            .with_min_secondary(min_secondary)
            .with_collapsible(collapsible);
        if let Some(max_p) = max_primary {
            s = s.with_max_primary(max_p);
        }
        s
    });

    {
        let mut s = state.write_unchecked();
        if (s.container_size() - container_size).abs() > 1e-3 {
            s.set_container_size(container_size);
        }
    }

    let base_id = test_id.as_deref().unwrap_or("nve-split-pane");
    let primary_id = format!("{base_id}-primary");
    let divider_id = format!("{base_id}-divider");
    let secondary_id = format!("{base_id}-secondary");

    let (primary_px, secondary_px, val_now, val_min, val_max, is_collapsed) = {
        let s = state.read();
        (
            s.primary_px(),
            s.secondary_px(),
            s.aria_value_now(),
            s.aria_value_min(),
            s.aria_value_max(),
            s.is_collapsed(),
        )
    };

    let is_horiz = orientation == Orientation::Horizontal;
    // WAI-ARIA: separator orientation describes the divider bar itself (vertical bar splits left/right).
    let aria_orientation = if is_horiz { "vertical" } else { "horizontal" };
    let a11y_label = accessibility_label.unwrap_or_else(|| "Resize split pane".to_owned());

    let root_style = if is_horiz {
        TypedStyleBuilder::new()
            .flex_row()
            .width_px(container_size)
            .height_pct(100.0)
            .finish()
    } else {
        TypedStyleBuilder::new()
            .flex_col()
            .width_pct(100.0)
            .height_px(container_size)
            .finish()
    };

    let primary_style = if is_horiz {
        TypedStyleBuilder::new()
            .width_px(primary_px)
            .height_pct(100.0)
            .overflow("hidden")
            .finish()
    } else {
        TypedStyleBuilder::new()
            .width_pct(100.0)
            .height_px(primary_px)
            .overflow("hidden")
            .finish()
    };

    let secondary_style = if is_horiz {
        TypedStyleBuilder::new()
            .width_px(secondary_px)
            .height_pct(100.0)
            .flex_grow(1.0)
            .overflow("hidden")
            .finish()
    } else {
        TypedStyleBuilder::new()
            .width_pct(100.0)
            .height_px(secondary_px)
            .flex_grow(1.0)
            .overflow("hidden")
            .finish()
    };

    let divider_base = if is_horiz {
        TypedStyleBuilder::new()
            .width_px(divider_size)
            .height_pct(100.0)
    } else {
        TypedStyleBuilder::new()
            .width_pct(100.0)
            .height_px(divider_size)
    };
    let divider_style = divider_base
        .justify_content(FlexJustify::Center)
        .align_items(FlexAlign::Center)
        .surface(TypedSurfaceStyle {
            background: if divider_focused() {
                colors.border_focus
            } else {
                colors.border
            },
            border_width: 0.0,
            border_color: colors.border,
            border_radius: 0.0,
            opacity: 1.0,
            box_shadow: "none",
        })
        .finish();

    let collapsed_attr = if is_collapsed { "true" } else { "false" };

    rsx! {
        view {
            test_id: "{base_id}",
            nve_style: root_style,
            style: style,
            view {
                test_id: "{primary_id}",
                "data-collapsed": "{collapsed_attr}",
                nve_style: primary_style,
                {primary}
            }
            pressable {
                test_id: "{divider_id}",
                accessibility_role: "separator",
                accessibility_label: "{a11y_label}",
                aria_orientation: "{aria_orientation}",
                aria_valuenow: "{val_now}",
                aria_valuemin: "{val_min}",
                aria_valuemax: "{val_max}",
                tab_index: 0,
                nve_style: divider_style,
                onfocus: move |_| divider_focused.set(true),
                onblur: move |_| divider_focused.set(false),
                onpress: move |_| {
                    if collapsible {
                        let _ = state.write().toggle_collapse();
                        if let Some(handler) = on_change {
                            handler.call(state.read().primary_ratio());
                        }
                    }
                },
                onkeydown: move |ev: KeyEvent| {
                    let changed = state
                        .write()
                        .handle_key(ev.data().key.as_str(), keyboard_step_px);
                    if changed && let Some(handler) = on_change {
                        handler.call(state.read().primary_ratio());
                    }
                },
            }
            view {
                test_id: "{secondary_id}",
                nve_style: secondary_style,
                {secondary}
            }
        }
    }
}
