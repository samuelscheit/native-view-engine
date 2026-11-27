//! Soundness, boundary, `#[no_inherit]`, `WideOpt`, and `NVE2` dynamic/keyframe wire tests
//! for `#[nve_table(inherited)]` and [`ResolvedRow`].

use nve_abi::table::{table_flags, Storage};
use nve_abi::Key64;

use crate::span_pool::StrSpan;
use crate::wire::{WireDecoder, WireEncoder, WireError};
use crate::{
    effect, ChangeSet, FlatResource, InheritedTable, NodeId, NodeStore, NveTable, Res, ResolvedRow,
    Rgba, Str,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct EdgeBlob(u32);

impl FlatResource for EdgeBlob {
    const KIND: Key64 = Key64::of("test.inherited.edge_blob");
    type View<'a> = u32;
    fn write_words(&self, out: &mut Vec<u32>) -> bool {
        out.push(self.0);
        true
    }
    fn validate(words: &[u32]) -> bool {
        words.len() == 1
    }
    fn view(words: crate::ValidatedWords<'_, Self>) -> u32 {
        words[0]
    }
}

#[allow(dead_code)]
#[derive(crate::NveTable)]
#[nve_table(name = "test.inherited.edge", storage = "sparse", flags = "publish", inherited)]
struct EdgeInherited {
    #[effect(paint)]
    #[default(Rgba::BLACK)]
    color: Rgba,
    #[effect(measure)]
    font_family: Str,
    // Two sibling WideOpt fields with identical effect class share one presence word!
    #[effect(paint)]
    opt_a: Option<f32>,
    #[effect(paint)]
    #[default(Some(Rgba::WHITE))]
    opt_b: Option<Rgba>,
    #[effect(paint)]
    tag64: u64,
    #[effect(paint)]
    blob: Res<EdgeBlob>,
    #[effect(measure)]
    #[no_inherit]
    local_u16: u16,
    #[effect(paint)]
    #[no_inherit]
    local_opt: Option<u32>,
    #[effect(measure)]
    #[no_inherit]
    local_str: Str,
}

#[test]
#[should_panic(expected = "different NodeStore")]
fn cross_store_resolve_panics() {
    let mut store_a = NodeStore::new();
    let mut store_b = NodeStore::new();
    let a = {
        let mut tx = store_a.tx();
        let n = tx.create();
        tx.edit::<EdgeInherited>(n).font_family("store-a-font");
        n
    };
    let b = {
        let mut tx = store_b.tx();
        let n = tx.create();
        tx.edit::<EdgeInherited>(n).color(Rgba::WHITE);
        n
    };
    let parent_a = ResolvedRow::<EdgeInherited>::resolve(
        &store_a,
        a,
        &ResolvedRow::<EdgeInherited>::root(&store_a),
    );
    let _ = ResolvedRow::<EdgeInherited>::resolve(&store_b, b, &parent_a);
}

#[test]
#[should_panic(expected = "different NodeStore")]
fn cross_store_resolve_ref_panics() {
    let store_a = NodeStore::new();
    let mut store_b = NodeStore::new();
    let b = {
        let mut tx = store_b.tx();
        tx.create()
    };
    let parent_a = ResolvedRow::<EdgeInherited>::root(&store_a);
    let node_b = store_b.get(b).expect("live node b");
    let _ = ResolvedRow::<EdgeInherited>::resolve_ref(node_b, &parent_a);
}

#[test]
fn try_from_words_validates_str_spans_and_rejects_forged_or_mid_utf8() {
    let mut store = NodeStore::new();
    let n = {
        let mut tx = store.tx();
        let n = tx.create();
        tx.edit::<EdgeInherited>(n).font_family("héllo");
        n
    };
    let resolved = ResolvedRow::<EdgeInherited>::resolve(
        &store,
        n,
        &ResolvedRow::<EdgeInherited>::root(&store),
    );
    let valid_words = resolved.into_words();
    let checked = ResolvedRow::<EdgeInherited>::try_from_words(valid_words, &store)
        .expect("valid resolved words must pass try_from_words");
    assert_eq!(checked.get().font_family(), "héllo");

    let str_w = EdgeInherited::FONT_FAMILY.word() as usize;
    let span = checked.get().font_family_span();
    assert_eq!(span.len(), "héllo".len() as u32);

    // Out-of-bounds forged span must return None.
    let mut oob_words = valid_words;
    oob_words[str_w] = 9999;
    oob_words[str_w + 1] = 4;
    assert!(ResolvedRow::<EdgeInherited>::try_from_words(oob_words, &store).is_none());

    // Mid-UTF-8 codepoint forged span (`'é'` is bytes 1..3 of `"héllo"`): slice starting at +2 splits `'é'`.
    let mut mid_utf8 = valid_words;
    mid_utf8[str_w] = span.off() + 2;
    mid_utf8[str_w + 1] = 2;
    assert!(
        store
            .text_arena()
            .try_str(StrSpan::new(mid_utf8[str_w], mid_utf8[str_w + 1]))
            .is_none()
    );
    assert!(ResolvedRow::<EdgeInherited>::try_from_words(mid_utf8, &store).is_none());
}

#[test]
fn override_prop_updates_value_sets_explicit_bit_and_propagates_to_children() {
    let mut store = NodeStore::new();
    let (parent, child) = {
        let mut tx = store.tx();
        let p = tx.create();
        let c = tx.create();
        tx.set_children(NodeId::ROOT, &[p]);
        tx.set_children(p, &[c]);
        (p, c)
    };
    let root_r = ResolvedRow::<EdgeInherited>::root(&store);
    let mut p_r = ResolvedRow::<EdgeInherited>::resolve(&store, parent, &root_r);
    assert!(!p_r.get().is_color_explicit());
    assert_eq!(p_r.get().color(), Rgba::BLACK);

    let accent = Rgba::rgb(10, 20, 30);
    p_r.override_prop(EdgeInherited::COLOR, accent);
    assert_eq!(p_r.get().color(), accent);
    assert!(
        p_r.get().is_color_explicit(),
        "override_prop must set the field's explicit bit in word 0"
    );

    let c_r = ResolvedRow::<EdgeInherited>::resolve(&store, child, &p_r);
    assert_eq!(c_r.get().color(), accent);
    assert!(c_r.get().is_color_explicit());
}

#[test]
fn sibling_wide_opts_no_inherit_clear_wide_opt_u64_and_node_mut_str_res() {
    // Verify two sibling WideOpt fields share the same presence word with distinct shifts/val_words.
    let fa = EdgeInherited::DESC.fields[2];
    let fb = EdgeInherited::DESC.fields[3];
    assert_eq!(fa.word, fb.word, "sibling WideOpt fields should share presence word");
    assert_ne!(fa.shift, fb.shift);
    assert_ne!(fa.val_word, fb.val_word);
    assert!(EdgeInherited::DESC.fields[6].no_inherit);
    assert!(EdgeInherited::DESC.fields[7].no_inherit);
    assert!(EdgeInherited::DESC.fields[8].no_inherit);
    assert!(!EdgeInherited::DESC.fields[0].no_inherit);

    let mut store = NodeStore::new();
    let (parent, attached_child, unattached_child, grandchild) = {
        let mut tx = store.tx();
        let p = tx.create();
        let ac = tx.create();
        let uc = tx.create();
        let gc = tx.create();
        tx.set_children(NodeId::ROOT, &[p]);
        tx.set_children(p, &[ac, uc]);
        tx.set_children(ac, &[gc]);

        tx.edit::<EdgeInherited>(p)
            .opt_a(Some(2.5))
            .opt_b(Some(Rgba::rgb(1, 2, 3)))
            .tag64(0xDEAD_BEEF_CAFE_BABEu64)
            .local_u16(99)
            .local_opt(Some(77))
            .local_str("parent-only");

        // Exercise NodeMut::{set_str, set_resource} explicit-bit propagation.
        tx.node_mut(p)
            .unwrap()
            .set_str(EdgeInherited::FONT_FAMILY, "Fira Code")
            .set_resource(EdgeInherited::BLOB, Some(EdgeBlob(42)));

        // Attached child overrides opt_b to None (despite default Some(WHITE)), overrides tag64/opt_a,
        // and sets its own local_u16 (which must NOT leak to grandchild).
        tx.edit::<EdgeInherited>(ac)
            .opt_a(None)
            .opt_b(None)
            .tag64(1)
            .local_u16(55);
        (p, ac, uc, gc)
    };

    // Root defaults include WideOpt #[default(Some(Rgba::WHITE))].
    let root_r = ResolvedRow::<EdgeInherited>::root(&store);
    assert_eq!(root_r.get().opt_a(), None);
    assert_eq!(root_r.get().opt_b(), Some(Rgba::WHITE));

    let p_own = store.try_row::<EdgeInherited>(parent).unwrap();
    assert!(p_own.is_font_family_explicit());
    assert!(p_own.is_blob_explicit());

    let p_r = ResolvedRow::<EdgeInherited>::resolve(&store, parent, &root_r);
    assert_eq!(p_r.get().local_u16(), 99);
    assert_eq!(p_r.get().local_opt(), Some(77));
    assert_eq!(p_r.get().local_str(), "parent-only");
    assert_eq!(p_r.get().font_family(), "Fira Code");
    assert_eq!(store.resource::<EdgeBlob>(p_r.get().blob()), Some(42));

    // Unattached child inherits inherited fields, NEVER #[no_inherit] fields!
    let uc_r = ResolvedRow::<EdgeInherited>::resolve(&store, unattached_child, &p_r);
    assert_eq!(uc_r.get().opt_a(), Some(2.5));
    assert_eq!(uc_r.get().opt_b(), Some(Rgba::rgb(1, 2, 3)));
    assert_eq!(uc_r.get().tag64(), 0xDEAD_BEEF_CAFE_BABEu64);
    assert_eq!(uc_r.get().font_family(), "Fira Code");
    assert_eq!(uc_r.get().local_u16(), 0);
    assert_eq!(uc_r.get().local_opt(), None);
    assert_eq!(uc_r.get().local_str(), "");
    assert!(!uc_r.get().is_local_u16_explicit());
    assert!(!uc_r.get().is_local_opt_explicit());
    assert!(!uc_r.get().is_local_str_explicit());

    // Attached child sees its own overrides + own local_u16, NOT parent's local_opt/local_str.
    let ac_r = ResolvedRow::<EdgeInherited>::resolve(&store, attached_child, &p_r);
    assert_eq!(ac_r.get().opt_a(), None);
    assert_eq!(ac_r.get().opt_b(), None);
    assert_eq!(ac_r.get().tag64(), 1);
    assert_eq!(ac_r.get().local_u16(), 55);
    assert!(ac_r.get().is_local_u16_explicit());
    assert_eq!(ac_r.get().local_opt(), None);
    assert_eq!(ac_r.get().local_str(), "");

    // Grandchild under attached_child must NOT inherit attached_child's local_u16!
    let gc_r = ResolvedRow::<EdgeInherited>::resolve(&store, grandchild, &ac_r);
    assert_eq!(gc_r.get().opt_b(), None);
    assert_eq!(gc_r.get().tag64(), 1);
    assert_eq!(gc_r.get().local_u16(), 0);
    assert!(!gc_r.get().is_local_u16_explicit());

    // Clear WideOpt + u64 on attached_child -> reverts to parent's values!
    {
        let mut tx = store.tx();
        tx.edit::<EdgeInherited>(attached_child)
            .clear_opt_a()
            .clear_opt_b()
            .clear_tag64();
    }
    let root_r = ResolvedRow::<EdgeInherited>::root(&store);
    let p_r = ResolvedRow::<EdgeInherited>::resolve(&store, parent, &root_r);
    let ac_after = ResolvedRow::<EdgeInherited>::resolve(&store, attached_child, &p_r);
    assert_eq!(ac_after.get().opt_a(), Some(2.5));
    assert_eq!(ac_after.get().opt_b(), Some(Rgba::rgb(1, 2, 3)));
    assert_eq!(ac_after.get().tag64(), 0xDEAD_BEEF_CAFE_BABEu64);
    assert_eq!(ac_after.get().local_u16(), 55);
}

#[test]
fn inherited_32_field_boundary_idx_31() {
    #[allow(dead_code)]
    #[derive(crate::NveTable)]
    #[nve_table(name = "test.inherited.fields32", storage = "sparse", inherited)]
    struct Fields32 {
        #[effect(paint)] f0: bool,  #[effect(paint)] f1: bool,  #[effect(paint)] f2: bool,  #[effect(paint)] f3: bool,
        #[effect(paint)] f4: bool,  #[effect(paint)] f5: bool,  #[effect(paint)] f6: bool,  #[effect(paint)] f7: bool,
        #[effect(paint)] f8: bool,  #[effect(paint)] f9: bool,  #[effect(paint)] f10: bool, #[effect(paint)] f11: bool,
        #[effect(paint)] f12: bool, #[effect(paint)] f13: bool, #[effect(paint)] f14: bool, #[effect(paint)] f15: bool,
        #[effect(paint)] f16: bool, #[effect(paint)] f17: bool, #[effect(paint)] f18: bool, #[effect(paint)] f19: bool,
        #[effect(paint)] f20: bool, #[effect(paint)] f21: bool, #[effect(paint)] f22: bool, #[effect(paint)] f23: bool,
        #[effect(paint)] f24: bool, #[effect(paint)] f25: bool, #[effect(paint)] f26: bool, #[effect(paint)] f27: bool,
        #[effect(paint)] f28: bool, #[effect(paint)] f29: bool, #[effect(paint)] f30: bool,
        #[effect(measure)] f31: u32,
    }
    assert_eq!(Fields32::DESC.fields.len(), 32);
    assert_eq!(<Fields32 as InheritedTable>::INHERITED_FIELD_MASK, u32::MAX);

    let mut store = NodeStore::new();
    let (p, c) = {
        let mut tx = store.tx();
        let p = tx.create();
        let c = tx.create();
        tx.set_children(NodeId::ROOT, &[p]);
        tx.set_children(p, &[c]);
        tx.edit::<Fields32>(p).f31(9001);
        tx.edit::<Fields32>(c).f31(0); // explicit default on bit 31
        (p, c)
    };
    let root_r = ResolvedRow::<Fields32>::root(&store);
    let p_r = ResolvedRow::<Fields32>::resolve(&store, p, &root_r);
    let c_r = ResolvedRow::<Fields32>::resolve(&store, c, &p_r);
    assert_eq!(p_r.get().explicit_mask(), 1u32 << 31);
    assert!(c_r.get().is_f31_explicit());
    assert_eq!(c_r.get().f31(), 0);

    {
        let mut tx = store.tx();
        tx.edit::<Fields32>(c).clear_f31();
    }
    let root_r = ResolvedRow::<Fields32>::root(&store);
    let p_r = ResolvedRow::<Fields32>::resolve(&store, p, &root_r);
    let c_cleared = ResolvedRow::<Fields32>::resolve(&store, c, &p_r);
    assert!(!store.try_row::<Fields32>(c).unwrap().is_f31_explicit());
    assert_eq!(c_cleared.get().f31(), 9001);
}

#[test]
fn nve2_wire_clear_flip_string_release_encode_full_and_dynamic_inherited_replica() {
    let mut src = NodeStore::new();
    let mut dst = NodeStore::replica();
    let mut changes = ChangeSet::default();
    let mut dst_changes = ChangeSet::default();
    let mut enc = WireEncoder::default();
    let mut dec = WireDecoder::default();
    let mut wire = Vec::new();

    let (parent, child) = {
        let mut tx = src.tx();
        let p = tx.create();
        let c = tx.create();
        tx.set_children(NodeId::ROOT, &[p]);
        tx.set_children(p, &[c]);
        tx.edit::<EdgeInherited>(p)
            .color(Rgba::WHITE)
            .font_family("Inter");
        // Child explicitly writes default BLACK (word 1 == default, only word 0 differs!) + "Mono".
        tx.edit::<EdgeInherited>(c)
            .color(Rgba::BLACK)
            .font_family("Mono");
        tx.commit_into(&mut changes);
        (p, c)
    };
    enc.encode(&src, &changes, &mut wire);
    {
        let mut rtx = dst.tx();
        dec.apply(&mut rtx, &wire).expect("frame 1");
        rtx.commit_into(&mut dst_changes);
    }

    // 1. `encode_full()` keyframe must preserve child's explicit-default word 0 (`color = BLACK`).
    let mut full_enc = WireEncoder::default();
    let mut full_wire = Vec::new();
    full_enc.encode_full(&src, &mut full_wire);
    let mut full_dst = NodeStore::replica();
    let mut full_dec = WireDecoder::default();
    {
        let mut rtx = full_dst.tx();
        full_dec.apply(&mut rtx, &full_wire).expect("full keyframe");
        let mut fc = ChangeSet::default();
        rtx.commit_into(&mut fc);
    }
    assert!(full_dst.try_row::<EdgeInherited>(child).unwrap().is_color_explicit());
    let fr = ResolvedRow::<EdgeInherited>::root(&full_dst);
    let fp = ResolvedRow::<EdgeInherited>::resolve(&full_dst, parent, &fr);
    let fc = ResolvedRow::<EdgeInherited>::resolve(&full_dst, child, &fp);
    assert_eq!(fc.get().color(), Rgba::BLACK);
    assert_eq!(fc.get().font_family(), "Mono");

    // 2. Incremental `clear_color()` + `clear_font_family()` over wire (1->0 flip + string release).
    let live_before = dst.text_arena().live();
    {
        let mut tx = src.tx();
        tx.edit::<EdgeInherited>(child)
            .clear_color()
            .clear_font_family();
        tx.commit_into(&mut changes);
    }
    wire.clear();
    enc.encode(&src, &changes, &mut wire);
    {
        let mut rtx = dst.tx();
        dec.apply(&mut rtx, &wire).expect("frame 2 clear");
        rtx.commit_into(&mut dst_changes);
    }
    let c_idx = dst_changes
        .touched
        .iter()
        .position(|&id| id == child)
        .expect("clearing explicit color + font_family must touch replica child");
    assert_eq!(dst_changes.effects[c_idx], effect::PAINT | effect::MEASURE);
    assert!(
        dst.text_arena().live() < live_before,
        "clearing font_family over wire must release string bytes"
    );
    let child_own = dst.try_row::<EdgeInherited>(child).unwrap();
    assert!(!child_own.is_color_explicit());
    assert!(!child_own.is_font_family_explicit());
    let dr = ResolvedRow::<EdgeInherited>::root(&dst);
    let dp = ResolvedRow::<EdgeInherited>::resolve(&dst, parent, &dr);
    let dc = ResolvedRow::<EdgeInherited>::resolve(&dst, child, &dp);
    assert_eq!(dc.get().color(), Rgba::WHITE);
    assert_eq!(dc.get().font_family(), "Inter");

    // 3. Dynamic wire-declared tables: inherited flag enables put_explicit_word; non-inherited
    //    table with word_effects[0] == 0 does NOT misroute through put_explicit_word.
    let build_dyn_frame = |frame_no: u32,
                           wire_slot: u8,
                           name: &str,
                           flags: u8,
                           word_fx: [u8; 2],
                           node: Option<NodeId>,
                           patch_w0: Option<u32>|
     -> Vec<u8> {
        let mut ops = Vec::<Vec<u8>>::new();
        let push_u32 = |v: &mut Vec<u8>, x: u32| v.extend_from_slice(&x.to_le_bytes());

        // Op::DeclareTables (1 table)
        let mut decl = Vec::new();
        let key = Key64::of(name);
        push_u32(&mut decl, key.lo());
        push_u32(&mut decl, key.hi());
        push_u32(&mut decl, key.lo() ^ 0x1234); // schema_hash lo
        push_u32(&mut decl, key.hi() ^ 0x5678); // schema_hash hi
        push_u32(
            &mut decl,
            (wire_slot as u32)
                | ((Storage::Sparse as u32) << 8)
                | ((flags as u32) << 16)
                | (2u32 << 24),
        );
        push_u32(&mut decl, 0); // str_words
        push_u32(&mut decl, 0); // resource_words
        push_u32(&mut decl, 0); // default[0]
        push_u32(&mut decl, 0); // default[1]
        decl.extend_from_slice(&[word_fx[0], word_fx[1], 0, 0]);
        push_u32(&mut decl, name.len() as u32);
        decl.extend_from_slice(name.as_bytes());
        while !decl.len().is_multiple_of(4) {
            decl.push(0);
        }
        let mut op0 = Vec::new();
        push_u32(&mut op0, (crate::wire::Op::DeclareTables as u32) | (1u32 << 16));
        push_u32(&mut op0, (decl.len() / 4) as u32);
        op0.extend_from_slice(&decl);
        ops.push(op0);

        if let Some(nid) = node {
            let mut op1 = Vec::new();
            push_u32(&mut op1, (crate::wire::Op::CreateNodes as u32) | (1u32 << 16));
            push_u32(&mut op1, 1);
            push_u32(&mut op1, nid.raw());
            ops.push(op1);

            if let Some(w0) = patch_w0 {
                let mut op2 = Vec::new();
                push_u32(
                    &mut op2,
                    (crate::wire::Op::Patch as u32) | ((wire_slot as u32) << 8) | (1u32 << 16),
                );
                push_u32(&mut op2, 3);
                push_u32(&mut op2, nid.raw());
                push_u32(&mut op2, 1); // mask: word 0 only
                push_u32(&mut op2, w0);
                ops.push(op2);
            }
        }

        let body_bytes: usize = ops.iter().map(Vec::len).sum();
        let total_words = (crate::wire::HEADER_WORDS + body_bytes / 4) as u32;
        let mut out = Vec::with_capacity((total_words as usize) * 4);
        push_u32(&mut out, crate::wire::MAGIC);
        push_u32(&mut out, u32::from(crate::wire::VERSION));
        push_u32(&mut out, frame_no);
        push_u32(&mut out, 0);
        push_u32(&mut out, ops.len() as u32);
        push_u32(&mut out, total_words);
        for op in ops {
            out.extend_from_slice(&op);
        }
        out
    };

    let mut dyn_store = NodeStore::replica();
    let mut dyn_dec = WireDecoder::default();
    let n_inh = NodeId::from_raw_or_null(10);
    let n_plain = NodeId::from_raw_or_null(11);

    // 3a. Corrupt dynamic inherited descriptor with non-zero word_effects[0] is rejected.
    let bad = build_dyn_frame(
        1,
        5,
        "dyn.bad_inh",
        table_flags::INHERITED,
        [effect::PAINT, effect::PAINT],
        None,
        None,
    );
    {
        let mut rtx = dyn_store.tx();
        assert!(matches!(
            dyn_dec.apply(&mut rtx, &bad),
            Err(WireError::SchemaMismatch { .. })
        ));
    }

    // 3b. Valid dynamic inherited table + valid dynamic non-inherited table with word_effects[0]==0.
    let f_inh = build_dyn_frame(
        2,
        6,
        "dyn.valid_inh",
        table_flags::INHERITED,
        [0, effect::PAINT],
        Some(n_inh),
        None,
    );
    let f_plain = build_dyn_frame(
        3,
        7,
        "dyn.plain_zero_fx",
        0,
        [0, effect::PAINT],
        Some(n_plain),
        None,
    );
    let mut dyn_c = ChangeSet::default();
    {
        let mut rtx = dyn_store.tx();
        dyn_dec.apply(&mut rtx, &f_inh).expect("declare dyn inh");
        dyn_dec.apply(&mut rtx, &f_plain).expect("declare dyn plain");
        rtx.commit_into(&mut dyn_c);
    }

    // Attach rows in frame 4 (no-op empty patch to initialize defaults, then clear ChangeSet).
    let patch_only = |frame_no: u32, wire_slot: u8, nid: NodeId, mask: u32, w0: Option<u32>| -> Vec<u8> {
        let mut out = Vec::new();
        let push_u32 = |v: &mut Vec<u8>, x: u32| v.extend_from_slice(&x.to_le_bytes());
        let payload_words = 2 + u32::from(w0.is_some());
        let total_words = (crate::wire::HEADER_WORDS as u32) + 2 + payload_words;
        push_u32(&mut out, crate::wire::MAGIC);
        push_u32(&mut out, u32::from(crate::wire::VERSION));
        push_u32(&mut out, frame_no);
        push_u32(&mut out, 0);
        push_u32(&mut out, 1);
        push_u32(&mut out, total_words);
        push_u32(
            &mut out,
            (crate::wire::Op::Patch as u32) | ((wire_slot as u32) << 8) | (1u32 << 16),
        );
        push_u32(&mut out, payload_words);
        push_u32(&mut out, nid.raw());
        push_u32(&mut out, mask);
        if let Some(v) = w0 {
            push_u32(&mut out, v);
        }
        out
    };
    {
        let mut rtx = dyn_store.tx();
        dyn_dec.apply(&mut rtx, &patch_only(4, 6, n_inh, 0, None)).unwrap();
        dyn_dec.apply(&mut rtx, &patch_only(5, 7, n_plain, 0, None)).unwrap();
        rtx.commit_into(&mut dyn_c);
    }

    // Frame 6: patch ONLY word 0 on both nodes.
    // Dynamic inherited table MUST emit effect::PAINT; non-inherited zero-fx table must NOT!
    {
        let mut rtx = dyn_store.tx();
        dyn_dec.apply(&mut rtx, &patch_only(6, 6, n_inh, 1, Some(1))).unwrap();
        dyn_dec.apply(&mut rtx, &patch_only(7, 7, n_plain, 1, Some(42))).unwrap();
        rtx.commit_into(&mut dyn_c);
    }
    let inh_pos = dyn_c
        .touched
        .iter()
        .position(|&id| id == n_inh)
        .expect("dynamic inherited word-0 flip must touch node");
    assert_eq!(dyn_c.effects[inh_pos], effect::PAINT);
    assert!(
        !dyn_c.touched.contains(&n_plain),
        "non-inherited table with word_effects[0]==0 must not spuriously fire table effects on word-0 write"
    );
}
