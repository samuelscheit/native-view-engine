//! Epoch-mark deduplication and accessibility change-classification helpers for
//! [`crate::AccessKitTreeBridge`].

use nve_abi::table::effect;
use nve_store::{ChangeSet, NodeId, NveTable, TableSlot};
use nve_tables::tables::{
    Accessibility, DebugId, Image, Interaction, Link, Scroll, TextContent, TextInput, Transform,
};

/// O(1) dense epoch mark counter replacing per-frame `HashSet` allocations.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct MarkEpoch(u32);

impl MarkEpoch {
    pub(crate) const ZERO: Self = Self(0);

    #[inline]
    pub(crate) fn bump(&mut self, marks: &mut [MarkEpoch]) -> Self {
        let next = self.0.wrapping_add(1);
        if next == 0 {
            marks.fill(Self::ZERO);
            self.0 = 1;
        } else {
            self.0 = next;
        }
        *self
    }
}

/// Single source of truth for non-structural tables eligible for O(dirty) targeted AccessKit sync.
#[inline]
fn a11y_targeted_slots() -> [TableSlot; 8] {
    [
        Accessibility::slot(),
        TextContent::slot(),
        TextInput::slot(),
        Interaction::slot(),
        Image::slot(),
        Link::slot(),
        Scroll::slot(),
        Transform::slot(),
    ]
}

/// Visits every node whose targeted accessibility table row was mutated or detached in `changes`.
#[inline]
pub(crate) fn for_each_a11y_dirty_node(changes: &ChangeSet, mut f: impl FnMut(NodeId)) {
    for slot in a11y_targeted_slots() {
        if let Some(table) = changes.table(slot) {
            for &id in table.nodes.iter().chain(table.detached.iter()) {
                f(id);
            }
        }
    }
}

/// Returns `true` when `changes` can be applied via O(dirty) targeted node rebuilds without a
/// full reachable-tree diff.
#[inline]
pub(crate) fn can_use_targeted_dirty(changes: &ChangeSet, force_full_diff: bool) -> bool {
    !force_full_diff
        && changes.created.is_empty()
        && changes.removed.is_empty()
        && changes.children.is_empty()
        && changes.resources_defined.is_empty()
        && changes.resources_released.is_empty()
        && (changes.effects_union() & (effect::LAYOUT | effect::MEASURE)) == 0
        && changes.table(DebugId::slot()).is_none()
}

/// Returns `true` when `changes` touches zero accessibility-relevant structure, layout, scroll,
/// transform, text, or semantic tables (e.g. 120 Hz `Visual::opacity`-only `COMPOSITE` frames).
#[inline]
pub(crate) fn is_composite_only_for_a11y(changes: &ChangeSet) -> bool {
    if !changes.created.is_empty()
        || !changes.removed.is_empty()
        || !changes.children.is_empty()
        || !changes.resources_defined.is_empty()
        || !changes.resources_released.is_empty()
    {
        return false;
    }
    let relevant_effects =
        effect::LAYOUT | effect::MEASURE | effect::SEMANTICS | effect::REALIZE | effect::EVENTS;
    if (changes.effects_union() & relevant_effects) != 0 {
        return false;
    }
    if changes.table(DebugId::slot()).is_some() {
        return false;
    }
    a11y_targeted_slots()
        .into_iter()
        .all(|slot| changes.table(slot).is_none())
}
