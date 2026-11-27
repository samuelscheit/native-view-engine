//! Comprehensive word-table packing & encoding suite: typed `#[default(...)]` (narrow `OptWord`,
//! `WideOpt`, conditional const exprs, 64-bit), `schema_hash` discrimination & delimiter boundary,
//! narrowed signed/unsigned `#[bits(N)]`, bit-exact `f32`/`f64` special values & 64-bit dirty
//! masks, `WideOpt` transitions, hostile/corrupt `NVE2` frames, and `pack_table` boundary invariants.

use nve_abi::table::{effect, pack_table, FieldKind, OptEncoding, PackInput, PackKind, NO_VAL_WORD};
use nve_abi::SurfaceId;

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
enum Tri {
    #[default]
    A = 0,
    B = 1,
    C = 2,
}

word_flags! {
    struct C2Flags(u8) {
        const BIT0 = 1 << 0;
        const BIT2 = 1 << 2;
        const BIT5 = 1 << 5;
    }
}

const CONST_OPT_F32_SOME: Option<f32> = Some(3.25);
const CONST_OPT_F32_NONE: Option<f32> = None;
const CONST_OPT_TRI_SOME: Option<Tri> = Some(Tri::C);
const CONST_FLAG: bool = true;

#[allow(dead_code)]
#[derive(crate::NveTable)]
#[nve_table(name = "test.packing.defaults", storage = "sparse", flags = "publish")]
struct OptionDefaultsTable {
    #[effect(paint)]
    #[default(Some(true))]
    opt_bool_t: Option<bool>,
    #[effect(paint)]
    #[default(Some(false))]
    opt_bool_f: Option<bool>,
    #[effect(paint)]
    #[default(None)]
    opt_bool_none: Option<bool>,
    #[effect(paint)]
    #[default(::core::option::Option::None)]
    opt_u8_qual_none: Option<u8>,
    #[effect(paint)]
    #[default(Some(42u8))]
    opt_u8_some: Option<u8>,
    #[effect(paint)]
    #[default(Some(-17i8))]
    opt_i8_some: Option<i8>,
    #[effect(paint)]
    #[default(Option::Some(1024u16))]
    opt_u16_some: Option<u16>,
    #[effect(paint)]
    #[default(::core::option::Option::Some(-500i16))]
    opt_i16_some: Option<i16>,
    #[effect(paint)]
    #[default(Some(Tri::A))]
    opt_enum_zero: Option<Tri>,
    #[effect(paint)]
    #[default(CONST_OPT_TRI_SOME)]
    opt_enum_const: Option<Tri>,
    #[effect(paint)]
    #[default(Some(C2Flags::BIT0.union(C2Flags::BIT5)))]
    opt_flags_some: Option<C2Flags>,
    #[effect(layout)]
    #[default(Some(Length::AUTO))]
    opt_len_auto: Option<Length>,
    #[effect(layout)]
    #[default(Some(GridLine::Line(-7)))]
    opt_grid_line: Option<GridLine>,
    #[effect(semantics)]
    #[default(Some(NodeId::ROOT))]
    opt_node_root: Option<NodeId>,
    #[effect(semantics)]
    #[default(Some(SurfaceId::ROOT))]
    opt_surf_root: Option<SurfaceId>,
    #[effect(paint)]
    #[default(Some(1.5))]
    wide_f32_some: Option<f32>,
    #[effect(paint)]
    #[default((None))]
    wide_f32_paren_none: Option<f32>,
    #[effect(paint)]
    #[default(::core::option::Option::None)]
    wide_f32_qual_none: Option<f32>,
    #[effect(paint)]
    #[default(CONST_OPT_F32_SOME)]
    wide_f32_const_some: Option<f32>,
    #[effect(paint)]
    #[default(CONST_OPT_F32_NONE)]
    wide_f32_const_none: Option<f32>,
    #[effect(paint)]
    #[default(if CONST_FLAG { Some(0xFFFF_FFFF) } else { None })]
    wide_u32_cond: Option<u32>,
    #[effect(paint)]
    #[default(if !CONST_FLAG { Some(2.5) } else { None })]
    wide_f32_cond_none: Option<f32>,
    #[effect(paint)]
    #[default(Some(Rgba::rgb(9, 8, 7)))]
    wide_rgba_some: Option<Rgba>,
    #[effect(paint)]
    #[default(Some(0u32))]
    wide_u32_zero: Option<u32>,
    #[effect(paint)]
    #[default(Some(-123456i32))]
    wide_i32_neg: Option<i32>,
    #[effect(paint)]
    #[default(0xFEDC_BA98_7654_3210u64)]
    def_u64: u64,
    #[effect(paint)]
    #[default(-9_000_000_000_000i64)]
    def_i64: i64,
    #[effect(paint)]
    #[default(-2.75f64)]
    def_f64: f64,
}

