//! Wire-level encoding, hostile/corrupt `NVE2` bit-pattern totality, 64-bit single-word dirty
//! deltas, and `pack_table` boundary invariant tests (split from `table_packing_tests.rs` to stay
//! under `[INV:max-650-lines]`).

use core::num::{NonZeroU16, NonZeroU32, NonZeroU8};

use nve_abi::table::{effect, pack_table, FieldKind, OptEncoding, PackInput, PackKind, NO_VAL_WORD};

use crate::wire::{WireDecoder, WireEncoder};
use crate::{
    word_flags, ChangeSet, GridLine, Length, NodeId, NodeStore, NveTable, Rgba, WordEnum,
};

fn commit_tx(tx: crate::Transaction<'_>) -> ChangeSet {
    let mut c = ChangeSet::default();
    tx.commit_into(&mut c);
    c
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, WordEnum)]
enum WireTri {
    #[default]
    A = 0,
    B = 1,
    C = 2,
}

word_flags! {
    struct WireFlags(u8) {
        const BIT0 = 1 << 0;
        const BIT2 = 1 << 2;
        const BIT5 = 1 << 5;
    }
}

#[test]
fn special_floats_wide_opt_transitions_and_64bit_single_word_dirty_delta() {
    #[allow(dead_code)]
    #[derive(crate::NveTable)]
    #[nve_table(name = "test.packing.wide_specials", storage = "sparse", flags = "publish")]
    struct WideSpecials {
        #[effect(paint)]
        wf32: Option<f32>,
        #[effect(paint)]
        wu32: Option<u32>,
        #[effect(paint)]
        wi32: Option<i32>,
        #[effect(paint)]
        wrgba: Option<Rgba>,
        #[effect(paint)]
        u64v: u64,
        #[effect(paint)]
        i64v: i64,
        #[effect(paint)]
        f64v: f64,
    }

    let mut src = NodeStore::new();
    let mut dst = NodeStore::replica();
    let mut enc = WireEncoder::default();
    let mut dec = WireDecoder::default();
    let mut wire = Vec::new();

    let n = {
        let mut tx = src.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<WideSpecials>(n)
            .wf32(Some(f32::from_bits(0x7FC0_1234))) // custom NaN payload
            .wu32(Some(0))
            .wi32(Some(0))
            .wrgba(Some(Rgba(u32::MAX)))
            .u64v(u64::MAX)
            .i64v(i64::MIN)
            .f64v(f64::from_bits(0x7FF8_0000_0000_1234));
        let c = commit_tx(tx);
        enc.encode(&src, &c, &mut wire);
        let mut rtx = dst.tx();
        dec.apply(&mut rtx, &wire).unwrap();
        commit_tx(rtx);
        n
    };

    let r = dst.row::<WideSpecials>(n);
    assert_eq!(r.wf32().map(f32::to_bits), Some(0x7FC0_1234));
    assert_eq!(r.wu32(), Some(0));
    assert_eq!(r.wi32(), Some(0));
    assert_eq!(r.wrgba(), Some(Rgba(u32::MAX)));
    assert_eq!(r.u64v(), u64::MAX);
    assert_eq!(r.i64v(), i64::MIN);
    assert_eq!(r.f64v().to_bits(), 0x7FF8_0000_0000_1234);

    // Special float bit-exact cases: quiet NaN, -0.0, +INF, -INF.
    for (f32_val, f64_val) in [
        (f32::NAN, f64::NAN),
        (-0.0f32, -0.0f64),
        (f32::INFINITY, f64::INFINITY),
        (f32::NEG_INFINITY, f64::NEG_INFINITY),
    ] {
        let mut tx = src.tx();
        tx.edit::<WideSpecials>(n).wf32(Some(f32_val)).f64v(f64_val);
        let c = commit_tx(tx);
        wire.clear();
        enc.encode(&src, &c, &mut wire);
        let mut rtx = dst.tx();
        dec.apply(&mut rtx, &wire).unwrap();
        commit_tx(rtx);
        let r = dst.row::<WideSpecials>(n);
        assert_eq!(r.wf32().map(f32::to_bits), Some(f32_val.to_bits()));
        assert_eq!(r.f64v().to_bits(), f64_val.to_bits());
    }

    // WideOpt transitions: None -> Some(0.0) -> Some(1.0) -> Some(2.0) -> None.
    for next in [None, Some(0.0f32), Some(1.0f32), Some(2.0f32), None] {
        let mut tx = src.tx();
        tx.edit::<WideSpecials>(n).wf32(next);
        let c = commit_tx(tx);
        wire.clear();
        enc.encode(&src, &c, &mut wire);
        let mut rtx = dst.tx();
        dec.apply(&mut rtx, &wire).unwrap();
        commit_tx(rtx);
        assert_eq!(
            dst.row::<WideSpecials>(n).wf32().map(f32::to_bits),
            next.map(f32::to_bits)
        );
    }

    // Single-word low-only and high-only 64-bit dirty mask + incremental NVE2 delta.
    let u64_word = WideSpecials::U64V.word as u32;
    let base: u64 = 0x1122_3344_5566_7788;
    let lo_changed: u64 = 0x1122_3344_AABB_CCDD;
    let hi_changed: u64 = 0x9988_7766_AABB_CCDD;
    {
        let mut tx = src.tx();
        tx.edit::<WideSpecials>(n).u64v(base).i64v(i64::MAX);
        let c = commit_tx(tx);
        wire.clear();
        enc.encode(&src, &c, &mut wire);
        let mut rtx = dst.tx();
        dec.apply(&mut rtx, &wire).unwrap();
        commit_tx(rtx);
    }
    // Mutate only low 32 bits -> only `u64_word` is dirty.
    {
        let mut tx = src.tx();
        tx.set(n, WideSpecials::U64V, lo_changed);
        let c = commit_tx(tx);
        let tc = c.table(WideSpecials::slot()).unwrap();
        assert_eq!(
            tc.masks,
            vec![1u32 << u64_word],
            "low-only u64 write must dirty only low word"
        );
        wire.clear();
        enc.encode(&src, &c, &mut wire);
        let mut rtx = dst.tx();
        dec.apply(&mut rtx, &wire).unwrap();
        commit_tx(rtx);
        assert_eq!(dst.row::<WideSpecials>(n).u64v(), lo_changed);
        assert_eq!(dst.row::<WideSpecials>(n).i64v(), i64::MAX);
    }
    // Mutate only high 32 bits -> only `u64_word + 1` is dirty.
    {
        let mut tx = src.tx();
        tx.set(n, WideSpecials::U64V, hi_changed);
        let c = commit_tx(tx);
        let tc = c.table(WideSpecials::slot()).unwrap();
        assert_eq!(
            tc.masks,
            vec![1u32 << (u64_word + 1)],
            "high-only u64 write must dirty only high word"
        );
        wire.clear();
        enc.encode(&src, &c, &mut wire);
        let mut rtx = dst.tx();
        dec.apply(&mut rtx, &wire).unwrap();
        commit_tx(rtx);
        assert_eq!(dst.row::<WideSpecials>(n).u64v(), hi_changed);
    }
}

