//! Unit & hardening tests for table-generic [`RowPatch`], flat-buffer [`ResourceStore`]
//! nested child refcounting, cycle rejection, topological `NVE2` wire transfer, and adversarial
//! corruption guards.

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::wire::{Op, WireDecoder, WireEncoder, WireError};
use crate::{
    ChangeSet, FlatResource, GridLine, Length, NodeId, NodeStore, Res, ResolvedRow, ResourceId,
    Rgba, RowPatch, RowPatchView, Str, ValidatedWords, WordEnum, WordMask,
};

fn commit_tx(tx: crate::Transaction<'_>) -> ChangeSet {
    let mut c = ChangeSet::default();
    tx.commit_into(&mut c);
    c
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, WordEnum)]
enum PatchAlign {
    #[default]
    Start = 0,
    Center = 1,
    End = 2,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChildBlob(Vec<u32>);

impl FlatResource for ChildBlob {
    const KIND: nve_abi::Key64 = nve_abi::Key64::of("test.patch.child_blob");
    type View<'a> = &'a [u32];

    fn write_words(&self, out: &mut Vec<u32>) -> bool {
        if self.0.is_empty() || self.0.len() > 64 {
            return false;
        }
        out.extend_from_slice(&self.0);
        true
    }

    fn validate(words: &[u32]) -> bool {
        !words.is_empty() && words.len() <= 64
    }

