//! Comprehensive word-table packing & encoding suite: typed `#[default(...)]` (narrow `OptWord`,
//! `WideOpt`, conditional const exprs, 64-bit), `schema_hash` discrimination & delimiter boundary,
//! narrowed signed/unsigned `#[bits(N)]`, bit-exact `f32`/`f64` special values & 64-bit dirty
//! masks, `WideOpt` transitions, hostile/corrupt `NVE2` frames, and `pack_table` boundary invariants.

use core::num::{NonZero, NonZeroU16, NonZeroU32, NonZeroU8};

use nve_abi::SurfaceId;

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

struct MacroConstHolder;
impl MacroConstHolder {
    const DEFAULT_VAL: u32 = 77;
}

#[test]
fn macro_rules_default_expr_precedence_word_count_scope_and_nonzero_opt() {
    // Verify user const named `WORD_COUNT` does not collide with generated internals,
    // `macro_rules!` `$a:expr` `Delim::None` keeps precedence in `$a * 2`, while `$t::CONST`,
    // `$p::new(...)`, and `Tri::$VAR` do NOT get wrapped in invalid `(T)::CONST` parens, and
    // `Option<NonZeroU8|NonZeroU16|NonZeroU32|NonZero<u16>>` defaults pack cleanly.
    const WORD_COUNT: u32 = 42;
    macro_rules! def_table {
        ($a:expr, $cond:expr, $t:ty, $p:path, $var:ident) => {
            #[allow(dead_code)]
            #[derive(crate::NveTable)]
            #[nve_table(name = "test.packing.macro_prec", storage = "sparse")]
            struct MacroPrecTable {
                #[effect(paint)]
                #[default($a * 2)]
                mul: u32,
                #[effect(paint)]
                #[default(if false && $cond { 1u32 } else { 2u32 })]
                if_prec: u32,
                #[effect(paint)]
                #[default(WORD_COUNT)]
                from_const: u32,
                #[effect(paint)]
                #[default($t::DEFAULT_VAL)]
                from_ty_const: u32,
                #[effect(paint)]
                #[default($p::new(3))]
                nz_lines: Option<NonZeroU16>,
                #[effect(paint)]
                #[default(Tri::$var)]
                tri_var: Tri,
                #[effect(paint)]
                #[default(NonZeroU8::new(9))]
                nz8: Option<NonZeroU8>,
                #[effect(paint)]
                #[default(NonZeroU32::new(123456))]
                nz32: Option<NonZeroU32>,
                #[effect(paint)]
                #[default(NonZero::new(7))]
                nz_generic: Option<core::num::NonZero<u16>>,
            }
        };
    }
    def_table!(1 + 2, false || true, MacroConstHolder, core::num::NonZeroU16, C);

    let store = NodeStore::new();
    let r = store.row::<MacroPrecTable>(NodeId::ROOT);
    assert_eq!(r.mul(), 6, "(1 + 2) * 2 must equal 6, not 1 + (2 * 2) = 5");
    assert_eq!(
        r.if_prec(),
        2,
        "if false && (false || true) must evaluate to 2, not (false && false) || true == 1"
    );
    assert_eq!(r.from_const(), 42);
    assert_eq!(r.from_ty_const(), 77);
    assert_eq!(r.nz_lines(), NonZeroU16::new(3));
    assert_eq!(r.tri_var(), Tri::C);
    assert_eq!(r.nz8(), NonZeroU8::new(9));
    assert_eq!(r.nz32(), NonZeroU32::new(123456));
    assert_eq!(r.nz_generic(), NonZero::new(7));
    assert_eq!(MacroPrecTable::DESC.fields[4].bits, 16);
    assert_eq!(MacroPrecTable::DESC.fields[6].bits, 8);
    assert_eq!(MacroPrecTable::DESC.fields[7].bits, 32);
    assert_eq!(MacroPrecTable::DESC.fields[8].bits, 16);
}
