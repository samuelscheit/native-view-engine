//! Values that pack into (part of) one row word, plus `OptWord`, `GridLine`, `Length`, and
//! `word_flags!`.

mod grid_line;
mod length;
mod opt;

use nve_abi::table::{FieldKind, OptEncoding};
use nve_abi::SurfaceId;

pub use grid_line::GridLine;
pub use length::Length;
pub use opt::{
    opt_bool_to_word, opt_i16_to_word, opt_i8_to_word, opt_node_to_word, opt_surf_to_word,
    opt_u16_to_word, opt_u8_to_word, OptWord,
};

/// A value stored in `BITS` bits of one row word.
///
/// `from_bits` must be total: any bit pattern (e.g. from a corrupt wire frame) decodes to some
/// value (enums fall back to their default variant).
pub trait WordValue: Copy {
    const BITS: u8;
    const KIND: FieldKind;
    /// Optionality & bit-encoding discriminator hashed into `TableDesc::schema_hash`.
    const OPT_ENCODING: OptEncoding = OptEncoding::Required;
    fn to_bits(self) -> u32;
    fn from_bits(bits: u32) -> Self;

    /// Encodes `self` into a field of `bit_width` (`1..=32`) bits.
    ///
    /// For narrowed integer fields (`#[bits(N)]`), debug builds assert that `self` fits in the
    /// narrowed signed/unsigned range; release builds truncate/wrap modulo `2^bit_width` via the
    /// caller's branchless bitmask write.
    #[inline(always)]
    fn to_masked_bits(self, _bit_width: u32) -> u32 {
        self.to_bits()
    }

    /// Decodes `bits` from a field of `bit_width` (`1..=32`) bits (sign-extending narrowed signed
    /// integers).
    #[inline(always)]
    fn from_masked_bits(bits: u32, _bit_width: u32) -> Self {
        Self::from_bits(bits)
    }
}

impl WordValue for f32 {
    const BITS: u8 = 32;
    const KIND: FieldKind = FieldKind::F32;
    #[inline(always)]
    fn to_bits(self) -> u32 {
        f32::to_bits(self)
    }
    #[inline(always)]
    fn from_bits(bits: u32) -> Self {
        f32::from_bits(bits)
    }
}

impl WordValue for u32 {
    const BITS: u8 = 32;
    const KIND: FieldKind = FieldKind::U32;
    #[inline(always)]
    fn to_bits(self) -> u32 {
        self
    }
    #[inline(always)]
    fn from_bits(bits: u32) -> Self {
        bits
    }
    #[inline(always)]
    fn to_masked_bits(self, bit_width: u32) -> u32 {
        debug_assert!((1..=32).contains(&bit_width), "bit_width {bit_width} must be in 1..=32");
        let w = bit_width.clamp(1, 32);
        debug_assert!(
            w >= 32 || self < (1u32 << w),
            "u32 value {self} overflows #[bits({w})]"
        );
        self
    }
}

impl WordValue for i32 {
    const BITS: u8 = 32;
    const KIND: FieldKind = FieldKind::I32;
    #[inline(always)]
    fn to_bits(self) -> u32 {
        self as u32
    }
    #[inline(always)]
    fn from_bits(bits: u32) -> Self {
        bits as i32
    }
    #[inline(always)]
    fn to_masked_bits(self, bit_width: u32) -> u32 {
        debug_assert!((1..=32).contains(&bit_width), "bit_width {bit_width} must be in 1..=32");
        let w = bit_width.clamp(1, 32);
        debug_assert!(
            w >= 32
                || (i64::from(self) >= -(1i64 << (w - 1))
                    && i64::from(self) < (1i64 << (w - 1))),
            "i32 value {self} overflows signed #[bits({w})]"
        );
        self as u32
    }
    #[inline(always)]
    fn from_masked_bits(bits: u32, bit_width: u32) -> Self {
        debug_assert!((1..=32).contains(&bit_width), "bit_width {bit_width} must be in 1..=32");
        let shift = 32 - bit_width.clamp(1, 32);
        ((bits << shift) as i32) >> shift
    }
}

impl WordValue for bool {
    const BITS: u8 = 1;
    const KIND: FieldKind = FieldKind::Bool;
    #[inline(always)]
    fn to_bits(self) -> u32 {
        self as u32
    }
    #[inline(always)]
    fn from_bits(bits: u32) -> Self {
        bits & 1 != 0
    }
}