#[test]
fn option_and_64bit_defaults_materialize_accurately_on_attach() {
    let mut store = NodeStore::new();
    let mut tx = store.tx();
    let n = tx.create();
    tx.attach::<OptionDefaultsTable>(n);
    commit_tx(tx);

    let r = store.row::<OptionDefaultsTable>(n);
    assert_eq!(r.opt_bool_t(), Some(true));
    assert_eq!(r.opt_bool_f(), Some(false));
    assert_eq!(r.opt_bool_none(), None);
    assert_eq!(r.opt_u8_qual_none(), None);
    assert_eq!(r.opt_u8_some(), Some(42));
    assert_eq!(r.opt_i8_some(), Some(-17));
    assert_eq!(r.opt_u16_some(), Some(1024));
    assert_eq!(r.opt_i16_some(), Some(-500));
    assert_eq!(r.opt_enum_zero(), Some(Tri::A));
    assert_eq!(r.opt_enum_const(), Some(Tri::C));
    assert_eq!(r.opt_flags_some(), Some(C2Flags::BIT0 | C2Flags::BIT5));
    assert_eq!(r.opt_len_auto(), Some(Length::AUTO));
    assert_eq!(r.opt_grid_line(), Some(GridLine::Line(-7)));
    assert_eq!(r.opt_node_root(), Some(NodeId::ROOT));
    assert_eq!(r.opt_surf_root(), Some(SurfaceId::ROOT));
    assert_eq!(r.wide_f32_some(), Some(1.5));
    assert_eq!(r.wide_f32_paren_none(), None);
    assert_eq!(r.wide_f32_qual_none(), None);
    assert_eq!(r.wide_f32_const_some(), Some(3.25));
    assert_eq!(r.wide_f32_const_none(), None);
    assert_eq!(r.wide_u32_cond(), Some(0xFFFF_FFFF));
    assert_eq!(r.wide_f32_cond_none(), None);
    assert_eq!(r.wide_rgba_some(), Some(Rgba::rgb(9, 8, 7)));
    assert_eq!(r.wide_u32_zero(), Some(0));
    assert_eq!(r.wide_i32_neg(), Some(-123456));
    assert_eq!(r.def_u64(), 0xFEDC_BA98_7654_3210u64);
    assert_eq!(r.def_i64(), -9_000_000_000_000i64);
    assert_eq!(r.def_f64(), -2.75f64);
}

#[test]
fn schema_hash_discriminates_t_vs_option_t_and_table_field_name_boundaries() {
    macro_rules! pair_check {
        ($mod_name:ident, $ty:ty) => {
            mod $mod_name {
                #[allow(unused_imports)]
                use super::*;
                #[allow(dead_code)]
                #[derive(crate::NveTable)]
                #[nve_table(name = "test.same_name", storage = "sparse")]
                pub struct Req {
                    #[effect(layout)]
                    pub x: $ty,
                }
                #[allow(dead_code)]
                #[derive(crate::NveTable)]
                #[nve_table(name = "test.same_name", storage = "sparse")]
                pub struct Opt {
                    #[effect(layout)]
                    pub x: Option<$ty>,
                }
            }
            assert_ne!(
                $mod_name::Req::DESC.schema_hash,
                $mod_name::Opt::DESC.schema_hash,
                "schema_hash collided between {} and Option<{}>",
                stringify!($ty),
                stringify!($ty)
            );
            assert_ne!(
                $mod_name::Req::DESC.fields[0].opt_encoding,
                $mod_name::Opt::DESC.fields[0].opt_encoding,
            );
        };
    }

    // Tri has 3 variants (BITS = 2, OPT_BITS = 2), GridLine (16, 16), Length (32, 32),
    // NodeId (32, 32), SurfaceId (32, 32) — all share exact width with their Option<T>!
    pair_check!(tri_pair, Tri);
    pair_check!(grid_pair, GridLine);
    pair_check!(len_pair, Length);
    pair_check!(node_pair, NodeId);
    pair_check!(surf_pair, SurfaceId);
    pair_check!(bool_pair, bool);
    pair_check!(u8_pair, u8);
    pair_check!(flags_pair, C2Flags);
    pair_check!(f32_pair, f32);

    // 0xFF delimiter prevents table-name/field-name concatenation ambiguity (`"ab"|"c"` vs `"a"|"bc"`).
    #[allow(dead_code)]
    #[derive(crate::NveTable)]
    #[nve_table(name = "ab", storage = "sparse")]
    struct BoundAb {
        #[effect(paint)]
        c: u32,
    }
    #[allow(dead_code)]
    #[derive(crate::NveTable)]
    #[nve_table(name = "a", storage = "sparse")]
    struct BoundA {
        #[effect(paint)]
        bc: u32,
    }
    assert_ne!(BoundAb::DESC.schema_hash, BoundA::DESC.schema_hash);
}

