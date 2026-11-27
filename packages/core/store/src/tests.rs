use nve_abi::table::effect;

use crate::{
    ChangeSet, Length, NodeId, NodeStore, NveTable, RealizationOrder, Res, Resource, Rgba, Str,
    TableSlot, WordValue,
};

crate::word_enum! {
    pub enum Align: 2 { Start = 0, Center = 1, End = 2 }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Shadows(pub Vec<f32>);

impl Resource for Shadows {
    const KIND: nve_abi::Key64 = nve_abi::Key64::of("test.shadows");
    fn encode(&self, out: &mut Vec<u8>) {
        for v in &self.0 {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    fn decode(bytes: &[u8]) -> Option<Self> {
        Some(Self(
            bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| f32::from_le_bytes(*c))
                .collect(),
        ))
    }
}

#[allow(dead_code)]
#[derive(crate::NveTable)]
#[nve_table(name = "test.visual", storage = "dense", flags = "publish")]
pub struct Visual {
    /// Background color.
    #[effect(paint)]
    pub bg: Rgba,
    #[effect(composite)]
    #[default(1.0)]
    pub opacity: f32,
    #[effect(layout)]
    #[default(Length::AUTO)]
    pub width: Length,
    #[effect(layout)]
    #[bits(2)]
    pub align: Align,
    #[effect(layout)]
    #[bits(1)]
    #[default(true)]
    pub wrap: bool,
    #[effect(semantics)]
    pub label: Str,
    #[effect(paint)]
    pub shadows: Res<Shadows>,
}

#[allow(dead_code)]
#[derive(crate::NveTable)]
#[nve_table(name = "test.text", storage = "sparse")]
pub struct Text {
    #[effect(measure)]
    pub text: Str,
    #[effect(paint)]
    #[default(Rgba::BLACK)]
    pub color: Rgba,
}

fn commit(store: &mut NodeStore) -> ChangeSet {
    let mut c = ChangeSet::default();
    store.tx().commit_into(&mut c);
    c
}

#[test]
fn derive_emits_exact_layout_and_defaults() {
    let d = Visual::DESC;
    assert_eq!(
        d.words(),
        7,
        "bg, opacity, width, packed(align|wrap), label(2), shadows"
    );
    assert_eq!(d.defaults[1], 1.0f32.to_bits());
    assert_eq!(d.defaults[2], Length::AUTO.0);
    assert_eq!(d.defaults[3], 1 << 2, "wrap=true packed after 2-bit align");
    assert_eq!(d.str_words, 1 << 4);
    assert_eq!(d.resource_words, 1 << 6);
    assert_eq!(
        d.word_effects,
        &[
            effect::PAINT,
            effect::COMPOSITE,
            effect::LAYOUT,
            effect::LAYOUT,
            effect::SEMANTICS,
            effect::SEMANTICS,
            effect::PAINT
        ]
    );
    assert_eq!(
        d.flags & nve_abi::table::table_flags::AFFECTS_LAYOUT,
        nve_abi::table::table_flags::AFFECTS_LAYOUT
    );
    assert_eq!(Visual::KEY, nve_abi::Key64::of("test.visual"));
    assert_eq!(Visual::ALIGN.mask, 0b11);
    assert_eq!(Visual::WRAP.mask, 0b100);
    assert_ne!(Visual::DESC.schema_hash, Text::DESC.schema_hash);
    assert_eq!(Visual::slot(), Visual::slot());
    assert_ne!(Visual::slot(), Text::slot());
}

#[test]
fn writes_track_dirty_words_effects_and_skip_noops() {
    let mut s = NodeStore::new();
    let mut tx = s.tx();
    let n = tx.create();
    tx.append_child(NodeId::ROOT, n);
    tx.edit::<Visual>(n)
        .bg(Rgba::rgb(1, 2, 3))
        .align(Align::End);
    let c = {
        let mut c = ChangeSet::default();
        tx.commit_into(&mut c);
        c
    };
    assert_eq!(c.created, vec![NodeId::ROOT, n]);
    assert_eq!(c.children, vec![NodeId::ROOT]);
    let t = c.table(Visual::slot()).unwrap();
    assert_eq!(t.nodes, vec![n]);
    assert_eq!(t.masks, vec![0b111_1111], "attach reports every word");
    let r = s.row::<Visual>(n);
    assert_eq!(
        (r.bg(), r.opacity(), r.align(), r.wrap()),
        (Rgba::rgb(1, 2, 3), 1.0, Align::End, true)
    );
    assert!(r.width().is_auto());

    // Same values: no change at all.
    let mut tx = s.tx();
    tx.edit::<Visual>(n).bg(Rgba::rgb(1, 2, 3)).opacity(1.0);
    tx.set(n, Visual::ALIGN, Align::End);
    let mut c = ChangeSet::default();
    tx.commit_into(&mut c);
    assert!(c.is_empty(), "{c:?}");

    // Composite-only change.
    let mut tx = s.tx();
    tx.set(n, Visual::OPACITY, 0.5);
    tx.set(n, Visual::WRAP, false);
    tx.commit_into(&mut c);
    let t = c.table(Visual::slot()).unwrap();
    assert_eq!(t.masks, vec![0b1010]);
    assert_eq!(c.touched, vec![n]);
    assert_eq!(c.effects, vec![effect::COMPOSITE | effect::LAYOUT]);
    assert!(c.needs_layout());
    assert_eq!(
        s.row::<Visual>(n).align(),
        Align::End,
        "packed neighbour untouched"
    );
}

#[test]
fn strings_compare_bytes_and_recycle_spans() {
    let mut s = NodeStore::new();
    let mut tx = s.tx();
    let a = tx.create();
    tx.edit::<Text>(a).text("abc");
    commit(&mut s);
    let span = s.row::<Text>(a).text_span();
    assert_eq!(s.row::<Text>(a).text(), "abc");
    assert_eq!(s.text(span), "abc");
    let mut tx = s.tx();
    tx.edit::<Text>(a).text("abd");
    let c = {
        let mut c = ChangeSet::default();
        tx.commit_into(&mut c);
        c
    };
    assert_eq!(
        s.row::<Text>(a).text_span(),
        span,
        "same class rewrites in place"
    );
    assert_eq!(
        c.table(Text::slot()).unwrap().masks,
        vec![0b11],
        "in-place content change is still dirty"
    );
    assert_eq!(s.row::<Text>(a).text(), "abd");
    assert_eq!(s.text(span), "abd");
    let mut tx = s.tx();
    tx.edit::<Text>(a).text("abd");
    let mut c = ChangeSet::default();
    tx.commit_into(&mut c);
    assert!(c.is_empty());
    let mut tx = s.tx();
    tx.remove(a);
    tx.commit_into(&mut c);
    assert_eq!(c.removed, vec![a]);
    assert_eq!(s.stats().text_live, 0);
}

#[test]
fn resources_dedup_refcount_and_ephemeral_in_place() {
    let mut s = NodeStore::new();
    let mut tx = s.tx();
    let a = tx.create();
    let b = tx.create();
    tx.edit::<Visual>(a).shadows(Some(Shadows(vec![1.0, 2.0])));
    tx.edit::<Visual>(b).shadows(Some(Shadows(vec![1.0, 2.0])));
    let c = commit_tx(tx);
    let (ra, rb) = (s.row::<Visual>(a).shadows(), s.row::<Visual>(b).shadows());
    assert_eq!(ra, rb, "deduplicated");
    assert_eq!(c.resources_defined, vec![ra]);
    assert_eq!(s.resource::<Shadows>(ra), Some(&Shadows(vec![1.0, 2.0])));

    let mut tx = s.tx();
    tx.edit::<Visual>(a).shadows(None);
    let c = commit_tx(tx);
    assert!(c.resources_released.is_empty(), "b still holds it");
    let mut tx = s.tx();
    tx.remove(b);
    let c = commit_tx(tx);
    assert_eq!(c.resources_released, vec![ra]);
    assert_eq!(s.stats().resources_live, 0);

    let mut tx = s.tx();
    tx.edit::<Visual>(a).shadows_ephemeral(Shadows(vec![1.0]));
    commit_tx(tx);
    let id = s.row::<Visual>(a).shadows();
    let mut tx = s.tx();
    tx.edit::<Visual>(a).shadows_ephemeral(Shadows(vec![2.0]));
    let c = commit_tx(tx);
    assert_eq!(s.row::<Visual>(a).shadows(), id, "rewritten in place");
    assert_eq!(c.resources_defined, vec![id]);
    assert_eq!(c.table(Visual::slot()).unwrap().masks, vec![1 << 6]);
    assert_eq!(s.resource::<Shadows>(id), Some(&Shadows(vec![2.0])));
}

fn commit_tx(tx: crate::Transaction<'_>) -> ChangeSet {
    let mut c = ChangeSet::default();
    tx.commit_into(&mut c);
    c
}

#[test]
fn removal_is_recursive_deferred_and_generational() {
    let mut s = NodeStore::new();
    let mut tx = s.tx();
    let p = tx.create();
    let c1 = tx.create();
    let c2 = tx.create();
    tx.set_children(NodeId::ROOT, &[p]);
    tx.set_children(p, &[c1, c2]);
    tx.edit::<Text>(c2).text("x");
    commit_tx(tx);
    assert_eq!(s.parent(c1), Some(p));

    let mut tx = s.tx();
    tx.remove(p);
    let fresh = tx.create();
    assert_ne!(fresh.index(), p.index(), "no intra-frame index reuse");
    let c = commit_tx(tx);
    let mut removed = c.removed.clone();
    removed.sort();
    assert_eq!(removed, vec![p, c1, c2]);
    assert_eq!(c.children, vec![NodeId::ROOT]);
    assert!(s.children(NodeId::ROOT).is_empty());
    assert!(!s.is_live(p));

    let mut tx = s.tx();
    let reused = tx.create();
    commit_tx(tx);
    assert!(
        reused.index() == p.index() || reused.index() == c1.index() || reused.index() == c2.index()
    );
    assert_ne!(reused.generation(), 0);
    assert!(!s.is_live(p) || reused == p);
    assert_eq!(s.stats().text_live, 0);
    assert_eq!(s.stats().children_live, 0);
}

#[test]
fn create_and_remove_in_one_frame_is_invisible() {
    let mut s = NodeStore::new();
    commit(&mut s);
    let mut tx = s.tx();
    let n = tx.create();
    tx.edit::<Visual>(n).opacity(0.3);
    tx.remove(n);
    let c = commit_tx(tx);
    assert!(c.is_empty(), "{c:?}");
}

#[test]
fn detach_reports_and_children_move_between_parents() {
    let mut s = NodeStore::new();
    let mut tx = s.tx();
    let (a, b, x) = (tx.create(), tx.create(), tx.create());
    tx.set_children(NodeId::ROOT, &[a, b]);
    tx.append_child(a, x);
    tx.edit::<Text>(x).text("t");
    commit_tx(tx);
    let mut tx = s.tx();
    tx.append_child(b, x);
    tx.detach::<Text>(x);
    let c = commit_tx(tx);
    assert_eq!(s.children(a), &[] as &[NodeId]);
    assert_eq!(s.children(b), &[x]);
    let mut kids = c.children.clone();
    kids.sort();
    assert_eq!(kids, vec![a, b]);
    assert_eq!(c.table(Text::slot()).unwrap().detached, vec![x]);
    assert!(!s.has::<Text>(x));
    assert_eq!(
        s.row::<Text>(x).color(),
        Rgba::BLACK,
        "absent rows read as defaults"
    );
    let mut tx = s.tx();
    tx.insert_child(b, 0, a);
    commit_tx(tx);
    assert_eq!(s.children(b), &[a, x]);
    assert_eq!(s.children(NodeId::ROOT), &[b]);
}

#[test]
fn primary_slot_is_derived_from_realization_order_at_commit() {
    let mut s = NodeStore::new();
    s.set_realization_order(RealizationOrder::new().with::<Text>());
    let mut tx = s.tx();
    let n = tx.create();
    tx.edit::<Visual>(n).opacity(0.5);
    commit_tx(tx);
    assert_eq!(s.primary_slot(n), TableSlot::NONE);
    let mut tx = s.tx();
    tx.edit::<Text>(n).text("hi");
    commit_tx(tx);
    assert_eq!(s.primary_slot(n), Text::slot());
    let mut tx = s.tx();
    tx.detach::<Text>(n);
    commit_tx(tx);
    assert_eq!(s.primary_slot(n), TableSlot::NONE);
}

#[test]
#[should_panic(expected = "RowRef::str_at")]
fn row_ref_str_at_panics_on_non_str_word() {
    let words = [0xDEAD_BEEFu32, 4u32];
    let arena = crate::StrArena::default();
    let row = crate::RowRef::new(&words, &arena, 0);
    let _ = row.str_at(0);
}

#[test]
fn sparse_swap_remove_keeps_rows_consistent() {
    let mut s = NodeStore::new();
    let mut tx = s.tx();
    let ids: Vec<NodeId> = (0..600).map(|_| tx.create()).collect();
    for (i, &n) in ids.iter().enumerate() {
        tx.edit::<Text>(n)
            .text(&format!("n{i}"))
            .color(Rgba(i as u32));
    }
    commit_tx(tx);
    let mut tx = s.tx();
    for &n in ids.iter().step_by(3) {
        tx.detach::<Text>(n);
    }
    commit_tx(tx);
    for (i, &n) in ids.iter().enumerate() {
        if i % 3 == 0 {
            assert!(!s.has::<Text>(n));
        } else {
            let r = s.row::<Text>(n);
            assert_eq!(r.text(), format!("n{i}"));
            assert_eq!(r.color(), Rgba(i as u32));
        }
    }
    let (_, owners, rows) = s.table_storage(Text::slot()).unwrap();
    assert_eq!(rows, 400);
    assert_eq!(owners.len(), 400);
}

#[test]
fn word_values_are_total() {
    assert_eq!(Align::from_bits(3), Align::Start);
    assert!(<bool as WordValue>::from_bits(3));
}

#[test]
fn publish_table_views_and_table_event_bus_roundtrip() {
    let mut store = NodeStore::new();
    let mut tx = store.tx();
    let n = tx.create();
    tx.set_children(NodeId::ROOT, &[n]);
    tx.edit::<Visual>(n).opacity(0.4).bg(Rgba::rgb(1, 2, 3));
    let mut changes = ChangeSet::default();
    tx.commit_into(&mut changes);

    let mut views = Vec::new();
    let mut dirty = Vec::new();
    store.publish_table_views(&changes, &mut views, &mut dirty);
    let view = views
        .iter()
        .find(|v| v.key_lo == Visual::KEY.lo() && v.key_hi == Visual::KEY.hi())
        .expect("published Visual NveTableView");
    assert_eq!(view.words as usize, Visual::DESC.words());
    assert!(view.row_count as usize > n.index());
    assert_eq!(view.dirty_count, 1);

    // Zero-serialization in-process TableEventBus + FFI NveDirectEvent + EventRing round-trip.
    let mut bus = crate::TableEventBus::new();
    bus.emit::<Visual, (f32, f32)>(7, n, 1234, 42, (10.0, 20.0));
    let events: Vec<_> = bus.drain().collect();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].payload::<(f32, f32)>(), Some(&(10.0, 20.0)));

    let native = events[0].to_native_event(|_| b"ok".to_vec());
    let direct = native.as_direct_event();
    let batch = [direct];
    // SAFETY: `batch` and `native` outlive `batch_ref` in this test.
    let batch_ref = unsafe { nve_abi::NveEventBatchRef::from_slice(&batch) };
    let decoded_direct = nve_abi::direct_event_to_native(batch_ref.get(0).unwrap()).unwrap();
    assert_eq!(decoded_direct, native);

    let ring = nve_abi::encode_events(std::slice::from_ref(&native));
    let decoded_ring = nve_abi::decode_events(&ring).unwrap();
    assert_eq!(decoded_ring, vec![native]);
}