impl WordValue for u8 {
    const BITS: u8 = 8;
    const KIND: FieldKind = FieldKind::U32;
    #[inline(always)]
    fn to_bits(self) -> u32 {
        self as u32
    }
    #[inline(always)]
    fn from_bits(bits: u32) -> Self {
        bits as u8
    }
    #[inline(always)]
    fn to_masked_bits(self, bit_width: u32) -> u32 {
        debug_assert!((1..=32).contains(&bit_width), "bit_width {bit_width} must be in 1..=32");
        let w = bit_width.clamp(1, 32);
        debug_assert!(
            w >= 8 || u32::from(self) < (1u32 << w),
            "u8 value {self} overflows #[bits({w})]"
        );
        u32::from(self)
    }
}

impl WordValue for i8 {
    const BITS: u8 = 8;
    const KIND: FieldKind = FieldKind::I32;
    #[inline(always)]
    fn to_bits(self) -> u32 {
        (self as u8) as u32
    }
    #[inline(always)]
    fn from_bits(bits: u32) -> Self {
        (bits as u8) as i8
    }
    #[inline(always)]
    fn to_masked_bits(self, bit_width: u32) -> u32 {
        debug_assert!((1..=32).contains(&bit_width), "bit_width {bit_width} must be in 1..=32");
        let w = bit_width.clamp(1, 32);
        debug_assert!(
            w >= 8
                || (i32::from(self) >= -(1i32 << (w - 1))
                    && i32::from(self) < (1i32 << (w - 1))),
            "i8 value {self} overflows signed #[bits({w})]"
        );
        (self as u8) as u32
    }
    #[inline(always)]
    fn from_masked_bits(bits: u32, bit_width: u32) -> Self {
        debug_assert!((1..=32).contains(&bit_width), "bit_width {bit_width} must be in 1..=32");
        let shift = 32 - bit_width.clamp(1, 8);
        (((bits << shift) as i32) >> shift) as i8
    }
}

impl WordValue for u16 {
    const BITS: u8 = 16;
    const KIND: FieldKind = FieldKind::U32;
    #[inline(always)]
    fn to_bits(self) -> u32 {
        self as u32
    }
    #[inline(always)]
    fn from_bits(bits: u32) -> Self {
        bits as u16
    }
    #[inline(always)]
    fn to_masked_bits(self, bit_width: u32) -> u32 {
        debug_assert!((1..=32).contains(&bit_width), "bit_width {bit_width} must be in 1..=32");
        let w = bit_width.clamp(1, 32);
        debug_assert!(
            w >= 16 || u32::from(self) < (1u32 << w),
            "u16 value {self} overflows #[bits({w})]"
        );
        u32::from(self)
    }
}

impl WordValue for i16 {
    const BITS: u8 = 16;
    const KIND: FieldKind = FieldKind::I32;
    #[inline(always)]
    fn to_bits(self) -> u32 {
        (self as u16) as u32
    }
    #[inline(always)]
    fn from_bits(bits: u32) -> Self {
        (bits as u16) as i16
    }
    #[inline(always)]
    fn to_masked_bits(self, bit_width: u32) -> u32 {
        debug_assert!((1..=32).contains(&bit_width), "bit_width {bit_width} must be in 1..=32");
        let w = bit_width.clamp(1, 32);
        debug_assert!(
            w >= 16
                || (i32::from(self) >= -(1i32 << (w - 1))
                    && i32::from(self) < (1i32 << (w - 1))),
            "i16 value {self} overflows signed #[bits({w})]"
        );
        (self as u16) as u32
    }
    #[inline(always)]
    fn from_masked_bits(bits: u32, bit_width: u32) -> Self {
        debug_assert!((1..=32).contains(&bit_width), "bit_width {bit_width} must be in 1..=32");
        let shift = 32 - bit_width.clamp(1, 16);
        (((bits << shift) as i32) >> shift) as i16
    }
}

impl WordValue for crate::NodeId {
    const BITS: u8 = 32;
    const KIND: FieldKind = FieldKind::Node;
    #[inline(always)]
    fn to_bits(self) -> u32 {
        self.raw()
    }
    #[inline(always)]
    fn from_bits(bits: u32) -> Self {
        Self::from_raw_or_null(bits)
    }
}

impl WordValue for SurfaceId {
    const BITS: u8 = 32;
    const KIND: FieldKind = FieldKind::U32;
    #[inline(always)]
    fn to_bits(self) -> u32 {
        self.raw()
    }
    #[inline(always)]
    fn from_bits(bits: u32) -> Self {
        Self::from_raw(bits).unwrap_or(Self::NULL)
    }
}

/// Packed `RGBA8888` color (`r | g<<8 | b<<16 | a<<24`).
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Rgba(pub u32);

