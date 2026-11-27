//! First-class, zero-serialization interactive pseudo-state styling (`:hover`, `:focus`,
//! `:focus-visible`, `:active`, `:disabled`), [`CursorKind`], deterministic cascade resolution,
//! and an O(1) stateful [`InteractionTracker`] shared by native backends (Headless / Skia).

use nve_abi::define_bitflags;
use nve_store::word_enum;

use crate::style_patch::{LayoutSnapshot, StylePatch, VisualSnapshot};
pub use crate::tracker::InteractionTracker;

const STACK_RULE_CAP: usize = 16;

define_bitflags! {
    /// Compact 1-byte interactive pseudo-state bitflags with built-in deterministic cascade priority:
    /// `Hover (bit 0) < Focus (bit 1) < FocusVisible (bit 2) < Active (bit 3) < Disabled (bit 4)`.
    ///
    /// Because bit positions ascend strictly in cascade order, `self.cascade_order()` places every
    /// single state in exact priority order (`Hover < Focus < FocusVisible < Active < Disabled`) and
    /// places any compound selector (e.g. `HOVER | FOCUS_VISIBLE`) strictly above every constituent
    /// subset while remaining below higher-priority tiers (`ACTIVE`, `DISABLED`).
    #[derive(Default)]
    pub struct InteractionState(u8) {
        const HOVER = 1 << 0;
        const FOCUS = 1 << 1;
        const FOCUS_VISIBLE = 1 << 2;
        const ACTIVE = 1 << 3;
        const DISABLED = 1 << 4;
    }
}

impl InteractionState {
    /// Returns `true` when a rule guarded by `self` matches `current`:
    /// requires a non-empty rule mask and `(current & self) == self`.
    #[inline]
    pub const fn matches(self, current: Self) -> bool {
        !self.is_empty() && current.contains(self)
    }

    /// Deterministic cascade sort key (`Hover < Focus < FocusVisible < Active < Disabled`,
    /// with compound combinations strictly after every constituent subset).
    #[inline]
    pub const fn cascade_order(self) -> u8 {
        self.bits()
    }

    /// Returns the 0..=4 cascade tier index (`0=Hover`, `1=Focus`, `2=FocusVisible`,
    /// `3=Active`, `4=Disabled`), or `None` for [`Self::NONE`].
    #[inline]
    pub const fn cascade_tier(self) -> Option<usize> {
        if self.is_empty() {
            None
        } else if self.intersects(Self::DISABLED) {
            Some(4)
        } else if self.intersects(Self::ACTIVE) {
            Some(3)
        } else if self.intersects(Self::FOCUS_VISIBLE) {
            Some(2)
        } else if self.intersects(Self::FOCUS) {
            Some(1)
        } else {
            Some(0)
        }
    }
}

word_enum! {
    /// Strictly typed cross-platform pointer cursor icon (`nve.interaction.cursor`, 4 bits;
    /// mirrored to hosts as `NVE_CURSOR_KIND_*`).
    pub enum CursorKind: 4 {
        Auto = 0,
        Default = 1,
        Pointer = 2,
        Text = 3,
        Grab = 4,
        Grabbing = 5,
        NotAllowed = 6,
        ColResize = 7,
        RowResize = 8,
        Move = 9,
        Crosshair = 10,
        Progress = 11,
        Wait = 12,
    }
}

impl CursorKind {
    /// Decodes a compact `u8` discriminant into [`CursorKind`].
    #[inline]
    pub const fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::Auto),
            1 => Some(Self::Default),
            2 => Some(Self::Pointer),
            3 => Some(Self::Text),
            4 => Some(Self::Grab),
            5 => Some(Self::Grabbing),
            6 => Some(Self::NotAllowed),
            7 => Some(Self::ColResize),
            8 => Some(Self::RowResize),
            9 => Some(Self::Move),
            10 => Some(Self::Crosshair),
            11 => Some(Self::Progress),
            12 => Some(Self::Wait),
            _ => None,
        }
    }

    /// Returns the exact standard CSS `cursor` keyword for this cursor kind.
    #[inline]
    pub const fn to_css_keyword(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Default => "default",
            Self::Pointer => "pointer",
            Self::Text => "text",
            Self::Grab => "grab",
            Self::Grabbing => "grabbing",
            Self::NotAllowed => "not-allowed",
            Self::ColResize => "col-resize",
            Self::RowResize => "row-resize",
            Self::Move => "move",
            Self::Crosshair => "crosshair",
            Self::Progress => "progress",
            Self::Wait => "wait",
        }
    }

    /// Parses a CSS `cursor` keyword into [`CursorKind`] with zero allocations.
    pub fn parse_css(raw: &str) -> Option<Self> {
        let s = raw.trim();
        if s.eq_ignore_ascii_case("auto") {
            Some(Self::Auto)
        } else if s.eq_ignore_ascii_case("default") {
            Some(Self::Default)
        } else if s.eq_ignore_ascii_case("pointer") {
            Some(Self::Pointer)
        } else if s.eq_ignore_ascii_case("text") {
            Some(Self::Text)
        } else if s.eq_ignore_ascii_case("grab") {
            Some(Self::Grab)
        } else if s.eq_ignore_ascii_case("grabbing") {
            Some(Self::Grabbing)
        } else if s.eq_ignore_ascii_case("not-allowed") {
            Some(Self::NotAllowed)
        } else if s.eq_ignore_ascii_case("col-resize") {
            Some(Self::ColResize)
        } else if s.eq_ignore_ascii_case("row-resize") {
            Some(Self::RowResize)
        } else if s.eq_ignore_ascii_case("move") {
            Some(Self::Move)
        } else if s.eq_ignore_ascii_case("crosshair") {
            Some(Self::Crosshair)
        } else if s.eq_ignore_ascii_case("progress") {
            Some(Self::Progress)
        } else if s.eq_ignore_ascii_case("wait") {
            Some(Self::Wait)
        } else {
            None
        }
    }
}

