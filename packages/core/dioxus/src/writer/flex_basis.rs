//! Single typed source of truth for `flex-basis`: [`LayoutExt::flex_basis`]. A node's effective
//! [`LayoutExt`] is its `nve_style` [`TypedStyle`] layout extension with the basis overridden by
//! the inline `style` (`flex-basis` or a `flex` shorthand) or a `flex` / `flex-basis` attribute,
//! so Web (`layout_ext_css`) and native (Yoga / Taffy) layout read one value, and removing the
//! inline declaration falls back to the typed basis (or `auto`).

use nve_abi::{Dimension, NodeId};
use nve_components::layout_ext::LayoutExt;

use super::{mark_ext_dirty, NodeExtras, NodeRecord, EXT_LAYOUT_EXT};
use crate::typed_style::TypedStyle;

/// Attribute keys whose value sets the flex basis (mirrors `nve_components::apply_typed_attr`).
#[inline]
pub(super) fn is_flex_basis_attr(name: &str) -> bool {
    matches!(name, "flex" | "flex-basis" | "flexBasis")
}

/// The basis a node falls back to without any inline override: its typed style's, else `auto`.
#[inline]
pub(super) fn typed_flex_basis(ext: Option<&NodeExtras>) -> Dimension {
    ext.and_then(|e| e.typed_style.as_ref())
        .and_then(|t| t.layout_ext.as_deref())
        .map_or(Dimension::Auto, |l| l.flex_basis)
}

/// The effective extension of `typed` with an optional inline basis override (`None` when it is
/// all-default, so plain nodes never carry a `LayoutExt`).
pub(super) fn effective_layout_ext(
    typed: Option<&TypedStyle>,
    inline_basis: Option<Dimension>,
) -> Option<LayoutExt> {
    let typed_ext = typed.and_then(|t| t.layout_ext.as_deref());
    let mut ext = match (typed_ext, inline_basis) {
        (None, None) => return None,
        (Some(e), _) => e.clone(),
        (None, Some(_)) => LayoutExt::default(),
    };
    if let Some(basis) = inline_basis {
        ext.flex_basis = basis;
    }
    (!ext.is_default()).then_some(ext)
}

/// Stores `next` as the node's effective [`LayoutExt`], marking it dirty on change.
pub(super) fn sync_layout_ext(
    dirty_extended: &mut Vec<NodeId>,
    rec: &mut NodeRecord,
    node_id: NodeId,
    next: Option<LayoutExt>,
) {
    if rec.extras().and_then(|e| e.layout_ext.as_ref()) == next.as_ref() {
        return;
    }
    let ext = rec.extras_mut();
    ext.layout_ext = next;
    mark_ext_dirty(dirty_extended, ext, node_id, EXT_LAYOUT_EXT);
}

/// Sets only the effective basis, keeping every other `LayoutExt` field.
pub(super) fn sync_flex_basis(
    dirty_extended: &mut Vec<NodeId>,
    rec: &mut NodeRecord,
    node_id: NodeId,
    basis: Dimension,
) {
    let current = rec.extras().and_then(|e| e.layout_ext.as_ref());
    if current.map_or(Dimension::Auto, |e| e.flex_basis) == basis {
        return;
    }
    let mut next = current.cloned().unwrap_or_default();
    next.flex_basis = basis;
    sync_layout_ext(dirty_extended, rec, node_id, (!next.is_default()).then_some(next));
}
