//! Packed CSS / Flexbox `Length` value with a free `0b11` niche for zero-overhead `Option<Length>`.

use nve_abi::table::FieldKind;

use super::{OptWord, WordValue};

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

impl OptWord for Length {
    const OPT_BITS: u8 = 32;
    const OPT_KIND: FieldKind = FieldKind::Length;

    #[inline(always)]
    fn opt_to_bits(v: Option<Self>) -> u32 {
        match v {
            None => 0,
            // Valid tags are {00, 01, 10}; XORing with 0b11 maps them to {11, 10, 01},
            // guaranteeing non-zero low tag bits so `0` is strictly `None`.
            Some(l) => Self::from_bits(l.0).0 ^ Self::TAG_MASK,
        }
    }

    #[inline(always)]
    fn opt_from_bits(bits: u32) -> Option<Self> {
        if bits & Self::TAG_MASK == 0 {
            None
        } else {
            Some(Self(bits ^ Self::TAG_MASK))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_and_opt_length_roundtrip() {
        for v in [0.0f32, 1.0, 16.0, 0.5, 1234.0, -8.0] {
            assert_eq!(Length::points(v).value(), v);
            assert_eq!(Length::percent(v).value(), v);
            assert!(Length::points(v).is_points());
            assert!(Length::percent(v).is_percent());

            let p = Length::points(v);
            let pb = Some(p).to_bits();
            assert_ne!(pb, 0);
            assert_eq!(<Option<Length> as WordValue>::from_bits(pb), Some(p));

            let pct = Length::percent(v);
            let pcb = Some(pct).to_bits();
            assert_ne!(pcb, 0);
            assert_eq!(<Option<Length> as WordValue>::from_bits(pcb), Some(pct));
        }
        let third = Length::percent(100.0 / 3.0).value();
        assert!((third - 100.0 / 3.0).abs() < 1e-4);
        assert!(Length::AUTO.is_auto());
        assert_eq!(Length::from_bits(0b11), Length::AUTO);
        assert_eq!(Length::percent(50.0).resolve(200.0), Some(100.0));
        assert_eq!(Length::AUTO.resolve(200.0), None);

        assert_eq!(<Option<Length> as WordValue>::to_bits(None), 0);
        assert_eq!(<Option<Length> as WordValue>::from_bits(0), None);
        assert_eq!(<Option<Length> as WordValue>::from_bits(0x1234_5600), None);
        assert_eq!(
            <Option<Length> as WordValue>::from_bits(Some(Length::AUTO).to_bits()),
            Some(Length::AUTO)
        );
        assert_eq!(
            <Option<Length> as WordValue>::from_bits(Some(Length::ZERO).to_bits()),
            Some(Length::ZERO)
        );
        // Raw invalid tag 0b11 normalizes to AUTO and never aliases None.
        assert_eq!(
            <Option<Length> as WordValue>::from_bits(Some(Length(0b11)).to_bits()),
            Some(Length::AUTO)
        );
    }
}
