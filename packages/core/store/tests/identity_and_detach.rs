//! Store identity (`NodeStore::instance_id`) and the change records of still-live children that
//! leave their parent.

use nve_abi::table::effect;
use nve_store::{ChangeSet, NodeId, NodeStore};

fn effects_of(c: &ChangeSet, id: NodeId) -> Option<u8> {
    c.touched.iter().position(|&t| t == id).map(|i| c.effects[i])
}

#[test]
fn every_store_has_a_unique_nonzero_identity_even_at_the_same_address() {
    let mut store = Box::new(NodeStore::new());
    let (addr, first) = (std::ptr::from_ref::<NodeStore>(&store), store.instance_id());
    assert_ne!(first, 0);
    *store = NodeStore::new();
    assert_eq!(std::ptr::from_ref::<NodeStore>(&store), addr, "replaced in place");
    assert_ne!(store.instance_id(), first, "an in-place replacement is another store");
    assert_ne!(NodeStore::replica().instance_id(), NodeStore::replica().instance_id());
}

#[test]
fn children_dropped_by_set_children_or_detach_child_are_touched() {
    let mut store = NodeStore::new();
    let mut tx = store.tx();
    let [a, b, c, d] = [(); 4].map(|_| tx.create());
    tx.set_children(NodeId::ROOT, &[a, b, c, d]);
    tx.commit_into(&mut ChangeSet::default());

    // `b` is dropped from the list, `c` moves inside it, `a` stays.
    let mut changes = ChangeSet::default();
    let mut tx = store.tx();
    tx.set_children(NodeId::ROOT, &[a, c, d]);
    tx.commit_into(&mut changes);
    assert_eq!(store.parent(b), None);
    let fx = effects_of(&changes, b).expect("dropped child is touched");
    assert_eq!(fx & (effect::LAYOUT | effect::PAINT), effect::LAYOUT | effect::PAINT);
    for kept in [a, c, d] {
        assert_eq!(effects_of(&changes, kept), None, "{kept:?} only stayed");
    }

    let mut changes = ChangeSet::default();
    let mut tx = store.tx();
    tx.detach_child(d);
    tx.commit_into(&mut changes);
    assert_eq!(store.parent(d), None);
    assert!(effects_of(&changes, d).is_some(), "detach_child touches the child");
    assert_eq!(store.children(NodeId::ROOT), &[a, c]);
}
