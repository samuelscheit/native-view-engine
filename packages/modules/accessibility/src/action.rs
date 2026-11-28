//! Translation from platform [`accesskit::ActionRequest`]s into epoch-validated host actions.

use accesskit::{Action, ActionData, ActionRequest, ScrollUnit, TreeId};
use nve_store::NodeId;

use crate::id::{decode_a11y_id, StoreEpoch};

/// Default logical-pixel step for `ScrollUnit::Item` scroll actions.
pub const DEFAULT_SCROLL_STEP_ITEM: f32 = 40.0;
/// Default logical-pixel step for `ScrollUnit::Page` scroll actions.
pub const DEFAULT_SCROLL_STEP_PAGE: f32 = 240.0;

/// Platform-independent semantic action resolved from an [`accesskit::ActionRequest`].
#[derive(Debug, Clone, PartialEq)]
pub enum A11yAction {
    Click,
    Focus,
    Blur,
    SetValue(String),
    SetNumericValue(f64),
    ReplaceSelectedText(String),
    /// Selection endpoints expressed as Unicode scalar (`char`) indices within the node's text.
    SetTextSelection {
        anchor_char: usize,
        focus_char: usize,
    },
    ScrollIntoView,
    ScrollBy {
        dx: f32,
        dy: f32,
    },
    SetScrollOffset {
        x: f32,
        y: f32,
    },
    Increment,
    Decrement,
    Expand,
    Collapse,
    ShowContextMenu,
}

/// Epoch-validated target [`NodeId`] paired with its translated [`A11yAction`].
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedA11yAction {
    pub target: NodeId,
    pub action: A11yAction,
}

/// Translates an incoming [`ActionRequest`] against `current_epoch`, rejecting stale epochs,
/// non-root tree IDs, null IDs, non-finite coordinates, and mismatched `SetTextSelection` targets.
pub fn translate_action_request(
    req: &ActionRequest,
    current_epoch: StoreEpoch,
    scroll_step_item: f32,
    scroll_step_page: f32,
) -> Option<ResolvedA11yAction> {
    if req.target_tree != TreeId::ROOT {
        return None;
    }
    let decoded = decode_a11y_id(req.target_node, current_epoch)?;
    let target = decoded.owner();
    let step = |unit: Option<&ActionData>| -> f32 {
        match unit {
            Some(ActionData::ScrollUnit(ScrollUnit::Page)) => scroll_step_page,
            _ => scroll_step_item,
        }
    };

    let action = match req.action {
        Action::Click => A11yAction::Click,
        Action::Focus => A11yAction::Focus,
        Action::Blur => A11yAction::Blur,
        Action::Expand => A11yAction::Expand,
        Action::Collapse => A11yAction::Collapse,
        Action::Increment => A11yAction::Increment,
        Action::Decrement => A11yAction::Decrement,
        Action::ShowContextMenu => A11yAction::ShowContextMenu,
        Action::ScrollIntoView => A11yAction::ScrollIntoView,
        Action::ScrollUp => A11yAction::ScrollBy {
            dx: 0.0,
            dy: -step(req.data.as_ref()),
        },
        Action::ScrollDown => A11yAction::ScrollBy {
            dx: 0.0,
            dy: step(req.data.as_ref()),
        },
        Action::ScrollLeft => A11yAction::ScrollBy {
            dx: -step(req.data.as_ref()),
            dy: 0.0,
        },
        Action::ScrollRight => A11yAction::ScrollBy {
            dx: step(req.data.as_ref()),
            dy: 0.0,
        },
        Action::SetScrollOffset => match req.data.as_ref() {
            Some(ActionData::SetScrollOffset(pt))
                if pt.x.is_finite() && pt.y.is_finite() =>
            {
                A11yAction::SetScrollOffset {
                    x: pt.x as f32,
                    y: pt.y as f32,
                }
            }
            _ => return None,
        },
        Action::SetValue => match req.data.as_ref() {
            Some(ActionData::Value(val)) => A11yAction::SetValue(val.as_ref().to_owned()),
            Some(ActionData::NumericValue(v)) if v.is_finite() => A11yAction::SetNumericValue(*v),
            _ => return None,
        },
        Action::ReplaceSelectedText => match req.data.as_ref() {
            Some(ActionData::Value(val)) => {
                A11yAction::ReplaceSelectedText(val.as_ref().to_owned())
            }
            _ => return None,
        },
        Action::SetTextSelection => match req.data.as_ref() {
            Some(ActionData::SetTextSelection(sel)) => {
                let anchor_id = decode_a11y_id(sel.anchor.node, current_epoch)?;
                let focus_id = decode_a11y_id(sel.focus.node, current_epoch)?;
                let owner = anchor_id.owner();
                if focus_id.owner() != owner {
                    return None;
                }
                // `SetTextSelection` may target either the TextInput container or its TextRun,
                // but selection endpoints MUST belong to the same owner node as `target_node`.
                if owner != target {
                    return None;
                }
                A11yAction::SetTextSelection {
                    anchor_char: sel.anchor.character_index,
                    focus_char: sel.focus.character_index,
                }
            }
            _ => return None,
        },
        Action::CustomAction
        | Action::HideTooltip
        | Action::ShowTooltip
        | Action::ScrollToPoint
        | Action::SetSequentialFocusNavigationStartingPoint => return None,
    };

    Some(ResolvedA11yAction { target, action })
}

/// Converts a UTF-8 byte offset within `text` into a Unicode scalar (`char`) index, clamping
/// safely if `byte_offset` lies past the end or inside a multibyte code point.
#[inline]
pub fn byte_offset_to_char_index(text: &str, byte_offset: usize) -> usize {
    let clamped = byte_offset.min(text.len());
    let mut count = 0usize;
    for (idx, _) in text.char_indices() {
        if idx >= clamped {
            return count;
        }
        count += 1;
    }
    count
}

/// Converts a Unicode scalar (`char`) index within `text` back to a valid UTF-8 byte offset.
#[inline]
pub fn char_index_to_byte_offset(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map_or(text.len(), |(byte_idx, _)| byte_idx)
}
