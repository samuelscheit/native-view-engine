//! Niche/packed `Option<T>` support (`OptWord`) for sub-word and niche-bearing `WordValue` types.
//!
//! Every `OptWord` encoding guarantees that bit pattern `0` represents `None`, so all-zero
//! default table words decode cleanly to `None` with zero per-field initialization cost.

use nve_abi::table::{FieldKind, OptEncoding};
use nve_abi::SurfaceId;

use super::WordValue;
use crate::NodeId;

/// A value whose `Option<Self>` packs into `OPT_BITS` (`<= 32`) bits of one row word,
/// with bit pattern `0` always representing `None`.
///
/// `opt_from_bits` must be total: any invalid or out-of-range bit pattern decodes to `None`.
///
/// # Compile-time guard examples (with positive controls)
///
/// Valid `#[derive(WordEnum)]` (positive control):
/// ```
/// use nve_store::{OptWord, WordEnum, WordValue};
/// #[repr(u8)]
/// #[derive(Clone, Copy, Debug, PartialEq, Eq, Default, WordEnum)]
/// enum Good { #[default] Off = 0, On = 1 }
/// assert_eq!(Good::try_from_word(1), Some(Good::On));
/// assert_eq!(Good::opt_to_word(Some(Good::Off)), 1);
/// assert_eq!(<Option<Good> as WordValue>::from_bits(0), None);
/// ```
///
/// Rejecting a hand-rolled `Unset` variant:
/// ```compile_fail
/// use nve_store::WordEnum;
/// #[repr(u8)]
/// #[derive(Clone, Copy, WordEnum)]
/// enum Bad { Unset = 0, On = 1 }
/// ```
///
/// Rejecting non-contiguous discriminants:
/// ```compile_fail
/// use nve_store::WordEnum;
/// #[repr(u8)]
/// #[derive(Clone, Copy, WordEnum)]
/// enum BadGap { A = 0, B = 2 }
/// ```
///
/// Rejecting data-carrying variants:
/// ```compile_fail
/// use nve_store::WordEnum;
/// #[repr(u8)]
/// #[derive(Clone, Copy, WordEnum)]
/// enum BadData { A = 0, B(u8) }
/// ```
///
/// Valid resource field via `Res<R>` (positive control):
/// ```
/// use nve_store::{NveTable, Res, Resource};
/// use nve_abi::Key64;
/// struct Blob(Vec<u8>);
/// impl Resource for Blob {
///     const KIND: Key64 = Key64::of("doctest.blob");
///     fn encode(&self, out: &mut Vec<u8>) { out.extend_from_slice(&self.0); }
///     fn decode(bytes: &[u8]) -> Option<Self> { Some(Self(bytes.to_vec())) }
/// }
/// #[derive(NveTable)]
/// #[nve_table(name = "doctest.res_ok", storage = "sparse")]
/// struct ResOk { #[effect(paint)] blob: Res<Blob> }
/// let _ = ResOk::KEY;
/// ```
///
/// Rejecting raw `ResourceId` scalar field (must use refcounted `Res<R>`):
/// ```compile_fail
/// use nve_store::{NveTable, ResourceId};
/// #[derive(NveTable)]
/// #[nve_table(name = "doctest.bad_res", storage = "sparse")]
/// struct BadRes { #[effect(paint)] raw: ResourceId }
/// ```
///
/// Rejecting raw `Option<ResourceId>` field (must use refcounted `Res<R>`):
/// ```compile_fail
/// use nve_store::{NveTable, ResourceId};
/// #[derive(NveTable)]
/// #[nve_table(name = "doctest.bad_opt_res", storage = "sparse")]
/// struct BadOptRes { #[effect(paint)] raw: Option<ResourceId> }
/// ```
///
/// Valid integer narrowing `#[bits(N)]` (positive control):
/// ```
/// use nve_store::NveTable;
/// #[derive(NveTable)]
/// #[nve_table(name = "doctest.bits_ok", storage = "sparse")]
/// struct BitsOk {
///     #[effect(paint)] #[bits(4)] #[default(-3i8)] small_i: i8,
///     #[effect(paint)] #[bits(6)] small_u: u8,
/// }
/// assert_eq!(<BitsOk as NveTable>::DESC.words(), 1);
/// ```
///
/// Rejecting redundant `#[bits(8)]` on `u8`:
/// ```compile_fail
/// use nve_store::NveTable;
/// #[derive(NveTable)]
/// #[nve_table(name = "doctest.bad_bits_u8", storage = "sparse")]
/// struct BadBitsU8 { #[effect(paint)] #[bits(8)] x: u8 }
/// ```
///
/// Rejecting `#[bits(1)]` on `bool`:
/// ```compile_fail
/// use nve_store::NveTable;
/// #[derive(NveTable)]
/// #[nve_table(name = "doctest.bad_bits_bool", storage = "sparse")]
/// struct BadBitsBool { #[effect(paint)] #[bits(1)] x: bool }
/// ```
///
/// Rejecting `#[bits(4)]` on `Option<u8>`:
/// ```compile_fail
/// use nve_store::NveTable;
/// #[derive(NveTable)]
/// #[nve_table(name = "doctest.bad_bits_opt", storage = "sparse")]
/// struct BadBitsOpt { #[effect(paint)] #[bits(4)] x: Option<u8> }
/// ```
///
/// Rejecting a table exceeding 32 words at const-eval time:
/// ```compile_fail
/// use nve_store::NveTable;
/// #[derive(NveTable)]
/// #[nve_table(name = "doctest.too_wide", storage = "sparse")]
/// struct TooWide {
///     #[effect(paint)] w00: u64, #[effect(paint)] w01: u64, #[effect(paint)] w02: u64,
///     #[effect(paint)] w03: u64, #[effect(paint)] w04: u64, #[effect(paint)] w05: u64,
///     #[effect(paint)] w06: u64, #[effect(paint)] w07: u64, #[effect(paint)] w08: u64,
///     #[effect(paint)] w09: u64, #[effect(paint)] w10: u64, #[effect(paint)] w11: u64,
///     #[effect(paint)] w12: u64, #[effect(paint)] w13: u64, #[effect(paint)] w14: u64,
///     #[effect(paint)] w15: u64, #[effect(paint)] w16: u64,
/// }
/// ```
pub trait OptWord: Copy {
    const OPT_BITS: u8;
    const OPT_KIND: FieldKind;
    const OPT_ENCODING: OptEncoding;
    fn opt_to_bits(v: Option<Self>) -> u32;
    fn opt_from_bits(bits: u32) -> Option<Self>;
}