#[test]
fn untrusted_nve2_corrupt_raw_word_bits_decode_totally_without_panicking() {
    #[repr(u8)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default, WordEnum)]
    enum Bi {
        #[default]
        X = 0,
        Y = 1,
    }

    #[allow(dead_code)]
    #[derive(crate::NveTable)]
    #[nve_table(name = "test.packing.corrupt", storage = "sparse", flags = "publish")]
    struct CorruptTarget {
        #[effect(paint)]
        req_enum: WireTri,
        #[effect(paint)]
        opt_bi: Option<Bi>,
        #[effect(paint)]
        opt_bool: Option<bool>,
        #[effect(paint)]
        opt_u8: Option<u8>,
        #[effect(paint)]
        opt_flags: Option<WireFlags>,
        #[effect(layout)]
        opt_len: Option<Length>,
        #[effect(layout)]
        opt_grid: Option<GridLine>,
        #[effect(paint)]
        opt_nz8: Option<NonZeroU8>,
        #[effect(paint)]
        opt_nz16: Option<NonZeroU16>,
        #[effect(paint)]
        opt_nz32: Option<NonZeroU32>,
    }

    // First produce a valid keyframe frame to register the table schema on `dst`.
    let mut src = NodeStore::new();
    let mut tx = src.tx();
    let n = tx.create();
    tx.set_children(NodeId::ROOT, &[n]);
    tx.attach::<CorruptTarget>(n);
    let c = commit_tx(tx);
    let mut enc = WireEncoder::default();
    let mut wire = Vec::new();
    enc.encode(&src, &c, &mut wire);

    let mut dst = NodeStore::replica();
    let mut dec = WireDecoder::default();
    {
        let mut rtx = dst.tx();
        dec.apply(&mut rtx, &wire).unwrap();
        commit_tx(rtx);
    }

    // Build a synthetic hostile 4-byte-aligned NVE2 Op::Patch frame setting ALL words to `0xFFFF_FFFF`.
    let word_count = CorruptTarget::DESC.words();
    let full_mask = if word_count == 32 {
        u32::MAX
    } else {
        (1u32 << word_count) - 1
    };
    let payload_words = (2 + word_count) as u32;
    let total_words = (crate::wire::HEADER_WORDS as u32) + 2 + payload_words;
    let mut hostile = Vec::with_capacity((total_words as usize) * 4);
    let push_u32 = |buf: &mut Vec<u8>, v: u32| buf.extend_from_slice(&v.to_le_bytes());
    push_u32(&mut hostile, crate::wire::MAGIC);
    push_u32(&mut hostile, u32::from(crate::wire::VERSION)); // version=1, flags=0
    push_u32(&mut hostile, 2u32); // frame lo
    push_u32(&mut hostile, 0u32); // frame hi
    push_u32(&mut hostile, 1u32); // op_count = 1
    push_u32(&mut hostile, total_words);
    push_u32(
        &mut hostile,
        (crate::wire::Op::Patch as u32)
            | (u32::from(CorruptTarget::slot().0) << 8)
            | (1u32 << 16),
    );
    push_u32(&mut hostile, payload_words);
    push_u32(&mut hostile, n.raw());
    push_u32(&mut hostile, full_mask);
    for _ in 0..word_count {
        push_u32(&mut hostile, 0xFFFF_FFFFu32);
    }

    let mut rtx = dst.tx();
    dec.apply(&mut rtx, &hostile)
        .expect("corrupt scalar bit patterns must apply cleanly");
    commit_tx(rtx);

    // Every getter MUST be total (no panic, valid invariants):
    let r = dst.row::<CorruptTarget>(n);
    assert_eq!(r.req_enum(), WireTri::A, "out-of-range WireTri (3) falls back to default");
    // Bi has 2 variants (0,1 -> opt codes 1,2; 0b11 == 3 is out of range -> None!).
    assert_eq!(r.opt_bi(), None, "out-of-range Option<Bi> (3) decodes to None");
    // Option<bool>: 0b11 (3) is invalid -> total fallback to None!
    assert_eq!(r.opt_bool(), None);
    assert_eq!(r.opt_u8(), Some(u8::MAX));
    // WireFlags masks undeclared bits -> only declared bits (BIT0 | BIT2 | BIT5) survive!
    assert_eq!(r.opt_flags(), Some(WireFlags::BIT0 | WireFlags::BIT2 | WireFlags::BIT5));
    // Option<Length> with non-zero bits (0xFFFF_FFFF) decodes totally to Some(...).
    assert!(r.opt_len().is_some());
    // 16-bit Option<GridLine> with 0xFFFF: xor 0xC000 -> 0x3FFF (tag 00 = Auto!).
    assert_eq!(r.opt_grid(), Some(GridLine::Auto));
    assert_eq!(r.opt_nz8(), NonZeroU8::new(u8::MAX));
    assert_eq!(r.opt_nz16(), NonZeroU16::new(u16::MAX));
    assert_eq!(r.opt_nz32(), NonZeroU32::new(u32::MAX));

    // Also patch all words to `0x0000_0000`: every NonZero option decodes cleanly to None!
    let mut zero_frame = Vec::with_capacity((total_words as usize) * 4);
    push_u32(&mut zero_frame, crate::wire::MAGIC);
    push_u32(&mut zero_frame, u32::from(crate::wire::VERSION));
    push_u32(&mut zero_frame, 3u32);
    push_u32(&mut zero_frame, 0u32);
    push_u32(&mut zero_frame, 1u32);
    push_u32(&mut zero_frame, total_words);
    push_u32(
        &mut zero_frame,
        (crate::wire::Op::Patch as u32)
            | (u32::from(CorruptTarget::slot().0) << 8)
            | (1u32 << 16),
    );
    push_u32(&mut zero_frame, payload_words);
    push_u32(&mut zero_frame, n.raw());
    push_u32(&mut zero_frame, full_mask);
    for _ in 0..word_count {
        push_u32(&mut zero_frame, 0u32);
    }
    let mut rtx = dst.tx();
    dec.apply(&mut rtx, &zero_frame)
        .expect("all-zero patch must apply cleanly");
    commit_tx(rtx);
    let r0 = dst.row::<CorruptTarget>(n);
    assert_eq!(r0.opt_nz8(), None);
    assert_eq!(r0.opt_nz16(), None);
    assert_eq!(r0.opt_nz32(), None);
}

