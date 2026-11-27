//! Multi-snap-point, velocity-aware, drag-to-dismiss [`BottomSheet`] component and pure
//! headless [`BottomSheetState`] state machine with seamless inner-scroll handoff.

use dioxus_core::EventHandler;
use dioxus_core_macro::Props;
use nve_abi::{ColorRgba, FlexAlign, FlexJustify};
use nve_dioxus::prelude::*;

use crate::style_buf::{TypedStyleBuilder, TypedSurfaceStyle, TypedTextStyle};
use crate::theme::use_theme_rgba;

const EPS: f32 = 1e-4;

/// Snap detent representing the sheet's height as a fraction (`0.0..=1.0`) of the viewport.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SheetDetent {
    Peek,
    Half,
    Full,
    Custom(f32),
}

impl SheetDetent {
    /// Returns the normalized viewport height fraction in `0.05..=1.0`.
    #[inline]
    pub fn fraction(self) -> f32 {
        match self {
            Self::Peek => 0.25,
            Self::Half => 0.50,
            Self::Full => 0.92,
            Self::Custom(f) => {
                if f.is_finite() {
                    f.clamp(0.05, 1.0)
                } else {
                    0.50
                }
            }
        }
    }
}

/// Outcome of routing a vertical drag increment through [`BottomSheetState::handle_drag_delta`].
///
/// Screen-space convention: positive `dy` drags downward, negative `dy` drags upward.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DragHandoff {
    /// The sheet absorbed part or all of the drag gesture (plus any overflow passed to inner scroll).
    SheetMoved {
        height_fraction: f32,
        consumed_dy: f32,
        remaining_scroll_dy: f32,
    },
    /// The drag gesture was handed off entirely to the inner scroll container.
    InnerScroll { delta_y: f32 },
}

/// Resolution returned by [`BottomSheetState::end_drag`] after velocity & position snapping.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SheetSnapOutcome {
    Snapped {
        index: usize,
        detent: SheetDetent,
        fraction: f32,
    },
    Dismissed,
}

/// Headless multi-detent bottom-sheet state machine supporting velocity-aware flick snapping,
/// drag-to-dismiss, keyboard handle control, and seamless inner-scroll handoff.
#[derive(Debug, Clone, PartialEq)]
pub struct BottomSheetState {
    detents: Vec<SheetDetent>,
    active_index: usize,
    height_fraction: f32,
    viewport_height: f32,
    open: bool,
    dismissible: bool,
    flick_velocity_threshold: f32,
    dragging: bool,
}

impl BottomSheetState {
    /// Creates a new sheet state with `detents` sorted ascending by fraction. Falls back to
    /// `[Peek, Half, Full]` when `detents` is empty.
    pub fn new(detents: Vec<SheetDetent>, initial_detent: usize, viewport_height: f32) -> Self {
        let mut sorted = if detents.is_empty() {
            vec![SheetDetent::Peek, SheetDetent::Half, SheetDetent::Full]
        } else {
            detents
        };
        sorted.sort_by(|a, b| a.fraction().total_cmp(&b.fraction()));
        sorted.dedup_by(|a, b| (a.fraction() - b.fraction()).abs() <= EPS);

        let idx = initial_detent.min(sorted.len() - 1);
        let frac = sorted[idx].fraction();
        let vp_h = if viewport_height.is_finite() {
            viewport_height.max(1.0)
        } else {
            800.0
        };

        Self {
            detents: sorted,
            active_index: idx,
            height_fraction: frac,
            viewport_height: vp_h,
            open: true,
            dismissible: true,
            flick_velocity_threshold: 600.0,
            dragging: false,
        }
    }

    pub fn with_dismissible(mut self, dismissible: bool) -> Self {
        self.dismissible = dismissible;
        self
    }

    pub fn with_flick_threshold(mut self, px_per_sec: f32) -> Self {
        if px_per_sec.is_finite() && px_per_sec > 0.0 {
            self.flick_velocity_threshold = px_per_sec;
        }
        self
    }

    pub fn with_open(mut self, open: bool) -> Self {
        self.open = open;
        if !open {
            self.dragging = false;
        }
        self
    }

    #[inline]
    pub fn is_open(&self) -> bool {
        self.open
    }

    #[inline]
    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    #[inline]
    pub fn detents(&self) -> &[SheetDetent] {
        &self.detents
    }

    #[inline]
    pub fn active_index(&self) -> usize {
        self.active_index
    }

    #[inline]
    pub fn active_detent(&self) -> SheetDetent {
        self.detents[self.active_index]
    }

    #[inline]
    pub fn height_fraction(&self) -> f32 {
        self.height_fraction
    }

    #[inline]
    pub fn min_fraction(&self) -> f32 {
        self.detents[0].fraction()
    }

