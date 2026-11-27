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

/// Reverse entry of a node slot that has never been bound.
const UNBOUND: ElementId = ElementId(usize::MAX);

#[derive(Debug, Default)]
pub(crate) struct ElementMap {
    /// `NodeId::NULL` = unbound.
    el_to_node: IndexVec<ElementSlot, NodeId>,
    /// Valid only when `el_to_node[entry] == node` (see module docs).
    node_to_el: IndexVec<NodeIndex, ElementId>,
}

impl ElementMap {
    pub(crate) fn with_root() -> Self {
        let mut map = Self::default();
        map.bind(ElementId(0), NodeId::ROOT);
        map
    }

    /// Binds `el` ↔ `node`, replacing `el`'s previous node (Dioxus recycles `ElementId`s).
    #[inline]
    pub(crate) fn bind(&mut self, el: ElementId, node: NodeId) {
        let slot = ElementSlot::from(el);
        if self.el_to_node.len() <= slot.as_usize() {
            self.el_to_node.resize(slot.as_usize() + 1, NodeId::NULL);
        }
        self.el_to_node[slot] = node;
        let idx = node.node_index();
        if self.node_to_el.len() <= idx.as_usize() {
            self.node_to_el.resize(idx.as_usize() + 1, UNBOUND);
        }
        self.node_to_el[idx] = el;
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
        let el = *self.node_to_el.get(node.node_index())?;
        (self.node_of(el) == Some(node)).then_some(el)
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
}