impl<T: OptWord> WordValue for Option<T> {
    const BITS: u8 = T::OPT_BITS;
    const KIND: FieldKind = T::OPT_KIND;
    const OPT_ENCODING: OptEncoding = T::OPT_ENCODING;

    #[inline(always)]
    fn to_bits(self) -> u32 {
        T::opt_to_bits(self)
    }

    #[inline(always)]
    fn from_bits(bits: u32) -> Self {
        T::opt_from_bits(bits)
    }
}

#[doc(hidden)]
#[inline(always)]
pub const fn opt_bool_to_word(v: Option<bool>) -> u32 {
    match v {
        None => 0,
        Some(false) => 1,
        Some(true) => 2,
    }
}

#[doc(hidden)]
#[inline(always)]
pub const fn opt_u8_to_word(v: Option<u8>) -> u32 {
    match v {
        None => 0,
        Some(x) => ((x as u32) << 1) | 1,
    }
}

#[doc(hidden)]
#[inline(always)]
pub const fn opt_i8_to_word(v: Option<i8>) -> u32 {
    match v {
        None => 0,
        Some(x) => (((x as u8) as u32) << 1) | 1,
    }
}

#[doc(hidden)]
#[inline(always)]
pub const fn opt_u16_to_word(v: Option<u16>) -> u32 {
    match v {
        None => 0,
        Some(x) => ((x as u32) << 1) | 1,
    }
}

#[doc(hidden)]
#[inline(always)]
pub const fn opt_i16_to_word(v: Option<i16>) -> u32 {
    match v {
        None => 0,
        Some(x) => (((x as u16) as u32) << 1) | 1,
    }
}

#[doc(hidden)]
#[inline(always)]
pub const fn opt_node_to_word(v: Option<NodeId>) -> u32 {
    match v {
        Some(id) if !id.is_null() => id.raw(),
        _ => 0,
    }
}

#[doc(hidden)]
#[inline(always)]
pub const fn opt_surf_to_word(v: Option<SurfaceId>) -> u32 {
    match v {
        Some(id) if !id.is_null() => id.raw(),
        _ => 0,
    }
}

impl OptWord for bool {
    const OPT_BITS: u8 = 2;
    const OPT_KIND: FieldKind = FieldKind::Enum;
    const OPT_ENCODING: OptEncoding = OptEncoding::PlusOne;

    #[inline(always)]
    fn opt_to_bits(v: Option<Self>) -> u32 {
        opt_bool_to_word(v)
    }

    #[inline(always)]
    fn opt_from_bits(bits: u32) -> Option<Self> {
        match bits & 0b11 {
            1 => Some(false),
            2 => Some(true),
            _ => None,
        }
    }
}

impl OptWord for u8 {
    const OPT_BITS: u8 = 9;
    const OPT_KIND: FieldKind = FieldKind::U32;
    const OPT_ENCODING: OptEncoding = OptEncoding::Bit0Present;

    #[inline(always)]
    fn opt_to_bits(v: Option<Self>) -> u32 {
        opt_u8_to_word(v)
    }

    #[inline(always)]
    fn opt_from_bits(bits: u32) -> Option<Self> {
        if bits & 1 == 0 {
            None
        } else {
            Some((bits >> 1) as u8)
        }
    }
}

impl OptWord for i8 {
    const OPT_BITS: u8 = 9;
    const OPT_KIND: FieldKind = FieldKind::I32;
    const OPT_ENCODING: OptEncoding = OptEncoding::Bit0Present;