#[test]
fn narrowed_signed_and_unsigned_bits_sign_extend_and_pack_cleanly() {
    #[allow(dead_code)]
    #[derive(crate::NveTable)]
    #[nve_table(name = "test.packing.narrow_bits", storage = "sparse")]
    struct NarrowBits {
        #[effect(paint)]
        #[bits(4)]
        #[default(-3i8)]
        s4: i8,
        #[effect(paint)]
        #[bits(1)]
        #[default(-1i8)]
        s1: i8,
        #[effect(paint)]
        #[bits(10)]
        #[default(-512i16)]
        s10: i16,
        #[effect(paint)]
        #[bits(10)]
        #[default(777u16)]
        u10: u16,
        #[effect(paint)]
        #[bits(7)]
        #[default(-64i32)]
        s7: i32,
        #[effect(paint)]
        #[bits(5)]
        #[default(29u8)]
        u5: u8,
        #[effect(paint)]
        #[bits(20)]
        #[default(1_048_575u32)]
        u20: u32,
    }

    // Word 0: 4 + 1 + 10 + 10 + 7 == 32 bits; Word 1: 5 + 20 == 25 bits -> 2 words total!
    assert_eq!(NarrowBits::DESC.words(), 2);

    let mut store = NodeStore::new();
    let mut tx = store.tx();
    let n = tx.create();
    tx.attach::<NarrowBits>(n);
    commit_tx(tx);

    let r = store.row::<NarrowBits>(n);
    assert_eq!(r.s4(), -3);
    assert_eq!(r.s1(), -1);
    assert_eq!(r.s10(), -512);
    assert_eq!(r.u10(), 777);
    assert_eq!(r.s7(), -64);
    assert_eq!(r.u5(), 29);
    assert_eq!(r.u20(), 1_048_575);

    // Exercise min/max boundaries for every narrowed signed & unsigned field.
    for (s4, s1, s10, u10, s7, u5, u20) in [
        (-8i8, -1i8, -512i16, 0u16, -64i32, 0u8, 0u32),
        (7i8, 0i8, 511i16, 1023u16, 63i32, 31u8, (1u32 << 20) - 1),
        (-1i8, -1i8, -1i16, 1u16, -1i32, 1u8, 1u32),
        (0i8, 0i8, 0i16, 512u16, 0i32, 16u8, 1u32 << 19),
    ] {
        let mut tx = store.tx();
        tx.edit::<NarrowBits>(n)
            .s4(s4)
            .s1(s1)
            .s10(s10)
            .u10(u10)
            .s7(s7)
            .u5(u5)
            .u20(u20);
        commit_tx(tx);
        let r = store.row::<NarrowBits>(n);
        assert_eq!(
            (r.s4(), r.s1(), r.s10(), r.u10(), r.s7(), r.u5(), r.u20()),
            (s4, s1, s10, u10, s7, u5, u20)
        );
    }
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "overflows")]
fn narrowed_u8_runtime_overflow_panics_in_debug() {
    use crate::WordValue;
    let _ = 16u8.to_masked_bits(4);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "overflows signed")]
fn narrowed_i8_runtime_overflow_panics_in_debug() {
    use crate::WordValue;
    let _ = 8i8.to_masked_bits(4);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "overflows signed")]
fn narrowed_i32_runtime_underflow_panics_in_debug() {
    use crate::WordValue;
    let _ = (-65i32).to_masked_bits(7);
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
        req_enum: Tri,
        #[effect(paint)]
        opt_bi: Option<Bi>,
        #[effect(paint)]
        opt_bool: Option<bool>,
        #[effect(paint)]
        opt_u8: Option<u8>,
        #[effect(paint)]
        opt_flags: Option<C2Flags>,
        #[effect(layout)]
        opt_len: Option<Length>,
        #[effect(layout)]
        opt_grid: Option<GridLine>,
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
    assert_eq!(r.req_enum(), Tri::A, "out-of-range Tri (3) falls back to default");
    // Bi has 2 variants (0,1 -> opt codes 1,2; 0b11 == 3 is out of range -> None!).
    assert_eq!(r.opt_bi(), None, "out-of-range Option<Bi> (3) decodes to None");
    // Option<bool>: 0b11 (3) is invalid -> total fallback to None!
    assert_eq!(r.opt_bool(), None);
    assert_eq!(r.opt_u8(), Some(u8::MAX));
    // C2Flags masks undeclared bits -> only declared bits (BIT0 | BIT2 | BIT5) survive!
    assert_eq!(r.opt_flags(), Some(C2Flags::BIT0 | C2Flags::BIT2 | C2Flags::BIT5));
    // Option<Length> with non-zero bits (0xFFFF_FFFF) decodes totally to Some(...).
    assert!(r.opt_len().is_some());
    // 16-bit Option<GridLine> with 0xFFFF: xor 0xC000 -> 0x3FFF (tag 00 = Auto!).
    assert_eq!(r.opt_grid(), Some(GridLine::Auto));
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
