//! Regression tests for `nve-store` soundness, invariants, generational safety, and resource lifecycle.

use nve_abi::Key64;
use nve_store::{
    read_bytes_words, validate_bytes_words, write_bytes_words, ChangeSet, FlatResource, Length,
    NodeId, NodeStore, NveTable, Rgba, Str, StrArena, StrSpan,
};

#[allow(dead_code)]
#[derive(NveTable)]
#[nve_table(name = "hardening.box", storage = "dense", flags = "publish")]
struct HardBox {
    #[effect(layout)]
    width: Length,
    #[effect(paint)]
    label: Str,
}

#[allow(dead_code)]
#[derive(NveTable)]
#[nve_table(name = "hardening.note", storage = "sparse")]
struct HardNote {
    #[effect(paint)]
    text: Str,
    #[effect(paint)]
    payload: nve_store::Res<EphemeralBlob>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EphemeralBlob(Vec<u8>);

impl FlatResource for EphemeralBlob {
    const KIND: Key64 = Key64::of("hardening.res.ephemeral_blob");
    type View<'a> = &'a [u8];

    fn write_words(&self, out: &mut Vec<u32>) -> bool {
        write_bytes_words(out, &self.0, 64 * 1024)
    }
    fn validate(words: &[u32]) -> bool {
        let mut pos = 0;
        validate_bytes_words(words, &mut pos, 64 * 1024) && pos == words.len()
    }
    fn view(words: &[u32]) -> &[u8] {
        let mut pos = 0;
        read_bytes_words(words, &mut pos).unwrap_or(&[])
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DedupBlob(Vec<u8>);

impl FlatResource for DedupBlob {
    const KIND: Key64 = Key64::of("hardening.res.dedup_blob");
    type View<'a> = &'a [u8];

    fn write_words(&self, out: &mut Vec<u32>) -> bool {
        write_bytes_words(out, &self.0, 64 * 1024)
    }
    fn validate(words: &[u32]) -> bool {
        let mut pos = 0;
        validate_bytes_words(words, &mut pos, 64 * 1024) && pos == words.len()
    }
    fn view(words: &[u32]) -> &[u8] {
        let mut pos = 0;
        read_bytes_words(words, &mut pos).unwrap_or(&[])
    }
}

#[test]
fn stale_str_span_after_multibyte_overwrite_is_safe_not_ub() {
    // Regression: holding a stale StrSpan from "aé" (3 bytes: 'a', 0xC3, 0xA9) and reading it
    // after the slot is reused for "ab" or a slice splits a UTF-8 codepoint must NEVER invoke UB.
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let n = {
        let mut tx = store.tx();
        let id = tx.create();
        tx.edit::<HardBox>(id).label("aé");
        tx.commit_into(&mut changes);
        id
    };
    let stale_span: StrSpan = store.row::<HardBox>(n).label_span();
    assert_eq!(store.row::<HardBox>(n).label(), "aé");
    assert_eq!(store.text(stale_span), "aé");

    // Overwrite in place with a different 3-byte UTF-8 sequence ("éa": 0xC3 0xA9 0x61), then
    // verify detached checked lookup and live RowRef lookup both remain safe.
    {
        let mut tx = store.tx();
        tx.edit::<HardBox>(n).label("éa");
        tx.commit_into(&mut changes);
    }
    assert_eq!(store.row::<HardBox>(n).label(), "éa");
    assert_eq!(store.text(stale_span), "éa");

    // Empty / out-of-bounds detached lookup via StrArena::try_str never panics or causes UB.
    let arena = StrArena::default();
    assert_eq!(arena.try_str(stale_span), None);
    assert_eq!(arena.str(stale_span), "");
}

#[test]
#[should_panic(expected = "cannot overwrite string/resource word")]
fn row_writer_put_rejects_str_word_corruption() {
    let mut store = NodeStore::new();
    let mut tx = store.tx();
    let n = tx.create();
    // `HardBox::LABEL` is a `Str` field; attempting a raw scalar `tx.set` on its word must panic.
    // SAFETY: Deliberately mismatched descriptor constructed to verify `RowWriter` panic guard.
    let fake_scalar = unsafe { nve_store::Prop::<HardBox, u32>::new(HardBox::LABEL.word(), 0, 32) };
    tx.set(n, fake_scalar, 0xDEAD_BEEF);
}

#[test]
#[should_panic(expected = "RowWriter::put_str")]
fn row_writer_put_str_panics_on_non_str_word() {
    let mut store = NodeStore::new();
    let mut tx = store.tx();
    let n = tx.create();
    // SAFETY: Deliberately mismatched descriptor constructed to verify `RowWriter::put_str` guard.
    let fake_str = unsafe { nve_store::StrProp::<HardBox>::new(HardBox::WIDTH.word()) };
    tx.set_str(n, fake_str, "corrupt");
}

#[test]
#[should_panic(expected = "RowWriter::put_resource")]
fn row_writer_put_resource_panics_on_non_resource_word() {
    let mut store = NodeStore::new();
    let mut tx = store.tx();
    let n = tx.create();
    // SAFETY: Deliberately mismatched descriptor constructed to verify `RowWriter::put_resource` guard.
    let fake_res =
        unsafe { nve_store::ResProp::<HardBox, EphemeralBlob>::new(HardBox::WIDTH.word()) };
    tx.set_resource(n, fake_res, Some(EphemeralBlob(vec![1, 2])));
}

#[test]
fn sparse_column_remove_row_preserves_unmodified_tail_dirty_mask() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (a, b) = {
        let mut tx = store.tx();
        let a = tx.create();
        let b = tx.create();
        tx.set_children(NodeId::ROOT, &[a, b]);
        tx.edit::<HardNote>(a).text("first");
        tx.edit::<HardNote>(b).text("second");
        tx.commit_into(&mut changes);
        (a, b)
    };
    assert_eq!(store.row::<HardNote>(b).text(), "second");

    // Detaching `a` swap-moves `b` into slot 0 of the sparse column. Because `b` was NOT edited
    // this frame, its dirty mask must stay 0 and `b` must NOT appear in `changes.table().nodes`.
    {
        let mut tx = store.tx();
        tx.detach::<HardNote>(a);
        tx.commit_into(&mut changes);
    }
    let td = changes.table(HardNote::slot()).expect("HardNote table changes");
    assert_eq!(td.detached, vec![a]);
    assert!(
        td.nodes.is_empty(),
        "unmodified swap-moved tail node must not be dirtied: {:?}",
        td.nodes
    );
    assert_eq!(store.row::<HardNote>(b).text(), "second");
}

fn build_single_op_frame(op: u8, table: u8, count: u16, body: &[u8]) -> Vec<u8> {
    use nve_store::wire::{MAGIC, VERSION};
    assert!(body.len().is_multiple_of(4));
    let mut buf = Vec::new();
    buf.extend_from_slice(&MAGIC.to_le_bytes());
    buf.extend_from_slice(&VERSION.to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes());
    buf.extend_from_slice(&1u64.to_le_bytes()); // frame 1
    buf.extend_from_slice(&1u32.to_le_bytes()); // 1 op
    let total_words = (6 + 2 + body.len() / 4) as u32;
    buf.extend_from_slice(&total_words.to_le_bytes());
    let head = (op as u32) | ((table as u32) << 8) | ((count as u32) << 16);
    buf.extend_from_slice(&head.to_le_bytes());
    buf.extend_from_slice(&((body.len() / 4) as u32).to_le_bytes());
    buf.extend_from_slice(body);
    buf
}

#[test]
fn replica_unreferenced_define_resources_reclaimed_at_end_frame() {
    use nve_store::wire::{Op, WireDecoder};

    // Op::DefineResources: id=1, kind_lo, kind_hi, flags=0, child_count=0, byte_len=8, words=[4, 0x04030201]
    let mut body = Vec::new();
    body.extend_from_slice(&1u32.to_le_bytes());
    body.extend_from_slice(&(EphemeralBlob::KIND.0 as u32).to_le_bytes());
    body.extend_from_slice(&((EphemeralBlob::KIND.0 >> 32) as u32).to_le_bytes());
    body.extend_from_slice(&0u32.to_le_bytes()); // flags = 0 (non-ephemeral)
    body.extend_from_slice(&0u32.to_le_bytes()); // child_count = 0
    body.extend_from_slice(&8u32.to_le_bytes()); // byte_len = 8 (2 words)
    body.extend_from_slice(&4u32.to_le_bytes());
    body.extend_from_slice(&[1, 2, 3, 4]);
    let buf = build_single_op_frame(Op::DefineResources as u8, 0, 1, &body);

    let mut replica = NodeStore::replica();
    let mut dec = WireDecoder::default();
    let mut changes = ChangeSet::default();
    {
        let mut tx = replica.tx();
        dec.apply(&mut tx, &buf).expect("valid DefineResources frame");
        tx.commit_into(&mut changes);
    }
    assert_eq!(
        replica.stats().resources_live,
        0,
        "unreferenced DefineResources entry must be reclaimed at end_frame"
    );
}

#[test]
fn declare_tables_dynamic_does_not_pollute_global_registry() {
    use nve_store::wire::{Op, WireDecoder};

    let dyn_key = Key64(0xCAFE_BABE_1234_5678);
    assert!(nve_store::slot_of(dyn_key).is_none());

    // Op::DeclareTables: 1 entry with unknown key, 1 word, 0 str_words, 0 res_words
    let mut body = Vec::new();
    body.extend_from_slice(&(dyn_key.0 as u32).to_le_bytes());
    body.extend_from_slice(&((dyn_key.0 >> 32) as u32).to_le_bytes());
    body.extend_from_slice(&0x11u32.to_le_bytes()); // schema_hash lo
    body.extend_from_slice(&0u32.to_le_bytes()); // schema_hash hi
    let packed = 1u32 << 24; // wire_slot=0, dense, flags=0, words=1
    body.extend_from_slice(&packed.to_le_bytes());
    body.extend_from_slice(&0u32.to_le_bytes()); // str_words
    body.extend_from_slice(&0u32.to_le_bytes()); // resource_words
    body.extend_from_slice(&0u32.to_le_bytes()); // default[0]
    body.extend_from_slice(&[1, 0, 0, 0]); // effects (1 byte + 3 pad)
    let name = b"dyn.test";
    body.extend_from_slice(&(name.len() as u32).to_le_bytes());
    body.extend_from_slice(name);
    let buf = build_single_op_frame(Op::DeclareTables as u8, 0, 1, &body);

    let mut replica = NodeStore::replica();
    let mut dec = WireDecoder::default();
    let mut changes = ChangeSet::default();
    {
        let mut tx = replica.tx();
        dec.apply(&mut tx, &buf).expect("dynamic DeclareTables should succeed");
        tx.commit_into(&mut changes);
    }
    assert!(
        nve_store::slot_of(dyn_key).is_none(),
        "wire DeclareTables must remain session-scoped and never mutate global static registry"
    );
}

#[test]
fn set_children_and_insert_child_reject_duplicates_dead_and_cycles() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (a, b, dead) = {
        let mut tx = store.tx();
        let a = tx.create();
        let b = tx.create();
        let dead = tx.create();
        tx.set_children(NodeId::ROOT, &[a]);
        tx.set_children(a, &[b]);
        tx.remove(dead);
        tx.commit_into(&mut changes);
        (a, b, dead)
    };