    #[inline(always)]
    fn opt_to_bits(v: Option<Self>) -> u32 {
        opt_i8_to_word(v)
    }

    #[inline(always)]
    fn opt_from_bits(bits: u32) -> Option<Self> {
        if bits & 1 == 0 {
            None
        } else {
            Some(((bits >> 1) as u8) as i8)
        }
    }
}

impl OptWord for u16 {
    const OPT_BITS: u8 = 17;
    const OPT_KIND: FieldKind = FieldKind::U32;
    const OPT_ENCODING: OptEncoding = OptEncoding::Bit0Present;

    #[inline(always)]
    fn opt_to_bits(v: Option<Self>) -> u32 {
        opt_u16_to_word(v)
    }

    #[inline(always)]
    fn opt_from_bits(bits: u32) -> Option<Self> {
        if bits & 1 == 0 {
            None
        } else {
            Some((bits >> 1) as u16)
        }
    }
}

impl OptWord for i16 {
    const OPT_BITS: u8 = 17;
    const OPT_KIND: FieldKind = FieldKind::I32;
    const OPT_ENCODING: OptEncoding = OptEncoding::Bit0Present;

    #[inline(always)]
    fn opt_to_bits(v: Option<Self>) -> u32 {
        opt_i16_to_word(v)
    }

    #[inline(always)]
    fn opt_from_bits(bits: u32) -> Option<Self> {
        if bits & 1 == 0 {
            None
        } else {
            Some(((bits >> 1) as u16) as i16)
        }
    }
}

impl OptWord for NodeId {
    const OPT_BITS: u8 = 32;
    const OPT_KIND: FieldKind = FieldKind::Node;
    const OPT_ENCODING: OptEncoding = OptEncoding::ZeroNiche;

    #[inline(always)]
    fn opt_to_bits(v: Option<Self>) -> u32 {
        opt_node_to_word(v)
    }

    #[inline(always)]
    fn opt_from_bits(bits: u32) -> Option<Self> {
        NodeId::from_raw(bits)
    }
}

impl OptWord for SurfaceId {
    const OPT_BITS: u8 = 32;
    const OPT_KIND: FieldKind = FieldKind::U32;
    const OPT_ENCODING: OptEncoding = OptEncoding::ZeroNiche;

    #[inline(always)]
    fn opt_to_bits(v: Option<Self>) -> u32 {
        opt_surf_to_word(v)
    }

    #[inline(always)]
    fn opt_from_bits(bits: u32) -> Option<Self> {
        SurfaceId::from_raw(bits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opt_word_primitives_roundtrip_and_zero_is_none() {
        assert_eq!(<Option<bool> as WordValue>::to_bits(None), 0);
        assert_eq!(<Option<bool> as WordValue>::from_bits(0), None);
        assert_eq!(<Option<bool> as WordValue>::from_bits(3), None);
        for b in [false, true] {
            let bits = Some(b).to_bits();
            assert_ne!(bits, 0);
            assert_eq!(<Option<bool> as WordValue>::from_bits(bits), Some(b));
        }

        for x in [0u8, 1, 127, 255] {
            let bits = Some(x).to_bits();
            assert!(bits != 0 && bits < (1 << 9));
            assert_eq!(<Option<u8> as WordValue>::from_bits(bits), Some(x));
        }
        assert_eq!(<Option<u8> as WordValue>::from_bits(0b10), None);

        for x in [i8::MIN, -1, 0, 1, i8::MAX] {
            let bits = Some(x).to_bits();
            assert!(bits != 0 && bits < (1 << 9));
            assert_eq!(<Option<i8> as WordValue>::from_bits(bits), Some(x));
        }

        for x in [0u16, 1, 128, u16::MAX] {
            let bits = Some(x).to_bits();
            assert!(bits != 0 && bits < (1 << 17));
            assert_eq!(<Option<u16> as WordValue>::from_bits(bits), Some(x));
        }

        for x in [i16::MIN, -1, 0, 1, i16::MAX] {
            let bits = Some(x).to_bits();
            assert!(bits != 0 && bits < (1 << 17));
            assert_eq!(<Option<i16> as WordValue>::from_bits(bits), Some(x));
        }
        assert_eq!(<Option<i16> as WordValue>::from_bits(0), None);
        assert_eq!(<Option<i16> as WordValue>::from_bits(0xFFFE), None);

        assert_eq!(<Option<NodeId> as WordValue>::from_bits(0), None);
        assert_eq!(<Option<NodeId> as WordValue>::to_bits(Some(NodeId::NULL)), 0);
        assert_eq!(
            <Option<NodeId> as WordValue>::from_bits(Some(NodeId::ROOT).to_bits()),
            Some(NodeId::ROOT)
        );
        assert_eq!(<Option<SurfaceId> as WordValue>::from_bits(0), None);
        assert_eq!(
            <Option<SurfaceId> as WordValue>::from_bits(Some(SurfaceId::ROOT).to_bits()),
            Some(SurfaceId::ROOT)
        );
    }
}
