//! Niche/packed `Option<T>` support (`OptWord`) for sub-word and niche-bearing `WordValue` types.
//!
//! Every `OptWord` encoding guarantees that bit pattern `0` represents `None`, so all-zero
//! default table words decode cleanly to `None` with zero per-field initialization cost.

use nve_abi::table::FieldKind;
use nve_abi::SurfaceId;

use super::WordValue;
use crate::{NodeId, ResourceId};

/// A value whose `Option<Self>` packs into `OPT_BITS` (`<= 32`) bits of one row word,
/// with bit pattern `0` always representing `None`.
///
/// `opt_from_bits` must be total: any invalid or out-of-range bit pattern decodes to `None`.
///
/// # Compile-fail guard examples for `#[derive(WordEnum)]`
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
pub trait OptWord: Copy {
    const OPT_BITS: u8;
    const OPT_KIND: FieldKind;
    fn opt_to_bits(v: Option<Self>) -> u32;
    fn opt_from_bits(bits: u32) -> Option<Self>;
}

impl<T: OptWord> WordValue for Option<T> {
    const BITS: u8 = T::OPT_BITS;
    const KIND: FieldKind = T::OPT_KIND;

    #[inline(always)]
    fn to_bits(self) -> u32 {
        T::opt_to_bits(self)
    }

    #[inline(always)]
    fn from_bits(bits: u32) -> Self {
        T::opt_from_bits(bits)
    }
}

impl OptWord for bool {
    const OPT_BITS: u8 = 2;
    const OPT_KIND: FieldKind = FieldKind::Enum;

    #[inline(always)]
    fn opt_to_bits(v: Option<Self>) -> u32 {
        match v {
            None => 0,
            Some(false) => 1,
            Some(true) => 2,
        }
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

    #[inline(always)]
    fn opt_to_bits(v: Option<Self>) -> u32 {
        match v {
            None => 0,
            Some(x) => ((x as u32) << 1) | 1,
        }
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

    #[inline(always)]
    fn opt_to_bits(v: Option<Self>) -> u32 {
        match v {
            None => 0,
            Some(x) => (((x as u8) as u32) << 1) | 1,
        }
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

    #[inline(always)]
    fn opt_to_bits(v: Option<Self>) -> u32 {
        match v {
            None => 0,
            Some(x) => ((x as u32) << 1) | 1,
        }
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

    #[inline(always)]
    fn opt_to_bits(v: Option<Self>) -> u32 {
        match v {
            None => 0,
            Some(x) => (((x as u16) as u32) << 1) | 1,
        }
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

    #[inline(always)]
    fn opt_to_bits(v: Option<Self>) -> u32 {
        match v {
            Some(id) if !id.is_null() => id.raw(),
            _ => 0,
        }
    }

    #[inline(always)]
    fn opt_from_bits(bits: u32) -> Option<Self> {
        NodeId::from_raw(bits)
    }
}

impl OptWord for ResourceId {
    const OPT_BITS: u8 = 32;
    const OPT_KIND: FieldKind = FieldKind::Resource;

    #[inline(always)]
    fn opt_to_bits(v: Option<Self>) -> u32 {
        match v {
            Some(id) if !id.is_none() => id.0,
            _ => 0,
        }
    }

    #[inline(always)]
    fn opt_from_bits(bits: u32) -> Option<Self> {
        (bits != 0).then_some(ResourceId(bits))
    }
}

impl OptWord for SurfaceId {
    const OPT_BITS: u8 = 32;
    const OPT_KIND: FieldKind = FieldKind::U32;

    #[inline(always)]
    fn opt_to_bits(v: Option<Self>) -> u32 {
        match v {
            Some(id) if !id.is_null() => id.raw(),
            _ => 0,
        }
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
        assert_eq!(<Option<ResourceId> as WordValue>::from_bits(0), None);
        assert_eq!(
            <Option<ResourceId> as WordValue>::from_bits(Some(ResourceId(7)).to_bits()),
            Some(ResourceId(7))
        );
        assert_eq!(<Option<SurfaceId> as WordValue>::from_bits(0), None);
        assert_eq!(
            <Option<SurfaceId> as WordValue>::from_bits(Some(SurfaceId::ROOT).to_bits()),
            Some(SurfaceId::ROOT)
        );
    }
}