impl Rgba {
    pub const TRANSPARENT: Self = Self(0);
    pub const BLACK: Self = Self::rgb(0, 0, 0);
    pub const WHITE: Self = Self::rgb(255, 255, 255);

    #[inline(always)]
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self(u32::from_le_bytes([r, g, b, a]))
    }
    #[inline(always)]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self::new(r, g, b, 255)
    }
    #[inline(always)]
    pub const fn r(self) -> u8 {
        self.0 as u8
    }
    #[inline(always)]
    pub const fn g(self) -> u8 {
        (self.0 >> 8) as u8
    }
    #[inline(always)]
    pub const fn b(self) -> u8 {
        (self.0 >> 16) as u8
    }
    #[inline(always)]
    pub const fn a(self) -> u8 {
        (self.0 >> 24) as u8
    }
    #[inline(always)]
    pub const fn with_alpha(self, a: u8) -> Self {
        Self(self.0 & 0x00FF_FFFF | (a as u32) << 24)
    }
    #[inline(always)]
    pub const fn to_word(self) -> u32 {
        self.0
    }
}

impl WordValue for Rgba {
    const BITS: u8 = 32;
    const KIND: FieldKind = FieldKind::Color;
    #[inline(always)]
    fn to_bits(self) -> u32 {
        self.0
    }
    #[inline(always)]
    fn from_bits(bits: u32) -> Self {
        Self(bits)
    }
}

/// Computes the minimum bit width needed to pack a bitmask `all_mask` (minimum 1 bit).
#[doc(hidden)]
pub const fn bitflags_width(all_mask: u32) -> u8 {
    if all_mask == 0 {
        1
    } else {
        (u32::BITS - all_mask.leading_zeros()) as u8
    }
}