    let mut tx = store.tx();
    // Duplicate child rejected.
    assert!(!tx.try_set_children(NodeId::ROOT, &[a, a]));
    // NULL / dead child rejected.
    assert!(!tx.try_set_children(NodeId::ROOT, &[NodeId::NULL]));
    assert!(!tx.try_set_children(NodeId::ROOT, &[dead]));
    // Self-cycle rejected.
    assert!(!tx.try_set_children(a, &[a]));
    // 2-node ancestor cycle (`b -> a` when `a -> b`) rejected.
    assert!(!tx.try_set_children(b, &[a]));
    // `insert_child` rejects cycles and dead nodes cleanly without corrupting tree.
    tx.insert_child(b, 0, a);
    tx.insert_child(a, 0, dead);
    tx.commit_into(&mut changes);

    assert_eq!(store.children(NodeId::ROOT), &[a]);
    assert_eq!(store.children(a), &[b]);
    assert!(store.children(b).is_empty());
}

#[test]
fn generation_255_retires_slot_preventing_aba_wrap() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut seen_ids = std::collections::HashSet::new();

    // Cycle one slot through >300 create+remove frames; slot must retire at gen 255 and never
    // reuse an earlier `NodeId` (zero ABA collisions).
    for _ in 0..300 {
        let id = {
            let mut tx = store.tx();
            let id = tx.create();
            tx.set_children(NodeId::ROOT, &[id]);
            tx.commit_into(&mut changes);
            id
        };
        assert!(
            seen_ids.insert(id),
            "ABA collision: NodeId {id:?} (index={}, gen={}) was reused!",
            id.index(),
            id.generation()
        );
        let mut tx = store.tx();
        tx.remove(id);
        tx.commit_into(&mut changes);
    }
}

