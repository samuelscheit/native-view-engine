//! Packed 16-bit CSS Grid line placement (`Auto`, `Line(i16)`, `Span(u16)`) with a free `0b11`
//! tag niche so `Option<GridLine>` also packs into 16 bits with `0 = None`.

use nve_abi::table::{FieldKind, OptEncoding};

use super::{OptWord, WordValue};

/// One grid line: `Auto`, a 1-based (negative = from end) line, or a span.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GridLine {
    #[default]
    Auto,
    Line(i16),
    Span(u16),
}

impl GridLine {
    /// Minimum representable signed 14-bit grid line (`-8192`).
    pub const MIN_LINE: i16 = -8192;
    /// Maximum representable signed 14-bit grid line (`8191`).
    pub const MAX_LINE: i16 = 8191;
    /// Maximum representable unsigned 14-bit grid span (`16383`).
    pub const MAX_SPAN: u16 = 16383;

    const TAG_SHIFT: u32 = 14;
    const TAG_NICHE: u32 = 0b11 << Self::TAG_SHIFT;

    /// 16-bit packing: tag in bits 14-15, saturating 14-bit payload below.
    #[inline(always)]
    pub const fn to_word(self) -> u32 {
        match self {
            Self::Auto => 0,
            Self::Line(v) => {
                let clamped = if v < Self::MIN_LINE {
                    Self::MIN_LINE
                } else if v > Self::MAX_LINE {
                    Self::MAX_LINE
                } else {
                    v
                };
                (1 << Self::TAG_SHIFT) | ((clamped as u32) & 0x3FFF)
            }
            Self::Span(v) => {
                let clamped = if v > Self::MAX_SPAN {
                    Self::MAX_SPAN
                } else {
                    v
                };
                (2 << Self::TAG_SHIFT) | (clamped as u32)
            }
        }
    }

    #[inline(always)]
    pub const fn from_word(bits: u32) -> Self {
        let v = bits & 0x3FFF;
        match (bits >> Self::TAG_SHIFT) & 0b11 {
            1 => Self::Line(((v << 18) as i32 >> 18) as i16),
            2 => Self::Span(v as u16),
            _ => Self::Auto,
        }
    }

    /// Encodes `Option<GridLine>` into 16 bits with `0 = None` via tag-niche XOR.
    #[inline(always)]
    pub const fn opt_to_word(v: Option<Self>) -> u32 {
        match v {
            None => 0,
            // Valid tags are {00, 01, 10}; XORing tag bits with 0b11 yields {11, 10, 01} != 00.
            Some(g) => g.to_word() ^ Self::TAG_NICHE,
        }
    }
}

impl WordValue for GridLine {
    const BITS: u8 = 16;
    const KIND: FieldKind = FieldKind::GridLine;
    #[inline(always)]
    fn to_bits(self) -> u32 {
        self.to_word()
    }
    #[inline(always)]
    fn from_bits(bits: u32) -> Self {
        Self::from_word(bits)
    }
}

impl OptWord for GridLine {
    const OPT_BITS: u8 = 16;
    const OPT_KIND: FieldKind = FieldKind::GridLine;
    const OPT_ENCODING: OptEncoding = OptEncoding::TagXor;

    #[inline(always)]
    fn opt_to_bits(v: Option<Self>) -> u32 {
        Self::opt_to_word(v)
    }

    #[inline(always)]
    fn opt_from_bits(bits: u32) -> Option<Self> {
        if (bits & Self::TAG_NICHE) == 0 {
            None
        } else {
            Some(Self::from_word(bits ^ Self::TAG_NICHE))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_line_and_opt_grid_line_roundtrip() {
        let samples = [
            GridLine::Auto,
            GridLine::Line(0),
            GridLine::Line(1),
            GridLine::Line(-1),
            GridLine::Line(GridLine::MAX_LINE),
            GridLine::Line(GridLine::MIN_LINE),
            GridLine::Span(0),
            GridLine::Span(3),
            GridLine::Span(GridLine::MAX_SPAN),
        ];
        for g in samples {
            assert_eq!(GridLine::from_bits(g.to_bits()), g);
            let ob = Some(g).to_bits();
            assert_eq!(ob, GridLine::opt_to_word(Some(g)));
            assert_ne!(ob, 0);
            assert!(ob <= 0xFFFF);
            assert_eq!(<Option<GridLine> as WordValue>::from_bits(ob), Some(g));
        }
        // Saturating bounds prevent sign inversion / wraparound on out-of-range inputs.
        assert_eq!(
            GridLine::from_word(GridLine::Line(8192).to_word()),
            GridLine::Line(GridLine::MAX_LINE)
        );
        assert_eq!(
            GridLine::from_word(GridLine::Line(i16::MAX).to_word()),
            GridLine::Line(GridLine::MAX_LINE)
        );
        assert_eq!(
            GridLine::from_word(GridLine::Line(-8193).to_word()),
            GridLine::Line(GridLine::MIN_LINE)
        );
        assert_eq!(
            GridLine::from_word(GridLine::Line(i16::MIN).to_word()),
            GridLine::Line(GridLine::MIN_LINE)
        );
        assert_eq!(
            GridLine::from_word(GridLine::Span(16384).to_word()),
            GridLine::Span(GridLine::MAX_SPAN)
        );
        assert_eq!(
            GridLine::from_word(GridLine::Span(u16::MAX).to_word()),
            GridLine::Span(GridLine::MAX_SPAN)
        );
        assert_eq!(<Option<GridLine> as WordValue>::to_bits(None), 0);
        assert_eq!(<Option<GridLine> as WordValue>::from_bits(0), None);
        assert_eq!(<Option<GridLine> as WordValue>::from_bits(0x0042), None);
    }
}
