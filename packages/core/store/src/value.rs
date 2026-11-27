//! Values that pack into (part of) one row word.

use nve_abi::table::{FieldKind, ValueSetDesc};

/// A value stored in `BITS` bits of one row word.
///
/// `from_bits` must be total: any bit pattern (e.g. from a corrupt wire frame) decodes to some
/// value (enums fall back to their default variant).
pub trait WordValue: Copy {
    const BITS: u8;
    const KIND: FieldKind;
    /// Named values (set by `word_enum!`), mirrored to hosts as `NVE_<TYPE>_<VARIANT>`.
    const VALUES: Option<&'static ValueSetDesc> = None;
    fn to_bits(self) -> u32;
    fn from_bits(bits: u32) -> Self;
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
}

impl WordValue for crate::NodeId {
    const BITS: u8 = 32;
    const KIND: FieldKind = FieldKind::Node;
    #[inline(always)]
    fn to_bits(self) -> u32 {
        self.0
    }
    #[inline(always)]
    fn from_bits(bits: u32) -> Self {
        Self(bits)
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

/// CSS / Flexbox length packed in one word: `f32` bits with the 2-bit unit tag in the low mantissa
/// bits (`00` points, `01` percent, `10` auto). Quantization is `< 2^-21` relative — sub-micro-pixel
/// for UI magnitudes, exact for integers and halves.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Length(pub u32);

impl Length {
    const TAG_MASK: u32 = 0b11;
    const TAG_POINTS: u32 = 0b00;
    const TAG_PERCENT: u32 = 0b01;
    const TAG_AUTO: u32 = 0b10;

    pub const AUTO: Self = Self(Self::TAG_AUTO);
    pub const ZERO: Self = Self(0);

    #[inline(always)]
    const fn quantize(v: f32) -> u32 {
        if !v.is_finite() || v == 0.0 {
            return 0;
        }
        let bits = v.to_bits();
        // Avoid exponent overflow into +/-Inf when adding 2 ulps near f32::MAX.
        if (bits & 0x7FFF_FFFF) >= 0x7F7F_FFFE {
            return bits & !Self::TAG_MASK;
        }
        // Round to nearest multiple of 4 ulps so the tag bits are free.
        (bits.wrapping_add(2)) & !Self::TAG_MASK
    }

    #[inline(always)]
    pub const fn points(v: f32) -> Self {
        Self(Self::quantize(v) | Self::TAG_POINTS)
    }

    /// `percent(50.0)` is 50 %.
    #[inline(always)]
    pub const fn percent(v: f32) -> Self {
        Self(Self::quantize(v) | Self::TAG_PERCENT)
    }

    #[inline(always)]
    pub const fn value(self) -> f32 {
        f32::from_bits(self.0 & !Self::TAG_MASK)
    }

    #[inline(always)]
    pub const fn is_auto(self) -> bool {
        self.0 & Self::TAG_MASK == Self::TAG_AUTO
    }

    #[inline(always)]
    pub const fn is_percent(self) -> bool {
        self.0 & Self::TAG_MASK == Self::TAG_PERCENT
    }

    #[inline(always)]
    pub const fn is_points(self) -> bool {
        self.0 & Self::TAG_MASK == Self::TAG_POINTS
    }

    #[inline(always)]
    pub const fn to_word(self) -> u32 {
        self.0
    }

    /// Resolves against `basis` (percent base); `None` for `auto`.
    #[inline]
    pub fn resolve(self, basis: f32) -> Option<f32> {
        match self.0 & Self::TAG_MASK {
            Self::TAG_POINTS => Some(self.value()),
            Self::TAG_PERCENT => Some(self.value() * basis / 100.0),
            _ => None,
        }
    }
}

impl Default for Length {
    fn default() -> Self {
        Self::AUTO
    }
}

impl WordValue for Length {
    const BITS: u8 = 32;
    const KIND: FieldKind = FieldKind::Length;
    #[inline(always)]
    fn to_bits(self) -> u32 {
        self.0
    }
    #[inline(always)]
    fn from_bits(bits: u32) -> Self {
        // Tag `11` is unused: normalize to auto so every pattern is valid.
        if bits & Self::TAG_MASK == Self::TAG_MASK {
            Self::AUTO
        } else {
            Self(bits)
        }
    }
}

/// The [`ValueSetDesc`] of a hand-written fieldless enum [`WordValue`] (one whose type also needs
/// derives or methods `word_enum!` cannot carry): `const VALUES = value_set!(Mode { A, B });`.
/// The list must name every variant: a missing one fails the exhaustive `match` at compile time.
#[macro_export]
macro_rules! value_set {
    ($ty:ident { $($variant:ident),+ $(,)? }) => {{
        const _: fn($ty) = |v| match v { $($ty::$variant)|+ => {} };
        Some(&$crate::__private::ValueSetDesc {
            name: stringify!($ty),
            flags: false,
            values: &[$((stringify!($variant), $ty::$variant as u32)),+],
        })
    }};
}

/// Declares a fieldless `#[repr(u8)]` enum as a packed [`WordValue`] with const `to_word`.
///
/// Out-of-range bits decode to the first (default) variant.
#[macro_export]
macro_rules! word_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident : $bits:literal {
            $( $(#[$vmeta:meta])* $variant:ident = $value:literal ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[repr(u8)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        $vis enum $name { $( $(#[$vmeta])* $variant = $value ),+ }

        #[allow(dead_code)]
        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];
            #[inline(always)]
            pub const fn to_word(self) -> u32 { self as u32 }
            #[inline(always)]
            pub const fn from_word(bits: u32) -> Self {
                match bits { $( $value => $name::$variant, )+ _ => Self::ALL[0] }
            }
        }

        impl Default for $name {
            #[inline(always)]
            fn default() -> Self { Self::ALL[0] }
        }

        impl $crate::WordValue for $name {
            const BITS: u8 = $bits;
            const KIND: $crate::__private::FieldKind = $crate::__private::FieldKind::Enum;
            const VALUES: Option<&'static $crate::__private::ValueSetDesc> =
                Some(&$crate::__private::ValueSetDesc {
                    name: stringify!($name),
                    flags: false,
                    values: &[$((stringify!($variant), $value)),+],
                });
            #[inline(always)]
            fn to_bits(self) -> u32 { self as u32 }
            #[inline(always)]
            fn from_bits(bits: u32) -> Self { Self::from_word(bits) }
        }

        const _: () = {
            assert!(
                ($name::ALL.len() as u32) <= (1u32 << $bits),
                "word_enum variants exceed the declared bit width"
            );
            $(
                assert!(
                    ($value as u32) < (1u32 << $bits),
                    "word_enum variant discriminant exceeds the declared bit width"
                );
            )+
        };
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    word_enum! {
        enum Dir: 2 { Column = 0, Row = 1, ColumnReverse = 2, RowReverse = 3 }
    }

    #[test]
    fn length_roundtrips_tags_and_exact_integers() {
        for v in [0.0f32, 1.0, 16.0, 0.5, 1234.0, -8.0] {
            assert_eq!(Length::points(v).value(), v);
            assert_eq!(Length::percent(v).value(), v);
            assert!(Length::points(v).is_points());
            assert!(Length::percent(v).is_percent());
        }
        let third = Length::percent(100.0 / 3.0).value();
        assert!((third - 100.0 / 3.0).abs() < 1e-4);
        assert!(Length::AUTO.is_auto());
        assert_eq!(Length::from_bits(0b11), Length::AUTO);
        assert_eq!(Length::percent(50.0).resolve(200.0), Some(100.0));
        assert_eq!(Length::AUTO.resolve(200.0), None);
    }

    #[test]
    fn word_enum_is_total_and_const() {
        const W: u32 = Dir::RowReverse.to_word();
        assert_eq!(W, 3);
        assert_eq!(Dir::from_bits(1), Dir::Row);
        assert_eq!(Dir::from_bits(9), Dir::Column);
        assert_eq!(Dir::default(), Dir::Column);
    }

    #[test]
    fn rgba_channels() {
        let c = Rgba::new(1, 2, 3, 4);
        assert_eq!((c.r(), c.g(), c.b(), c.a()), (1, 2, 3, 4));
        assert_eq!(c.with_alpha(9).a(), 9);
    }
}