/// Declares a packed `#[repr(transparent)]` bitflags type backed by `u8` / `u16` / `u32` that
/// wraps `nve_abi::define_bitflags!` and automatically implements `Default`, [`WordValue`], and
/// (for `u8`/`u16`) [`OptWord`] with exact compile-time bit widths.
#[macro_export]
macro_rules! word_flags {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident(u8) {
            $(
                $(#[$flag_meta:meta])*
                const $flag:ident = $val:expr;
            )*
        }
    ) => {
        $crate::__private::define_bitflags! {
            $(#[$meta])*
            #[derive(Default)]
            $vis struct $name(u8) {
                $(
                    $(#[$flag_meta])*
                    const $flag = $val;
                )*
            }
        }
        $crate::__word_flags_impl!($name, u8, opt);
    };
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident(u16) {
            $(
                $(#[$flag_meta:meta])*
                const $flag:ident = $val:expr;
            )*
        }
    ) => {
        $crate::__private::define_bitflags! {
            $(#[$meta])*
            #[derive(Default)]
            $vis struct $name(u16) {
                $(
                    $(#[$flag_meta])*
                    const $flag = $val;
                )*
            }
        }
        $crate::__word_flags_impl!($name, u16, opt);
    };
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident(u32) {
            $(
                $(#[$flag_meta:meta])*
                const $flag:ident = $val:expr;
            )*
        }
    ) => {
        $crate::__private::define_bitflags! {
            $(#[$meta])*
            #[derive(Default)]
            $vis struct $name(u32) {
                $(
                    $(#[$flag_meta])*
                    const $flag = $val;
                )*
            }
        }
        $crate::__word_flags_impl!($name, u32, wide);
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __word_flags_impl {
    ($name:ident, $repr:ty, opt) => {
        $crate::__word_flags_impl!($name, $repr, wide);

        #[allow(dead_code)]
        impl $name {
            #[inline(always)]
            pub const fn opt_to_word(v: ::core::option::Option<Self>) -> u32 {
                match v {
                    ::core::option::Option::None => 0,
                    ::core::option::Option::Some(f) => {
                        (((f.bits() & Self::ALL.bits()) as u32) << 1) | 1
                    }
                }
            }
        }

        impl $crate::OptWord for $name {
            const OPT_BITS: u8 = <Self as $crate::WordValue>::BITS + 1;
            const OPT_KIND: $crate::__private::FieldKind = $crate::__private::FieldKind::U32;
            const OPT_ENCODING: $crate::__private::OptEncoding =
                $crate::__private::OptEncoding::Bit0Present;

            #[inline(always)]
            fn opt_to_bits(v: ::core::option::Option<Self>) -> u32 {
                Self::opt_to_word(v)
            }

            #[inline(always)]
            fn opt_from_bits(bits: u32) -> ::core::option::Option<Self> {
                if bits & 1 == 0 {
                    ::core::option::Option::None
                } else {
                    ::core::option::Option::Some(Self::from_bits_truncate((bits >> 1) as $repr))
                }
            }
        }
    };
    ($name:ident, $repr:ty, wide) => {
        #[allow(dead_code)]
        impl $name {
            pub const BITS: u8 = $crate::__private::bitflags_width(Self::ALL.bits() as u32);

            #[inline(always)]
            pub const fn to_word(self) -> u32 {
                (self.bits() as u32) & (Self::ALL.bits() as u32)
            }

            #[inline(always)]
            pub const fn from_word(bits: u32) -> Self {
                Self::from_bits_truncate(bits as $repr)
            }
        }

        impl $crate::WordValue for $name {
            const BITS: u8 = Self::BITS;
            const KIND: $crate::__private::FieldKind = $crate::__private::FieldKind::U32;

            #[inline(always)]
            fn to_bits(self) -> u32 {
                self.to_word()
            }

            #[inline(always)]
            fn from_bits(bits: u32) -> Self {
                Self::from_bits_truncate(bits as $repr)
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WordEnum;

    #[repr(u8)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, WordEnum)]
    enum Dir {
        #[default]
        Column = 0,
        Row = 1,
        ColumnReverse = 2,
        RowReverse = 3,
    }

    word_flags! {
        struct Deco(u8) {
            const UNDERLINE = 1 << 0;
            const OVERLINE = 1 << 1;
            const LINE_THROUGH = 1 << 2;
        }
    }

    word_flags! {
        struct SparseFlags(u8) {
            const A = 1 << 0;
            const C = 1 << 2;
        }
    }

    #[test]
    fn word_enum_and_word_flags_are_total_and_const() {
        const W: u32 = Dir::RowReverse.to_word();
        assert_eq!(W, 3);
        assert_eq!(Dir::COUNT, 4);
        assert_eq!(Dir::BITS, 2);
        assert_eq!(<Option<Dir> as WordValue>::BITS, 3);
        assert_eq!(Dir::try_from_word(1), Some(Dir::Row));
        assert_eq!(Dir::try_from_word(4), None);
        assert_eq!(Dir::from_bits(1), Dir::Row);
        assert_eq!(Dir::from_bits(9), Dir::Column);
        assert_eq!(Dir::default(), Dir::Column);

        assert_eq!(<Option<Dir> as WordValue>::from_bits(0), None);
        for &d in Dir::ALL {
            let bits = Some(d).to_bits();
            assert_eq!(bits, Dir::opt_to_word(Some(d)));
            assert_ne!(bits, 0);
            assert_eq!(<Option<Dir> as WordValue>::from_bits(bits), Some(d));
        }
        assert_eq!(<Option<Dir> as WordValue>::from_bits(7), None);

        assert_eq!(Deco::BITS, 3);
        assert_eq!(<Option<Deco> as WordValue>::BITS, 4);
        let d = Deco::UNDERLINE | Deco::LINE_THROUGH;
        assert_eq!(Deco::from_bits(d.to_bits()), d);
        assert_eq!(Deco::from_bits(0xFF), Deco::ALL);
        assert_eq!(<Option<Deco> as WordValue>::from_bits(0), None);
        assert_eq!(
            <Option<Deco> as WordValue>::from_bits(Some(Deco::NONE).to_bits()),
            Some(Deco::NONE)
        );
        assert_eq!(
            <Option<Deco> as WordValue>::from_bits(Some(d).to_bits()),
            Some(d)
        );

        // Sparse flags must mask undeclared/reserved bits in both `to_word` and `opt_to_word`.
        let dirty = SparseFlags::from_bits_retain(0xFF);
        assert_eq!(dirty.to_word(), 0b101);
        assert_eq!(dirty.to_bits(), 0b101);
        assert_eq!(SparseFlags::opt_to_word(Some(dirty)), (0b101 << 1) | 1);
        assert_eq!(Some(dirty).to_bits(), (0b101 << 1) | 1);
        assert_eq!(
            <Option<SparseFlags> as WordValue>::from_bits(Some(dirty).to_bits()),
            Some(SparseFlags::A | SparseFlags::C)
        );
    }

    #[test]
    fn rgba_channels() {
        let c = Rgba::new(1, 2, 3, 4);
        assert_eq!((c.r(), c.g(), c.b(), c.a()), (1, 2, 3, 4));
        assert_eq!(c.with_alpha(9).a(), 9);
    }
}