    fn view(words: ValidatedWords<'_, Self>) -> &[u32] {
        words.as_slice()
    }
}

#[allow(dead_code)]
#[derive(crate::NveTable)]
#[nve_table(name = "test.patch.box", storage = "sparse", flags = "publish")]
struct PatchBox {
    #[effect(layout)]
    #[default(Length::AUTO)]
    width: Length,
    #[effect(paint)]
    bg: Rgba,
    #[effect(layout)]
    align: PatchAlign,
    #[effect(layout)]
    wrap: bool,
    #[effect(layout)]
    grid: GridLine,
    #[effect(paint)]
    opt_tint: Option<Rgba>,
    #[effect(paint)]
    stamp: u64,
    #[effect(semantics)]
    label: Str,
    #[effect(paint)]
    child_blob: Res<ChildBlob>,
    #[effect(paint)]
    patch_res: Res<RowPatch>,
}

#[allow(dead_code)]
#[derive(crate::NveTable)]
#[nve_table(name = "test.patch.typo", storage = "sparse", flags = "publish", inherited)]
struct PatchTypo {
    #[effect(paint)]
    #[default(Rgba::BLACK)]
    color: Rgba,
    #[effect(measure)]
    #[default(16.0)]
    font_size: f32,
    #[effect(measure)]
    #[default(400u16)]
    font_weight: u16,
    #[effect(measure)]
    family: Str,
}

#[test]
fn row_patch_preserves_packed_siblings_wide_opt_64bit_and_inherited_explicit_mask() {
    let mut patch = RowPatch::new();
    patch
        .set(PatchBox::ALIGN, PatchAlign::End)
        .set(PatchBox::OPT_TINT, Some(Rgba::rgb(11, 22, 33)))
        .set(PatchBox::STAMP, 0x0123_4567_89AB_CDEF)
        .set(PatchBox::GRID, GridLine::Span(3))
        .set(PatchTypo::COLOR, Rgba::WHITE)
        .set(PatchTypo::FONT_WEIGHT, 700);

    // Encode to flat words, validate, and apply via zero-copy RowPatchView.
    let mut words = Vec::new();
    assert!(patch.write_words(&mut words));
    assert!(RowPatch::validate(&words));
    let view = RowPatchView::from_words(&words).expect("valid patch words");

    assert_eq!(view.get_prop(PatchBox::ALIGN), Some(PatchAlign::End));
    assert_eq!(view.get_prop(PatchBox::WRAP), None);
    assert_eq!(view.get_prop(PatchBox::OPT_TINT), Some(Some(Rgba::rgb(11, 22, 33))));
    assert_eq!(view.get_prop(PatchBox::STAMP), Some(0x0123_4567_89AB_CDEF));
    assert_eq!(view.get_prop(PatchBox::GRID), Some(GridLine::Span(3)));
    assert_eq!(view.get_prop(PatchTypo::COLOR), Some(Rgba::WHITE));
    assert_eq!(view.get_prop(PatchTypo::FONT_SIZE), None);

    // Apply onto a row where sibling `wrap = true` shares the packed word with `align`:
    // `wrap` MUST remain intact!
    let mut store = NodeStore::new();
    let (n, child_n) = {
        let mut tx = store.tx();
        let n = tx.create();
        let c = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.set_children(n, &[c]);
        tx.edit::<PatchBox>(n)
            .wrap(true)
            .bg(Rgba::rgb(5, 6, 7))
            .label("keep-str");
        tx.edit::<PatchTypo>(n).font_size(24.0).family("Inter");
        commit_tx(tx);
        (n, c)
    };

    // Apply via Transaction::apply_row_patch and verify packed sibling & Str preservation.
    {
        let mut tx = store.tx();
        tx.apply_row_patch(n, view);
        commit_tx(tx);
    }
    let row = store.row::<PatchBox>(n);
    assert_eq!(row.align(), PatchAlign::End);
    assert!(row.wrap(), "packed sibling bit must stay untouched");
    assert_eq!(row.bg(), Rgba::rgb(5, 6, 7));
    assert_eq!(row.opt_tint(), Some(Rgba::rgb(11, 22, 33)));
    assert_eq!(row.stamp(), 0x0123_4567_89AB_CDEF);
    assert_eq!(row.grid(), GridLine::Span(3));
    assert_eq!(row.label(), "keep-str");

    // Inherited ResolvedRow::apply_patch propagates explicit bit to descendants!
    let root_typo = ResolvedRow::<PatchTypo>::root(&store);
    let mut parent_typo = ResolvedRow::<PatchTypo>::resolve(&store, n, &root_typo);
    assert_eq!(parent_typo.get().color(), Rgba::WHITE);
    assert_eq!(parent_typo.get().font_size(), 24.0);
    assert_eq!(parent_typo.get().font_weight(), 700);
    assert_eq!(parent_typo.get().family(), "Inter");

    let mut override_p = RowPatch::new();
    override_p.set(PatchTypo::COLOR, Rgba::BLACK); // explicit table default!
    let mut ov_words = Vec::new();
    assert!(override_p.write_words(&mut ov_words));
    assert!(parent_typo.apply_patch(RowPatchView::from_words(&ov_words).unwrap()));
    let child_typo = ResolvedRow::<PatchTypo>::resolve(&store, child_n, &parent_typo);
    assert_eq!(child_typo.get().color(), Rgba::BLACK);
    assert_eq!(child_typo.get().font_size(), 24.0);
    assert_eq!(child_typo.get().font_weight(), 700);
    assert_eq!(child_typo.get().family(), "Inter");
}

#[test]
fn nested_resource_refcounting_temp_scope_wire_topo_sort_and_reclaim() {
    let mut src = NodeStore::new();
    let mut dst = NodeStore::replica();
    let mut enc = WireEncoder::default();
    let mut dec = WireDecoder::default();
    let mut wire = Vec::new();
    let mut dst_changes = ChangeSet::default();

    let (n, child_id, patch_id) = {
        let mut tx = src.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        let (cid, pid) = tx.with_temp_resources(|scope| {
            let cid = scope.intern(&ChildBlob(vec![10, 20, 30]));
            let mut p = RowPatch::new();
            p.set(PatchBox::BG, Rgba::rgb(1, 2, 3))
                .set_res_id(PatchBox::CHILD_BLOB, cid);
            let mut sub_tx = scope.tx();
            sub_tx.edit::<PatchBox>(n).patch_res(Some(p));
            let pid = sub_tx.store().row::<PatchBox>(n).patch_res();
            (cid, pid)
        });
        let c = commit_tx(tx);
        enc.encode(&src, &c, &mut wire);
        let mut rtx = dst.tx();
        dec.apply(&mut rtx, &wire).expect("decode nested resource frame");
        rtx.commit_into(&mut dst_changes);
        (n, cid, pid)
    };

    // Both primary and replica own exactly 2 live resources (parent RowPatch + child ChildBlob).
    assert_eq!(src.stats().resources_live, 2);
    assert_eq!(dst.stats().resources_live, 2);
    let view = dst.resource::<RowPatch>(patch_id).expect("replica RowPatch view");
    assert_eq!(view.get_res_id(PatchBox::CHILD_BLOB), Some(child_id));
    assert_eq!(dst.resource::<ChildBlob>(child_id), Some(&[10u32, 20, 30][..]));

    // Full keyframe (`encode_full`) must also transitively discover `child_id` and emit it before `patch_id`.
    let mut keyframe = Vec::new();
    let mut kf_enc = WireEncoder::default();
    kf_enc.encode_full(&src, &mut keyframe);
    let mut kf_dst = NodeStore::replica();
    let mut kf_dec = WireDecoder::default();
    {
        let mut rtx = kf_dst.tx();
        kf_dec.apply(&mut rtx, &keyframe).expect("keyframe with nested resources");
        commit_tx(rtx);
    }
    assert_eq!(kf_dst.stats().resources_live, 2);
    assert_eq!(kf_dst.resource::<ChildBlob>(child_id), Some(&[10u32, 20, 30][..]));

    // Ephemeral parent updated in-place to reference a brand-new child in the SAME frame:
    // parent id was pushed to `defined` before child -> topological sort must emit child before parent!
    let new_child_id = {
        let mut tx = src.tx();
        let c0 = tx.with_temp_resources(|scope| {
            let cid = scope.intern(&ChildBlob(vec![99, 100]));
            let mut p = RowPatch::new();
            p.set_res_id(PatchBox::CHILD_BLOB, cid);
            scope.tx().edit::<PatchBox>(n).patch_res_ephemeral(p);
            cid
        });
        let c = commit_tx(tx);
        wire.clear();
        enc.encode(&src, &c, &mut wire);
        let mut rtx = dst.tx();
        dec.apply(&mut rtx, &wire).expect("decode ephemeral switch");
        rtx.commit_into(&mut dst_changes);
        c0
    };
    assert_eq!(dst.resource::<ChildBlob>(new_child_id), Some(&[99u32, 100][..]));

    // Now remove node `n` -> cascading release drops both parent and child to 0 live resources & 0 slab words.
    {
        let mut tx = src.tx();
        tx.remove(n);
        let c = commit_tx(tx);
        wire.clear();
        enc.encode(&src, &c, &mut wire);
        let mut rtx = dst.tx();
        dec.apply(&mut rtx, &wire).expect("decode remove cascade");
        rtx.commit_into(&mut dst_changes);
    }
    assert_eq!(src.stats().resources_live, 0);
    assert_eq!(src.resources().words_live(), 0);
    assert_eq!(src.resources().child_edges_live(), 0);
    assert_eq!(dst.stats().resources_live, 0);
    assert_eq!(dst.resources().words_live(), 0);
    assert_eq!(dst.resources().child_edges_live(), 0);
}

#[test]
fn row_patch_validation_rejects_corrupt_and_hostile_wire_inputs() {
    // Empty slice, trailing words, non-zero masked-out bits, invalid flags, non-monotonic tables/words,
    // resource op with non-FULL clear_mask, and undeclared wire child_ids must all fail cleanly.
    assert!(!RowPatch::validate(&[]));
    assert!(RowPatch::validate(&[0]));
    assert!(!RowPatch::validate(&[0, 1])); // trailing word
    assert!(!RowPatch::validate(&[33])); // > MAX_PATCH_TABLES

    // Bad set_bits outside clear_mask.
    let key = PatchBox::KEY;
    assert!(!RowPatch::validate(&[1, key.lo(), key.hi(), 1, 0, 0x0F, 0xFF]));
    // Bad resource op with partial clear_mask.
    assert!(!RowPatch::validate(&[
        1,
        key.lo(),
        key.hi(),
        1,
        1u32 << 16,
        0x0000_FFFF,
        1
    ]));
    // Bad unknown flag = 2.
    assert!(!RowPatch::validate(&[
        1,
        key.lo(),
        key.hi(),
        1,
        2u32 << 16,
        u32::MAX,
        1
    ]));
    // Duplicate / non-ascending word index.
    assert!(!RowPatch::validate(&[
        1,
        key.lo(),
        key.hi(),
        2,
        1,
        u32::MAX,
        0,
        1,
        u32::MAX,
        0,
    ]));

    // Wire `define_at` rejects undeclared child references at `get::<RowPatch>` and rejects cycles.
    let mut replica = NodeStore::replica();
    let child_bytes = 42u32.to_le_bytes();
    assert!(replica
        .resources
        .define_at(ResourceId(1), ChildBlob::KIND, false, &[], &child_bytes));
    let mut rp = RowPatch::new();
    rp.set_res_id(PatchBox::CHILD_BLOB, ResourceId(1));
    let mut rp_words = Vec::new();
    assert!(rp.write_words(&mut rp_words));
    let rp_bytes = crate::words_as_le_bytes(&rp_words).to_vec();
    // Define RowPatch WITHOUT listing ResourceId(1) in child_ids -> `get::<RowPatch>` MUST reject!
    assert!(replica
        .resources
        .define_at(ResourceId(2), RowPatch::KIND, false, &[], &rp_bytes));
    assert!(
        replica.resource::<RowPatch>(ResourceId(2)).is_none(),
        "wire RowPatch with undeclared child ResourceId must fail validation on get"
    );
    // Redefine WITH ResourceId(1) in child_ids -> `get::<RowPatch>` succeeds!
    assert!(replica.resources.define_at(
        ResourceId(2),
        RowPatch::KIND,
        false,
        &[ResourceId(1)],
        &rp_bytes
    ));
    assert!(replica.resource::<RowPatch>(ResourceId(2)).is_some());

    // Self-cycle and 2-node cycle on `define_at` MUST be rejected.
    assert!(!replica.resources.define_at(
        ResourceId(2),
        RowPatch::KIND,
        false,
        &[ResourceId(2)],
        &rp_bytes
    ));
    assert!(!replica.resources.define_at(
        ResourceId(1),
        ChildBlob::KIND,
        false,
        &[ResourceId(2)],
        &child_bytes
    ));

    // Hostile NVE2 DefineResources with non-zero reserved flags or misaligned byte_len rejected.
    let mut body = Vec::new();
    body.extend_from_slice(&1u32.to_le_bytes());
    body.extend_from_slice(&ChildBlob::KIND.lo().to_le_bytes());
    body.extend_from_slice(&ChildBlob::KIND.hi().to_le_bytes());
    body.extend_from_slice(&2u32.to_le_bytes()); // invalid flags bit 1!
    body.extend_from_slice(&0u32.to_le_bytes()); // child_count = 0
    body.extend_from_slice(&4u32.to_le_bytes()); // byte_len = 4
    body.extend_from_slice(&1u32.to_le_bytes());
    let mut frame = Vec::new();
    frame.extend_from_slice(&crate::wire::MAGIC.to_le_bytes());
    frame.extend_from_slice(&(crate::wire::VERSION as u32).to_le_bytes());
    frame.extend_from_slice(&1u64.to_le_bytes());
    frame.extend_from_slice(&1u32.to_le_bytes());
    let total_words = (6 + 2 + body.len() / 4) as u32;
    frame.extend_from_slice(&total_words.to_le_bytes());
    let head = (Op::DefineResources as u32) | (1u32 << 16);
    frame.extend_from_slice(&head.to_le_bytes());
    frame.extend_from_slice(&((body.len() / 4) as u32).to_le_bytes());
    frame.extend_from_slice(&body);
    let mut dec = WireDecoder::default();
    let mut r2 = NodeStore::replica();
    let mut tx = r2.tx();
    assert_eq!(
        dec.apply(&mut tx, &frame),
        Err(WireError::Resource {
            id: 1,
            kind: ChildBlob::KIND.0
        })
    );
}

#[test]
fn wire_resource_validation_is_cached_once_across_repeated_gets() {
    static VALIDATE_CALLS: AtomicUsize = AtomicUsize::new(0);

    #[derive(Debug, Clone)]
    struct CountedBlob(u32);

    impl FlatResource for CountedBlob {
        const KIND: nve_abi::Key64 = nve_abi::Key64::of("test.patch.counted_blob");
        type View<'a> = u32;

        fn write_words(&self, out: &mut Vec<u32>) -> bool {
            out.push(self.0);
            true
        }
        fn validate(words: &[u32]) -> bool {
            VALIDATE_CALLS.fetch_add(1, Ordering::Relaxed);
            words.len() == 1
        }
        fn view(words: ValidatedWords<'_, Self>) -> u32 {
            words[0]
        }
    }

    let mut replica = NodeStore::replica();
    let payload = 0x1234_5678u32.to_le_bytes();
    assert!(replica
        .resources
        .define_at(ResourceId::new(1, 0), CountedBlob::KIND, false, &[], &payload));
    replica.resources.retain(ResourceId::new(1, 0));

    VALIDATE_CALLS.store(0, Ordering::Relaxed);
    for _ in 0..100 {
        assert_eq!(
            replica.resource::<CountedBlob>(ResourceId::new(1, 0)),
            Some(0x1234_5678)
        );
    }
    assert_eq!(
        VALIDATE_CALLS.load(Ordering::Relaxed),
        1,
        "wire-origin ResourceStore::get must validate at most once and cache via OnceLock"
    );
}

#[test]
fn three_node_indirect_cycle_live_child_swap_and_ephemeral_cycle_fallback() {
    let mut replica = NodeStore::replica();
    let w = 7u32.to_le_bytes();
    let a = ResourceId::new(1, 0);
    let b = ResourceId::new(2, 0);
    let c = ResourceId::new(3, 0);
    assert!(replica.resources.define_at(c, ChildBlob::KIND, false, &[], &w));
    assert!(replica.resources.define_at(b, ChildBlob::KIND, false, &[c], &w));
    assert!(replica.resources.define_at(a, ChildBlob::KIND, false, &[b], &w));
    replica.resources.retain(a);
    assert_eq!(replica.stats().resources_live, 3);

    // 1. Direct `define_at` rejects 3-node indirect cycle C -> A (forming A -> B -> C -> A)
    //    without altering existing store state or refcounts.
    assert!(!replica.resources.define_at(c, ChildBlob::KIND, false, &[a], &w));
    assert_eq!(replica.stats().resources_live, 3);
    assert_eq!(replica.resources().children_of(c), &[]);

    // 2. Crafted NVE2 wire frame attempting the same 3-node indirect cycle C -> A MUST fail cleanly.
    let mut body = Vec::new();
    body.extend_from_slice(&c.0.to_le_bytes());
    body.extend_from_slice(&ChildBlob::KIND.lo().to_le_bytes());
    body.extend_from_slice(&ChildBlob::KIND.hi().to_le_bytes());
    body.extend_from_slice(&0u32.to_le_bytes()); // flags = 0
    body.extend_from_slice(&1u32.to_le_bytes()); // child_count = 1
    body.extend_from_slice(&a.0.to_le_bytes()); // child = A -> cycle!
    body.extend_from_slice(&4u32.to_le_bytes());
    body.extend_from_slice(&w);
    let mut frame = Vec::new();
    frame.extend_from_slice(&crate::wire::MAGIC.to_le_bytes());
    frame.extend_from_slice(&(crate::wire::VERSION as u32).to_le_bytes());
    frame.extend_from_slice(&1u64.to_le_bytes());
    frame.extend_from_slice(&1u32.to_le_bytes());
    frame.extend_from_slice(&((6 + 2 + body.len() / 4) as u32).to_le_bytes());
    frame.extend_from_slice(&((Op::DefineResources as u32) | (1u32 << 16)).to_le_bytes());
    frame.extend_from_slice(&((body.len() / 4) as u32).to_le_bytes());
    frame.extend_from_slice(&body);
    let mut dec = WireDecoder::default();
    {
        let mut rtx = replica.tx();
        assert_eq!(
            dec.apply(&mut rtx, &frame),
            Err(WireError::Resource {
                id: c.0,
                kind: ChildBlob::KIND.0
            })
        );
    }
    assert_eq!(replica.stats().resources_live, 3);
    assert_eq!(replica.resources().children_of(c), &[]);

    // 3. Live wire entry redefinition swapping children: A -> [B] redefined to A -> [D].
    let d = ResourceId::new(4, 0);
    assert!(replica.resources.define_at(d, ChildBlob::KIND, false, &[], &w));
    assert!(replica.resources.define_at(a, ChildBlob::KIND, false, &[d], &w));
    replica.resources.end_frame();
    assert!(replica.resources().is_live(a));
    assert!(replica.resources().is_live(d), "new child D must be retained");
    assert!(!replica.resources().is_live(b), "old child B must be released");
    assert!(!replica.resources().is_live(c), "transitive old grandchild C must be released");

    // 4. Ephemeral `creates_cycle` fresh-slot fallback in `finish_ephemeral_scratch`.
    let mut store = NodeStore::new();
    let eph = store.resources.set_ephemeral(ResourceId::NONE, &RowPatch::new());
    assert!(!eph.is_none());
    let mut self_ref = RowPatch::new();
    self_ref.set_res_id(PatchBox::PATCH_RES, eph);
    let fresh = store.resources.set_ephemeral(eph, &self_ref);
    assert!(!fresh.is_none());
    assert_ne!(
        fresh, eph,
        "ephemeral rewrite referencing `old` must fall back to a fresh slot instead of creating a cycle"
    );
    assert_eq!(store.resources().children_of(fresh), &[eph]);
}

#[test]
fn hostile_wire_cpu_budget_and_multiframe_sparse_id_ram_bound() {
    // 1. Multi-source cycle budget caps hostile chain redefinitions cleanly.
    let mut store = NodeStore::replica();
    let w = 1u32.to_le_bytes();
    for i in 1..=64u32 {
        let kids = if i == 1 {
            Vec::new()
        } else {
            vec![ResourceId::new(i - 1, 0)]
        };
        let mut budget = 10_000;
        assert_eq!(
            store.resources.define_at_with_budget(
                ResourceId::new(i, 0),
                ChildBlob::KIND,
                false,
                &kids,
                &w,
                &mut budget,
                None
            ),
            Ok(true)
        );
    }
    // Redefining node 1 with child 64 under a tiny budget (8 visits) stops early and rejects cleanly.
    let mut tight_budget = 8;
    assert_eq!(
        store.resources.define_at_with_budget(
            ResourceId::new(1, 0),
            ChildBlob::KIND,
            false,
            &[ResourceId::new(64, 0)],
            &w,
            &mut tight_budget,
            None
        ),
        Ok(false)
    );
    assert_eq!(tight_budget, 0);

    // 2. Multi-frame sparse-ID RAM amplification attack: 56-byte frames cannot jump +1023 slots/frame!
    let make_single_def_frame = |frame_no: u64, res_id: u32| -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&res_id.to_le_bytes());
        body.extend_from_slice(&ChildBlob::KIND.lo().to_le_bytes());
        body.extend_from_slice(&ChildBlob::KIND.hi().to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes()); // flags
        body.extend_from_slice(&0u32.to_le_bytes()); // child_count
        body.extend_from_slice(&4u32.to_le_bytes()); // byte_len
        body.extend_from_slice(&1u32.to_le_bytes()); // payload
        let mut frame = Vec::new();
        frame.extend_from_slice(&crate::wire::MAGIC.to_le_bytes());
        frame.extend_from_slice(&(crate::wire::VERSION as u32).to_le_bytes());
        frame.extend_from_slice(&frame_no.to_le_bytes());
        frame.extend_from_slice(&1u32.to_le_bytes());
        frame.extend_from_slice(&((6 + 2 + body.len() / 4) as u32).to_le_bytes());
        frame.extend_from_slice(&((Op::DefineResources as u32) | (1u32 << 16)).to_le_bytes());
        frame.extend_from_slice(&((body.len() / 4) as u32).to_le_bytes());
        frame.extend_from_slice(&body);
        frame
    };

    let mut victim = NodeStore::replica();
    let mut dec = WireDecoder::default();
    // Frame 1: id=1024 allowed within initial 1024 slack.
    {
        let f1 = make_single_def_frame(1, 1024);
        let mut tx = victim.tx();
        dec.apply(&mut tx, &f1).expect("frame 1 within initial slack");
        commit_tx(tx);
    }
    let slots_after_f1 = victim.resources().slots();
    // Frame 2: a tiny frame jumping to the last legal slot would need ~MAX_RESOURCE_SLOTS entries,
    // far above the session slot budget (16 MiB + frame bytes), so it MUST be rejected with Budget
    // before any allocation (the budget is byte-based: +1023 slots no longer exceeds it).
    {
        let last = crate::wire::budget::MAX_RESOURCE_SLOTS - 1;
        assert!(
            last as usize * crate::resource::ResourceStore::ENTRY_BYTES
                > crate::wire::budget::SESSION_BASE_SLOT_BYTES * 2,
            "premise: the jump outspends the budget"
        );
        let f2 = make_single_def_frame(2, last);
        let mut tx = victim.tx();
        assert_eq!(dec.apply(&mut tx, &f2), Err(WireError::Budget));
    }
    assert_eq!(victim.resources().slots(), slots_after_f1);
}

