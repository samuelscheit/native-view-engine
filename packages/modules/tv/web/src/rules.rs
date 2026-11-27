//! Pure DOM decision rules of the `wasm32` controller. The DOM glue only gathers facts
//! (attributes, `:disabled`, rects, computed style) and asks these, so every rule is natively
//! testable.

/// Facts about one `[data-nve-id]` element matched by the focusable selector.
#[derive(Debug, Clone, Copy, Default)]
pub struct FocusableFacts<'a> {
    /// Uppercase DOM tag.
    pub tag: &'a str,
    pub has_href: bool,
    /// `<input type>` (empty for other tags).
    pub input_type: &'a str,
    pub tabindex: Option<&'a str>,
    pub contenteditable: Option<&'a str>,
    /// Matches `:disabled` (own `disabled`, or inside `fieldset[disabled]`).
    pub disabled: bool,
    /// Self or an ancestor has `aria-disabled="true"`.
    pub aria_disabled: bool,
    /// Self or an ancestor has `[inert]` / `[hidden]`.
    pub inert_or_hidden: bool,
}

impl FocusableFacts<'_> {
    /// Whether D-Pad navigation may land on the element (layout and `visibility` are checked
    /// separately, only for candidates that pass).
    pub fn is_focusable(&self) -> bool {
        if self.disabled || self.aria_disabled || self.inert_or_hidden {
            return false;
        }
        match parse_tabindex(self.tabindex) {
            Some(index) => index >= 0,
            None => self.natively_focusable() || is_editable(self.contenteditable),
        }
    }

    fn natively_focusable(&self) -> bool {
        match self.tag {
            "A" => self.has_href,
            "BUTTON" | "SELECT" | "TEXTAREA" | "SUMMARY" => true,
            "INPUT" => !self.input_type.eq_ignore_ascii_case("hidden"),
            _ => false,
        }
    }
}

/// `tabindex` as the HTML integer parser reads it (`" -1 "` is `-1`; garbage is absent).
pub fn parse_tabindex(tabindex: Option<&str>) -> Option<i32> {
    tabindex.and_then(|t| t.trim().parse().ok())
}

/// `contenteditable` present with any value but `"false"`.
pub fn is_editable(contenteditable: Option<&str>) -> bool {
    contenteditable.is_some_and(|v| !v.trim().eq_ignore_ascii_case("false"))
}

/// Computed `visibility` that hides an element (it keeps its layout box).
pub fn is_hidden_visibility(visibility: &str) -> bool {
    matches!(visibility, "hidden" | "collapse")
}

/// Facts about a focus-trap candidate: an open `<dialog>` anywhere in the document, or a
/// `[data-nve-focus-trap]` container inside the surface.
#[derive(Debug, Clone, Copy, Default)]
pub struct TrapFacts<'a> {
    /// An open `<dialog>`.
    pub open_dialog: bool,
    /// `matches(":modal")`; `None` where the engine lacks `:modal` (Chromium < 105, i.e. most TVs).
    pub modal: Option<bool>,
    pub aria_modal: Option<&'a str>,
    pub focus_trap: Option<&'a str>,
    /// Self or an ancestor has `[inert]` / `[hidden]`.
    pub inert_or_hidden: bool,
    /// Has a non-empty layout box.
    pub rendered: bool,
}

impl TrapFacts<'_> {
    /// A rendered, non-inert explicit trap (`data-nve-focus-trap` other than `"false"`), or a
    /// modal dialog (`:modal`, else `aria-modal="true"`, which NVE writes for modal dialogs).
    pub fn is_active(&self) -> bool {
        if self.inert_or_hidden || !self.rendered {
            return false;
        }
        if self
            .focus_trap
            .is_some_and(|v| !v.trim().eq_ignore_ascii_case("false"))
        {
            return true;
        }
        self.open_dialog && (self.modal == Some(true) || self.aria_modal == Some("true"))
    }
}

/// Facts about the element a Select would activate, resolved inside the active focus scope.
#[derive(Debug, Clone, Copy, Default)]
pub struct ActivationFacts<'a> {
    pub tag: &'a str,
    pub tabindex: Option<&'a str>,
    /// `disabled` / `:disabled`.
    pub disabled: bool,
    /// Self or an ancestor has `[inert]`.
    pub inert: bool,
}

impl ActivationFacts<'_> {
    /// Never synthesize clicks on disabled / inert elements, programmatic-only focus targets
    /// (`tabindex < 0`), or a `<dialog>` itself (a click at `0,0` reads as a backdrop click).
    pub fn is_activatable(&self) -> bool {
        !self.disabled
            && !self.inert
            && !self.tag.eq_ignore_ascii_case("DIALOG")
            && parse_tabindex(self.tabindex).is_none_or(|i| i >= 0)
    }
}

/// Gamepad repeater slots tracked per pad index (`Gamepad.index`).
pub const MAX_GAMEPADS: usize = 64;

/// Repeater slots `0..len` whose pad was absent this frame (`present` has bit `i` for pad `i`):
/// they must forget held buttons, or a reconnecting pad fires an instant repeat.
pub fn absent_pad_slots(present: u64, len: usize) -> impl Iterator<Item = usize> {
    (0..len.min(MAX_GAMEPADS)).filter(move |&slot| present & (1u64 << slot) == 0)
}