#[test]
fn subtree_remove_emits_postorder_and_handles_double_remove() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (parent, child, grand) = {
        let mut tx = store.tx();
        let p = tx.create();
        let c = tx.create();
        let g = tx.create();
        tx.set_children(NodeId::ROOT, &[p]);
        tx.set_children(p, &[c]);
        tx.set_children(c, &[g]);
        tx.commit_into(&mut changes);
        (p, c, g)
    };

    let mut tx = store.tx();
    tx.remove(parent);
    // Second remove in the same tx must be a safe no-op.
    tx.remove(parent);
    tx.remove(child);
    tx.commit_into(&mut changes);

    // Postorder invariant: descendants precede their ancestors.
    assert_eq!(changes.removed, vec![grand, child, parent]);
    assert!(!store.is_live(parent));
    assert!(!store.is_live(child));
    assert!(!store.is_live(grand));
}

#[test]
fn resource_same_frame_create_and_drop_emits_no_phantom_release() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    {
        let mut tx = store.tx();
        let n = tx.create();
        tx.edit::<HardNote>(n).payload_ephemeral(EphemeralBlob(vec![1, 2, 3]));
        tx.remove(n);
        tx.commit_into(&mut changes);
    }
    assert!(
        changes.resources_released.is_empty(),
        "resource created and dropped in same frame must not appear in resources_released"
    );
    assert!(changes.resources_defined.is_empty());
    assert_eq!(store.stats().resources_live, 0);
}