#[test]
fn resource_id_lifecycle_rejects_zombie_cross_kind_and_aba_reuse() {
    #[derive(Debug, Clone)]
    struct OtherBlob(u32);
    impl FlatResource for OtherBlob {
        const KIND: nve_abi::Key64 = nve_abi::Key64::of("test.patch.other_blob");
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

    let mut store = NodeStore::new();
    let (n1, n2, zombie_id, other_id) = {
        let mut tx = store.tx();
        let n1 = tx.create();
        let n2 = tx.create();
        tx.set_children(NodeId::ROOT, &[n1, n2]);
        tx.edit::<PatchBox>(n1).child_blob(Some(ChildBlob(vec![77])));
        let zombie_id = tx.store().row::<PatchBox>(n1).child_blob();
        let other_id = tx.store_mut().resources.intern(&OtherBlob(999));
        // Release zombie_id in the SAME frame (refs: 1 -> 0, evicted from dedup, still pending_free).
        tx.edit::<PatchBox>(n1).child_blob(None);
        let reinterned = tx.store_mut().resources.intern(&ChildBlob(vec![77]));
        assert_ne!(reinterned, zombie_id, "released zombie must be evicted from dedup");
        tx.store_mut().resources.release(reinterned);

        // 1. Same-frame zombie resurrection MUST be rejected across all local APIs.
        assert!(!tx.set_resource_id(n1, PatchBox::CHILD_BLOB, zombie_id));
        tx.edit::<PatchBox>(n1).child_blob_id(zombie_id);
        assert_eq!(tx.store().row::<PatchBox>(n1).child_blob(), ResourceId::NONE);
        let mut rp_zombie = RowPatch::new();
        rp_zombie.set_res_id(PatchBox::CHILD_BLOB, zombie_id);
        let mut z_words = Vec::new();
        assert!(rp_zombie.write_words(&mut z_words));
        tx.apply_row_patch(n2, RowPatchView::from_words(&z_words).unwrap());
        assert!(
            !tx.store().has::<PatchBox>(n2),
            "apply_row_patch must not spuriously attach table when all ops are skipped"
        );

        // 2. Cross-kind binding (OtherBlob -> PatchBox::CHILD_BLOB expecting ChildBlob) MUST fail.
        assert!(!tx.set_resource_id(n1, PatchBox::CHILD_BLOB, other_id));
        tx.edit::<PatchBox>(n1).child_blob_id(other_id);
        assert_eq!(tx.store().row::<PatchBox>(n1).child_blob(), ResourceId::NONE);
        let mut rp_wrong = RowPatch::new();
        // Force raw resource op targeting PatchBox::CHILD_BLOB with wrong-kind id.
        let mut w_wrong = vec![
            1,
            PatchBox::KEY.lo(),
            PatchBox::KEY.hi(),
            1,
            u32::from(PatchBox::CHILD_BLOB.word) | (u32::from(crate::WORD_OP_RESOURCE) << 16),
            u32::MAX,
            other_id.0,
        ];
        rp_wrong.merge_view(RowPatchView::from_words(&w_wrong).unwrap());
        w_wrong.clear();
        assert!(rp_wrong.write_words(&mut w_wrong));
        tx.apply_row_patch(n2, RowPatchView::from_words(&w_wrong).unwrap());
        assert!(!tx.store().has::<PatchBox>(n2));

        commit_tx(tx);
        (n1, n2, zombie_id, other_id)
    };
    let _ = n2;

    // 3. ABA reuse protection: after `commit_tx` (`end_frame`), `zombie_id`'s slot is recycled
    //    with generation + 1; stale `zombie_id` MUST NOT alias the newly interned ChildBlob!
    let recycled_id = {
        let mut tx = store.tx();
        tx.edit::<PatchBox>(n1).child_blob(Some(ChildBlob(vec![88])));
        let rid = tx.store().row::<PatchBox>(n1).child_blob();
        commit_tx(tx);
        rid
    };
    assert_eq!(recycled_id.index(), zombie_id.index(), "slot index reused");
    assert_ne!(
        recycled_id.generation(),
        zombie_id.generation(),
        "generation must bump on recycle"
    );
    assert_eq!(store.resource::<ChildBlob>(zombie_id), None, "stale ABA handle rejected");
    assert_eq!(store.resource::<ChildBlob>(recycled_id), Some(&[88u32][..]));
    store.resources.release(other_id);
}

#[test]
fn row_patch_unbounded_merge_word_mask_and_view_hardening() {
    // 1. WordMask::apply masks out bits outside `clear_mask`.
    assert_eq!(WordMask(0x00FF).apply(0xAA00, 0xFFFF), 0xAAFF);
    assert_eq!(WordMask(0x000F).apply(0x0000, 0x00FF), 0x000F);

    // 2. Merging two valid 20-table RowPatchViews (40 distinct tables > MAX_PATCH_TABLES = 32)
    //    must NEVER panic in memory; `write_words` returns `false` cleanly.
    let build_20_table_words = |base_key: u64| -> Vec<u32> {
        let mut w = vec![20u32];
        for i in 0..20u64 {
            let k = nve_abi::Key64(base_key + i);
            w.extend_from_slice(&[k.lo(), k.hi(), 1, 0, u32::MAX, i as u32]);
        }
        w
    };
    let w_a = build_20_table_words(100);
    let w_b = build_20_table_words(200);
    let va = RowPatchView::from_words(&w_a).expect("valid 20-table patch A");
    let vb = RowPatchView::from_words(&w_b).expect("valid 20-table patch B");
    let mut merged = RowPatch::new();
    merged.merge_view(va);
    merged.merge_view(vb);
    assert_eq!(merged.table_count(), 40);
    let mut encoded = Vec::new();
    assert!(
        !merged.write_words(&mut encoded),
        "encoding > MAX_PATCH_TABLES must fail fallibly instead of panicking"
    );

    // 3. `RowPatchView::from_words` rejects invalid slices and unknown flags / word >= 32.
    assert!(RowPatchView::from_words(&[]).is_none());
    assert!(RowPatchView::from_words(&[1, 1, 0, 1, 256, u32::MAX, 1]).is_none());
    assert!(RowPatchView::from_words(&[1, 1, 0, 1, 2u32 << 16, u32::MAX, 1]).is_none());
}

#[test]
fn apply_row_patch_inherited_rejected_resource_preserves_parent_and_explicit_mask() {
    #[derive(Debug, Clone)]
    struct WrongKindRes(u32);
    impl FlatResource for WrongKindRes {
        const KIND: nve_abi::Key64 = nve_abi::Key64::of("test.patch.inherited_wrong_kind");
        type View<'a> = u32;
        fn write_words(&self, out: &mut Vec<u32>) -> bool {
            out.push(self.0);
            true
        }
        fn validate(w: &[u32]) -> bool {
            w.len() == 1
        }
        fn view(w: ValidatedWords<'_, Self>) -> u32 {
            w[0]
        }
    }

    #[allow(dead_code)]
    #[derive(crate::NveTable)]
    #[nve_table(name = "test.patch.inherited_res", storage = "sparse", flags = "publish", inherited)]
    struct InheritedWithRes {
        #[effect(paint)]
        #[default(Rgba::BLACK)]
        color: Rgba,
        #[effect(paint)]
        blob: Res<ChildBlob>,
    }

    let mut store = NodeStore::new();
    let (parent, child_only_bad, child_mixed, child_own_res, parent_blob_id, own_blob_id, dead_id, wrong_kind_id) = {
        let mut tx = store.tx();
        let parent = tx.create();
        let child_only_bad = tx.create();
        let child_mixed = tx.create();
        let child_own_res = tx.create();
        tx.set_children(NodeId::ROOT, &[parent]);
        tx.set_children(parent, &[child_only_bad, child_mixed, child_own_res]);

        tx.edit::<InheritedWithRes>(parent)
            .color(Rgba::rgb(10, 20, 30))
            .blob(Some(ChildBlob(vec![111, 222])));
        let parent_blob_id = tx.store().row::<InheritedWithRes>(parent).blob();

        tx.edit::<InheritedWithRes>(child_own_res)
            .blob(Some(ChildBlob(vec![333])));
        let own_blob_id = tx.store().row::<InheritedWithRes>(child_own_res).blob();

        // Create a live resource of the wrong kind, and a dead/stale ChildBlob id.
        let wrong_kind_id = tx.store_mut().resources.intern(&WrongKindRes(55));
        let temp_dead = tx.store_mut().resources.intern(&ChildBlob(vec![999]));
        tx.store_mut().resources.release(temp_dead);

        commit_tx(tx);
        (
            parent,
            child_only_bad,
            child_mixed,
            child_own_res,
            parent_blob_id,
            own_blob_id,
            temp_dead,
            wrong_kind_id,
        )
    };

    // Case 1: Standalone inherited patch carrying ONLY a dead or wrong-kind `ResourceId`:
    // MUST NOT attach the table and MUST preserve parent's inherited resource.
    for bad_id in [dead_id, wrong_kind_id] {
        let mut patch = RowPatch::new();
        patch.set_res_id(InheritedWithRes::BLOB, bad_id);
        let mut words = Vec::new();
        assert!(patch.write_words(&mut words));
        let mut tx = store.tx();
        tx.apply_row_patch(child_only_bad, RowPatchView::from_words(&words).unwrap());
        commit_tx(tx);

        assert!(
            !store.has::<InheritedWithRes>(child_only_bad),
            "rejected inherited resource op must not spuriously attach table (bad_id={bad_id:?})"
        );
        let root_r = ResolvedRow::<InheritedWithRes>::root(&store);
        let parent_r = ResolvedRow::<InheritedWithRes>::resolve(&store, parent, &root_r);
        let child_r = ResolvedRow::<InheritedWithRes>::resolve(&store, child_only_bad, &parent_r);
        assert_eq!(child_r.get().blob(), parent_blob_id);
        assert_eq!(store.resource::<ChildBlob>(child_r.get().blob()), Some(&[111u32, 222][..]));
    }

    // Case 2: Mixed patch (valid scalar `color` + rejected wrong-kind `blob`):
    // `color` MUST apply and set bit 0, while `blob` bit 1 MUST remain unset so parent's
    // inherited `blob` is NOT overridden with `ResourceId::NONE`.
    {
        let mut patch = RowPatch::new();
        patch
            .set(InheritedWithRes::COLOR, Rgba::WHITE)
            .set_res_id(InheritedWithRes::BLOB, wrong_kind_id);
        let mut words = Vec::new();
        assert!(patch.write_words(&mut words));
        let mut tx = store.tx();
        tx.apply_row_patch(child_mixed, RowPatchView::from_words(&words).unwrap());
        commit_tx(tx);
    }
    let mixed_row = store.row::<InheritedWithRes>(child_mixed);
    assert!(mixed_row.is_color_explicit(), "valid scalar op must set its explicit bit");
    assert!(
        !mixed_row.is_blob_explicit(),
        "rejected resource op must NOT set word-0 explicit bit"
    );
    let root_r = ResolvedRow::<InheritedWithRes>::root(&store);
    let parent_r = ResolvedRow::<InheritedWithRes>::resolve(&store, parent, &root_r);
    let mixed_resolved = ResolvedRow::<InheritedWithRes>::resolve(&store, child_mixed, &parent_r);
    assert_eq!(mixed_resolved.get().color(), Rgba::WHITE);
    assert_eq!(
        mixed_resolved.get().blob(),
        parent_blob_id,
        "parent's inherited resource must remain intact"
    );

    // Case 3: Mixed patch on a child that ALREADY explicitly owns a valid resource:
    // rejected resource op must NOT clobber the child's existing explicit bit or resource!
    {
        let mut patch = RowPatch::new();
        patch
            .set(InheritedWithRes::COLOR, Rgba::rgb(99, 88, 77))
            .set_res_id(InheritedWithRes::BLOB, dead_id);
        let mut words = Vec::new();
        assert!(patch.write_words(&mut words));
        let mut tx = store.tx();
        tx.apply_row_patch(child_own_res, RowPatchView::from_words(&words).unwrap());
        commit_tx(tx);
    }
    let own_row = store.row::<InheritedWithRes>(child_own_res);
    assert!(own_row.is_color_explicit());
    assert!(
        own_row.is_blob_explicit(),
        "existing explicit resource bit must stay set when incoming resource op is rejected"
    );
    assert_eq!(own_row.blob(), own_blob_id);

    store.resources.release(wrong_kind_id);
}

#[test]
fn apply_row_patch_inherited_64bit_hi_word_skip_and_orphan_word0_preserve_inheritance() {
    #[allow(dead_code)]
    #[derive(crate::NveTable)]
    #[nve_table(name = "test.patch.inherited_wide64", storage = "sparse", flags = "publish", inherited)]
    struct InheritedWide64 {
        #[effect(paint)]
        #[default(Rgba::BLACK)]
        color: Rgba,
        #[effect(paint)]
        stamp: u64,
        #[effect(paint)]
        blob: Res<ChildBlob>,
    }

    let mut store = NodeStore::new();
    let (parent, child_str_hi, child_wide64_hi, child_orphan_typo, child_orphan_res, parent_blob) = {
        let mut tx = store.tx();
        let parent = tx.create();
        let child_str_hi = tx.create();
        let child_wide64_hi = tx.create();
        let child_orphan_typo = tx.create();
        let child_orphan_res = tx.create();
        tx.set_children(NodeId::ROOT, &[parent]);
        tx.set_children(
            parent,
            &[child_str_hi, child_wide64_hi, child_orphan_typo, child_orphan_res],
        );

        tx.edit::<PatchTypo>(parent)
            .color(Rgba::rgb(10, 20, 30))
            .font_size(22.0)
            .family("Inter");
        tx.edit::<InheritedWide64>(parent)
            .color(Rgba::rgb(40, 50, 60))
            .stamp(0x1122_3344_5566_7788)
            .blob(Some(ChildBlob(vec![7, 8, 9])));
        let parent_blob = tx.store().row::<InheritedWide64>(parent).blob();
        commit_tx(tx);
        (
            parent,
            child_str_hi,
            child_wide64_hi,
            child_orphan_typo,
            child_orphan_res,
            parent_blob,
        )
    };

    // Helper: construct a canonical single-table wire RowPatchView with ascending word ops.
    let build_wire_patch = |key: nve_abi::table::TableKey, ops: &[(u8, u16, u32, u32)]| -> Vec<u32> {
        let mut sorted = ops.to_vec();
        sorted.sort_by_key(|o| o.0);
        let mut w = vec![1u32, key.lo(), key.hi(), sorted.len() as u32];
        for (word, flags, clear_mask, set_bits) in sorted {
            w.push(u32::from(word) | (u32::from(flags) << 16));
            w.push(clear_mask);
            w.push(set_bits);
        }
        w
    };

    // Scenario A1: `Str` field (`PatchTypo::FAMILY`, `bits == 64`): valid scalar op on `color` +
    // skipped scalar op on `family.word + 1` (high word of the 2-word Str) + word-0 explicit bits
    // for BOTH `color` (bit 0) and `family` (bit 3). Must NOT flip `family`'s explicit bit!
    {
        let explicit_mask = (1u32 << PatchTypo::COLOR.explicit_bit)
            | (1u32 << PatchTypo::FAMILY.explicit_bit);
        let words = build_wire_patch(
            PatchTypo::KEY,
            &[
                (0, crate::WORD_OP_SCALAR, explicit_mask, explicit_mask),
                (PatchTypo::COLOR.word, crate::WORD_OP_SCALAR, u32::MAX, Rgba::WHITE.0),
                (PatchTypo::FAMILY.word + 1, crate::WORD_OP_SCALAR, u32::MAX, 123),
            ],
        );
        let view = RowPatchView::from_words(&words).expect("valid crafted Str hi-word patch");
        let mut tx = store.tx();
        tx.apply_row_patch(child_str_hi, view);
        commit_tx(tx);
    }
    let row_str = store.row::<PatchTypo>(child_str_hi);
    assert!(row_str.is_color_explicit(), "valid scalar op must set its explicit bit");
    assert!(
        !row_str.is_family_explicit(),
        "skipped op on Str f.word + 1 must NOT flip 64-bit Str explicit bit"
    );
    let root_typo = ResolvedRow::<PatchTypo>::root(&store);
    let parent_typo = ResolvedRow::<PatchTypo>::resolve(&store, parent, &root_typo);
    let resolved_str = ResolvedRow::<PatchTypo>::resolve(&store, child_str_hi, &parent_typo);
    assert_eq!(resolved_str.get().color(), Rgba::WHITE);
    assert_eq!(
        resolved_str.get().family(),
        "Inter",
        "parent's inherited font family must not be wiped"
    );

    // Scenario A2: `Wide64` (`u64`) field (`InheritedWide64::STAMP`, `bits == 64`): valid scalar
    // op on `color` + valid low word `stamp.word` + skipped mismatched `WORD_OP_RESOURCE` op on
    // `stamp.word + 1` + word-0 explicit bits for `color` and `stamp`. Must NOT flip `stamp`'s
    // explicit bit or wipe parent inheritance!
    {
        let explicit_mask = (1u32 << InheritedWide64::COLOR.explicit_bit)
            | (1u32 << InheritedWide64::STAMP.explicit_bit);
        let words = build_wire_patch(
            InheritedWide64::KEY,
            &[
                (0, crate::WORD_OP_SCALAR, explicit_mask, explicit_mask),
                (
                    InheritedWide64::COLOR.word,
                    crate::WORD_OP_SCALAR,
                    u32::MAX,
                    Rgba::rgb(1, 2, 3).0,
                ),
                (
                    InheritedWide64::STAMP.word,
                    crate::WORD_OP_SCALAR,
                    u32::MAX,
                    0xDEAD_BEEF,
                ),
                (
                    InheritedWide64::STAMP.word + 1,
                    crate::WORD_OP_RESOURCE,
                    u32::MAX,
                    1,
                ),
            ],
        );
        let view = RowPatchView::from_words(&words).expect("valid crafted Wide64 hi-word patch");
        let mut tx = store.tx();
        tx.apply_row_patch(child_wide64_hi, view);
        commit_tx(tx);
    }
    let row_w64 = store.row::<InheritedWide64>(child_wide64_hi);
    assert!(row_w64.is_color_explicit());
    assert!(
        !row_w64.is_stamp_explicit(),
        "skipped op on Wide64 f.word + 1 must NOT flip 64-bit stamp explicit bit"
    );
    let root_w64 = ResolvedRow::<InheritedWide64>::root(&store);
    let parent_w64 = ResolvedRow::<InheritedWide64>::resolve(&store, parent, &root_w64);
    let resolved_w64 = ResolvedRow::<InheritedWide64>::resolve(&store, child_wide64_hi, &parent_w64);
    assert_eq!(resolved_w64.get().color(), Rgba::rgb(1, 2, 3));
    assert_eq!(
        resolved_w64.get().stamp(),
        0x1122_3344_5566_7788,
        "parent's inherited 64-bit stamp must remain intact"
    );

    // Control check: a complete valid 64-bit `RowPatch::set` on `STAMP` DOES apply and flip its bit.
    {
        let mut valid_p = RowPatch::new();
        valid_p.set(InheritedWide64::STAMP, 0xAABB_CCDD_EEFF_0011);
        let mut v_words = Vec::new();
        assert!(valid_p.write_words(&mut v_words));
        let mut tx = store.tx();
        tx.apply_row_patch(child_wide64_hi, RowPatchView::from_words(&v_words).unwrap());
        commit_tx(tx);
    }
    let row_w64_after = store.row::<InheritedWide64>(child_wide64_hi);
    assert!(row_w64_after.is_stamp_explicit());
    assert_eq!(row_w64_after.stamp(), 0xAABB_CCDD_EEFF_0011);

    // Scenario B1: Crafted wire `RowPatchView` setting `color` + forged/orphan word-0 explicit
    // bits for unpatched `font_size` and `family` (with NO ops targeting their words at all).
    {
        let forged_mask = (1u32 << PatchTypo::COLOR.explicit_bit)
            | (1u32 << PatchTypo::FONT_SIZE.explicit_bit)
            | (1u32 << PatchTypo::FAMILY.explicit_bit)
            | (1u32 << 31); // out-of-range undeclared bit
        let words = build_wire_patch(
            PatchTypo::KEY,
            &[
                (0, crate::WORD_OP_SCALAR, forged_mask, forged_mask),
                (
                    PatchTypo::COLOR.word,
                    crate::WORD_OP_SCALAR,
                    u32::MAX,
                    Rgba::rgb(9, 8, 7).0,
                ),
            ],
        );
        let view = RowPatchView::from_words(&words).expect("valid crafted orphan word-0 patch");
        let mut tx = store.tx();
        tx.apply_row_patch(child_orphan_typo, view);
        commit_tx(tx);
    }
    let row_ot = store.row::<PatchTypo>(child_orphan_typo);
    assert!(row_ot.is_color_explicit());
    assert!(
        !row_ot.is_font_size_explicit(),
        "orphan word-0 explicit bit without data op must be rejected"
    );
    assert!(
        !row_ot.is_family_explicit(),
        "orphan word-0 explicit bit for Str must be rejected"
    );
    assert_eq!(
        row_ot.explicit_mask(),
        1u32 << PatchTypo::COLOR.explicit_bit,
        "only genuinely patched fields may have explicit bits set (no undeclared high bits)"
    );
    let root_typo = ResolvedRow::<PatchTypo>::root(&store);
    let parent_typo = ResolvedRow::<PatchTypo>::resolve(&store, parent, &root_typo);
    let resolved_ot = ResolvedRow::<PatchTypo>::resolve(&store, child_orphan_typo, &parent_typo);
    assert_eq!(resolved_ot.get().color(), Rgba::rgb(9, 8, 7));
    assert_eq!(resolved_ot.get().font_size(), 22.0);
    assert_eq!(resolved_ot.get().family(), "Inter");

    // Scenario B2: Crafted wire `RowPatchView` setting `color` + forged/orphan word-0 explicit
    // bit for unpatched `Res<ChildBlob>` (`blob`) and `stamp` with NO ops for them.
    {
        let forged_mask = (1u32 << InheritedWide64::COLOR.explicit_bit)
            | (1u32 << InheritedWide64::STAMP.explicit_bit)
            | (1u32 << InheritedWide64::BLOB.explicit_bit);
        let words = build_wire_patch(
            InheritedWide64::KEY,
            &[
                (0, crate::WORD_OP_SCALAR, forged_mask, forged_mask),
                (
                    InheritedWide64::COLOR.word,
                    crate::WORD_OP_SCALAR,
                    u32::MAX,
                    Rgba::rgb(77, 88, 99).0,
                ),
            ],
        );
        let view = RowPatchView::from_words(&words).expect("valid crafted orphan res patch");
        let mut tx = store.tx();
        tx.apply_row_patch(child_orphan_res, view);
        commit_tx(tx);
    }
    let row_or = store.row::<InheritedWide64>(child_orphan_res);
    assert!(row_or.is_color_explicit());
    assert!(!row_or.is_stamp_explicit());
    assert!(
        !row_or.is_blob_explicit(),
        "orphan word-0 explicit bit for unpatched Res<R> must be rejected"
    );
    let root_w64 = ResolvedRow::<InheritedWide64>::root(&store);
    let parent_w64 = ResolvedRow::<InheritedWide64>::resolve(&store, parent, &root_w64);
    let resolved_or =
        ResolvedRow::<InheritedWide64>::resolve(&store, child_orphan_res, &parent_w64);
    assert_eq!(resolved_or.get().color(), Rgba::rgb(77, 88, 99));
    assert_eq!(resolved_or.get().stamp(), 0x1122_3344_5566_7788);
    assert_eq!(
        resolved_or.get().blob(),
        parent_blob,
        "parent's inherited resource must not be clobbered by orphan word-0 bit"
    );
}

#[test]
fn apply_row_patch_inherited_packed_sibling_and_partial_mask_reject_forged_explicit_bits() {
    #[allow(dead_code)]
    #[derive(crate::NveTable)]
    #[nve_table(name = "test.patch.inherited_packed", storage = "sparse", flags = "publish", inherited)]
    struct InheritedPacked {
        #[effect(paint)]
        align: PatchAlign,
        #[effect(paint)]
        wrap: bool,
        #[effect(paint)]
        #[default(400u16)]
        weight: u16,
        #[effect(paint)]
        opt_tint: Option<Rgba>,
    }

    // Premise check: `align`, `wrap`, `weight`, and `opt_tint`'s presence bit all pack into word 1.
    assert_eq!(InheritedPacked::ALIGN.word, InheritedPacked::WRAP.word);
    assert_eq!(InheritedPacked::ALIGN.word, InheritedPacked::WEIGHT.word);
    assert_eq!(InheritedPacked::ALIGN.word, InheritedPacked::OPT_TINT.word);

    let mut store = NodeStore::new();
    let (parent, child_sibling, child_partial) = {
        let mut tx = store.tx();
        let parent = tx.create();
        let child_sibling = tx.create();
        let child_partial = tx.create();
        tx.set_children(NodeId::ROOT, &[parent]);
        tx.set_children(parent, &[child_sibling, child_partial]);
        tx.edit::<InheritedPacked>(parent)
            .align(PatchAlign::End)
            .wrap(true)
            .weight(700)
            .opt_tint(Some(Rgba::rgb(12, 34, 56)));
        commit_tx(tx);
        (parent, child_sibling, child_partial)
    };

    let build_wire_patch = |ops: &[(u8, u16, u32, u32)]| -> Vec<u32> {
        let mut sorted = ops.to_vec();
        sorted.sort_by_key(|o| o.0);
        let key = InheritedPacked::KEY;
        let mut w = vec![1u32, key.lo(), key.hi(), sorted.len() as u32];
        for (word, flags, clear_mask, set_bits) in sorted {
            w.push(u32::from(word) | (u32::from(flags) << 16));
            w.push(clear_mask);
            w.push(set_bits);
        }
        w
    };

    // Case 1: Patch ONLY `align` in shared packed word 1, while forging word-0 explicit bits
    // for sibling fields `wrap`, `weight`, and `opt_tint` that share word 1.
    {
        let forged_explicit = (1u32 << InheritedPacked::ALIGN.explicit_bit)
            | (1u32 << InheritedPacked::WRAP.explicit_bit)
            | (1u32 << InheritedPacked::WEIGHT.explicit_bit)
            | (1u32 << InheritedPacked::OPT_TINT.explicit_bit);
        let align_bits = (PatchAlign::Center as u32) << InheritedPacked::ALIGN.shift;
        let words = build_wire_patch(&[
            (0, crate::WORD_OP_SCALAR, forged_explicit, forged_explicit),
            (
                InheritedPacked::ALIGN.word,
                crate::WORD_OP_SCALAR,
                InheritedPacked::ALIGN.mask,
                align_bits,
            ),
        ]);
        let view = RowPatchView::from_words(&words).expect("valid crafted packed-sibling patch");
        let mut tx = store.tx();
        tx.apply_row_patch(child_sibling, view);
        commit_tx(tx);
    }
    let row_sib = store.row::<InheritedPacked>(child_sibling);
    assert!(row_sib.is_align_explicit(), "fully patched packed field must set its explicit bit");
    assert!(!row_sib.is_wrap_explicit(), "unpatched sibling in same word must NOT set explicit bit");
    assert!(!row_sib.is_weight_explicit(), "unpatched sibling in same word must NOT set explicit bit");
    assert!(!row_sib.is_opt_tint_explicit(), "unpatched wide-opt in same word must NOT set explicit bit");
    {
        let root_r = ResolvedRow::<InheritedPacked>::root(&store);
        let parent_r = ResolvedRow::<InheritedPacked>::resolve(&store, parent, &root_r);
        let sib_r = ResolvedRow::<InheritedPacked>::resolve(&store, child_sibling, &parent_r);
        assert_eq!(sib_r.get().align(), PatchAlign::Center);
        assert!(sib_r.get().wrap(), "parent wrap must remain inherited");
        assert_eq!(sib_r.get().weight(), 700, "parent weight must remain inherited");
        assert_eq!(
            sib_r.get().opt_tint(),
            Some(Rgba::rgb(12, 34, 56)),
            "parent opt_tint must remain inherited"
        );
    }

    // Case 2: Patch `align` fully + only a PARTIAL `clear_mask` on 16-bit `weight` (low 8 bits)
    // and only a PARTIAL `clear_mask` on `opt_tint`'s `val_word` (low 16 bits) + `opt_tint`'s
    // presence bit. Neither `weight` nor `opt_tint` has full mask coverage -> neither explicit
    // bit may flip!
    {
        let forged_explicit = (1u32 << InheritedPacked::ALIGN.explicit_bit)
            | (1u32 << InheritedPacked::WEIGHT.explicit_bit)
            | (1u32 << InheritedPacked::OPT_TINT.explicit_bit);
        let partial_weight_mask = 0x00FFu32 << InheritedPacked::WEIGHT.shift;
        let opt_presence_mask = 1u32 << InheritedPacked::OPT_TINT.shift;
        let word1_mask = InheritedPacked::ALIGN.mask | partial_weight_mask | opt_presence_mask;
        let word1_set = ((PatchAlign::Center as u32) << InheritedPacked::ALIGN.shift)
            | (0x42u32 << InheritedPacked::WEIGHT.shift)
            | opt_presence_mask;
        let words = build_wire_patch(&[
            (0, crate::WORD_OP_SCALAR, forged_explicit, forged_explicit),
            (InheritedPacked::ALIGN.word, crate::WORD_OP_SCALAR, word1_mask, word1_set),
            (
                InheritedPacked::OPT_TINT.val_word,
                crate::WORD_OP_SCALAR,
                0x0000_FFFF, // partial val_word mask instead of u32::MAX
                0x0000_1234,
            ),
        ]);
        let view = RowPatchView::from_words(&words).expect("valid crafted partial-mask patch");
        let mut tx = store.tx();
        tx.apply_row_patch(child_partial, view);
        commit_tx(tx);
    }
    let row_part = store.row::<InheritedPacked>(child_partial);
    assert!(row_part.is_align_explicit());
    assert!(
        !row_part.is_weight_explicit(),
        "partial clear_mask on multi-bit packed field must NOT set explicit bit"
    );
    assert!(
        !row_part.is_opt_tint_explicit(),
        "partial val_word clear_mask on wide Option<T> must NOT set explicit bit"
    );
    {
        let root_r = ResolvedRow::<InheritedPacked>::root(&store);
        let parent_r = ResolvedRow::<InheritedPacked>::resolve(&store, parent, &root_r);
        let part_r = ResolvedRow::<InheritedPacked>::resolve(&store, child_partial, &parent_r);
        assert_eq!(part_r.get().align(), PatchAlign::Center);
        assert_eq!(part_r.get().weight(), 700, "parent weight must stay inherited");
        assert_eq!(part_r.get().opt_tint(), Some(Rgba::rgb(12, 34, 56)));
    }

    // Control: genuine full `RowPatch::set` on `wrap`, `weight`, and `opt_tint` sets all bits.
    {
        let mut patch = RowPatch::new();
        patch
            .set(InheritedPacked::WRAP, false)
            .set(InheritedPacked::WEIGHT, 300)
            .set(InheritedPacked::OPT_TINT, Some(Rgba::WHITE));
        let mut words = Vec::new();
        assert!(patch.write_words(&mut words));
        let mut tx = store.tx();
        tx.apply_row_patch(child_partial, RowPatchView::from_words(&words).unwrap());
        commit_tx(tx);
    }
    let row_after = store.row::<InheritedPacked>(child_partial);
    assert!(row_after.is_wrap_explicit());
    assert!(row_after.is_weight_explicit());
    assert!(row_after.is_opt_tint_explicit());
    assert!(!row_after.wrap());
    assert_eq!(row_after.weight(), 300);
    assert_eq!(row_after.opt_tint(), Some(Rgba::WHITE));
}