#[test]
fn replica_define_at_preserves_in_free_and_dedups_pending_free() {
    crate::register_codec::<Shadows>();
    let mut store = NodeStore::replica();
    let mut encoded = Vec::new();
    Shadows(vec![1.0, 2.0]).encode(&mut encoded);

    // Repeatedly defining + reclaiming the same unreferenced resource id across many frames must
    // preserve `RES_IN_FREE` and never grow `free` unboundedly.
    for _ in 0..20 {
        assert!(
            store
                .resources
                .define_at(crate::ResourceId(1), Shadows::KIND, false, &encoded)
        );
        // Duplicate define_at in the same frame must not push a duplicate into `pending_free`.
        assert!(
            store
                .resources
                .define_at(crate::ResourceId(1), Shadows::KIND, false, &encoded)
        );
        assert_eq!(
            store.resources.pending_free_len(),
            1,
            "duplicate define_at in same frame must not duplicate pending_free"
        );
        store.resources.end_frame();
        assert_eq!(store.stats().resources_live, 0);
        assert!(
            store.resources.free_len() <= 1,
            "free list must not grow on repeated replica recycle: len={}",
            store.resources.free_len()
        );
    }
}

#[test]
fn replica_define_at_retain_not_clobbered_by_intern() {
    crate::register_codec::<Shadows>();
    let mut store = NodeStore::replica();
    let mut enc_a = Vec::new();
    Shadows(vec![1.0, 2.0]).encode(&mut enc_a);
    let mut enc_b = Vec::new();
    Shadows(vec![9.0, 8.0]).encode(&mut enc_b);

    // Frame 1: define unreferenced id=1 -> end_frame moves it onto `free` with `RES_IN_FREE`.
    assert!(
        store
            .resources
            .define_at(crate::ResourceId(1), Shadows::KIND, false, &enc_a)
    );
    store.resources.end_frame();

    // Frame 2: wire redefines id=1 AND retains it (still carries `RES_IN_FREE` until popped).
    assert!(
        store
            .resources
            .define_at(crate::ResourceId(1), Shadows::KIND, false, &enc_a)
    );
    store.resources.retain(crate::ResourceId(1));
    store.resources.end_frame();
    assert_eq!(
        store.resource::<Shadows>(crate::ResourceId(1)),
        Some(&Shadows(vec![1.0, 2.0]))
    );

    // Local `intern` pops id=1 from `free`, must see it is live and skip it instead of clobbering!
    let local_id = store.resources.intern(Shadows(vec![9.0, 8.0]));
    assert_ne!(
        local_id,
        crate::ResourceId(1),
        "live replica resource slot must not be clobbered"
    );
    assert_eq!(
        store.resource::<Shadows>(crate::ResourceId(1)),
        Some(&Shadows(vec![1.0, 2.0]))
    );
    assert_eq!(
        store.resource::<Shadows>(local_id),
        Some(&Shadows(vec![9.0, 8.0]))
    );
    let _ = enc_b;
}