    #[inline]
    pub fn max_fraction(&self) -> f32 {
        self.detents[self.detents.len() - 1].fraction()
    }

    #[inline]
    pub fn viewport_height(&self) -> f32 {
        self.viewport_height
    }

    pub fn set_viewport_height(&mut self, viewport_height: f32) {
        if viewport_height.is_finite() && viewport_height > 0.0 {
            self.viewport_height = viewport_height;
        }
    }

    #[inline]
    pub fn sheet_height_px(&self) -> f32 {
        if self.open {
            self.viewport_height * self.height_fraction
        } else {
            0.0
        }
    }

    pub fn open_at(&mut self, index: usize) {
        let idx = index.min(self.detents.len() - 1);
        self.active_index = idx;
        self.height_fraction = self.detents[idx].fraction();
        self.open = true;
        self.dragging = false;
    }

    pub fn dismiss(&mut self) -> bool {
        if !self.dismissible || !self.open {
            return false;
        }
        self.open = false;
        self.dragging = false;
        true
    }

    /// Processes an incremental vertical drag delta (`+dy` = downward, `-dy` = upward) with
    /// seamless handoff between sheet resizing and inner content scrolling:
    /// - Downward drag when `inner_scroll_top <= 0.0` drags the sheet down.
    /// - Downward drag when `inner_scroll_top > 0.0` scrolls inner content toward the top first;
    ///   any remainder past `0.0` seamlessly drags the sheet.
    /// - Upward drag when the sheet is already at its maximum detent hands off completely to inner
    ///   scroll; otherwise expands the sheet up to `max_fraction` and passes leftover delta to
    ///   inner scroll.
    pub fn handle_drag_delta(&mut self, dy: f32, inner_scroll_top: f32) -> DragHandoff {
        if !self.open || !dy.is_finite() || dy.abs() <= EPS {
            return DragHandoff::InnerScroll { delta_y: 0.0 };
        }
        let scroll_top = if inner_scroll_top.is_finite() {
            inner_scroll_top.max(0.0)
        } else {
            0.0
        };
        let max_f = self.max_fraction();
        let min_drag_f = if self.dismissible {
            0.0
        } else {
            self.min_fraction() * 0.5
        };

        if dy > 0.0 {
            // Downward drag: scroll inner content back to top before dragging sheet down.
            if scroll_top >= dy {
                return DragHandoff::InnerScroll { delta_y: dy };
            }
            let sheet_dy = dy - scroll_top;
            self.dragging = true;
            self.height_fraction =
                (self.height_fraction - sheet_dy / self.viewport_height).clamp(min_drag_f, max_f);
            DragHandoff::SheetMoved {
                height_fraction: self.height_fraction,
                consumed_dy: sheet_dy,
                remaining_scroll_dy: scroll_top,
            }
        } else {
            // Upward drag (`dy < 0.0`): expand sheet until full, then hand off to inner scroll.
            if self.height_fraction >= max_f - EPS {
                return DragHandoff::InnerScroll { delta_y: dy };
            }
            self.dragging = true;
            let needed_px = (max_f - self.height_fraction) * self.viewport_height;
            let upward_px = -dy;
            if upward_px <= needed_px {
                self.height_fraction =
                    (self.height_fraction + upward_px / self.viewport_height).clamp(min_drag_f, max_f);
                DragHandoff::SheetMoved {
                    height_fraction: self.height_fraction,
                    consumed_dy: dy,
                    remaining_scroll_dy: 0.0,
                }
            } else {
                self.height_fraction = max_f;
                DragHandoff::SheetMoved {
                    height_fraction: max_f,
                    consumed_dy: -needed_px,
                    remaining_scroll_dy: dy + needed_px,
                }
            }
        }
    }

    /// Resolves the final detent or dismissal when a drag finishes.
    ///
    /// `velocity_px_per_sec`: positive = downward flick, negative = upward flick.
    pub fn end_drag(&mut self, velocity_px_per_sec: f32) -> SheetSnapOutcome {
        self.dragging = false;
        let v = if velocity_px_per_sec.is_finite() {
            velocity_px_per_sec
        } else {
            0.0
        };

        if v >= self.flick_velocity_threshold {
            // Fast downward flick: step to the next lower detent or dismiss.
            let lower = self
                .detents
                .iter()
                .rposition(|d| d.fraction() < self.height_fraction - EPS);
            return match lower {
                Some(idx) if !(self.dismissible && v >= self.flick_velocity_threshold * 2.5 && idx == 0 && self.height_fraction <= self.detents[1 % self.detents.len()].fraction()) => {
                    self.snap_to(idx)
                }
                None if self.dismissible => {
                    self.open = false;
                    SheetSnapOutcome::Dismissed
                }
                Some(idx) => self.snap_to(idx),
                None => self.snap_to(0),
            };
        }

        if v <= -self.flick_velocity_threshold {
            // Fast upward flick: step to the next higher detent.
            let higher = self
                .detents
                .iter()
                .position(|d| d.fraction() > self.height_fraction + EPS)
                .unwrap_or(self.detents.len() - 1);
            return self.snap_to(higher);
        }

        // Slow release: dismiss if dragged below half of the lowest detent, else snap to nearest.
        if self.dismissible && self.height_fraction < self.min_fraction() * 0.5 {
            self.open = false;
            return SheetSnapOutcome::Dismissed;
        }

        let mut nearest = 0;
        let mut best_dist = (self.detents[0].fraction() - self.height_fraction).abs();
        for (idx, d) in self.detents.iter().enumerate().skip(1) {
            let dist = (d.fraction() - self.height_fraction).abs();
            if dist < best_dist {
                best_dist = dist;
                nearest = idx;
            }
        }
        self.snap_to(nearest)
    }