const fn dummy_input(
    name: &'static str,
    pack_kind: PackKind,
    bits: u8,
    effects: u8,
    default_present: bool,
) -> PackInput {
    PackInput {
        name,
        field_kind: FieldKind::U32,
        opt_encoding: OptEncoding::Required,
        pack_kind,
        bits,
        effects,
        default_lo: 0,
        default_hi: 0,
        default_present,
        no_inherit: false,
    }
}

#[test]
fn pack_table_first_fit_effect_segregation_exact_32_words_and_determinism() {
    let inputs = [
        dummy_input("a", PackKind::Scalar, 16, effect::LAYOUT, false),
        dummy_input("b", PackKind::Scalar, 8, effect::PAINT, false),
        // Same effect as `a` (LAYOUT), 16 + 16 == 32 -> packs into word 0!
        dummy_input("c", PackKind::Scalar, 16, effect::LAYOUT, false),
        // Same effect as `b` (PAINT), 8 + 8 == 16 -> packs into word 1!
        dummy_input("d", PackKind::Scalar, 8, effect::PAINT, false),
    ];
    let p1 = pack_table(inputs, false, false);
    let p2 = pack_table(inputs, false, false);
    assert_eq!(p1.word_count, 2);
    assert_eq!(p1.fields[0].word, 0);
    assert_eq!(p1.fields[0].shift, 0);
    assert_eq!(p1.fields[1].word, 1);
    assert_eq!(p1.fields[1].shift, 0);
    assert_eq!(p1.fields[2].word, 0);
    assert_eq!(p1.fields[2].shift, 16);
    assert_eq!(p1.fields[3].word, 1);
    assert_eq!(p1.fields[3].shift, 8);
    assert_eq!(p1.word_effects[0], effect::LAYOUT);
    assert_eq!(p1.word_effects[1], effect::PAINT);
    assert_eq!(p1.defaults, p2.defaults);
    assert_eq!(p1.word_effects, p2.word_effects);
    assert_eq!(p1.events_word, NO_VAL_WORD);

    // Exact 32-word fit succeeds.
    let exact32 = [dummy_input("w", PackKind::Scalar, 32, effect::PAINT, false); 32];
    assert_eq!(pack_table(exact32, false, false).word_count, 32);
}

