//! Hostile `NVE2` resource ingress: first-patch static-table kind checks on fresh replicas and
//! per-edge cycle-check budgeting over dense resource DAGs.

use crate::wire::{Op, WireDecoder, WireEncoder, WireError, MAGIC, VERSION};
use crate::{
    ChangeSet, FlatResource, NodeId, NodeStore, NveTable, Res, ResourceId, ValidatedWords,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct WireBlob(u32);

impl FlatResource for WireBlob {
    const KIND: nve_abi::Key64 = nve_abi::Key64::of("test.wire_res.blob");
    type View<'a> = u32;

    fn write_words(&self, out: &mut Vec<u32>) -> bool {
        out.push(self.0);
        true
    }
    fn validate(words: &[u32]) -> bool {
        words.len() == 1
    }
    fn view(words: ValidatedWords<'_, Self>) -> u32 {
        words[0]
    }
}

const OTHER_KIND: nve_abi::Key64 = nve_abi::Key64::of("test.wire_res.other");

#[allow(dead_code)]
#[derive(crate::NveTable)]
#[nve_table(name = "test.wire_res.holder", storage = "sparse", flags = "publish")]
struct BlobHolder {
    #[effect(paint)]
    blob: Res<WireBlob>,
}

fn replace_all(bytes: &mut [u8], from: &[u8], to: &[u8]) -> usize {
    let mut hits = 0;
    let mut i = 0;
    while i + from.len() <= bytes.len() {
        if &bytes[i..i + from.len()] == from {
            bytes[i..i + from.len()].copy_from_slice(to);
            hits += 1;
            i += from.len();
        } else {
            i += 1;
        }
    }
    hits
}

#[test]
fn fresh_replica_first_patch_rejects_wrong_kind_resource_on_static_table() {
    let mut src = NodeStore::new();
    let mut tx = src.tx();
    let n = tx.create();
    tx.set_children(NodeId::ROOT, &[n]);
    tx.edit::<BlobHolder>(n).blob(Some(WireBlob(7)));
    let mut changes = ChangeSet::default();
    tx.commit_into(&mut changes);
    let mut frame = Vec::new();
    WireEncoder::default().encode(&src, &changes, &mut frame);

    // Control: the untampered frame applies on a fresh replica.
    let mut ok = NodeStore::replica();
    WireDecoder::default()
        .apply(&mut ok.tx(), &frame)
        .expect("untampered frame applies");

    // Relabel the defined resource's kind: the first `Op::Patch` on the (not yet allocated)
    // static `BlobHolder` column must still enforce `Res<WireBlob>`.
    let mut kind_bytes = WireBlob::KIND.lo().to_le_bytes().to_vec();
    kind_bytes.extend_from_slice(&WireBlob::KIND.hi().to_le_bytes());
    let mut other = OTHER_KIND.lo().to_le_bytes().to_vec();
    other.extend_from_slice(&OTHER_KIND.hi().to_le_bytes());
    assert_eq!(replace_all(&mut frame, &kind_bytes, &other), 1);

    let mut replica = NodeStore::replica();
    assert!(replica.table_meta(BlobHolder::slot()).is_none(), "column must start unallocated");
    match WireDecoder::default().apply(&mut replica.tx(), &frame) {
        Err(WireError::Resource { kind, .. }) => assert_eq!(kind, WireBlob::KIND.0),
        other => panic!("expected WireError::Resource, got {other:?}"),
    }
}

fn define_frame(defs: &[(ResourceId, &[ResourceId])]) -> Vec<u8> {
    let mut body = Vec::new();
    for (id, kids) in defs {
        body.extend_from_slice(&id.0.to_le_bytes());
        body.extend_from_slice(&WireBlob::KIND.lo().to_le_bytes());
        body.extend_from_slice(&WireBlob::KIND.hi().to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes()); // flags
        body.extend_from_slice(&(kids.len() as u32).to_le_bytes());
        for k in *kids {
            body.extend_from_slice(&k.0.to_le_bytes());
        }
        body.extend_from_slice(&4u32.to_le_bytes()); // byte_len
        body.extend_from_slice(&1u32.to_le_bytes()); // payload
    }
    let mut frame = Vec::new();
    frame.extend_from_slice(&MAGIC.to_le_bytes());
    frame.extend_from_slice(&(VERSION as u32).to_le_bytes());
    frame.extend_from_slice(&1u64.to_le_bytes());
    frame.extend_from_slice(&1u32.to_le_bytes());
    frame.extend_from_slice(&((6 + 2 + body.len() / 4) as u32).to_le_bytes());
    let head = (Op::DefineResources as u32) | ((defs.len() as u32) << 16);
    frame.extend_from_slice(&head.to_le_bytes());
    frame.extend_from_slice(&((body.len() / 4) as u32).to_le_bytes());
    frame.extend_from_slice(&body);
    frame
}

/// Builds `k` leaves `B`, `k` mids `A` (each `A -> all B`, `k²` edges) and an unrelated live
/// target `T`, returning `(mids, target)`.
fn dense_dag(store: &mut NodeStore, k: u32) -> (Vec<ResourceId>, ResourceId) {
    let w = 1u32.to_le_bytes();
    let leaves: Vec<_> = (1..=k).map(|i| ResourceId::new(i, 0)).collect();
    let mids: Vec<_> = (k + 1..=2 * k).map(|i| ResourceId::new(i, 0)).collect();
    let target = ResourceId::new(2 * k + 1, 0);
    for &b in &leaves {
        assert!(store.resources.define_at(b, WireBlob::KIND, false, &[], &w));
    }
    for &a in &mids {
        assert!(store.resources.define_at(a, WireBlob::KIND, false, &leaves, &w));
    }
    assert!(store.resources.define_at(target, WireBlob::KIND, false, &[], &w));
    (mids, target)
}

#[test]
fn wire_cycle_check_budgets_inspected_edges_over_dense_dag() {
    // 96 + 96² = 9 312 inspected edges exceed the small frame's 8 192 budget even though only
    // 192 distinct nodes are reachable: per-node budgeting would accept this redefinition.
    let mut store = NodeStore::replica();
    let (mids, target) = dense_dag(&mut store, 96);
    let frame = define_frame(&[(target, &mids)]);
    assert!(frame.len() / 4 < 1024, "frame stays on the minimum 8 192-edge budget");
    match WireDecoder::default().apply(&mut store.tx(), &frame) {
        Err(WireError::Resource { id, .. }) => assert_eq!(id, target.0),
        other => panic!("edge budget must reject the redefinition, got {other:?}"),
    }
    assert!(store.resources().children_of(target).is_empty());

    // 64 + 64² = 4 160 edges fit the budget: the acyclic redefinition is accepted.
    let mut store = NodeStore::replica();
    let (mids, target) = dense_dag(&mut store, 64);
    let frame = define_frame(&[(target, &mids)]);
    WireDecoder::default()
        .apply(&mut store.tx(), &frame)
        .expect("acyclic redefinition within budget applies");
    assert_eq!(store.resources().children_of(target), mids.as_slice());
}

#[test]
fn unsupported_wire_version_is_rejected_before_any_op() {
    let mut frame = define_frame(&[(ResourceId::new(1, 0), &[])]);
    frame[4..8].copy_from_slice(&1u32.to_le_bytes());
    let mut replica = NodeStore::replica();
    assert_eq!(
        WireDecoder::default().apply(&mut replica.tx(), &frame),
        Err(WireError::UnsupportedVersion(1))
    );
    assert_eq!(replica.stats().resources_live, 0);
}

#[test]
fn invalid_resource_id_never_attaches_the_table() {
    let mut store = NodeStore::new();
    let mut tx = store.tx();
    let n = tx.create();
    tx.set_children(NodeId::ROOT, &[n]);
    let bogus = ResourceId::new(999, 0);
    assert!(!tx.set_resource_id(n, BlobHolder::BLOB, bogus));
    assert!(!tx.store().has::<BlobHolder>(n), "TxCore must reject before attaching");
    tx.node_mut(n)
        .expect("live node")
        .set_resource_id(BlobHolder::BLOB, bogus);
    assert!(!tx.store().has::<BlobHolder>(n), "NodeMut must reject before attaching");
    // A live id of the right kind still attaches.
    let ok = tx.with_temp_resources(|scope| {
        let id = scope.intern(&WireBlob(3));
        assert!(scope.tx().set_resource_id(n, BlobHolder::BLOB, id));
        id
    });
    assert_eq!(tx.store().row::<BlobHolder>(n).blob(), ok);
}