    fn snap_to(&mut self, index: usize) -> SheetSnapOutcome {
        let idx = index.min(self.detents.len() - 1);
        self.active_index = idx;
        let detent = self.detents[idx];
        let frac = detent.fraction();
        self.height_fraction = frac;
        SheetSnapOutcome::Snapped {
            index: idx,
            detent,
            fraction: frac,
        }
    }

    /// Expands the sheet by one detent step.
    pub fn snap_next(&mut self) -> SheetSnapOutcome {
        let next = (self.active_index + 1).min(self.detents.len() - 1);
        self.snap_to(next)
    }

    /// Collapses the sheet by one detent step, or dismisses if already at the lowest detent and
    /// `dismissible == true`.
    pub fn snap_prev(&mut self) -> SheetSnapOutcome {
        if self.active_index > 0 {
            self.snap_to(self.active_index - 1)
        } else if self.dismissible {
            self.open = false;
            SheetSnapOutcome::Dismissed
        } else {
            self.snap_to(0)
        }
    }

    /// Accessible keyboard handler for the drag handle (`ArrowUp`, `ArrowDown`, `Home`, `End`,
    /// `Escape`).
    pub fn handle_key(&mut self, key: &str) -> Option<SheetSnapOutcome> {
        if !self.open {
            return None;
        }
        match key {
            "ArrowUp" => Some(self.snap_next()),
            "ArrowDown" => Some(self.snap_prev()),
            "Home" => Some(self.snap_to(0)),
            "End" => Some(self.snap_to(self.detents.len() - 1)),
            "Escape" | "Esc" if self.dismissible => {
                self.open = false;
                Some(SheetSnapOutcome::Dismissed)
            }
            _ => None,
        }
    }
}