#[test]
#[should_panic(expected = "table exceeds 32 words")]
fn pack_table_rejects_wide64_at_word_31() {
    let mut inputs = [dummy_input("w", PackKind::Scalar, 32, effect::PAINT, false); 32];
    inputs[31] = dummy_input("wide", PackKind::Wide64, 64, effect::PAINT, false);
    let _ = pack_table(inputs, false, false);
}

#[test]
#[should_panic(expected = "table exceeds 32 words")]
fn pack_table_rejects_str_at_word_31() {
    let mut inputs = [dummy_input("w", PackKind::Scalar, 32, effect::PAINT, false); 32];
    inputs[31] = dummy_input("s", PackKind::Str, 64, effect::SEMANTICS, false);
    let _ = pack_table(inputs, false, false);
}

#[test]
#[should_panic(expected = "table exceeds 32 words")]
fn pack_table_rejects_wide_opt_at_word_31() {
    // 31 full words + 1 WideOpt needs word 31 for presence AND word 32 for value -> overflow!
    let mut inputs = [dummy_input("w", PackKind::Scalar, 32, effect::PAINT, false); 32];
    inputs[31] = dummy_input("wo", PackKind::WideOpt, 32, effect::PAINT, false);
    let _ = pack_table(inputs, false, false);
}

#[test]
#[should_panic(expected = "cannot combine `inherited` and `events`")]
fn pack_table_rejects_inherited_plus_events() {
    let inputs = [dummy_input("a", PackKind::Scalar, 8, effect::PAINT, false)];
    let _ = pack_table(inputs, true, true);
}

#[test]
#[should_panic(expected = "at most 32 fields")]
fn pack_table_rejects_inherited_over_32_fields() {
    let inputs = [dummy_input("b", PackKind::Scalar, 1, effect::PAINT, false); 33];
    let _ = pack_table(inputs, true, false);
}

#[test]
#[should_panic(expected = "only valid for PackKind::WideOpt")]
fn pack_table_rejects_default_present_on_scalar() {
    let inputs = [dummy_input("bad", PackKind::Scalar, 8, effect::PAINT, true)];
    let _ = pack_table(inputs, false, false);
}

#[test]
#[should_panic(expected = "only valid for PackKind::WideOpt")]
fn pack_table_rejects_default_present_on_wide64() {
    let inputs = [dummy_input("bad", PackKind::Wide64, 64, effect::PAINT, true)];
    let _ = pack_table(inputs, false, false);
}

#[test]
#[should_panic(expected = "only valid for PackKind::WideOpt")]
fn pack_table_rejects_default_present_on_str() {
    let inputs = [dummy_input("bad", PackKind::Str, 64, effect::SEMANTICS, true)];
    let _ = pack_table(inputs, false, false);
}

#[test]
#[should_panic(expected = "only valid for PackKind::WideOpt")]
fn pack_table_rejects_default_present_on_resource() {
    let inputs = [dummy_input("bad", PackKind::Resource, 32, effect::PAINT, true)];
    let _ = pack_table(inputs, false, false);
}