#[test]
fn install_dynamic_table_rejects_static_slot_and_schema_mismatch() {
    use crate::table::{MAX_STATIC_TABLE_SLOTS, OwnedTableDesc};
    use nve_abi::table::{Storage, table_flags};
    use std::sync::Arc;

    let mut store = NodeStore::new();
    let make_desc = |schema_hash: u64| {
        Arc::new(OwnedTableDesc {
            key: nve_hash::Key64(0xDEAD_BEEF_0000_0001),
            name: "dyn.check".into(),
            schema_hash,
            storage: Storage::Sparse,
            flags: table_flags::HOST_SLOT,
            defaults: vec![0u32].into_boxed_slice(),
            word_effects: vec![1u8].into_boxed_slice(),
            str_words: 0,
            resource_words: 0,
        })
    };

    // Static partition slot (< MAX_STATIC_TABLE_SLOTS) must be rejected.
    assert!(
        !store.install_dynamic_table(TableSlot((MAX_STATIC_TABLE_SLOTS - 1) as u8), make_desc(1))
    );
    // Dynamic partition slot succeeds on first install and on exact match, rejects schema mismatch.
    let dyn_slot = TableSlot(MAX_STATIC_TABLE_SLOTS as u8);
    assert!(store.install_dynamic_table(dyn_slot, make_desc(1)));
    assert!(store.install_dynamic_table(dyn_slot, make_desc(1)));
    assert!(!store.install_dynamic_table(dyn_slot, make_desc(2)));
}

#[test]
#[should_panic(expected = "distinct Rust types")]
fn intern_type_mismatch_on_kind_collision_panics() {
    #[derive(Debug, PartialEq)]
    struct CollideA(u32);
    #[derive(Debug, PartialEq)]
    struct CollideB(u32);
    const SHARED_KIND: nve_hash::Key64 = nve_hash::Key64(nve_hash::const_id("test.collide.kind"));
    impl Resource for CollideA {
        const KIND: nve_hash::Key64 = SHARED_KIND;
        fn encode(&self, out: &mut Vec<u8>) {
            out.extend_from_slice(&self.0.to_le_bytes());
        }
        fn decode(b: &[u8]) -> Option<Self> {
            b.try_into().ok().map(|a| Self(u32::from_le_bytes(a)))
        }
    }
    impl Resource for CollideB {
        const KIND: nve_hash::Key64 = SHARED_KIND;
        fn encode(&self, out: &mut Vec<u8>) {
            out.extend_from_slice(&self.0.to_le_bytes());
        }
        fn decode(b: &[u8]) -> Option<Self> {
            b.try_into().ok().map(|a| Self(u32::from_le_bytes(a)))
        }
    }

    let mut store = NodeStore::new();
    let _ = store.resources.intern(CollideA(42));
    // Identical bytes + identical KIND, different Rust type -> must miss dedup and panic in register_codec.
    let _ = store.resources.intern(CollideB(42));
}

#[test]
fn define_at_retain_redefine_release_same_frame_does_not_duplicate_pending_free() {
    crate::register_codec::<Shadows>();
    let mut store = NodeStore::replica();
    let mut enc_a = Vec::new();
    Shadows(vec![1.0, 2.0]).encode(&mut enc_a);
    let mut enc_b = Vec::new();
    Shadows(vec![3.0, 4.0]).encode(&mut enc_b);

    let id = crate::ResourceId(1);
    // Same-frame define_at -> retain -> define_at -> release must preserve RES_PENDING_FREE.
    assert!(store.resources.define_at(id, Shadows::KIND, false, &enc_a));
    assert_eq!(store.resources.pending_free_len(), 1);
    store.resources.retain(id);
    assert!(store.resources.define_at(id, Shadows::KIND, false, &enc_b));
    store.resources.release(id);
    assert_eq!(
        store.resources.pending_free_len(),
        1,
        "redefine while retained must preserve RES_PENDING_FREE so release does not double-push"
    );

    store.resources.end_frame();
    assert_eq!(store.resources.pending_free_len(), 0);
    assert_eq!(store.stats().resources_live, 0);
    assert_eq!(
        store.resources.free_len(),
        1,
        "reclaimed slot must push exactly one free-list entry"
    );
    assert_eq!(store.resource::<Shadows>(id), None);
}

#[test]
fn tables_schema_hash_is_order_and_duplicate_invariant_and_detects_schema_diff() {
    let h_ab = crate::tables_schema_hash(&[&Visual::DESC, &Text::DESC]);
    let h_ba = crate::tables_schema_hash(&[&Text::DESC, &Visual::DESC]);
    let h_dup = crate::tables_schema_hash(&[&Visual::DESC, &Text::DESC, &Visual::DESC]);
    let h_a = crate::tables_schema_hash(&[&Visual::DESC]);
    assert_eq!(h_ab, h_ba);
    assert_eq!(h_ab, h_dup);
    assert_ne!(h_ab, h_a);

    // Same TableKey with a conflicting schema_hash must NOT be deduplicated away.
    static CONFLICT_DESC: nve_abi::table::TableDesc = nve_abi::table::TableDesc {
        schema_hash: Visual::DESC.schema_hash ^ 1,
        ..Visual::DESC
    };
    let h_conflict = crate::tables_schema_hash(&[&Visual::DESC, &CONFLICT_DESC]);
    assert_ne!(h_conflict, h_a);
}

#[test]
fn sparse_column_reserve_bulk_does_not_double_on_repeated_patches() {
    use crate::wire::{WireDecoder, WireEncoder};

    let mut src = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut enc = WireEncoder::default();
    let mut dec = WireDecoder::default();
    let mut replica = NodeStore::replica();

    let mut ids = Vec::new();
    {
        let mut tx = src.tx();
        for _ in 0..64 {
            let n = tx.create();
            tx.edit::<Text>(n).color(Rgba(1));
            ids.push(n);
        }
        tx.commit_into(&mut changes);
    }
    let mut buf = Vec::new();
    enc.encode(&src, &changes, &mut buf);
    {
        let mut tx = replica.tx();
        dec.apply(&mut tx, &buf).expect("apply frame 1");
        tx.commit_into(&mut changes);
    }
    let slot = Text::slot();
    let cap_after_f1 = replica.columns[slot.0 as usize]
        .as_ref()
        .expect("sparse Text column")
        .owners
        .capacity();

    // 10 repeated full-table sparse patches must NOT double owners/rows capacity.
    for round in 2..=11u32 {
        {
            let mut tx = src.tx();
            for &id in &ids {
                tx.edit::<Text>(id).color(Rgba(round));
            }
            tx.commit_into(&mut changes);
        }
        buf.clear();
        enc.encode(&src, &changes, &mut buf);
        let mut tx = replica.tx();
        dec.apply(&mut tx, &buf).expect("apply delta patch");
        tx.commit_into(&mut changes);
    }
    let cap_after_deltas = replica.columns[slot.0 as usize]
        .as_ref()
        .expect("sparse Text column")
        .owners
        .capacity();
    assert_eq!(
        cap_after_deltas, cap_after_f1,
        "repeated sparse patches must not double owners capacity"
    );
}

