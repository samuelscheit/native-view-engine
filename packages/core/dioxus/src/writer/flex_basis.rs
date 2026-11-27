//! Single typed source of truth for `flex-basis`: [`LayoutExt::flex_basis`]. Each of the three
//! independent sources (`nve_style` [`TypedStyle`], inline `style`, and standalone `flex` /
//! `flex-basis` attribute) tracks its own slot and resolves through one deterministic precedence
//! (`inline_style` > `standalone_attr` > `typed_style` > `auto`) independent of attribute order.

use nve_abi::{Dimension, NodeId};
use nve_components::layout_ext::LayoutExt;

use super::{mark_ext_dirty, NodeRecord, EXT_LAYOUT_EXT};
use crate::typed_style::TypedStyle;

/// Attribute keys whose value sets the flex basis (mirrors `nve_components::apply_typed_attr`).
#[inline]
pub(super) fn is_flex_basis_attr(name: &str) -> bool {
    matches!(name, "flex" | "flex-basis" | "flexBasis")
}

/// Effective [`LayoutExt`] combining `typed`, `inline_basis`, and `attr_basis` (`None` when all
/// fields are default so plain nodes never carry a `LayoutExt`).
pub(super) fn effective_layout_ext(
    typed: Option<&TypedStyle>,
    inline_basis: Option<Dimension>,
    attr_basis: Option<Dimension>,
) -> Option<LayoutExt> {
    let typed_ext = typed.and_then(|t| t.layout_ext.as_deref());
    let override_basis = inline_basis.or(attr_basis);
    let mut ext = match (typed_ext, override_basis) {
        (None, None) => return None,
        (Some(e), _) => e.clone(),
        (None, Some(_)) => LayoutExt::default(),
    };
    if let Some(basis) = override_basis {
        ext.flex_basis = basis;
    }
    (!ext.is_default()).then_some(ext)
}

/// Recomputes and syncs the node's effective [`LayoutExt`] from its three tracked sources.
pub(super) fn sync_effective_layout_ext(
    dirty_extended: &mut Vec<NodeId>,
    rec: &mut NodeRecord,
    node_id: NodeId,
) {
    let next = match rec.extras() {
        Some(e) => effective_layout_ext(
            e.typed_style.as_ref(),
            e.inline_flex_basis,
            e.attr_flex_basis,
        ),
        None => None,
    };
    if rec.extras().and_then(|e| e.layout_ext.as_ref()) == next.as_ref() {
        return;
    }
    let ext = rec.extras_mut();
    ext.layout_ext = next;
    mark_ext_dirty(dirty_extended, ext, node_id, EXT_LAYOUT_EXT);
}

/// Updates the inline `style` basis slot (`None` when the inline `style` omits `flex`/`flex-basis`
/// or was removed) and recomputes the effective [`LayoutExt`].
pub(super) fn set_inline_flex_basis(
    dirty_extended: &mut Vec<NodeId>,
    rec: &mut NodeRecord,
    node_id: NodeId,
    basis: Option<Dimension>,
) {
    if basis.is_none() && rec.extras.is_none() {
        return;
    }
    rec.extras_mut().inline_flex_basis = basis;
    sync_effective_layout_ext(dirty_extended, rec, node_id);
}

/// Updates the standalone `flex`/`flex-basis` attribute basis slot (`None` on removal) and
/// recomputes the effective [`LayoutExt`].
pub(super) fn set_attr_flex_basis(
    dirty_extended: &mut Vec<NodeId>,
    rec: &mut NodeRecord,
    node_id: NodeId,
    basis: Option<Dimension>,
) {
    if basis.is_none() && rec.extras.is_none() {
        return;
    }
    rec.extras_mut().attr_flex_basis = basis;
    sync_effective_layout_ext(dirty_extended, rec, node_id);
}