/// Accessible multi-detent bottom sheet component with backdrop dismissal, keyboard-controllable
/// slider drag handle, and inner scroll tracking.
#[component]
pub fn BottomSheet(
    open: bool,
    #[props(default = vec![SheetDetent::Peek, SheetDetent::Half, SheetDetent::Full])]
    detents: Vec<SheetDetent>,
    #[props(default = 1)] initial_detent: usize,
    #[props(default = 800.0)] viewport_height: f32,
    #[props(default = true)] dismissible: bool,
    #[props(default)] title: Option<String>,
    #[props(default)] on_dismiss: Option<EventHandler<()>>,
    #[props(default)] on_detent_change: Option<EventHandler<SheetDetent>>,
    #[props(default)] test_id: Option<String>,
    #[props(default)] style: Option<String>,
    children: Element,
) -> Element {
    let (theme, colors) = use_theme_rgba();
    let mut handle_focused = use_signal(|| false);
    let mut inner_scroll_top = use_signal(|| 0.0f32);
    let mut state = use_signal(|| {
        BottomSheetState::new(detents.clone(), initial_detent, viewport_height)
            .with_dismissible(dismissible)
            .with_open(open)
    });
    let mut was_open = use_signal(|| open);

    if open && !*was_open.peek() {
        *was_open.write_unchecked() = true;
        state.write_unchecked().open_at(initial_detent);
        *inner_scroll_top.write_unchecked() = 0.0;
    } else if !open && *was_open.peek() {
        *was_open.write_unchecked() = false;
        let mut s = state.write_unchecked();
        s.open = false;
        s.dragging = false;
    }

    if !open {
        return rsx! {};
    }

    let base_id = test_id.as_deref().unwrap_or("nve-bottom-sheet");
    let backdrop_id = format!("{base_id}-backdrop");
    let handle_id = format!("{base_id}-handle");
    let scroll_id = format!("{base_id}-scroll");
    let title_id = title.as_ref().map(|_| format!("{base_id}-title"));

    let (sheet_h, frac, min_pct, max_pct, cur_pct) = {
        let s = state.read();
        (
            s.sheet_height_px(),
            s.height_fraction(),
            (s.min_fraction() * 100.0).round() as i32,
            (s.max_fraction() * 100.0).round() as i32,
            (s.height_fraction() * 100.0).round() as i32,
        )
    };

    let backdrop_style = TypedStyleBuilder::new()
        .width_pct(100.0)
        .height_px(viewport_height)
        .flex_col()
        .justify_content(FlexJustify::FlexEnd)
        .align_items(FlexAlign::Stretch)
        .z_index(1000)
        .surface(TypedSurfaceStyle {
            background: theme.backdrop_scrim_rgba(),
            border_width: 0.0,
            border_color: ColorRgba::TRANSPARENT,
            border_radius: 0.0,
            opacity: 1.0,
            box_shadow: "none",
        })
        .finish();

    let sheet_style = TypedStyleBuilder::new()
        .width_pct(100.0)
        .height_px(sheet_h)
        .flex_col()
        .padding(theme.spacing.lg)
        .gap(theme.spacing.md)
        .z_index(1001)
        .surface(TypedSurfaceStyle {
            background: colors.surface_elevated,
            border_width: if theme.is_high_contrast() { 2.0 } else { 1.0 },
            border_color: colors.border,
            border_radius: theme.radii.lg,
            opacity: 1.0,
            box_shadow: theme.elevation_shadow(),
        })
        .finish();

    let handle_wrap_style = TypedStyleBuilder::new()
        .width_pct(100.0)
        .padding_xy(0.0, theme.spacing.xs)
        .justify_content(FlexJustify::Center)
        .align_items(FlexAlign::Center)
        .finish();

    let handle_bar_style = TypedStyleBuilder::new()
        .width_px(40.0)
        .height_px(4.0)
        .surface(TypedSurfaceStyle {
            background: if handle_focused() {
                colors.border_focus
            } else {
                colors.border
            },
            border_width: 0.0,
            border_color: ColorRgba::TRANSPARENT,
            border_radius: theme.radii.full,
            opacity: 1.0,
            box_shadow: "none",
        })
        .finish();

    let title_style = TypedStyleBuilder::new()
        .text_style(TypedTextStyle {
            color: colors.text_primary,
            font_size: theme.typography.lg,
            font_weight: "700",
            text_align: "left",
            opacity: 1.0,
        })
        .finish();

    let scroll_style = TypedStyleBuilder::new()
        .flex_grow(1.0)
        .width_pct(100.0)
        .overflow("scroll")
        .finish();

    let frac_attr = format!("{frac:.2}");

    rsx! {
        view {
            test_id: "{backdrop_id}",
            accessibility_role: "presentation",
            nve_style: backdrop_style,
            onpress: move |_| {
                if dismissible && state.write().dismiss() {
                    was_open.set(false);
                    if let Some(handler) = on_dismiss {
                        handler.call(());
                    }
                }
            },
            view {
                test_id: "{base_id}",
                accessibility_role: "dialog",
                aria_modal: "true",
                aria_labelledby: title_id.clone(),
                nve_table: super::html_tables::html_data_attrs(vec![("detent-fraction".to_owned(), frac_attr.clone())]),
                nve_style: sheet_style,
                style: style,
                onpress: move |ev: PressEvent| ev.stop_propagation(),
                pressable {
                    test_id: "{handle_id}",
                    accessibility_role: "slider",
                    accessibility_label: "Sheet height",
                    aria_valuenow: "{cur_pct}",
                    aria_valuemin: "{min_pct}",
                    aria_valuemax: "{max_pct}",
                    tab_index: 0,
                    nve_style: handle_wrap_style,
                    onfocus: move |_| handle_focused.set(true),
                    onblur: move |_| handle_focused.set(false),
                    onkeydown: move |ev: KeyEvent| {
                        let outcome = state.write().handle_key(ev.data().key.as_str());
                        match outcome {
                            Some(SheetSnapOutcome::Snapped { detent, .. }) => {
                                if let Some(handler) = on_detent_change {
                                    handler.call(detent);
                                }
                            }
                            Some(SheetSnapOutcome::Dismissed) => {
                                was_open.set(false);
                                if let Some(handler) = on_dismiss {
                                    handler.call(());
                                }
                            }
                            None => {}
                        }
                    },
                    view {
                        nve_style: handle_bar_style,
                    }
                }
                if let Some(ref heading) = title {
                    text {
                        test_id: title_id.clone(),
                        nve_style: title_style,
                        "{heading}"
                    }
                }
                scrollview {
                    test_id: "{scroll_id}",
                    nve_style: scroll_style,
                    onscroll: move |ev: ScrollEvent| {
                        inner_scroll_top.set(ev.data().y.max(0.0));
                    },
                    {children}
                }
            }
        }
    }
}
