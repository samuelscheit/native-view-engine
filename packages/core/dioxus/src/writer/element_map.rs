//! Dense bidirectional Dioxus `ElementId` ↔ [`NodeId`] bindings.
//!
//! `ElementId`s are dense Dioxus slab indices and `NodeId::index()` is a dense store slot, so both
//! directions are flat tables instead of hash maps (one indexed load per mutation / event). The
//! reverse entry is valid only while the forward slot still points at the exact `NodeId`
//! (generation included), so stale bindings never alias a recycled `ElementId` or node slot.

use dioxus_core::ElementId;
use nve_store::{Idx, IndexVec, NodeId, NodeIndex};

/// Dense index of a Dioxus `ElementId` slab slot.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ElementSlot(usize);

impl Idx for ElementSlot {
    #[inline(always)]
    fn as_usize(self) -> usize {
        self.0
    }

    #[inline(always)]
    fn from_usize(v: usize) -> Self {
        Self(v)
    }
}

impl From<ElementId> for ElementSlot {
    #[inline(always)]
    fn from(el: ElementId) -> Self {
        Self(el.0)
    }
}

/// Reverse entry of a node slot that is not bound (`u32` entries halve the reverse table).
const UNBOUND: u32 = u32::MAX;

/// Dioxus `ElementId`s are dense slab indices of live elements, and every bound element maps to
/// a distinct node of the 24-bit `NodeId` index space, so a valid id stays below this bound. A
/// larger id is a corrupt mutation stream: refusing it caps `el_to_node` at 64 MiB instead of
/// letting one bogus id `resize` the table to gigabytes.
const MAX_ELEMENTS: usize = 1 << NodeId::INDEX_BITS;

#[derive(Debug, Default)]
pub(crate) struct ElementMap {
    /// `NodeId::NULL` = unbound.
    el_to_node: IndexVec<ElementSlot, NodeId>,
    /// `ElementId` slot of each node; valid only when `el_to_node[entry] == node` (module docs).
    node_to_el: IndexVec<NodeIndex, u32>,
}

impl ElementMap {
    pub(crate) fn with_root() -> Self {
        let mut map = Self::default();
        map.bind(ElementId(0), NodeId::ROOT);
        map
    }

    /// Binds `el` ↔ `node`, replacing `el`'s previous node (Dioxus recycles `ElementId`s) and
    /// `node`'s previous element (`assign_node_id` may rebind a node), so no stale forward entry
    /// keeps resolving to `node`.
    #[inline]
    pub(crate) fn bind(&mut self, el: ElementId, node: NodeId) {
        // Refused ids stay unbound, so later mutations naming them resolve to no node and are
        // skipped like any other unknown id.
        let Some(raw) = u32::try_from(el.0).ok().filter(|_| el.0 < MAX_ELEMENTS) else {
            return;
        };
        if let Some(prev) = self.element_of(node)
            && prev != el
        {
            self.el_to_node[ElementSlot::from(prev)] = NodeId::NULL;
        }
        let slot = ElementSlot::from(el);
        if self.el_to_node.len() <= slot.as_usize() {
            self.el_to_node.resize(slot.as_usize() + 1, NodeId::NULL);
        }
        self.el_to_node[slot] = node;
        let idx = node.node_index();
        if self.node_to_el.len() <= idx.as_usize() {
            self.node_to_el.resize(idx.as_usize() + 1, UNBOUND);
        }
        self.node_to_el[idx] = raw;
    }

    #[inline]
    pub(crate) fn node_of(&self, el: ElementId) -> Option<NodeId> {
        self.el_to_node
            .get(ElementSlot::from(el))
            .copied()
            .filter(|&node| node != NodeId::NULL)
    }

    #[inline]
    pub(crate) fn element_of(&self, node: NodeId) -> Option<ElementId> {
        let raw = *self.node_to_el.get(node.node_index())?;
        let el = ElementId(usize::try_from(raw).ok()?);
        (raw != UNBOUND && self.node_of(el) == Some(node)).then_some(el)
    }

    /// Drops `node`'s binding (no-op when unbound or already rebound elsewhere).
    #[inline]
    pub(crate) fn unbind_node(&mut self, node: NodeId) {
        if let Some(el) = self.element_of(node) {
            self.el_to_node[ElementSlot::from(el)] = NodeId::NULL;
            self.node_to_el[node.node_index()] = UNBOUND;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recycled_element_and_node_slots_never_alias() {
        let mut map = ElementMap::with_root();
        assert_eq!(map.node_of(ElementId(0)), Some(NodeId::ROOT));
        assert_eq!(map.element_of(NodeId::ROOT), Some(ElementId(0)));
        assert_eq!(map.node_of(ElementId(7)), None);
        assert_eq!(map.element_of(NodeId::new(9, 1)), None);

        let a = NodeId::new(5, 1);
        map.bind(ElementId(3), a);
        assert_eq!(
            (map.node_of(ElementId(3)), map.element_of(a)),
            (Some(a), Some(ElementId(3)))
        );

        // Same node slot, next generation: the old binding must not resolve for it.
        let a2 = NodeId::new(5, 2);
        assert_eq!(map.element_of(a2), None);

        // Dioxus recycles ElementId(3) for another node: `a` loses its reverse binding.
        let b = NodeId::new(6, 1);
        map.bind(ElementId(3), b);
        assert_eq!(map.element_of(a), None);
        assert_eq!(map.element_of(b), Some(ElementId(3)));

        // Unbinding a node that was rebound elsewhere leaves the live binding intact.
        map.unbind_node(a);
        assert_eq!(map.node_of(ElementId(3)), Some(b));
        map.unbind_node(b);
        assert_eq!((map.node_of(ElementId(3)), map.element_of(b)), (None, None));
        map.unbind_node(b);
        assert_eq!(map.element_of(NodeId::ROOT), Some(ElementId(0)));
    }

    #[test]
    fn rebinding_a_node_to_another_element_drops_its_old_forward_entry() {
        let mut map = ElementMap::with_root();
        let node = NodeId::new(4, 1);
        map.bind(ElementId(2), node);
        // `assign_node_id` moves the node to a new element.
        map.bind(ElementId(9), node);
        assert_eq!(map.node_of(ElementId(2)), None, "stale forward alias");
        assert_eq!(map.node_of(ElementId(9)), Some(node));
        assert_eq!(map.element_of(node), Some(ElementId(9)));
        // Rebinding to the same element is idempotent.
        map.bind(ElementId(9), node);
        assert_eq!(map.element_of(node), Some(ElementId(9)));
        map.unbind_node(node);
        assert_eq!(
            (map.node_of(ElementId(2)), map.node_of(ElementId(9))),
            (None, None)
        );
    }

    #[test]
    fn out_of_range_element_ids_are_refused_without_growing_the_table() {
        let mut map = ElementMap::with_root();
        let node = NodeId::new(4, 1);
        for el in [
            ElementId(MAX_ELEMENTS),
            ElementId(u32::MAX as usize - 1),
            ElementId(u32::MAX as usize),
            ElementId(usize::MAX),
        ] {
            map.bind(el, node);
            assert_eq!(map.element_of(node), None);
            assert_eq!(map.node_of(el), None);
            assert_eq!(
                map.el_to_node.len(),
                1,
                "a refused id must not grow the table"
            );
        }
        // The largest valid id still binds.
        map.bind(ElementId(MAX_ELEMENTS - 1), node);
        assert_eq!(map.element_of(node), Some(ElementId(MAX_ELEMENTS - 1)));
    }
}
