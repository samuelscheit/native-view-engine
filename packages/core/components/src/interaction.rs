//! First-class, zero-serialization interactive pseudo-state styling (`:hover`, `:focus`,
//! `:focus-visible`, `:active`, `:disabled`), [`CursorKind`], deterministic cascade resolution,
//! and an O(1) stateful [`InteractionTracker`] shared by native backends (Headless / Skia).

use nve_abi::define_bitflags;
use nve_store::{RowPatch, RowPatchView};

use crate::snapshots::{
    apply_patch_view_to_layout, apply_patch_view_to_visual, LayoutSnapshot, VisualSnapshot,
};
use crate::tables::{Interaction, StateRulesView};
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

/// Strictly typed cross-platform pointer cursor icon (`#[repr(u8)]`).
#[repr(u8)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, nve_store::WordEnum)]
pub enum CursorKind {
    #[default]
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

impl CursorKind {
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
/// with a table-generic [`RowPatch`].
#[derive(Debug, Clone, PartialEq)]
pub struct StateStyleRule {
    pub state: InteractionState,
    pub patch: RowPatch,
}

impl StateStyleRule {
    #[inline]
    pub const fn new(state: InteractionState, patch: RowPatch) -> Self {
        Self { state, patch }
    }
}

/// Visits every matching [`RowPatchView`] in `rules` in deterministic cascade order
/// (`Hover < Focus < FocusVisible < Active < Disabled`, preserving declaration order for ties)
/// with zero heap allocations for up to 16 matching rules.
pub fn visit_state_rules_in_cascade<'a>(
    rules: StateRulesView<'a>,
    state: InteractionState,
    mut f: impl FnMut(RowPatchView<'a>),
) {
    if rules.is_empty() || state.is_empty() {
        return;
    }
    let mut stack = [(0u8, RowPatchView::EMPTY); STACK_RULE_CAP];
    let mut count = 0usize;
    let mut overflow: Option<Vec<(u8, RowPatchView<'a>)>> = None;

    for (rule_state, patch) in rules.iter() {
        if patch.is_empty() || !rule_state.matches(state) {
            continue;
        }
        let item = (rule_state.cascade_order(), patch);
        if let Some(ref mut vec) = overflow {
            vec.push(item);
        } else if count < STACK_RULE_CAP {
            stack[count] = item;
            count += 1;
        } else {
            let mut vec = Vec::with_capacity(STACK_RULE_CAP * 2);
            vec.extend_from_slice(&stack);
            vec.push(item);
            overflow = Some(vec);
        }
    }

    if let Some(mut vec) = overflow {
        vec.sort_by_key(|&(ord, _)| ord);
        for (_, patch) in vec {
            f(patch);
        }
    } else {
        let matched = &mut stack[..count];
        for i in 1..matched.len() {
            let cur = matched[i];
            let mut j = i;
            while j > 0 && matched[j - 1].0 > cur.0 {
                matched[j] = matched[j - 1];
                j -= 1;
            }
            matched[j] = cur;
        }
        for &(_, patch) in matched.iter() {
            f(patch);
        }
    }
}

/// Merges all matching rules from a zero-copy [`StateRulesView`] in deterministic cascade order.
pub fn resolve_patch_from_view(rules: StateRulesView<'_>, state: InteractionState) -> RowPatch {
    let mut merged = RowPatch::new();
    visit_state_rules_in_cascade(rules, state, |patch| merged.merge_view(patch));
    merged
}

/// Resolves the effective [`VisualSnapshot`] for `base` after applying every rule matching `state`
/// in deterministic cascade order (zero allocations).
pub fn resolve_style_from_view(
    base: &VisualSnapshot,
    rules: StateRulesView<'_>,
    state: InteractionState,
) -> VisualSnapshot {
    let mut resolved = *base;
    visit_state_rules_in_cascade(rules, state, |patch| {
        apply_patch_view_to_visual(patch, &mut resolved);
    });
    resolved
}

/// Resolves both [`LayoutSnapshot`] and [`VisualSnapshot`] after applying every rule matching
/// `state` in deterministic cascade order (zero allocations).
pub fn resolve_layout_and_style_from_view(
    base_layout: &LayoutSnapshot,
    base_style: &VisualSnapshot,
    rules: StateRulesView<'_>,
    state: InteractionState,
) -> (LayoutSnapshot, VisualSnapshot) {
    let mut layout = *base_layout;
    let mut style = *base_style;
    visit_state_rules_in_cascade(rules, state, |patch| {
        apply_patch_view_to_layout(patch, &mut layout);
        apply_patch_view_to_visual(patch, &mut style);
    });
    (layout, style)
}

/// Resolves the effective [`CursorKind`] for `base` after applying every rule matching `state` in
/// deterministic cascade order (zero allocations).
pub fn resolve_cursor_from_view(
    base: CursorKind,
    rules: StateRulesView<'_>,
    state: InteractionState,
) -> CursorKind {
    let mut resolved = base;
    visit_state_rules_in_cascade(rules, state, |patch| {
        if let Some(c) = patch.get_prop(Interaction::CURSOR) {
            resolved = c;
        }
    });
    resolved
}