/// Input modality that produced the current focus, driving `:focus` vs `:focus-visible`.
#[repr(u8)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum FocusModality {
    #[default]
    Pointer = 0,
    Keyboard = 1,
}

/// One conditional interactive pseudo-state style rule pairing an [`InteractionState`] bitmask
/// with a sparse [`StylePatch`].
#[derive(Debug, Clone, PartialEq)]
pub struct StateStyleRule {
    pub state: InteractionState,
    pub patch: StylePatch,
}

impl StateStyleRule {
    #[inline]
    pub const fn new(state: InteractionState, patch: StylePatch) -> Self {
        Self { state, patch }
    }
}

/// Merges all matching [`StateStyleRule`]s for `state` into a single effective [`StylePatch`] in
/// deterministic cascade order (`Hover < Focus < FocusVisible < Active < Disabled`, preserving
/// declaration order for ties). Uses a zero-allocation stack path for up to 16 rules.
pub fn resolve_patch(rules: &[StateStyleRule], state: InteractionState) -> StylePatch {
    if rules.is_empty() || state.is_empty() {
        return StylePatch::new();
    }

    let mut merged = StylePatch::new();
    if rules.len() <= STACK_RULE_CAP {
        let mut indices = [0usize; STACK_RULE_CAP];
        let mut count = 0usize;
        for (idx, rule) in rules.iter().enumerate() {
            if !rule.patch.is_empty() && rule.state.matches(state) {
                indices[count] = idx;
                count += 1;
            }
        }
        if count == 0 {
            return merged;
        }
        let matched = &mut indices[..count];
        // Insertion sort is stable, O(k^2) with k <= 16, and zero-allocation.
        for i in 1..matched.len() {
            let cur = matched[i];
            let cur_key = rules[cur].state.cascade_order();
            let mut j = i;
            while j > 0 && rules[matched[j - 1]].state.cascade_order() > cur_key {
                matched[j] = matched[j - 1];
                j -= 1;
            }
            matched[j] = cur;
        }
        for &idx in matched.iter() {
            merged.merge(&rules[idx].patch);
        }
    } else {
        let mut matched: Vec<usize> = rules
            .iter()
            .enumerate()
            .filter_map(|(idx, rule)| {
                (!rule.patch.is_empty() && rule.state.matches(state)).then_some(idx)
            })
            .collect();
        matched.sort_by_key(|&idx| rules[idx].state.cascade_order());
        for idx in matched {
            merged.merge(&rules[idx].patch);
        }
    }
    merged
}

/// Resolves the effective [`VisualSnapshot`] for `base` after applying all matching `rules` for
/// `state` in deterministic cascade order.
pub fn resolve_style(
    base: &VisualSnapshot,
    rules: &[StateStyleRule],
    state: InteractionState,
) -> VisualSnapshot {
    let patch = resolve_patch(rules, state);
    if patch.is_empty() {
        return *base;
    }
    let mut resolved = *base;
    patch.apply_style_to(&mut resolved);
    resolved
}

/// Resolves both [`LayoutSnapshot`] and [`VisualSnapshot`] after applying all matching `rules` for
/// `state` in deterministic cascade order.
pub fn resolve_layout_and_style(
    base_layout: &LayoutSnapshot,
    base_style: &VisualSnapshot,
    rules: &[StateStyleRule],
    state: InteractionState,
) -> (LayoutSnapshot, VisualSnapshot) {
    let patch = resolve_patch(rules, state);
    if patch.is_empty() {
        return (*base_layout, *base_style);
    }
    let mut layout = *base_layout;
    let mut style = *base_style;
    patch.apply_to(&mut layout, &mut style);
    (layout, style)
}

/// Resolves the effective [`CursorKind`] for `base` after applying all matching `rules` for
/// `state` in deterministic cascade order.
pub fn resolve_cursor(
    base: CursorKind,
    rules: &[StateStyleRule],
    state: InteractionState,
) -> CursorKind {
    resolve_patch(rules, state).cursor.unwrap_or(base)
}
