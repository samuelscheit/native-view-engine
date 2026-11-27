//! Unit & hardening tests for table-generic [`RowPatch`], flat-buffer [`ResourceStore`]
//! nested child refcounting, cycle rejection, topological `NVE2` wire transfer, and adversarial
//! corruption guards.

use crate::wire::{Op, WireDecoder, WireEncoder, WireError};
use crate::{
    ChangeSet, FlatResource, GridLine, Length, NodeId, NodeStore, Res, ResolvedRow, ResourceId,
    Rgba, RowPatch, Str, WordEnum,
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

    fn view(words: &[u32]) -> &[u32] {
        words
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
    let view = RowPatch::view(&words);

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
    assert!(parent_typo.apply_patch(RowPatch::view(&ov_words)));
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
