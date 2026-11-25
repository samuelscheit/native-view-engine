use serde::{Deserialize, Serialize};

pub use crate::ffi::{ComponentKind, Opcode};
use crate::{FrameId, SurfaceId};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub enum Dimension {
    #[default]
    Auto,
    Points(f32),
    Percent(f32),
}

impl Dimension {
    #[inline(always)]
    pub(super) fn encode(&self, dst: &mut [u8; 5]) {
        match *self {
            Dimension::Auto => {
                *dst = [0, 0, 0, 0, 0];
            }
            Dimension::Points(v) => {
                let b = v.to_le_bytes();
                *dst = [1, b[0], b[1], b[2], b[3]];
            }
            Dimension::Percent(v) => {
                let b = v.to_le_bytes();
                *dst = [2, b[0], b[1], b[2], b[3]];
            }
        }
    }

    #[inline(always)]
    pub(super) fn decode_from_bytes(b: &[u8]) -> Result<Self, DecodeError> {
        let tag = b[0];
        let val = f32::from_le_bytes([b[1], b[2], b[3], b[4]]);
        match tag {
            0 => Ok(Dimension::Auto),
            1 => Ok(Dimension::Points(val)),
            2 => Ok(Dimension::Percent(val)),
            other => Err(DecodeError::InvalidEnum("Dimension", other as u32)),
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum FlexDirection {
    #[default]
    Column = 0,
    Row = 1,
    ColumnReverse = 2,
    RowReverse = 3,
}

impl FlexDirection {
    #[inline(always)]
    pub(super) fn from_u8(v: u8) -> Result<Self, DecodeError> {
        match v {
            0 => Ok(Self::Column),
            1 => Ok(Self::Row),
            2 => Ok(Self::ColumnReverse),
            3 => Ok(Self::RowReverse),
            other => Err(DecodeError::InvalidEnum("FlexDirection", other as u32)),
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum FlexJustify {
    #[default]
    FlexStart = 0,
    Center = 1,
    FlexEnd = 2,
    SpaceBetween = 3,
    SpaceAround = 4,
    SpaceEvenly = 5,
}

impl FlexJustify {
    #[inline(always)]
    pub(super) fn from_u8(v: u8) -> Result<Self, DecodeError> {
        match v {
            0 => Ok(Self::FlexStart),
            1 => Ok(Self::Center),
            2 => Ok(Self::FlexEnd),
            3 => Ok(Self::SpaceBetween),
            4 => Ok(Self::SpaceAround),
            5 => Ok(Self::SpaceEvenly),
            other => Err(DecodeError::InvalidEnum("FlexJustify", other as u32)),
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum FlexAlign {
    #[default]
    Stretch = 0,
    FlexStart = 1,
    Center = 2,
    FlexEnd = 3,
    Baseline = 4,
}

impl FlexAlign {
    #[inline(always)]
    pub(super) fn from_u8(v: u8) -> Result<Self, DecodeError> {
        match v {
            0 => Ok(Self::Stretch),
            1 => Ok(Self::FlexStart),
            2 => Ok(Self::Center),
            3 => Ok(Self::FlexEnd),
            4 => Ok(Self::Baseline),
            other => Err(DecodeError::InvalidEnum("FlexAlign", other as u32)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct EdgeInsets {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl EdgeInsets {
    pub const fn all(v: f32) -> Self {
        Self {
            top: v,
            right: v,
            bottom: v,
            left: v,
        }
    }

    pub const fn symmetric(vertical: f32, horizontal: f32) -> Self {
        Self {
            top: vertical,
            right: horizontal,
            bottom: vertical,
            left: horizontal,
        }
    }

    #[inline(always)]
    pub(super) fn encode(&self, dst: &mut [u8; 16]) {
        let top = self.top.to_le_bytes();
        let right = self.right.to_le_bytes();
        let bottom = self.bottom.to_le_bytes();
        let left = self.left.to_le_bytes();
        dst[0..4].copy_from_slice(&top);
        dst[4..8].copy_from_slice(&right);
        dst[8..12].copy_from_slice(&bottom);
        dst[12..16].copy_from_slice(&left);
    }

    #[inline(always)]
    pub(super) fn decode_from_bytes(b: &[u8]) -> Self {
        Self {
            top: f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            right: f32::from_le_bytes([b[4], b[5], b[6], b[7]]),
            bottom: f32::from_le_bytes([b[8], b[9], b[10], b[11]]),
            left: f32::from_le_bytes([b[12], b[13], b[14], b[15]]),
        }
    }
}

/// Yoga/Taffy-aligned Flexbox layout properties.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LayoutProps {
    pub width: Dimension,
    pub height: Dimension,
    pub min_width: Dimension,
    pub min_height: Dimension,
    pub max_width: Dimension,
    pub max_height: Dimension,
    pub flex_direction: FlexDirection,
    pub justify_content: FlexJustify,
    pub align_items: FlexAlign,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub gap: f32,
    pub padding: EdgeInsets,
    pub margin: EdgeInsets,
}

impl Default for LayoutProps {
    fn default() -> Self {
        Self {
            width: Dimension::Auto,
            height: Dimension::Auto,
            min_width: Dimension::Auto,
            min_height: Dimension::Auto,
            max_width: Dimension::Auto,
            max_height: Dimension::Auto,
            flex_direction: FlexDirection::Column,
            justify_content: FlexJustify::FlexStart,
            align_items: FlexAlign::Stretch,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            gap: 0.0,
            padding: EdgeInsets::default(),
            margin: EdgeInsets::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ColorRgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl ColorRgba {
    pub const TRANSPARENT: Self = Self::rgba(0, 0, 0, 0);
    pub const WHITE: Self = Self::rgb(255, 255, 255);
    pub const BLACK: Self = Self::rgb(0, 0, 0);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn to_u32(self) -> u32 {
        u32::from_le_bytes([self.r, self.g, self.b, self.a])
    }

    pub const fn from_u32(v: u32) -> Self {
        let [r, g, b, a] = v.to_le_bytes();
        Self { r, g, b, a }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StyleProps {
    pub background: ColorRgba,
    pub text_color: ColorRgba,
    pub border_color: ColorRgba,
    pub border_width: f32,
    pub border_radius: f32,
    pub opacity: f32,
    pub font_size: f32,
    pub z_index: i32,
}

impl Default for StyleProps {
    fn default() -> Self {
        Self {
            background: ColorRgba::TRANSPARENT,
            text_color: ColorRgba::BLACK,
            border_color: ColorRgba::TRANSPARENT,
            border_width: 0.0,
            border_radius: 0.0,
            opacity: 1.0,
            font_size: 16.0,
            z_index: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RawPropValue {
    Null,
    Bool(bool),
    I64(i64),
    F64(f64),
    String(String),
    Bytes(Vec<u8>),
}

/// Zero-copy borrowed view of a [`RawPropValue`] decoded directly from a wire buffer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RawPropValueRef<'a> {
    Null,
    Bool(bool),
    I64(i64),
    F64(f64),
    String(&'a str),
    Bytes(&'a [u8]),
}

impl<'a> RawPropValueRef<'a> {
    #[inline]
    pub fn to_owned(self) -> RawPropValue {
        match self {
            Self::Null => RawPropValue::Null,
            Self::Bool(b) => RawPropValue::Bool(b),
            Self::I64(v) => RawPropValue::I64(v),
            Self::F64(v) => RawPropValue::F64(v),
            Self::String(s) => RawPropValue::String(s.to_owned()),
            Self::Bytes(b) => RawPropValue::Bytes(b.to_vec()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandBufferHeader {
    pub magic: u32,
    pub version: u16,
    pub flags: u16,
    pub surface_id: SurfaceId,
    pub frame_id: FrameId,
    pub op_count: u32,
}

pub use crate::DecodeError;