fn build_wire_frame_num(frame_num: u64, ops: &[(u8, u8, u32, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&crate::wire::MAGIC.to_le_bytes());
    out.extend_from_slice(&crate::wire::VERSION.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&frame_num.to_le_bytes());
    out.extend_from_slice(&(ops.len() as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for &(tag, slot, count, body) in ops {
        let inline = count.min(0xFFFF);
        let head = (tag as u32) | ((slot as u32) << 8) | (inline << 16);
        let extra = usize::from(count >= 0xFFFF) * 4;
        let padded_body = (body.len() + extra).div_ceil(4) * 4;
        out.extend_from_slice(&head.to_le_bytes());
        out.extend_from_slice(&((padded_body / 4) as u32).to_le_bytes());
        if count >= 0xFFFF {
            out.extend_from_slice(&count.to_le_bytes());
        }
        out.extend_from_slice(body);
        while !out.len().is_multiple_of(4) {
            out.push(0);
        }
    }
    let words = (out.len() / 4) as u32;
    out[20..24].copy_from_slice(&words.to_le_bytes());
    out
}

fn build_wire_frame(ops: &[(u8, u8, u32, &[u8])]) -> Vec<u8> {
    build_wire_frame_num(1, ops)
}

fn encode_table_decl(desc: &nve_abi::table::TableDesc, wire_slot: u8) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&desc.key.lo().to_le_bytes());
    b.extend_from_slice(&desc.key.hi().to_le_bytes());
    b.extend_from_slice(&(desc.schema_hash as u32).to_le_bytes());
    b.extend_from_slice(&((desc.schema_hash >> 32) as u32).to_le_bytes());
    let packed = (wire_slot as u32)
        | ((desc.storage as u32) << 8)
        | ((desc.flags as u32) << 16)
        | ((desc.defaults.len() as u32) << 24);
    b.extend_from_slice(&packed.to_le_bytes());
    b.extend_from_slice(&desc.str_words.to_le_bytes());
    b.extend_from_slice(&desc.resource_words.to_le_bytes());
    for &d in desc.defaults {
        b.extend_from_slice(&d.to_le_bytes());
    }
    b.extend_from_slice(desc.word_effects);
    while !b.len().is_multiple_of(4) {
        b.push(0);
    }
    b.extend_from_slice(&(desc.name.len() as u32).to_le_bytes());
    b.extend_from_slice(desc.name.as_bytes());
    while !b.len().is_multiple_of(4) {
        b.push(0);
    }
    b
}

#[test]
fn wire_decoder_rejects_hostile_sparse_node_and_resource_keyframes_before_allocating() {
    use crate::wire::{Op, WireDecoder, WireError};

    // 1. Hostile padded keyframe jumping to NodeId(600_000): 600_000 * 32 B = 19.2 MiB >
    //    (16 MiB + 32 KiB * 16 = 16.5 MiB) -> rejected with WireError::Budget before allocating.
    let pad_body = vec![0u8; 8_192 * 4];
    let create_body = 600_000u32.to_le_bytes();
    let frame = build_wire_frame(&[
        (Op::ReleaseResources as u8, 0, 8_192, &pad_body),
        (Op::CreateNodes as u8, 0, 1, &create_body),
    ]);

    let mut replica = NodeStore::replica();
    let mut dec = WireDecoder::default();
    let mut tx = replica.tx();
    assert_eq!(dec.apply(&mut tx, &frame), Err(WireError::Budget));
    assert_eq!(tx.store().capacity(), 1);

    // 2. Hostile padded keyframe jumping to ResourceId(400_000): 400_000 * ENTRY_BYTES (~56 B) =
    //    ~22.4 MiB > (16 MiB + 64 KiB * 16 = 17 MiB) -> rejected with WireError::Budget before
    //    allocating; ResourceId(0) short-circuits to Resource; ResourceId(1 << 20) hits Budget.
    crate::register_codec::<Shadows>();
    let mut shadow_blob = Vec::new();
    Shadows(vec![1.0]).encode(&mut shadow_blob);
    let build_res_op = |rid: u32| {
        let mut def_res = Vec::new();
        def_res.extend_from_slice(&rid.to_le_bytes());
        def_res.extend_from_slice(&Shadows::KIND.lo().to_le_bytes());
        def_res.extend_from_slice(&Shadows::KIND.hi().to_le_bytes());
        def_res.extend_from_slice(&0u32.to_le_bytes());
        def_res.extend_from_slice(&(shadow_blob.len() as u32).to_le_bytes());
        def_res.extend_from_slice(&shadow_blob);
        def_res
    };
    let res_pad = vec![0u8; 16_384 * 4];
    let def_400k = build_res_op(400_000);
    let res_frame = build_wire_frame(&[
        (Op::ReleaseResources as u8, 0, 16_384, &res_pad),
        (Op::DefineResources as u8, 0, 1, &def_400k),
    ]);
    let mut res_replica = NodeStore::replica();
    let mut res_dec = WireDecoder::default();
    let mut res_tx = res_replica.tx();
    assert_eq!(
        res_dec.apply(&mut res_tx, &res_frame),
        Err(WireError::Budget)
    );
    assert_eq!(res_tx.store().resources().slots(), 1);

    let def_over_cap = build_res_op(1 << 20);
    let cap_frame = build_wire_frame(&[(Op::DefineResources as u8, 0, 1, &def_over_cap)]);
    let mut cap_replica = NodeStore::replica();
    let mut cap_dec = WireDecoder::default();
    let mut cap_tx = cap_replica.tx();
    assert_eq!(
        cap_dec.apply(&mut cap_tx, &cap_frame),
        Err(WireError::Budget)
    );

    let def_zero = build_res_op(0);
    let zero_frame = build_wire_frame(&[(Op::DefineResources as u8, 0, 1, &def_zero)]);
    let mut z_replica = NodeStore::replica();
    let mut z_dec = WireDecoder::default();
    let mut z_tx = z_replica.tx();
    assert_eq!(
        z_dec.apply(&mut z_tx, &zero_frame),
        Err(WireError::Resource { id: 0, kind: 0 })
    );

    // 3. Overflow / un-reservable sparse request surfaces WireError::Alloc, while dense budget
    //    overrun surfaces WireError::Budget cleanly.
    let mut store = NodeStore::new();
    let sparse_slot = Text::slot();
    let mut huge_budget = crate::wire::budget::SessionBudget::with_limit(usize::MAX);
    assert_eq!(
        store.reserve_column_bulk(
            sparse_slot,
            usize::MAX / 2,
            usize::MAX / 2,
            &mut huge_budget
        ),
        Err(WireError::Alloc)
    );
    let dense_slot = Visual::slot();
    let mut tiny_budget = crate::wire::budget::SessionBudget::with_limit(16);
    assert_eq!(
        store.reserve_column_bulk(dense_slot, 1, 1024, &mut tiny_budget),
        Err(WireError::Budget)
    );
}

#[test]
fn wire_dynamic_dense_tables_forced_sparse_and_static_dense_budget_rejects_amplification() {
    use crate::wire::{Op, WireDecoder, WireError};

    // Part A: 32 wire-declared dynamic tables claiming Storage::Dense (32 words each) over 4,096
    // nodes with 1 patched node each must stay Storage::Sparse (row_count == 1, zero dense blow-up).
    let n_nodes: u32 = 4_096;
    let mut create_body = Vec::with_capacity((n_nodes as usize) * 4);
    for id in 1..=n_nodes {
        create_body.extend_from_slice(&id.to_le_bytes());
    }
    let mut decls = Vec::new();
    for i in 0..32u8 {
        let key = nve_hash::Key64(0xCAFE_D000_0000_0000 + u64::from(i));
        let name = format!("hostile.dyn.dense.{i:02}");
        decls.extend_from_slice(&key.lo().to_le_bytes());
        decls.extend_from_slice(&key.hi().to_le_bytes());
        decls.extend_from_slice(&(0x100u32 + u32::from(i)).to_le_bytes());
        decls.extend_from_slice(&0u32.to_le_bytes());
        // wire_slot = i, storage = 0 (Dense!), flags = 0, words = 32
        let packed = u32::from(i) | (32u32 << 24);
        decls.extend_from_slice(&packed.to_le_bytes());
        decls.extend_from_slice(&0u32.to_le_bytes());
        decls.extend_from_slice(&0u32.to_le_bytes());
        decls.extend_from_slice(&[0u8; 32 * 4]);
        decls.extend_from_slice(&[1u8; 32]);
        decls.extend_from_slice(&(name.len() as u32).to_le_bytes());
        decls.extend_from_slice(name.as_bytes());
        while !decls.len().is_multiple_of(4) {
            decls.push(0);
        }
    }
    let mut patch_root = Vec::new();
    patch_root.extend_from_slice(&NodeId::ROOT.raw().to_le_bytes());
    patch_root.extend_from_slice(&0u32.to_le_bytes());

    let mut ops: Vec<(u8, u8, u32, &[u8])> = Vec::with_capacity(34);
    ops.push((Op::DeclareTables as u8, 0, 32, &decls));
    ops.push((Op::CreateNodes as u8, 0, n_nodes, &create_body));
    for i in 0..32u8 {
        ops.push((Op::Patch as u8, i, 1, &patch_root));
    }
    let dyn_frame = build_wire_frame(&ops);

    let mut replica = NodeStore::replica();
    let mut dec = WireDecoder::default();
    let mut changes = ChangeSet::default();
    {
        let mut tx = replica.tx();
        dec.apply(&mut tx, &dyn_frame)
            .expect("dynamic dense tables decode safely as sparse");
        tx.commit_into(&mut changes);
    }
    let dyn_cols: Vec<_> = replica
        .columns
        .iter()
        .skip(crate::table::MAX_STATIC_TABLE_SLOTS)
        .flatten()
        .collect();
    assert_eq!(dyn_cols.len(), 32);
    for col in dyn_cols {
        assert!(
            !col.is_dense(),
            "wire-declared dynamic table must always use sparse storage"
        );
        assert_eq!(
            col.row_count(),
            1,
            "sparse dynamic column must allocate only 1 row, not {n_nodes}"
        );
    }

    // Re-declaring dynamic table 0 with mismatched storage flag must still fail SchemaMismatch.
    let mut mismatch_decl = Vec::new();
    let key0 = nve_hash::Key64(0xCAFE_D000_0000_0000);
    let name0 = "hostile.dyn.dense.00";
    mismatch_decl.extend_from_slice(&key0.lo().to_le_bytes());
    mismatch_decl.extend_from_slice(&key0.hi().to_le_bytes());
    mismatch_decl.extend_from_slice(&0x100u32.to_le_bytes());
    mismatch_decl.extend_from_slice(&0u32.to_le_bytes());
    let packed_sparse = (1u32 << 8) | (32u32 << 24); // Sparse instead of originally declared Dense
    mismatch_decl.extend_from_slice(&packed_sparse.to_le_bytes());
    mismatch_decl.extend_from_slice(&0u32.to_le_bytes());
    mismatch_decl.extend_from_slice(&0u32.to_le_bytes());
    mismatch_decl.extend_from_slice(&[0u8; 32 * 4]);
    mismatch_decl.extend_from_slice(&[1u8; 32]);
    mismatch_decl.extend_from_slice(&(name0.len() as u32).to_le_bytes());
    mismatch_decl.extend_from_slice(name0.as_bytes());
    while !mismatch_decl.len().is_multiple_of(4) {
        mismatch_decl.push(0);
    }
    let mismatch_frame = build_wire_frame(&[(Op::DeclareTables as u8, 0, 1, &mismatch_decl)]);
    {
        let mut tx = replica.tx();
        assert_eq!(
            dec.apply(&mut tx, &mismatch_frame),
            Err(WireError::SchemaMismatch { key: key0.0 })
        );
    }

    // Part B: Static Dense 32-word table amplification exceeds session slot budget and
    // returns WireError::Budget BEFORE allocating the dense column.
    #[allow(dead_code)]
    #[derive(NveTable)]
    #[nve_table(name = "test.wire.dense_budget_32w", storage = "dense")]
    struct BigDense32 {
        #[effect(paint)]
        w00: u32,
        #[effect(paint)]
        w01: u32,
        #[effect(paint)]
        w02: u32,
        #[effect(paint)]
        w03: u32,
        #[effect(paint)]
        w04: u32,
        #[effect(paint)]
        w05: u32,
        #[effect(paint)]
        w06: u32,
        #[effect(paint)]
        w07: u32,
        #[effect(paint)]
        w08: u32,
        #[effect(paint)]
        w09: u32,
        #[effect(paint)]
        w10: u32,
        #[effect(paint)]
        w11: u32,
        #[effect(paint)]
        w12: u32,
        #[effect(paint)]
        w13: u32,
        #[effect(paint)]
        w14: u32,
        #[effect(paint)]
        w15: u32,
        #[effect(paint)]
        w16: u32,
        #[effect(paint)]
        w17: u32,
        #[effect(paint)]
        w18: u32,
        #[effect(paint)]
        w19: u32,
        #[effect(paint)]
        w20: u32,
        #[effect(paint)]
        w21: u32,
        #[effect(paint)]
        w22: u32,
        #[effect(paint)]
        w23: u32,
        #[effect(paint)]
        w24: u32,
        #[effect(paint)]
        w25: u32,
        #[effect(paint)]
        w26: u32,
        #[effect(paint)]
        w27: u32,
        #[effect(paint)]
        w28: u32,
        #[effect(paint)]
        w29: u32,
        #[effect(paint)]
        w30: u32,
        #[effect(paint)]
        w31: u32,
    }
    let big_slot = BigDense32::slot();
    let big_decl = encode_table_decl(&BigDense32::DESC, 0);
    // 250_000 nodes -> ~1.00 MiB wire -> budget ~32.8 MiB; 250_001 * 136 B = ~34.0 MiB > budget.
    let big_n: u32 = 250_000;
    let mut big_create = Vec::with_capacity((big_n as usize) * 4);
    for id in 1..=big_n {
        big_create.extend_from_slice(&id.to_le_bytes());
    }
    let budget_frame = build_wire_frame(&[
        (Op::DeclareTables as u8, 0, 1, &big_decl),
        (Op::CreateNodes as u8, 0, big_n, &big_create),
        (Op::Patch as u8, 0, 1, &patch_root),
    ]);
    let mut budget_replica = NodeStore::replica();
    let mut budget_dec = WireDecoder::default();
    {
        let mut tx = budget_replica.tx();
        assert_eq!(
            budget_dec.apply(&mut tx, &budget_frame),
            Err(WireError::Budget)
        );
    }
    assert_eq!(
        budget_replica.columns[big_slot.0 as usize]
            .as_ref()
            .map_or(0, |c| c.row_count()),
        0,
        "dense column must not be allocated when session slot budget is exceeded"
    );

    // Part C (P1 #2): Relaying a wire-declared dynamic `Storage::Dense` table preserves
    // `Storage::Dense` on the wire via `encode_full` (no SchemaMismatch on a downstream peer that
    // statically registered the table as Dense) while the intermediate relay backs it sparsely.
    #[allow(dead_code)]
    #[derive(NveTable)]
    #[nve_table(name = "test.wire.relay_dense_fidelity", storage = "dense")]
    struct RelayDense {
        #[effect(paint)]
        val: u32,
    }
    assert!(
        crate::slot_of(RelayDense::DESC.key).is_none(),
        "RelayDense must start unregistered so intermediate relay allocates a dynamic slot"
    );
    let relay_decl = encode_table_decl(&RelayDense::DESC, 0);
    let create_root = NodeId::ROOT.raw().to_le_bytes();
    let mut patch_val = Vec::new();
    patch_val.extend_from_slice(&NodeId::ROOT.raw().to_le_bytes());
    patch_val.extend_from_slice(&1u32.to_le_bytes()); // mask: word 0
    patch_val.extend_from_slice(&0xCAFE_1234u32.to_le_bytes());
    let incoming = build_wire_frame(&[
        (Op::DeclareTables as u8, 0, 1, &relay_decl),
        (Op::CreateNodes as u8, 0, 1, &create_root),
        (Op::Patch as u8, 0, 1, &patch_val),
    ]);

    let mut relay_store = NodeStore::replica();
    let mut relay_dec = WireDecoder::default();
    {
        let mut tx = relay_store.tx();
        relay_dec
            .apply(&mut tx, &incoming)
            .expect("intermediate relay decodes dynamic dense table");
        tx.commit_into(&mut changes);
    }
    let relay_col = relay_store
        .columns
        .iter()
        .skip(crate::table::MAX_STATIC_TABLE_SLOTS)
        .flatten()
        .find(|c| c.desc.key() == RelayDense::DESC.key)
        .expect("dynamic RelayDense column");
    assert!(
        !relay_col.is_dense(),
        "intermediate relay must back dynamic Dense table sparsely"
    );
    assert_eq!(relay_col.row_count(), 1);
    assert_eq!(
        relay_col.desc.storage(),
        nve_abi::table::Storage::Dense,
        "TableMeta::storage() must preserve declared wire Storage::Dense for relaying"
    );

    let mut relayed_bytes = Vec::new();
    crate::wire::WireEncoder::default().encode_full(&relay_store, &mut relayed_bytes);

    // Downstream peer compiles & registers RelayDense statically as Storage::Dense.
    let static_relay_slot = RelayDense::slot();
    let mut peer_b = NodeStore::replica();
    let mut peer_b_dec = WireDecoder::default();
    {
        let mut tx = peer_b.tx();
        peer_b_dec
            .apply(&mut tx, &relayed_bytes)
            .expect("downstream peer with static Dense table must accept relayed keyframe");
        tx.commit_into(&mut changes);
    }
    assert_eq!(peer_b.row::<RelayDense>(NodeId::ROOT).val(), 0xCAFE_1234);
    assert!(
        peer_b.columns[static_relay_slot.0 as usize]
            .as_ref()
            .expect("peer_b static column")
            .is_dense()
    );
}

#[test]
fn dense_reserve_bulk_repeated_root_patches_and_empty_patch_do_not_inflate_rows() {
    use crate::wire::{Op, WireDecoder};

    let vis_slot = Visual::slot();
    let vis_decl = encode_table_decl(&Visual::DESC, 0);
    let create_root = NodeId::ROOT.raw().to_le_bytes();

    // 1. Empty Op::Patch (count == 0) must return Ok(()) without lazily allocating the column.
    let empty_patch_frame = build_wire_frame(&[
        (Op::DeclareTables as u8, 0, 1, &vis_decl),
        (Op::CreateNodes as u8, 0, 1, &create_root),
        (Op::Patch as u8, 0, 0, &[]),
        (Op::SetStrings as u8, 0, 0, &[]),
    ]);
    let mut replica = NodeStore::replica();
    let mut dec = WireDecoder::default();
    let mut changes = ChangeSet::default();
    {
        let mut tx = replica.tx();
        dec.apply(&mut tx, &empty_patch_frame)
            .expect("empty Patch/SetStrings succeeds");
        tx.commit_into(&mut changes);
    }
    assert!(
        replica.columns[vis_slot.0 as usize].is_none(),
        "empty Op::Patch / Op::SetStrings must not allocate column"
    );

    // 2. 1,000 repeated (NodeId::ROOT, mask = 0) patches on a 1-node store must keep dense
    //    row_count() <= 64 instead of inflating to expected_rows + 1 (1,001).
    let repeat_count: u32 = 1_000;
    let mut repeated_body = Vec::with_capacity((repeat_count as usize) * 8);
    for _ in 0..repeat_count {
        repeated_body.extend_from_slice(&NodeId::ROOT.raw().to_le_bytes());
        repeated_body.extend_from_slice(&0u32.to_le_bytes());
    }
    let repeat_frame = build_wire_frame(&[
        (Op::Patch as u8, 0, repeat_count, &repeated_body),
        (Op::Patch as u8, 0, repeat_count, &repeated_body),
    ]);
    {
        let mut tx = replica.tx();
        dec.apply(&mut tx, &repeat_frame)
            .expect("repeated ROOT patches succeed");
        tx.commit_into(&mut changes);
    }
    let col = replica.columns[vis_slot.0 as usize]
        .as_ref()
        .expect("Visual dense column");
    assert!(
        col.row_count() <= 64,
        "repeated patches to NodeId::ROOT must keep dense row_count() <= 64, got {}",
        col.row_count()
    );

    // 3. Two-op sparse Patch frame including NodeId(300) (>= 256, page 1) in Op 2 exercises
    //    fallible try_writer -> try_attach + PagedIndex::try_set's dir resize and new-page branch.
    let text_slot = Text::slot();
    let text_decl = encode_table_decl(&Text::DESC, 1);
    let mut nodes_body = Vec::new();
    for id in 2..=65u32 {
        nodes_body.extend_from_slice(&id.to_le_bytes());
    }
    nodes_body.extend_from_slice(&300u32.to_le_bytes());
    let mut op1_patch = Vec::new();
    op1_patch.extend_from_slice(&NodeId::ROOT.raw().to_le_bytes());
    op1_patch.extend_from_slice(&0u32.to_le_bytes());
    let mut op2_patch = Vec::new();
    for id in 2..=65u32 {
        op2_patch.extend_from_slice(&id.to_le_bytes());
        op2_patch.extend_from_slice(&0u32.to_le_bytes());
    }
    op2_patch.extend_from_slice(&300u32.to_le_bytes());
    op2_patch.extend_from_slice(&0u32.to_le_bytes());
    let two_op_sparse = build_wire_frame_num(
        3,
        &[
            (Op::DeclareTables as u8, 0, 1, &text_decl),
            (Op::CreateNodes as u8, 0, 65, &nodes_body),
            (Op::Patch as u8, 1, 1, &op1_patch),
            (Op::Patch as u8, 1, 65, &op2_patch),
        ],
    );
    {
        let mut tx = replica.tx();
        dec.apply(&mut tx, &two_op_sparse)
            .expect("two-op sparse frame succeeds via try_writer -> try_attach + try_set");
        tx.commit_into(&mut changes);
    }
    assert!(replica.has::<Text>(NodeId(300)));
    assert_eq!(
        replica.columns[text_slot.0 as usize]
            .as_ref()
            .expect("Text sparse column")
            .row_count(),
        66
    );
}

#[test]
fn wire_detach_on_high_index_node_across_sparse_and_dynamic_tables_dedups_sparsely() {
    use crate::wire::{Op, WireDecoder};

    // Frame 1: create high-index NodeId(100_000) (3.2 MiB << 16 MiB), declare 32 dynamic sparse
    // tables + static sparse Text, and attach all 33 to NodeId(100_000).
    let high_id = NodeId(100_000);
    let create_body = high_id.raw().to_le_bytes();
    let text_slot = Text::slot();
    let mut decls = encode_table_decl(&Text::DESC, 0);
    for i in 1..=32u8 {
        let key = nve_hash::Key64(0xCAFE_E000_0000_0000 + u64::from(i));
        let name = format!("sparse.detach.{i:02}");
        decls.extend_from_slice(&key.lo().to_le_bytes());
        decls.extend_from_slice(&key.hi().to_le_bytes());
        decls.extend_from_slice(&(0x200u32 + u32::from(i)).to_le_bytes());
        decls.extend_from_slice(&0u32.to_le_bytes());
        let packed = u32::from(i) | (1u32 << 8) | (1u32 << 24);
        decls.extend_from_slice(&packed.to_le_bytes());
        decls.extend_from_slice(&0u32.to_le_bytes());
        decls.extend_from_slice(&0u32.to_le_bytes());
        decls.extend_from_slice(&0u32.to_le_bytes());
        decls.extend_from_slice(&[1u8, 0, 0, 0]);
        decls.extend_from_slice(&(name.len() as u32).to_le_bytes());
        decls.extend_from_slice(name.as_bytes());
        while !decls.len().is_multiple_of(4) {
            decls.push(0);
        }
    }
    let mut patch_high = Vec::new();
    patch_high.extend_from_slice(&high_id.raw().to_le_bytes());
    patch_high.extend_from_slice(&0u32.to_le_bytes());

    let mut f1_ops: Vec<(u8, u8, u32, &[u8])> = Vec::with_capacity(35);
    f1_ops.push((Op::DeclareTables as u8, 0, 33, &decls));
    f1_ops.push((Op::CreateNodes as u8, 0, 1, &create_body));
    for wire_slot in 0..=32u8 {
        f1_ops.push((Op::Patch as u8, wire_slot, 1, &patch_high));
    }
    let f1 = build_wire_frame_num(1, &f1_ops);

    let mut replica = NodeStore::replica();
    let mut dec = WireDecoder::default();
    let mut changes = ChangeSet::default();
    {
        let mut tx = replica.tx();
        dec.apply(&mut tx, &f1).expect("apply frame 1");
        tx.commit_into(&mut changes);
    }

    // Frame 2: detach across all 33 sparse tables; also detach->re-attach->detach slot 0 (must
    // dedup to 1 entry), detach->re-attach slot 1 (must NOT appear in ChangeSet::detached), and
    // CreateNodes + Patch + Detach on a brand-new CREATED node (must NOT appear in detached).
    let fresh_id = NodeId(100_001);
    let fresh_create = fresh_id.raw().to_le_bytes();
    let mut fresh_patch = Vec::new();
    fresh_patch.extend_from_slice(&fresh_id.raw().to_le_bytes());
    fresh_patch.extend_from_slice(&0u32.to_le_bytes());
    let fresh_detach = fresh_id.raw().to_le_bytes();

    let detach_body = high_id.raw().to_le_bytes();
    let mut f2_ops: Vec<(u8, u8, u32, &[u8])> = Vec::with_capacity(41);
    // Slot 0 (Text): Detach -> Patch -> Detach => single deduplicated detached entry.
    f2_ops.push((Op::Detach as u8, 0, 1, &detach_body));
    f2_ops.push((Op::Patch as u8, 0, 1, &patch_high));
    f2_ops.push((Op::Detach as u8, 0, 1, &detach_body));
    // Same-frame CREATED node: CreateNodes -> Patch -> Detach on Text => zero detached entry.
    f2_ops.push((Op::CreateNodes as u8, 0, 1, &fresh_create));
    f2_ops.push((Op::Patch as u8, 0, 1, &fresh_patch));
    f2_ops.push((Op::Detach as u8, 0, 1, &fresh_detach));
    // Slot 1 (dynamic): Detach -> Patch => stays attached, zero detached entry.
    f2_ops.push((Op::Detach as u8, 1, 1, &detach_body));
    f2_ops.push((Op::Patch as u8, 1, 1, &patch_high));
    // Slots 2..=32: plain Detach.
    for wire_slot in 2..=32u8 {
        f2_ops.push((Op::Detach as u8, wire_slot, 1, &detach_body));
    }
    let f2 = build_wire_frame_num(2, &f2_ops);
    {
        let mut tx = replica.tx();
        dec.apply(&mut tx, &f2).expect("apply frame 2 detaches");
        tx.commit_into(&mut changes);
    }

    assert!(!replica.has::<Text>(fresh_id));
    assert_eq!(
        changes
            .table(text_slot)
            .expect("Text table changes")
            .detached,
        vec![high_id],
        "detach->re-attach->detach must deduplicate and same-frame CREATED node must not emit detached"
    );
    let mut reattached_seen = 0usize;
    let mut detached_dyn_seen = 0usize;
    for (idx, col) in replica
        .columns
        .iter()
        .enumerate()
        .skip(crate::table::MAX_STATIC_TABLE_SLOTS)
    {
        let Some(col) = col else { continue };
        let slot = TableSlot(idx as u8);
        assert!(
            col.detached.capacity() <= 8,
            "sparse column detached buffer must stay O(1), not dense"
        );
        assert!(col.relocated_epoch.is_empty());
        if col.row_count() == 1 {
            reattached_seen += 1;
            assert!(
                changes.table(slot).is_none_or(|t| t.detached.is_empty()),
                "re-attached table must not report detached in ChangeSet"
            );
        } else {
            assert_eq!(col.row_count(), 0);
            detached_dyn_seen += 1;
            assert_eq!(
                changes.table(slot).expect("detached dyn change").detached,
                vec![high_id]
            );
        }
    }
    assert_eq!(reattached_seen, 1);
    assert_eq!(detached_dyn_seen, 31);
}

#[test]
fn session_budget_allows_churned_keyframes_and_blocks_multiframe_stride_1024_amplification() {
    use crate::wire::{Op, WireDecoder, WireEncoder, WireError};

    // 1. Legitimate churned keyframe (create 2,500 nodes, remove 2..2499; define & drop 2,000
    //    resources keeping the last one) must round-trip cleanly via encode_full() into a fresh
    //    replica without hitting any arbitrary +1024 index cap.
    let mut src = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut created_ids = Vec::with_capacity(2_500);
    {
        let mut tx = src.tx();
        for _ in 0..2_500 {
            created_ids.push(tx.create());
        }
        tx.commit_into(&mut changes);
    }
    let last_node = *created_ids.last().expect("last churned node");
    assert!(last_node.index() >= 2_500);
    {
        let mut tx = src.tx();
        for &id in &created_ids[..created_ids.len() - 1] {
            tx.remove(id);
        }
        tx.set_children(NodeId::ROOT, &[last_node]);
        tx.commit_into(&mut changes);
    }
    // Churn 2,000 resources in one frame, then attach the last one so its ResourceId is > 2,000.
    let mut last_shadow = Shadows(vec![0.0]);
    {
        let mut tx = src.tx();
        for i in 1..=2_000u32 {
            let s = Shadows(vec![i as f32]);
            tx.edit::<Visual>(last_node).shadows(Some(s));
            last_shadow = Shadows(vec![i as f32]);
        }
        tx.commit_into(&mut changes);
    }
    let last_rid = src.row::<Visual>(last_node).shadows();
    assert!(
        last_rid.0 >= 2_000,
        "expected churned ResourceId >= 2,000, got {}",
        last_rid.0
    );

    let mut keyframe = Vec::new();
    WireEncoder::default().encode_full(&src, &mut keyframe);
    let mut replica = NodeStore::replica();
    let mut dec = WireDecoder::default();
    {
        let mut tx = replica.tx();
        dec.apply(&mut tx, &keyframe)
            .expect("churned keyframe must round-trip cleanly under SessionBudget");
        tx.commit_into(&mut changes);
    }
    assert_eq!(
        replica.live_nodes().collect::<Vec<_>>(),
        vec![NodeId::ROOT, last_node]
    );
    assert_eq!(
        replica.resource::<Shadows>(replica.row::<Visual>(last_node).shadows()),
        Some(&last_shadow)
    );

    // 2. Multi-frame stride-1024 node + dense-Visual attack (spoofing frame == 1 on every delta
    //    to prove non-fresh stores NEVER refill the 16 MiB base!) must hit WireError::Budget.
    let vis_decl = encode_table_decl(&Visual::DESC, 0);
    let mut atk_store = NodeStore::replica();
    let mut atk_dec = WireDecoder::default();
    let mut hit_budget_at = None;
    for step in 0..500u32 {
        let nid = (step + 1) * 1_024;
        let create = nid.to_le_bytes();
        let mut patch = Vec::new();
        patch.extend_from_slice(&nid.to_le_bytes());
        patch.extend_from_slice(&0u32.to_le_bytes());
        let frame = if step == 0 {
            build_wire_frame_num(
                1,
                &[
                    (Op::DeclareTables as u8, 0, 1, &vis_decl),
                    (Op::CreateNodes as u8, 0, 1, &create),
                    (Op::Patch as u8, 0, 1, &patch),
                ],
            )
        } else {
            // Alternate spoofed frame == 1 and normal delta frame numbers.
            let fnum = if step % 2 == 0 {
                1
            } else {
                u64::from(step + 1)
            };
            build_wire_frame_num(
                fnum,
                &[
                    (Op::CreateNodes as u8, 0, 1, &create),
                    (Op::Patch as u8, 0, 1, &patch),
                ],
            )
        };
        let mut tx = atk_store.tx();
        match atk_dec.apply(&mut tx, &frame) {
            Ok(_) => tx.commit_into(&mut changes),
            Err(WireError::Budget) => {
                hit_budget_at = Some(step);
                break;
            }
            Err(other) => panic!("unexpected error on stride-1024 step {step}: {other:?}"),
        }
    }
    assert!(
        hit_budget_at.is_some(),
        "500-frame stride-1024 node+dense stream must hit WireError::Budget"
    );
    assert!(
        atk_store.capacity() < 300_000,
        "stride-1024 stream must cap store capacity, got {}",
        atk_store.capacity()
    );

    // 3. Multi-frame stride-1024 resource attack must hit WireError::Budget well before 500 frames.
    crate::register_codec::<Shadows>();
    let mut blob = Vec::new();
    Shadows(vec![42.0]).encode(&mut blob);
    let mut res_store = NodeStore::replica();
    let mut res_dec = WireDecoder::default();
    let mut res_hit_at = None;
    for step in 0..500u32 {
        let rid = (step + 1) * 1_024;
        let mut def = Vec::new();
        def.extend_from_slice(&rid.to_le_bytes());
        def.extend_from_slice(&Shadows::KIND.lo().to_le_bytes());
        def.extend_from_slice(&Shadows::KIND.hi().to_le_bytes());
        def.extend_from_slice(&0u32.to_le_bytes());
        def.extend_from_slice(&(blob.len() as u32).to_le_bytes());
        def.extend_from_slice(&blob);
        let frame = build_wire_frame_num(
            u64::from(step + 1),
            &[(Op::DefineResources as u8, 0, 1, &def)],
        );
        let mut tx = res_store.tx();
        match res_dec.apply(&mut tx, &frame) {
            Ok(_) => tx.commit_into(&mut changes),
            Err(WireError::Budget) => {
                res_hit_at = Some(step);
                break;
            }
            Err(other) => panic!("unexpected resource error at step {step}: {other:?}"),
        }
    }
    assert!(
        res_hit_at.is_some(),
        "500-frame stride-1024 resource stream must hit WireError::Budget"
    );
    assert!(
        res_store.resources().slots() < 350_000,
        "stride-1024 resource stream must cap slots, got {}",
        res_store.resources().slots()
    );
}

#[test]
fn dirty_list_try_reserve_plus_one_prevents_grow_at_exact_64_and_patch_detach_repatch() {
    use crate::wire::{Op, WireDecoder};

    let vis_slot = Visual::slot();
    let vis_decl = encode_table_decl(&Visual::DESC, 0);
    let mut create_64 = Vec::with_capacity(64 * 4);
    let mut patch_64 = Vec::with_capacity(64 * 12);
    for id in 1..=64u32 {
        create_64.extend_from_slice(&id.to_le_bytes());
        patch_64.extend_from_slice(&id.to_le_bytes());
        patch_64.extend_from_slice(&1u32.to_le_bytes()); // mask: word 0 (bg)
        patch_64.extend_from_slice(&id.to_le_bytes());
    }
    let f1 = build_wire_frame_num(
        1,
        &[
            (Op::DeclareTables as u8, 0, 1, &vis_decl),
            (Op::CreateNodes as u8, 0, 64, &create_64),
            (Op::Patch as u8, 0, 64, &patch_64),
        ],
    );

    let mut replica = NodeStore::replica();
    let mut dec = WireDecoder::default();
    let mut changes = ChangeSet::default();
    {
        let mut tx = replica.tx();
        dec.apply(&mut tx, &f1)
            .expect("exact 64-node dense patch succeeds");
        let col = tx.store().columns[vis_slot.0 as usize]
            .as_ref()
            .expect("Visual column");
        assert_eq!(col.dirty.as_slice().len(), 64);
        assert_eq!(
            col.dirty.buf_len(),
            65,
            "DirtyList::try_reserve(64) must allocate 65 slots (+1 scratch) and never double to 128 via grow()"
        );
        tx.commit_into(&mut changes);
    }

    // Frame 2: Write MODIFIED values to all 64 nodes so `changed == 1` actually fills
    // `dirty.len() == 64` (buf_len == 65), re-patch already-dirty NodeId(2) (exercising
    // `try_reserve(0)` when `len + 1 == buf_len`), then Detach ROOT and re-Patch ROOT (pushing
    // the 65th dirty entry so `dirty.len() == 65` and `buf_len() == 66`, never 128 via grow()).
    let mut patch_64_mod = Vec::with_capacity(64 * 12);
    for id in 1..=64u32 {
        patch_64_mod.extend_from_slice(&id.to_le_bytes());
        patch_64_mod.extend_from_slice(&1u32.to_le_bytes());
        patch_64_mod.extend_from_slice(&(id ^ 0xFF00_0000).to_le_bytes());
    }
    let mut already_dirty_n2 = Vec::new();
    already_dirty_n2.extend_from_slice(&2u32.to_le_bytes());
    already_dirty_n2.extend_from_slice(&1u32.to_le_bytes());
    already_dirty_n2.extend_from_slice(&0xAABB_CCDDu32.to_le_bytes());
    let detach_root = NodeId::ROOT.raw().to_le_bytes();
    let mut repatch_root = Vec::new();
    repatch_root.extend_from_slice(&NodeId::ROOT.raw().to_le_bytes());
    repatch_root.extend_from_slice(&1u32.to_le_bytes());
    repatch_root.extend_from_slice(&0x1122_3344u32.to_le_bytes());
    let f2 = build_wire_frame_num(
        2,
        &[
            (Op::Patch as u8, 0, 64, &patch_64_mod),
            (Op::Patch as u8, 0, 1, &already_dirty_n2),
            (Op::Detach as u8, 0, 1, &detach_root),
            (Op::Patch as u8, 0, 1, &repatch_root),
        ],
    );
    {
        let mut tx = replica.tx();
        dec.apply(&mut tx, &f2).expect(
            "same-frame modified Patch(64) + already-dirty Patch + Detach + re-Patch succeeds",
        );
        let col = tx.store().columns[vis_slot.0 as usize]
            .as_ref()
            .expect("Visual column");
        assert_eq!(
            col.dirty.as_slice().len(),
            65,
            "all 64 modified nodes + re-attached ROOT must produce 65 dirty entries"
        );
        assert_eq!(
            col.dirty.buf_len(),
            66,
            "fallible dirty.try_reserve(+1 scratch) must allocate 66 slots, never 128 via grow()"
        );
        tx.commit_into(&mut changes);
    }
    assert_eq!(replica.row::<Visual>(NodeId::ROOT).bg(), Rgba(0x1122_3344));
    assert_eq!(replica.row::<Visual>(NodeId(2)).bg(), Rgba(0xAABB_CCDD));

    // Direct DirtyList boundary: when `len == buf_len` (64 == 64), `try_reserve(0)` must add the
    // 1 scratch slot (65) so a subsequent no-op `push_if(_, false)` does NOT double to 128.
    let mut dl = crate::row::DirtyList::default();
    for i in 1..=64u32 {
        dl.push_if(crate::id::NodeIndex(i), true);
    }
    assert_eq!(dl.as_slice().len(), 64);
    assert_eq!(dl.buf_len(), 64);
    dl.try_reserve(0).expect("try_reserve(0) adds scratch slot");
    assert_eq!(dl.buf_len(), 65);
    dl.push_if(crate::id::NodeIndex(1), false);
    assert_eq!(dl.as_slice().len(), 64);
    assert_eq!(
        dl.buf_len(),
        65,
        "no-op push_if after try_reserve(0) must use scratch slot instead of 2x grow()"
    );
}

#[test]
fn paged_index_duplicate_patch_avoids_over_reserve_and_stride_256_hits_budget() {
    use crate::wire::{Op, WireDecoder, WireError};

    let build_32_dyn_sparse_decls = |key_base: u64, prefix: &str| {
        let mut decls = Vec::new();
        for i in 0..32u8 {
            let key = nve_hash::Key64(key_base + u64::from(i));
            let name = format!("{prefix}.{i:02}");
            decls.extend_from_slice(&key.lo().to_le_bytes());
            decls.extend_from_slice(&key.hi().to_le_bytes());
            decls.extend_from_slice(&(0x300u32 + u32::from(i)).to_le_bytes());
            decls.extend_from_slice(&0u32.to_le_bytes());
            let packed = u32::from(i) | (1u32 << 8) | (1u32 << 24);
            decls.extend_from_slice(&packed.to_le_bytes());
            decls.extend_from_slice(&0u32.to_le_bytes());
            decls.extend_from_slice(&0u32.to_le_bytes());
            decls.extend_from_slice(&0u32.to_le_bytes());
            decls.extend_from_slice(&[1u8, 0, 0, 0]);
            decls.extend_from_slice(&(name.len() as u32).to_le_bytes());
            decls.extend_from_slice(name.as_bytes());
            while !decls.len().is_multiple_of(4) {
                decls.push(0);
            }
        }
        decls
    };

    // Vector 1: 1 high-index live node (NodeId(500_000)) + 32 dynamic sparse tables each receiving
    // 2,000 repeated `(NodeId(500_000), mask = 0)` patches -> succeeds without 128x pool over-reserve
    // (`paged_pool_cap() <= 8`, NOT 1,954!).
    let high_id = NodeId(500_000);
    let dup_decls = build_32_dyn_sparse_decls(0xCAFE_F000_0000_0000, "dup.paged");
    let create_high = high_id.raw().to_le_bytes();
    let repeat_n: u32 = 2_000;
    let mut dup_patch = Vec::with_capacity((repeat_n as usize) * 8);
    for _ in 0..repeat_n {
        dup_patch.extend_from_slice(&high_id.raw().to_le_bytes());
        dup_patch.extend_from_slice(&0u32.to_le_bytes());
    }
    let mut dup_ops: Vec<(u8, u8, u32, &[u8])> = Vec::with_capacity(34);
    dup_ops.push((Op::DeclareTables as u8, 0, 32, &dup_decls));
    dup_ops.push((Op::CreateNodes as u8, 0, 1, &create_high));
    for i in 0..32u8 {
        dup_ops.push((Op::Patch as u8, i, repeat_n, &dup_patch));
    }
    let dup_frame = build_wire_frame(&dup_ops);

    let mut dup_replica = NodeStore::replica();
    let mut dup_dec = WireDecoder::default();
    let mut changes = ChangeSet::default();
    {
        let mut tx = dup_replica.tx();
        dup_dec
            .apply(&mut tx, &dup_frame)
            .expect("duplicate high-index sparse patches must stay within SessionBudget");
        tx.commit_into(&mut changes);
    }
    let dup_cols: Vec<_> = dup_replica
        .columns
        .iter()
        .skip(crate::table::MAX_STATIC_TABLE_SLOTS)
        .flatten()
        .collect();
    assert_eq!(dup_cols.len(), 32);
    for col in dup_cols {
        assert_eq!(col.row_count(), 1);
        assert!(
            col.paged_pool_cap() <= 8,
            "PagedIndex::reserve_for_nodes must cap pool_pages by expected_rows.div_ceil(256) (<= 8), got {}",
            col.paged_pool_cap()
        );
    }

    // Vector 2: 600 stride-256 live nodes (`NodeId(256..=153_600)`, only 4.92 MiB of NodeRows)
    // patched across 32 dynamic sparse tables -> `PagedIndex::pool` growth (32 * 600 * 1 KiB =
    // 19.66 MiB + 4.92 MiB nodes > ~18.6 MiB budget) MUST fail with WireError::Budget.
    let stride_decls = build_32_dyn_sparse_decls(0xCAFE_F100_0000_0000, "stride.paged");
    let stride_n: u32 = 600;
    let mut stride_create = Vec::with_capacity((stride_n as usize) * 4);
    let mut stride_patch = Vec::with_capacity((stride_n as usize) * 8);
    for k in 1..=stride_n {
        let nid = k * 256;
        stride_create.extend_from_slice(&nid.to_le_bytes());
        stride_patch.extend_from_slice(&nid.to_le_bytes());
        stride_patch.extend_from_slice(&0u32.to_le_bytes());
    }
    let mut stride_ops: Vec<(u8, u8, u32, &[u8])> = Vec::with_capacity(34);
    stride_ops.push((Op::DeclareTables as u8, 0, 32, &stride_decls));
    stride_ops.push((Op::CreateNodes as u8, 0, stride_n, &stride_create));
    for i in 0..32u8 {
        stride_ops.push((Op::Patch as u8, i, stride_n, &stride_patch));
    }
    let stride_frame = build_wire_frame(&stride_ops);

    let mut stride_replica = NodeStore::replica();
    let mut stride_dec = WireDecoder::default();
    let mut tx = stride_replica.tx();
    assert_eq!(
        stride_dec.apply(&mut tx, &stride_frame),
        Err(WireError::Budget),
        "stride-256 32-dynamic-table PagedIndex::pool amplification must hit WireError::Budget"
    );
}