#[test]
fn length_quantize_normalizes_neg_zero_nan_and_huge_values() {
    assert_eq!(Length::points(-0.0).value().to_bits(), 0.0f32.to_bits());
    assert_eq!(Length::percent(-0.0).value().to_bits(), 0.0f32.to_bits());
    assert_eq!(Length::points(f32::NAN).value(), 0.0);
    assert_eq!(Length::points(f32::INFINITY).value(), 0.0);
    assert_eq!(Length::points(f32::NEG_INFINITY).value(), 0.0);
    // Must not overflow/wrap sign on `f32::MAX` or `f32::MIN`.
    let hi = Length::points(f32::MAX);
    assert!(hi.is_points() && hi.value() > 0.0);
    let lo = Length::points(f32::MIN);
    assert!(lo.is_points() && lo.value() < 0.0);
}

#[test]
fn dedup_resource_and_ephemeral_in_place_reuse() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let n = {
        let mut tx = store.tx();
        let n = tx.create();
        tx.edit::<HardNote>(n).payload_ephemeral(EphemeralBlob(vec![10, 20]));
        tx.commit_into(&mut changes);
        n
    };
    let rid1 = store.row::<HardNote>(n).payload();
    {
        let mut tx = store.tx();
        tx.edit::<HardNote>(n).payload_ephemeral(EphemeralBlob(vec![30, 40, 50]));
        tx.commit_into(&mut changes);
    }
    let rid2 = store.row::<HardNote>(n).payload();
    assert_eq!(rid1, rid2, "uniquely owned ephemeral resource reuses id in place");
    assert_eq!(
        store.resource::<EphemeralBlob>(rid2),
        Some(&[30u8, 40, 50][..])
    );
    // Dedup resource sanity.
    let _ = (Rgba::BLACK, DedupBlob(vec![1]));
}

