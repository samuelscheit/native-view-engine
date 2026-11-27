use nve_abi::table::effect;

use crate::{
    ChangeSet, Length, NodeId, NodeStore, NveTable, RealizationOrder, Res, Resource, Rgba, Str,
    TableSlot, WordEnum, WordValue,
};

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, WordEnum)]
pub enum Align {
    #[default]
    Start = 0,
    Center = 1,
    End = 2,
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
        Some(Self(bytes.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect()))
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
    pub align: Align,
    #[effect(layout)]
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
    assert_eq!(d.words(), 7, "bg, opacity, width, packed(align|wrap), label(2), shadows");
    assert_eq!(d.defaults[1], 1.0f32.to_bits());
    assert_eq!(d.defaults[2], Length::AUTO.0);
    assert_eq!(d.defaults[3], 1 << 2, "wrap=true packed after 2-bit align");
    assert_eq!(d.str_words, 1 << 4);
    assert_eq!(d.resource_words, 1 << 6);
    assert_eq!(d.word_effects, &[effect::PAINT, effect::COMPOSITE, effect::LAYOUT, effect::LAYOUT, effect::SEMANTICS, effect::SEMANTICS, effect::PAINT]);
    assert_eq!(d.flags & nve_abi::table::table_flags::AFFECTS_LAYOUT, nve_abi::table::table_flags::AFFECTS_LAYOUT);
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
    tx.edit::<Visual>(n).bg(Rgba::rgb(1, 2, 3)).align(Align::End);
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
    assert_eq!((r.bg(), r.opacity(), r.align(), r.wrap()), (Rgba::rgb(1, 2, 3), 1.0, Align::End, true));
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
    assert_eq!(s.row::<Visual>(n).align(), Align::End, "packed neighbour untouched");
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
    assert_eq!(s.row::<Text>(a).text_span(), span, "same class rewrites in place");
    assert_eq!(c.table(Text::slot()).unwrap().masks, vec![0b11], "in-place content change is still dirty");
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
    assert!(reused.index() == p.index() || reused.index() == c1.index() || reused.index() == c2.index());
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
    assert_eq!(s.row::<Text>(x).color(), Rgba::BLACK, "absent rows read as defaults");
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
        tx.edit::<Text>(n).text(&format!("n{i}")).color(Rgba(i as u32));
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
    bus.emit::<Visual, (f32, f32)>(nve_abi::SurfaceId::new(7, 0), n, 1234, 42, (10.0, 20.0));
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
        assert!(store.resources.define_at(crate::ResourceId(1), Shadows::KIND, false, &encoded));
        // Duplicate define_at in the same frame must not push a duplicate into `pending_free`.
        assert!(store.resources.define_at(crate::ResourceId(1), Shadows::KIND, false, &encoded));
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
    assert!(store.resources.define_at(crate::ResourceId(1), Shadows::KIND, false, &enc_a));
    store.resources.end_frame();

    // Frame 2: wire redefines id=1 AND retains it (still carries `RES_IN_FREE` until popped).
    assert!(store.resources.define_at(crate::ResourceId(1), Shadows::KIND, false, &enc_a));
    store.resources.retain(crate::ResourceId(1));
    store.resources.end_frame();
    assert_eq!(store.resource::<Shadows>(crate::ResourceId(1)), Some(&Shadows(vec![1.0, 2.0])));

    // Local `intern` pops id=1 from `free`, must see it is live and skip it instead of clobbering!
    let local_id = store.resources.intern(Shadows(vec![9.0, 8.0]));
    assert_ne!(local_id, crate::ResourceId(1), "live replica resource slot must not be clobbered");
    assert_eq!(store.resource::<Shadows>(crate::ResourceId(1)), Some(&Shadows(vec![1.0, 2.0])));
    assert_eq!(store.resource::<Shadows>(local_id), Some(&Shadows(vec![9.0, 8.0])));
    let _ = enc_b;
}

