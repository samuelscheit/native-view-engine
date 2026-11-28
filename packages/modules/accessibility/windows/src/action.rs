//! C-ABI action payload types and translation from [`nve_accessibility::ResolvedA11yAction`].

use nve_accessibility::{A11yAction, ResolvedA11yAction};

/// Discriminant mirroring `NveA11yWinActionKind` in `include/nve_a11y_win.h`.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NveA11yWinActionKind {
    Click = 1,
    Focus = 2,
    Blur = 3,
    SetValue = 4,
    SetNumericValue = 5,
    ReplaceSelectedText = 6,
    SetTextSelection = 7,
    ScrollIntoView = 8,
    ScrollBy = 9,
    SetScrollOffset = 10,
    Increment = 11,
    Decrement = 12,
    Expand = 13,
    Collapse = 14,
    ShowContextMenu = 15,
}

/// C-ABI POD struct mirroring `NveA11yWinAction` in `include/nve_a11y_win.h`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NveA11yWinAction {
    pub node_id: u32,
    pub kind: u32,
    pub x: f32,
    pub y: f32,
    pub numeric_value: f64,
    pub anchor_char: usize,
    pub focus_char: usize,
    pub text_ptr: *const u8,
    pub text_len: usize,
}

impl NveA11yWinAction {
    #[inline]
    const fn blank(node_id: u32, kind: NveA11yWinActionKind) -> Self {
        Self {
            node_id,
            kind: kind as u32,
            x: 0.0,
            y: 0.0,
            numeric_value: 0.0,
            anchor_char: 0,
            focus_char: 0,
            text_ptr: core::ptr::null(),
            text_len: 0,
        }
    }
}

/// Borrows `resolved` into a transient [`NveA11yWinAction`] and passes it to `f` without copying
/// text buffers.
pub fn with_ffi_action<R>(
    resolved: &ResolvedA11yAction,
    f: impl FnOnce(&NveA11yWinAction) -> R,
) -> R {
    let id = resolved.target.raw();
    let payload = match &resolved.action {
        A11yAction::Click => NveA11yWinAction::blank(id, NveA11yWinActionKind::Click),
        A11yAction::Focus => NveA11yWinAction::blank(id, NveA11yWinActionKind::Focus),
        A11yAction::Blur => NveA11yWinAction::blank(id, NveA11yWinActionKind::Blur),
        A11yAction::SetValue(s) => NveA11yWinAction {
            text_ptr: if s.is_empty() {
                core::ptr::null()
            } else {
                s.as_ptr()
            },
            text_len: s.len(),
            ..NveA11yWinAction::blank(id, NveA11yWinActionKind::SetValue)
        },
        A11yAction::SetNumericValue(v) => NveA11yWinAction {
            numeric_value: *v,
            ..NveA11yWinAction::blank(id, NveA11yWinActionKind::SetNumericValue)
        },
        A11yAction::ReplaceSelectedText(s) => NveA11yWinAction {
            text_ptr: if s.is_empty() {
                core::ptr::null()
            } else {
                s.as_ptr()
            },
            text_len: s.len(),
            ..NveA11yWinAction::blank(id, NveA11yWinActionKind::ReplaceSelectedText)
        },
        A11yAction::SetTextSelection {
            anchor_char,
            focus_char,
        } => NveA11yWinAction {
            anchor_char: *anchor_char,
            focus_char: *focus_char,
            ..NveA11yWinAction::blank(id, NveA11yWinActionKind::SetTextSelection)
        },
        A11yAction::ScrollIntoView => {
            NveA11yWinAction::blank(id, NveA11yWinActionKind::ScrollIntoView)
        }
        A11yAction::ScrollBy { dx, dy } => NveA11yWinAction {
            x: *dx,
            y: *dy,
            ..NveA11yWinAction::blank(id, NveA11yWinActionKind::ScrollBy)
        },
        A11yAction::SetScrollOffset { x, y } => NveA11yWinAction {
            x: *x,
            y: *y,
            ..NveA11yWinAction::blank(id, NveA11yWinActionKind::SetScrollOffset)
        },
        A11yAction::Increment => NveA11yWinAction::blank(id, NveA11yWinActionKind::Increment),
        A11yAction::Decrement => NveA11yWinAction::blank(id, NveA11yWinActionKind::Decrement),
        A11yAction::Expand => NveA11yWinAction::blank(id, NveA11yWinActionKind::Expand),
        A11yAction::Collapse => NveA11yWinAction::blank(id, NveA11yWinActionKind::Collapse),
        A11yAction::ShowContextMenu => {
            NveA11yWinAction::blank(id, NveA11yWinActionKind::ShowContextMenu)
        }
    };
    f(&payload)
}