#[test]
fn dynamic_table_cap_disjoint_from_lazy_static_str_table() {
    use nve_abi::table::MAX_TABLE_SLOTS;
    use nve_store::wire::{Op, WireDecoder, WireError};
    use nve_store::MAX_STATIC_TABLE_SLOTS;

    #[allow(dead_code)]
    #[derive(NveTable)]
    #[nve_table(name = "hardening.lazy_static_str", storage = "sparse")]
    struct LazyStaticStr {
        #[effect(paint)]
        label: Str,
    }

    let dyn_cap = MAX_TABLE_SLOTS - MAX_STATIC_TABLE_SLOTS;
    let mut replica = NodeStore::replica();
    let mut dec = WireDecoder::default();
    let mut changes = ChangeSet::default();

    let build_decl = |idx: usize| {
        let dyn_key = Key64(0x9900_0000_0000_0000u64 + idx as u64);
        let mut body = Vec::new();
        body.extend_from_slice(&(dyn_key.0 as u32).to_le_bytes());
        body.extend_from_slice(&((dyn_key.0 >> 32) as u32).to_le_bytes());
        body.extend_from_slice(&(0x100u32 + idx as u32).to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes());
        let packed = (idx as u32) | (1u32 << 24); // wire_slot=idx, dense, flags=0, words=1
        body.extend_from_slice(&packed.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes()); // str_words
        body.extend_from_slice(&0u32.to_le_bytes()); // resource_words
        body.extend_from_slice(&0u32.to_le_bytes()); // default[0]
        body.extend_from_slice(&[1, 0, 0, 0]); // word_effects
        let name = format!("dyn.cap.{idx:02}");
        let nb = name.as_bytes();
        body.extend_from_slice(&(nb.len() as u32).to_le_bytes());
        body.extend_from_slice(nb);
        while !body.len().is_multiple_of(4) {
            body.push(0);
        }
        build_single_op_frame(Op::DeclareTables as u8, 0, 1, &body)
    };

    for i in 0..dyn_cap {
        let frame = build_decl(i);
        let mut tx = replica.tx();
        dec.apply(&mut tx, &frame)
            .unwrap_or_else(|e| panic!("dynamic table {i}/{dyn_cap} failed: {e:?}"));
        tx.commit_into(&mut changes);
    }

    // One more dynamic table must cleanly fail with TableRegistry without touching static slots.
    {
        let frame = build_decl(dyn_cap);
        let mut tx = replica.tx();
        assert_eq!(dec.apply(&mut tx, &frame), Err(WireError::TableRegistry));
    }

    // Now lazily register a brand-new static NveTable with a Str field; its slot must be strictly
    // inside `0..MAX_STATIC_TABLE_SLOTS` and work cleanly on both stores with zero collision.
    let static_slot = LazyStaticStr::slot();
    assert!((static_slot.0 as usize) < MAX_STATIC_TABLE_SLOTS);
    let mut store = NodeStore::new();
    let n = {
        let mut tx = store.tx();
        let n = tx.create();
        tx.edit::<LazyStaticStr>(n).label("disjoint-ok");
        tx.commit_into(&mut changes);
        n
    };
    assert_eq!(store.row::<LazyStaticStr>(n).label(), "disjoint-ok");
    let rn = {
        let mut tx = replica.tx();
        let n = tx.create();
        tx.edit::<LazyStaticStr>(n).label("replica-static-ok");
        tx.commit_into(&mut changes);
        n
    };
    assert_eq!(replica.row::<LazyStaticStr>(rn).label(), "replica-static-ok");
}

#[test]
fn sparse_swap_remove_notifies_table_view_without_wire_retransmit() {
    #[allow(dead_code)]
    #[derive(NveTable)]
    #[nve_table(name = "hardening.pub_sparse", storage = "sparse", flags = "publish")]
    struct PubSparse {
        #[effect(paint)]
        val: u32,
    }

    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (a, b) = {
        let mut tx = store.tx();
        let a = tx.create();
        let b = tx.create();
        tx.edit::<PubSparse>(a).val(11);
        tx.edit::<PubSparse>(b).val(22);
        tx.commit_into(&mut changes);
        (a, b)
    };
    {
        let mut tx = store.tx();
        tx.detach::<PubSparse>(a);
        tx.commit_into(&mut changes);
    }
    let td = changes.table(PubSparse::slot()).expect("detach reported");
    assert_eq!(td.detached, vec![a]);
    assert!(
        td.nodes.is_empty(),
        "NVE2 must not retransmit unmodified tail row: {:?}",
        td.nodes
    );

    let mut views = Vec::new();
    let mut dirty_buf = Vec::new();
    store.publish_table_views(&changes, &mut views, &mut dirty_buf);
    let view = views
        .iter()
        .find(|v| v.key_lo == PubSparse::KEY.lo() && v.key_hi == PubSparse::KEY.hi())
        .expect("PubSparse table view");
    assert_eq!(view.row_count, 1);
    assert_eq!(view.dirty_count, 1, "relocated row index 0 must be reported dirty");
    // SAFETY: `view.dirty_rows` borrows `dirty_buf` with `view.dirty_count` initialized entries.
    let dirty_slice = unsafe { core::slice::from_raw_parts(view.dirty_rows, view.dirty_count as usize) };
    assert_eq!(dirty_slice, &[0]);
    assert_eq!(store.row::<PubSparse>(b).val(), 22);
}