#[test]
fn install_dynamic_table_rejects_static_slot_and_schema_mismatch() {
    use std::sync::Arc;
    use crate::table::{OwnedTableDesc, MAX_STATIC_TABLE_SLOTS};
    use nve_abi::table::{table_flags, Storage};

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
    assert!(!store.install_dynamic_table(TableSlot((MAX_STATIC_TABLE_SLOTS - 1) as u8), make_desc(1)));
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
        fn encode(&self, out: &mut Vec<u8>) { out.extend_from_slice(&self.0.to_le_bytes()); }
        fn decode(b: &[u8]) -> Option<Self> {
            b.try_into().ok().map(|a| Self(u32::from_le_bytes(a)))
        }
    }
    impl Resource for CollideB {
        const KIND: nve_hash::Key64 = SHARED_KIND;
        fn encode(&self, out: &mut Vec<u8>) { out.extend_from_slice(&self.0.to_le_bytes()); }
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
fn opt_word_wide_option_64bit_flags_and_grid_line_roundtrip_and_wire() {
    use crate::wire::{WireDecoder, WireEncoder};
    use crate::{word_flags, GridLine, ResourceId};

    word_flags! {
        pub struct SampleFlags(u8) {
            const A = 1 << 0;
            const B = 1 << 1;
            const C = 1 << 2;
        }
    }

    #[derive(crate::NveTable)]
    #[nve_table(name = "test.c2_all", storage = "sparse", flags = "publish")]
    #[allow(dead_code)]
    pub struct C2All {
        #[effect(paint)]
        pub opt_bool: Option<bool>,
        #[effect(paint)]
        pub opt_u8: Option<u8>,
        #[effect(paint)]
        pub opt_i8: Option<i8>,
        #[effect(paint)]
        pub opt_u16: Option<u16>,
        #[effect(paint)]
        pub opt_i16: Option<i16>,
        #[effect(paint)]
        pub opt_align: Option<Align>,
        #[effect(paint)]
        pub opt_flags: Option<SampleFlags>,
        #[effect(layout)]
        pub opt_len: Option<Length>,
        #[effect(layout)]
        pub opt_grid: Option<GridLine>,
        #[effect(semantics)]
        pub opt_node: Option<NodeId>,
        #[effect(semantics)]
        pub opt_res: Option<ResourceId>,
        #[effect(semantics)]
        pub opt_surf: Option<nve_abi::SurfaceId>,
        #[effect(paint)]
        pub wide_f32: Option<f32>,
        #[effect(paint)]
        pub wide_u32: Option<u32>,
        #[effect(paint)]
        pub wide_i32: Option<i32>,
        #[effect(paint)]
        pub wide_rgba: Option<Rgba>,
        #[effect(paint)]
        pub wide_u64: u64,
        #[effect(paint)]
        pub wide_i64: i64,
        #[effect(paint)]
        pub wide_f64: f64,
    }

    let mut src = NodeStore::new();
    let mut tx = src.tx();
    let def_n = tx.create();
    let full_n = tx.create();
    tx.set_children(NodeId::ROOT, &[def_n, full_n]);
    let _ = tx.edit::<C2All>(def_n);
    tx.edit::<C2All>(full_n)
        .opt_bool(Some(false))
        .opt_u8(Some(u8::MAX))
        .opt_i8(Some(i8::MIN))
        .opt_u16(Some(u16::MAX))
        .opt_i16(Some(i16::MIN))
        .opt_align(Some(Align::Start))
        .opt_flags(Some(SampleFlags::A | SampleFlags::C))
        .opt_len(Some(Length::points(0.0)))
        .opt_grid(Some(GridLine::Span(3)))
        .opt_node(Some(NodeId::ROOT))
        .opt_res(Some(ResourceId(7)))
        .opt_surf(Some(nve_abi::SurfaceId::ROOT))
        .wide_f32(Some(0.0))
        .wide_u32(Some(u32::MAX))
        .wide_i32(Some(i32::MIN))
        .wide_rgba(Some(Rgba::TRANSPARENT))
        .wide_u64(0x1234_5678_9ABC_DEF0)
        .wide_i64(-0x1234_5678_9ABC_DEF0)
        .wide_f64(core::f64::consts::PI);

    // Typed `tx.set` via generated `Prop` handles (including wide & 64-bit).
    tx.set(def_n, C2All::WIDE_F32, Some(2.5));
    tx.set(def_n, C2All::WIDE_U64, 99u64);
    let changes = commit_tx(tx);

    // Wire roundtrip into a replica store.
    let mut enc = WireEncoder::default();
    let mut wire = Vec::new();
    enc.encode(&src, &changes, &mut wire);

    let mut dst = NodeStore::replica();
    let mut dec = WireDecoder::default();
    let mut dst_tx = dst.tx();
    dec.apply(&mut dst_tx, &wire).expect("NVE2 decode C2All");
    commit_tx(dst_tx);

    let d = dst.try_row::<C2All>(def_n).unwrap();
    assert_eq!(d.opt_bool(), None);
    assert_eq!(d.opt_u8(), None);
    assert_eq!(d.opt_i16(), None);
    assert_eq!(d.opt_align(), None);
    assert_eq!(d.opt_flags(), None);
    assert_eq!(d.opt_len(), None);
    assert_eq!(d.opt_grid(), None);
    assert_eq!(d.opt_node(), None);
    assert_eq!(d.wide_f32(), Some(2.5));
    assert_eq!(d.wide_u32(), None);
    assert_eq!(d.wide_rgba(), None);
    assert_eq!(d.wide_u64(), 99);
    assert_eq!(d.wide_i64(), 0);
    assert_eq!(d.wide_f64(), 0.0);

    // Sentinel boundary values must roundtrip as `Some(...)` (never conflated with `None`).
    let f = dst.try_row::<C2All>(full_n).unwrap();
    assert_eq!(f.opt_bool(), Some(false));
    assert_eq!(f.opt_u8(), Some(u8::MAX));
    assert_eq!(f.opt_i8(), Some(i8::MIN));
    assert_eq!(f.opt_u16(), Some(u16::MAX));
    assert_eq!(f.opt_i16(), Some(i16::MIN));
    assert_eq!(f.opt_align(), Some(Align::Start));
    assert_eq!(f.opt_flags(), Some(SampleFlags::A | SampleFlags::C));
    assert_eq!(f.opt_len(), Some(Length::points(0.0)));
    assert_eq!(f.opt_grid(), Some(GridLine::Span(3)));
    assert_eq!(f.opt_node(), Some(NodeId::ROOT));
    assert_eq!(f.opt_res(), Some(ResourceId(7)));
    assert_eq!(f.opt_surf(), Some(nve_abi::SurfaceId::ROOT));
    assert_eq!(f.wide_f32(), Some(0.0));
    assert_eq!(f.wide_u32(), Some(u32::MAX));
    assert_eq!(f.wide_i32(), Some(i32::MIN));
    assert_eq!(f.wide_rgba(), Some(Rgba::TRANSPARENT));
    assert_eq!(f.wide_u64(), 0x1234_5678_9ABC_DEF0);
    assert_eq!(f.wide_i64(), -0x1234_5678_9ABC_DEF0);
    assert_eq!(f.wide_f64(), core::f64::consts::PI);

    // Clearing a wide Option back to None works cleanly.
    let mut tx2 = dst.tx();
    tx2.edit::<C2All>(full_n).wide_f32(None).wide_rgba(None);
    commit_tx(tx2);
    let f2 = dst.try_row::<C2All>(full_n).unwrap();
    assert_eq!(f2.wide_f32(), None);
    assert_eq!(f2.wide_rgba(), None);
    assert_eq!(f2.wide_u32(), Some(u32::MAX));
}
