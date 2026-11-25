use serde::{Deserialize, Serialize};

use crate::{FrameId, NodeId, SurfaceId};

pub const NVE_ABI_MAGIC: u32 = u32::from_le_bytes(*b"NVE1");
pub const NVE_ABI_VERSION: u16 = 1;
pub const HEADER_SIZE: usize = 24;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opcode {
    CreateNode = 1,
    RemoveNode = 2,
    SetChildren = 3,
    SetText = 4,
    SetLayout = 5,
    SetStyle = 6,
    SetRawProp = 7,
    SetEventMask = 8,
    DispatchCommand = 9,
    Commit = 10,
}

impl Opcode {
    #[inline(always)]
    fn from_u8(v: u8) -> Result<Self, DecodeError> {
        match v {
            1 => Ok(Self::CreateNode),
            2 => Ok(Self::RemoveNode),
            3 => Ok(Self::SetChildren),
            4 => Ok(Self::SetText),
            5 => Ok(Self::SetLayout),
            6 => Ok(Self::SetStyle),
            7 => Ok(Self::SetRawProp),
            8 => Ok(Self::SetEventMask),
            9 => Ok(Self::DispatchCommand),
            10 => Ok(Self::Commit),
            other => Err(DecodeError::UnknownOpcode(other)),
        }
    }
}

#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ComponentKind {
    Root = 0,
    View = 1,
    Text = 2,
    ScrollView = 3,
    Image = 4,
    TextInput = 5,
    Pressable = 6,
    SkiaCanvas = 7,
    Custom = 0xFFFF,
}

impl ComponentKind {
    #[inline(always)]
    pub fn from_u16(v: u16) -> Result<Self, DecodeError> {
        match v {
            0 => Ok(Self::Root),
            1 => Ok(Self::View),
            2 => Ok(Self::Text),
            3 => Ok(Self::ScrollView),
            4 => Ok(Self::Image),
            5 => Ok(Self::TextInput),
            6 => Ok(Self::Pressable),
            7 => Ok(Self::SkiaCanvas),
            0xFFFF => Ok(Self::Custom),
            other => Err(DecodeError::InvalidEnum("ComponentKind", other as u32)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub enum Dimension {
    #[default]
    Auto,
    Points(f32),
    Percent(f32),
}

impl Dimension {
    fn encode(&self, buf: &mut Vec<u8>) {
        match *self {
            Dimension::Auto => {
                buf.push(0);
                buf.extend_from_slice(&0.0f32.to_le_bytes());
            }
            Dimension::Points(v) => {
                buf.push(1);
                buf.extend_from_slice(&v.to_le_bytes());
            }
            Dimension::Percent(v) => {
                buf.push(2);
                buf.extend_from_slice(&v.to_le_bytes());
            }
        }
    }

    #[inline(always)]
    fn decode_from_bytes(b: &[u8]) -> Result<Self, DecodeError> {
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
    fn from_u8(v: u8) -> Result<Self, DecodeError> {
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
    fn from_u8(v: u8) -> Result<Self, DecodeError> {
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
    fn from_u8(v: u8) -> Result<Self, DecodeError> {
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

    fn encode(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(&self.top.to_le_bytes());
        buf.extend_from_slice(&self.right.to_le_bytes());
        buf.extend_from_slice(&self.bottom.to_le_bytes());
        buf.extend_from_slice(&self.left.to_le_bytes());
    }

    #[inline(always)]
    fn decode_from_bytes(b: &[u8]) -> Self {
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

/// Zero-allocation iterator over little-endian `NodeId` values in a `SetChildren` payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChildIdIter<'a> {
    bytes: &'a [u8],
}

impl<'a> ChildIdIter<'a> {
    #[inline]
    pub fn len(&self) -> usize {
        self.bytes.len() / 4
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    #[inline]
    pub fn as_bytes(&self) -> &'a [u8] {
        self.bytes
    }

    #[inline]
    pub fn get(&self, index: usize) -> Option<NodeId> {
        let offset = index.checked_mul(4)?;
        let chunk = self.bytes.get(offset..offset + 4)?;
        Some(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
    }

    #[inline]
    pub fn iter(&self) -> Self {
        *self
    }

    #[inline]
    pub fn to_vec(&self) -> Vec<NodeId> {
        let mut out = Vec::with_capacity(self.len());
        self.copy_into(&mut out);
        out
    }

    #[inline]
    pub fn copy_into(&self, dst: &mut Vec<NodeId>) {
        dst.clear();
        dst.reserve(self.len());
        for chunk in self.bytes.chunks_exact(4) {
            dst.push(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
        }
    }
}

impl<'a> Iterator for ChildIdIter<'a> {
    type Item = NodeId;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.bytes.len() < 4 {
            return None;
        }
        let (head, rest) = self.bytes.split_at(4);
        self.bytes = rest;
        Some(u32::from_le_bytes([head[0], head[1], head[2], head[3]]))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.bytes.len() / 4;
        (n, Some(n))
    }
}

impl<'a> DoubleEndedIterator for ChildIdIter<'a> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        let len = self.bytes.len();
        if len < 4 {
            return None;
        }
        let (rest, tail) = self.bytes.split_at(len - 4);
        self.bytes = rest;
        Some(u32::from_le_bytes([tail[0], tail[1], tail[2], tail[3]]))
    }
}

impl<'a> ExactSizeIterator for ChildIdIter<'a> {}
impl<'a> std::iter::FusedIterator for ChildIdIter<'a> {}

/// Zero-copy borrowed command decoded directly from a `CommandBuffer` byte slice.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CommandRef<'a> {
    CreateNode {
        node_id: NodeId,
        kind: ComponentKind,
        custom_name: Option<&'a str>,
    },
    RemoveNode {
        node_id: NodeId,
    },
    SetChildren {
        parent_id: NodeId,
        children: ChildIdIter<'a>,
    },
    SetText {
        node_id: NodeId,
        text: &'a str,
    },
    SetLayout {
        node_id: NodeId,
        layout: LayoutProps,
    },
    SetStyle {
        node_id: NodeId,
        style: StyleProps,
    },
    SetRawProp {
        node_id: NodeId,
        key: &'a str,
        value: RawPropValueRef<'a>,
    },
    SetEventMask {
        node_id: NodeId,
        mask: u64,
    },
    DispatchCommand {
        node_id: NodeId,
        command_name: &'a str,
        payload: &'a [u8],
    },
    Commit {
        sync_mount: bool,
    },
}

impl<'a> CommandRef<'a> {
    #[inline]
    pub fn to_owned(self) -> Command {
        match self {
            Self::CreateNode {
                node_id,
                kind,
                custom_name,
            } => Command::CreateNode {
                node_id,
                kind,
                custom_name: custom_name.map(ToOwned::to_owned),
            },
            Self::RemoveNode { node_id } => Command::RemoveNode { node_id },
            Self::SetChildren {
                parent_id,
                children,
            } => Command::SetChildren {
                parent_id,
                children: children.to_vec(),
            },
            Self::SetText { node_id, text } => Command::SetText {
                node_id,
                text: text.to_owned(),
            },
            Self::SetLayout { node_id, layout } => Command::SetLayout { node_id, layout },
            Self::SetStyle { node_id, style } => Command::SetStyle { node_id, style },
            Self::SetRawProp {
                node_id,
                key,
                value,
            } => Command::SetRawProp {
                node_id,
                key: key.to_owned(),
                value: value.to_owned(),
            },
            Self::SetEventMask { node_id, mask } => Command::SetEventMask { node_id, mask },
            Self::DispatchCommand {
                node_id,
                command_name,
                payload,
            } => Command::DispatchCommand {
                node_id,
                command_name: command_name.to_owned(),
                payload: payload.to_vec(),
            },
            Self::Commit { sync_mount } => Command::Commit { sync_mount },
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    CreateNode {
        node_id: NodeId,
        kind: ComponentKind,
        custom_name: Option<String>,
    },
    RemoveNode {
        node_id: NodeId,
    },
    SetChildren {
        parent_id: NodeId,
        children: Vec<NodeId>,
    },
    SetText {
        node_id: NodeId,
        text: String,
    },
    SetLayout {
        node_id: NodeId,
        layout: LayoutProps,
    },
    SetStyle {
        node_id: NodeId,
        style: StyleProps,
    },
    SetRawProp {
        node_id: NodeId,
        key: String,
        value: RawPropValue,
    },
    SetEventMask {
        node_id: NodeId,
        mask: u64,
    },
    DispatchCommand {
        node_id: NodeId,
        command_name: String,
        payload: Vec<u8>,
    },
    Commit {
        sync_mount: bool,
    },
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    Truncated,
    InvalidMagic(u32),
    UnsupportedVersion(u16),
    UnknownOpcode(u8),
    InvalidEnum(&'static str, u32),
    InvalidUtf8,
    TrailingBytes(usize),
    MaxDepthExceeded,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for DecodeError {}

/// Flat binary command buffer builder. Reuse across frames via `reset` for zero steady-state allocations.
pub struct CommandBufferBuilder {
    buf: Vec<u8>,
    surface_id: SurfaceId,
    frame_id: FrameId,
    op_count: u32,
}

impl CommandBufferBuilder {
    pub fn new(surface_id: SurfaceId, frame_id: FrameId) -> Self {
        let mut this = Self {
            buf: Vec::with_capacity(4096),
            surface_id,
            frame_id,
            op_count: 0,
        };
        this.write_header_placeholder();
        this
    }

    pub fn reset(&mut self, surface_id: SurfaceId, frame_id: FrameId) {
        self.buf.clear();
        self.surface_id = surface_id;
        self.frame_id = frame_id;
        self.op_count = 0;
        self.write_header_placeholder();
    }

    fn write_header_placeholder(&mut self) {
        self.buf.extend_from_slice(&NVE_ABI_MAGIC.to_le_bytes());
        self.buf.extend_from_slice(&NVE_ABI_VERSION.to_le_bytes());
        self.buf.extend_from_slice(&0u16.to_le_bytes());
        self.buf.extend_from_slice(&self.surface_id.to_le_bytes());
        self.buf.extend_from_slice(&self.frame_id.to_le_bytes());
        self.buf.extend_from_slice(&0u32.to_le_bytes());
        debug_assert_eq!(self.buf.len(), HEADER_SIZE);
    }

    pub fn op_count(&self) -> u32 {
        self.op_count
    }

    pub fn is_empty(&self) -> bool {
        self.op_count == 0
    }

    pub fn create_node(&mut self, node_id: NodeId, kind: ComponentKind, custom_name: Option<&str>) {
        self.buf.push(Opcode::CreateNode as u8);
        self.buf.extend_from_slice(&node_id.to_le_bytes());
        self.buf.extend_from_slice(&(kind as u16).to_le_bytes());
        let name_bytes = custom_name.unwrap_or("").as_bytes();
        let len = u16::try_from(name_bytes.len()).expect("custom component name too long");
        self.buf.extend_from_slice(&len.to_le_bytes());
        self.buf.extend_from_slice(name_bytes);
        self.op_count += 1;
    }

    pub fn remove_node(&mut self, node_id: NodeId) {
        self.buf.push(Opcode::RemoveNode as u8);
        self.buf.extend_from_slice(&node_id.to_le_bytes());
        self.op_count += 1;
    }

    pub fn set_children(&mut self, parent_id: NodeId, children: &[NodeId]) {
        self.buf.push(Opcode::SetChildren as u8);
        self.buf.extend_from_slice(&parent_id.to_le_bytes());
        let len = u32::try_from(children.len()).expect("too many children");
        self.buf.extend_from_slice(&len.to_le_bytes());
        for &child in children {
            self.buf.extend_from_slice(&child.to_le_bytes());
        }
        self.op_count += 1;
    }

    pub fn set_text(&mut self, node_id: NodeId, text: &str) {
        self.buf.push(Opcode::SetText as u8);
        self.buf.extend_from_slice(&node_id.to_le_bytes());
        let bytes = text.as_bytes();
        let len = u32::try_from(bytes.len()).expect("text too long");
        self.buf.extend_from_slice(&len.to_le_bytes());
        self.buf.extend_from_slice(bytes);
        self.op_count += 1;
    }

    pub fn set_layout(&mut self, node_id: NodeId, layout: &LayoutProps) {
        self.buf.push(Opcode::SetLayout as u8);
        self.buf.extend_from_slice(&node_id.to_le_bytes());
        layout.width.encode(&mut self.buf);
        layout.height.encode(&mut self.buf);
        layout.min_width.encode(&mut self.buf);
        layout.min_height.encode(&mut self.buf);
        layout.max_width.encode(&mut self.buf);
        layout.max_height.encode(&mut self.buf);
        self.buf.push(layout.flex_direction as u8);
        self.buf.push(layout.justify_content as u8);
        self.buf.push(layout.align_items as u8);
        self.buf.extend_from_slice(&layout.flex_grow.to_le_bytes());
        self.buf.extend_from_slice(&layout.flex_shrink.to_le_bytes());
        self.buf.extend_from_slice(&layout.gap.to_le_bytes());
        layout.padding.encode(&mut self.buf);
        layout.margin.encode(&mut self.buf);
        self.op_count += 1;
    }

    pub fn set_style(&mut self, node_id: NodeId, style: &StyleProps) {
        self.buf.push(Opcode::SetStyle as u8);
        self.buf.extend_from_slice(&node_id.to_le_bytes());
        self.buf.extend_from_slice(&style.background.to_u32().to_le_bytes());
        self.buf.extend_from_slice(&style.text_color.to_u32().to_le_bytes());
        self.buf.extend_from_slice(&style.border_color.to_u32().to_le_bytes());
        self.buf.extend_from_slice(&style.border_width.to_le_bytes());
        self.buf.extend_from_slice(&style.border_radius.to_le_bytes());
        self.buf.extend_from_slice(&style.opacity.to_le_bytes());
        self.buf.extend_from_slice(&style.font_size.to_le_bytes());
        self.buf.extend_from_slice(&style.z_index.to_le_bytes());
        self.op_count += 1;
    }

    pub fn set_raw_prop(&mut self, node_id: NodeId, key: &str, value: &RawPropValue) {
        let value_ref = match value {
            RawPropValue::Null => RawPropValueRef::Null,
            RawPropValue::Bool(b) => RawPropValueRef::Bool(*b),
            RawPropValue::I64(v) => RawPropValueRef::I64(*v),
            RawPropValue::F64(v) => RawPropValueRef::F64(*v),
            RawPropValue::String(s) => RawPropValueRef::String(s.as_str()),
            RawPropValue::Bytes(b) => RawPropValueRef::Bytes(b.as_slice()),
        };
        self.set_raw_prop_ref(node_id, key, value_ref);
    }

    /// Writes a borrowed raw property without requiring heap allocation for string or byte payloads.
    pub fn set_raw_prop_ref(&mut self, node_id: NodeId, key: &str, value: RawPropValueRef<'_>) {
        self.buf.push(Opcode::SetRawProp as u8);
        self.buf.extend_from_slice(&node_id.to_le_bytes());
        let key_bytes = key.as_bytes();
        let key_len = u16::try_from(key_bytes.len()).expect("prop key too long");
        self.buf.extend_from_slice(&key_len.to_le_bytes());
        self.buf.extend_from_slice(key_bytes);
        match value {
            RawPropValueRef::Null => self.buf.push(0),
            RawPropValueRef::Bool(b) => {
                self.buf.push(1);
                self.buf.push(u8::from(b));
            }
            RawPropValueRef::I64(v) => {
                self.buf.push(2);
                self.buf.extend_from_slice(&v.to_le_bytes());
            }
            RawPropValueRef::F64(v) => {
                self.buf.push(3);
                self.buf.extend_from_slice(&v.to_le_bytes());
            }
            RawPropValueRef::String(s) => {
                self.buf.push(4);
                let b = s.as_bytes();
                let len = u32::try_from(b.len()).expect("raw prop string too long");
                self.buf.extend_from_slice(&len.to_le_bytes());
                self.buf.extend_from_slice(b);
            }
            RawPropValueRef::Bytes(b) => {
                self.buf.push(5);
                let len = u32::try_from(b.len()).expect("raw prop bytes too long");
                self.buf.extend_from_slice(&len.to_le_bytes());
                self.buf.extend_from_slice(b);
            }
        }
        self.op_count += 1;
    }

    /// Convenience helper to write a string raw property directly from `&str` with zero allocation.
    #[inline]
    pub fn set_raw_prop_str(&mut self, node_id: NodeId, key: &str, value: &str) {
        self.set_raw_prop_ref(node_id, key, RawPropValueRef::String(value));
    }

    pub fn set_event_mask(&mut self, node_id: NodeId, mask: u64) {
        self.buf.push(Opcode::SetEventMask as u8);
        self.buf.extend_from_slice(&node_id.to_le_bytes());
        self.buf.extend_from_slice(&mask.to_le_bytes());
        self.op_count += 1;
    }

    pub fn dispatch_command(&mut self, node_id: NodeId, command_name: &str, payload: &[u8]) {
        self.buf.push(Opcode::DispatchCommand as u8);
        self.buf.extend_from_slice(&node_id.to_le_bytes());
        let name = command_name.as_bytes();
        self.buf.extend_from_slice(&(name.len() as u16).to_le_bytes());
        self.buf.extend_from_slice(name);
        self.buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        self.buf.extend_from_slice(payload);
        self.op_count += 1;
    }

    pub fn commit(&mut self, sync_mount: bool) {
        self.buf.push(Opcode::Commit as u8);
        self.buf.push(u8::from(sync_mount));
        self.op_count += 1;
    }

    /// Patches the op_count in the header and returns the finished byte slice.
    pub fn finish(&mut self) -> &[u8] {
        self.buf[20..24].copy_from_slice(&self.op_count.to_le_bytes());
        &self.buf
    }

    pub fn into_bytes(mut self) -> Vec<u8> {
        self.finish();
        self.buf
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    #[inline(always)]
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    #[inline(always)]
    fn remaining(&self) -> usize {
        self.bytes.len()
    }

    #[inline(always)]
    fn read_bytes(&mut self, len: usize) -> Result<&'a [u8], DecodeError> {
        if self.bytes.len() < len {
            return Err(DecodeError::Truncated);
        }
        let (head, rest) = self.bytes.split_at(len);
        self.bytes = rest;
        Ok(head)
    }

    #[inline(always)]
    fn read_array<const N: usize>(&mut self) -> Result<&'a [u8; N], DecodeError> {
        let slice = self.read_bytes(N)?;
        Ok(slice.try_into().unwrap())
    }

    #[inline(always)]
    fn read_u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.read_array::<1>()?[0])
    }

    #[inline(always)]
    fn read_u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(*self.read_array::<4>()?))
    }

    #[inline(always)]
    fn read_i64(&mut self) -> Result<i64, DecodeError> {
        Ok(i64::from_le_bytes(*self.read_array::<8>()?))
    }

    #[inline(always)]
    fn read_f64(&mut self) -> Result<f64, DecodeError> {
        Ok(f64::from_le_bytes(*self.read_array::<8>()?))
    }

    #[inline(always)]
    fn read_str_ref(&mut self, len: usize) -> Result<&'a str, DecodeError> {
        let raw = self.read_bytes(len)?;
        std::str::from_utf8(raw).map_err(|_| DecodeError::InvalidUtf8)
    }
}

pub struct CommandDecoder<'a> {
    header: CommandBufferHeader,
    reader: Reader<'a>,
    decoded: u32,
}

impl<'a> CommandDecoder<'a> {
    #[inline]
    pub fn new(bytes: &'a [u8]) -> Result<Self, DecodeError> {
        let mut reader = Reader::new(bytes);
        let hdr = reader.read_array::<HEADER_SIZE>()?;
        let magic = u32::from_le_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]);
        if magic != NVE_ABI_MAGIC {
            return Err(DecodeError::InvalidMagic(magic));
        }
        let version = u16::from_le_bytes([hdr[4], hdr[5]]);
        if version != NVE_ABI_VERSION {
            return Err(DecodeError::UnsupportedVersion(version));
        }
        let flags = u16::from_le_bytes([hdr[6], hdr[7]]);
        let surface_id = u32::from_le_bytes([hdr[8], hdr[9], hdr[10], hdr[11]]);
        let frame_id = u64::from_le_bytes([
            hdr[12], hdr[13], hdr[14], hdr[15], hdr[16], hdr[17], hdr[18], hdr[19],
        ]);
        let op_count = u32::from_le_bytes([hdr[20], hdr[21], hdr[22], hdr[23]]);
        Ok(Self {
            header: CommandBufferHeader {
                magic,
                version,
                flags,
                surface_id,
                frame_id,
                op_count,
            },
            reader,
            decoded: 0,
        })
    }

    #[inline]
    pub fn header(&self) -> CommandBufferHeader {
        self.header
    }

    /// Streams every command in the buffer as a zero-copy [`CommandRef<'a>`], performing zero
    /// heap allocations and rejecting trailing bytes at the end of the buffer.
    #[inline]
    pub fn for_each<E, F>(mut self, mut f: F) -> Result<CommandBufferHeader, E>
    where
        E: From<DecodeError>,
        F: FnMut(CommandRef<'a>) -> Result<(), E>,
    {
        let remaining_ops = self.header.op_count - self.decoded;
        for _ in 0..remaining_ops {
            let cmd = self.decode_one_ref().map_err(E::from)?;
            self.decoded += 1;
            f(cmd)?;
        }
        if self.reader.remaining() != 0 {
            return Err(E::from(DecodeError::TrailingBytes(self.reader.remaining())));
        }
        Ok(self.header)
    }

    pub fn decode_all(self) -> Result<(CommandBufferHeader, Vec<Command>), DecodeError> {
        let initial_cap = (self.header.op_count as usize).min(self.reader.remaining() / 2);
        let mut cmds = Vec::with_capacity(initial_cap);
        let header = self.for_each(|cmd| {
            cmds.push(cmd.to_owned());
            Ok::<(), DecodeError>(())
        })?;
        Ok((header, cmds))
    }

    #[inline]
    pub fn next_command(&mut self) -> Result<Option<Command>, DecodeError> {
        Ok(self.next_command_ref()?.map(CommandRef::to_owned))
    }

    /// Decodes the next command directly from the underlying wire slice without heap allocations.
    #[inline]
    pub fn next_command_ref(&mut self) -> Result<Option<CommandRef<'a>>, DecodeError> {
        if self.decoded >= self.header.op_count {
            return Ok(None);
        }
        let cmd = self.decode_one_ref()?;
        self.decoded += 1;
        Ok(Some(cmd))
    }

    #[inline(always)]
    fn decode_one_ref(&mut self) -> Result<CommandRef<'a>, DecodeError> {
        let op = Opcode::from_u8(self.reader.read_u8()?)?;
        let cmd = match op {
            Opcode::CreateNode => {
                let b = self.reader.read_array::<8>()?;
                let node_id = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                let kind = ComponentKind::from_u16(u16::from_le_bytes([b[4], b[5]]))?;
                let name_len = u16::from_le_bytes([b[6], b[7]]) as usize;
                let custom_name = if name_len == 0 {
                    None
                } else {
                    Some(self.reader.read_str_ref(name_len)?)
                };
                CommandRef::CreateNode {
                    node_id,
                    kind,
                    custom_name,
                }
            }
            Opcode::RemoveNode => CommandRef::RemoveNode {
                node_id: self.reader.read_u32()?,
            },
            Opcode::SetChildren => {
                let b = self.reader.read_array::<8>()?;
                let parent_id = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                let count = u32::from_le_bytes([b[4], b[5], b[6], b[7]]) as usize;
                let byte_len = count.checked_mul(4).ok_or(DecodeError::Truncated)?;
                let bytes = self.reader.read_bytes(byte_len)?;
                CommandRef::SetChildren {
                    parent_id,
                    children: ChildIdIter { bytes },
                }
            }
            Opcode::SetText => {
                let b = self.reader.read_array::<8>()?;
                let node_id = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                let len = u32::from_le_bytes([b[4], b[5], b[6], b[7]]) as usize;
                let text = self.reader.read_str_ref(len)?;
                CommandRef::SetText { node_id, text }
            }
            Opcode::SetLayout => {
                let b = self.reader.read_array::<81>()?;
                let node_id = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                let layout = LayoutProps {
                    width: Dimension::decode_from_bytes(&b[4..9])?,
                    height: Dimension::decode_from_bytes(&b[9..14])?,
                    min_width: Dimension::decode_from_bytes(&b[14..19])?,
                    min_height: Dimension::decode_from_bytes(&b[19..24])?,
                    max_width: Dimension::decode_from_bytes(&b[24..29])?,
                    max_height: Dimension::decode_from_bytes(&b[29..34])?,
                    flex_direction: FlexDirection::from_u8(b[34])?,
                    justify_content: FlexJustify::from_u8(b[35])?,
                    align_items: FlexAlign::from_u8(b[36])?,
                    flex_grow: f32::from_le_bytes([b[37], b[38], b[39], b[40]]),
                    flex_shrink: f32::from_le_bytes([b[41], b[42], b[43], b[44]]),
                    gap: f32::from_le_bytes([b[45], b[46], b[47], b[48]]),
                    padding: EdgeInsets::decode_from_bytes(&b[49..65]),
                    margin: EdgeInsets::decode_from_bytes(&b[65..81]),
                };
                CommandRef::SetLayout { node_id, layout }
            }
            Opcode::SetStyle => {
                let b = self.reader.read_array::<36>()?;
                let node_id = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                let style = StyleProps {
                    background: ColorRgba::from_u32(u32::from_le_bytes([b[4], b[5], b[6], b[7]])),
                    text_color: ColorRgba::from_u32(u32::from_le_bytes([b[8], b[9], b[10], b[11]])),
                    border_color: ColorRgba::from_u32(u32::from_le_bytes([
                        b[12], b[13], b[14], b[15],
                    ])),
                    border_width: f32::from_le_bytes([b[16], b[17], b[18], b[19]]),
                    border_radius: f32::from_le_bytes([b[20], b[21], b[22], b[23]]),
                    opacity: f32::from_le_bytes([b[24], b[25], b[26], b[27]]),
                    font_size: f32::from_le_bytes([b[28], b[29], b[30], b[31]]),
                    z_index: i32::from_le_bytes([b[32], b[33], b[34], b[35]]),
                };
                CommandRef::SetStyle { node_id, style }
            }
            Opcode::SetRawProp => {
                let b = self.reader.read_array::<6>()?;
                let node_id = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                let key_len = u16::from_le_bytes([b[4], b[5]]) as usize;
                let key = self.reader.read_str_ref(key_len)?;
                let val_tag = self.reader.read_u8()?;
                let value = match val_tag {
                    0 => RawPropValueRef::Null,
                    1 => RawPropValueRef::Bool(self.reader.read_u8()? != 0),
                    2 => RawPropValueRef::I64(self.reader.read_i64()?),
                    3 => RawPropValueRef::F64(self.reader.read_f64()?),
                    4 => {
                        let len = self.reader.read_u32()? as usize;
                        RawPropValueRef::String(self.reader.read_str_ref(len)?)
                    }
                    5 => {
                        let len = self.reader.read_u32()? as usize;
                        RawPropValueRef::Bytes(self.reader.read_bytes(len)?)
                    }
                    other => return Err(DecodeError::InvalidEnum("RawPropValue", other as u32)),
                };
                CommandRef::SetRawProp {
                    node_id,
                    key,
                    value,
                }
            }
            Opcode::SetEventMask => {
                let b = self.reader.read_array::<12>()?;
                let node_id = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                let mask = u64::from_le_bytes([
                    b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11],
                ]);
                CommandRef::SetEventMask { node_id, mask }
            }
            Opcode::DispatchCommand => {
                let b = self.reader.read_array::<6>()?;
                let node_id = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                let name_len = u16::from_le_bytes([b[4], b[5]]) as usize;
                let command_name = self.reader.read_str_ref(name_len)?;
                let payload_len = self.reader.read_u32()? as usize;
                let payload = self.reader.read_bytes(payload_len)?;
                CommandRef::DispatchCommand {
                    node_id,
                    command_name,
                    payload,
                }
            }
            Opcode::Commit => CommandRef::Commit {
                sync_mount: self.reader.read_u8()? != 0,
            },
        };
        Ok(cmd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_all_command_variants() {
        let mut b = CommandBufferBuilder::new(7, 42);
        b.create_node(1, ComponentKind::Root, None);
        b.create_node(2, ComponentKind::View, None);
        b.create_node(3, ComponentKind::Text, None);
        b.create_node(4, ComponentKind::Custom, Some("RNSVGPath"));
        b.set_children(1, &[2, 4]);
        b.set_children(2, &[3]);
        b.set_text(3, "Hello Fabric + Skia + Web! 🚀");
        b.set_layout(
            2,
            &LayoutProps {
                width: Dimension::Points(320.0),
                height: Dimension::Percent(100.0),
                flex_direction: FlexDirection::Row,
                justify_content: FlexJustify::SpaceBetween,
                align_items: FlexAlign::Center,
                flex_grow: 1.0,
                gap: 12.0,
                padding: EdgeInsets::symmetric(8.0, 16.0),
                ..Default::default()
            },
        );
        b.set_style(
            2,
            &StyleProps {
                background: ColorRgba::rgba(15, 23, 42, 240),
                text_color: ColorRgba::WHITE,
                border_color: ColorRgba::rgb(56, 189, 248),
                border_width: 2.0,
                border_radius: 12.0,
                opacity: 0.95,
                font_size: 18.0,
                z_index: 5,
            },
        );
        b.set_raw_prop(4, "d", &RawPropValue::String("M0 0 L10 10".into()));
        b.set_raw_prop(4, "fillOpacity", &RawPropValue::F64(0.75));
        b.set_raw_prop(4, "visible", &RawPropValue::Bool(true));
        b.set_raw_prop(4, "seed", &RawPropValue::I64(-99));
        b.set_raw_prop(4, "blob", &RawPropValue::Bytes(vec![1, 2, 3, 4]));
        b.set_raw_prop(4, "empty", &RawPropValue::Null);
        b.set_event_mask(2, crate::EventMask::PRESS | crate::EventMask::LAYOUT);
        b.dispatch_command(2, "scrollTo", &[0, 1, 2]);
        b.remove_node(4);
        b.commit(true);

        let bytes = b.into_bytes();
        let (header, cmds) = CommandDecoder::new(&bytes).unwrap().decode_all().unwrap();
        assert_eq!(header.surface_id, 7);
        assert_eq!(header.frame_id, 42);
        assert_eq!(header.op_count as usize, cmds.len());
        assert_eq!(cmds.len(), 19);
    }

    #[test]
    fn rejects_corrupt_buffers() {
        assert_eq!(
            CommandDecoder::new(&[0u8; 4]).err(),
            Some(DecodeError::Truncated)
        );
        let mut bad_magic = CommandBufferBuilder::new(1, 1).into_bytes();
        bad_magic[0] ^= 0xFF;
        assert!(matches!(
            CommandDecoder::new(&bad_magic),
            Err(DecodeError::InvalidMagic(_))
        ));

        let mut b = CommandBufferBuilder::new(1, 1);
        b.remove_node(5);
        let mut truncated = b.into_bytes();
        truncated.pop();
        assert_eq!(
            CommandDecoder::new(&truncated).unwrap().decode_all().err(),
            Some(DecodeError::Truncated)
        );
    }

    #[test]
    fn set_raw_prop_str_matches_owned_encoding() {
        let mut b1 = CommandBufferBuilder::new(1, 1);
        b1.set_raw_prop(7, "label", &RawPropValue::String("hello world".into()));
        let mut b2 = CommandBufferBuilder::new(1, 1);
        b2.set_raw_prop_str(7, "label", "hello world");
        assert_eq!(b1.into_bytes(), b2.into_bytes());
    }

    #[test]
    fn for_each_matches_decode_all_on_valid_truncated_and_corrupt_buffers() {
        fn decode_via_for_each(
            bytes: &[u8],
        ) -> Result<(CommandBufferHeader, Vec<Command>), DecodeError> {
            let mut out = Vec::new();
            let hdr = CommandDecoder::new(bytes)?.for_each(|cmd| {
                if let CommandRef::SetChildren { children, .. } = cmd {
                    assert_eq!(children.len(), children.iter().count());
                    assert_eq!(children.is_empty(), children.len() == 0);
                    let rev: Vec<NodeId> = children.iter().rev().collect();
                    let mut fwd = Vec::new();
                    children.copy_into(&mut fwd);
                    assert_eq!(fwd, children.to_vec());
                    let mut rev_expected = fwd.clone();
                    rev_expected.reverse();
                    assert_eq!(rev, rev_expected);
                    for (idx, &cid) in fwd.iter().enumerate() {
                        assert_eq!(children.get(idx), Some(cid));
                    }
                    assert_eq!(children.get(fwd.len()), None);
                }
                out.push(cmd.to_owned());
                Ok::<(), DecodeError>(())
            })?;
            Ok((hdr, out))
        }

        // Build a rich buffer covering every opcode and RawPropValue variant.
        let mut b = CommandBufferBuilder::new(3, 99);
        b.create_node(1, ComponentKind::Root, None);
        b.create_node(2, ComponentKind::Custom, Some("MyView"));
        b.set_children(1, &[2, 3, 4, 5]);
        b.set_children(2, &[]);
        b.set_text(3, "Streaming zero-copy 🚀");
        b.set_layout(
            2,
            &LayoutProps {
                width: Dimension::Points(120.5),
                height: Dimension::Percent(50.0),
                min_width: Dimension::Points(10.0),
                max_height: Dimension::Percent(95.0),
                flex_direction: FlexDirection::RowReverse,
                justify_content: FlexJustify::SpaceEvenly,
                align_items: FlexAlign::Baseline,
                flex_grow: 2.0,
                flex_shrink: 0.5,
                gap: 8.0,
                padding: EdgeInsets::symmetric(4.0, 12.0),
                margin: EdgeInsets::all(6.0),
                ..Default::default()
            },
        );
        b.set_style(
            2,
            &StyleProps {
                background: ColorRgba::rgba(1, 2, 3, 250),
                text_color: ColorRgba::WHITE,
                border_color: ColorRgba::rgb(9, 8, 7),
                border_width: 1.5,
                border_radius: 10.0,
                opacity: 0.85,
                font_size: 18.0,
                z_index: -3,
            },
        );
        b.set_raw_prop_str(2, "strKey", "zero-alloc");
        b.set_raw_prop_ref(2, "bytesKey", RawPropValueRef::Bytes(&[0xDE, 0xAD, 0xBE, 0xEF]));
        b.set_raw_prop(2, "nullKey", &RawPropValue::Null);
        b.set_raw_prop(2, "boolKey", &RawPropValue::Bool(true));
        b.set_raw_prop(2, "i64Key", &RawPropValue::I64(-123456789));
        b.set_raw_prop(2, "f64Key", &RawPropValue::F64(3.25));
        b.set_event_mask(2, 0x00FF_00AA_5511_2233);
        b.dispatch_command(2, "focus", &[1, 2, 3]);
        b.remove_node(5);
        b.commit(false);
        let full = b.into_bytes();

        // 1. Exact equivalence on valid buffer.
        assert_eq!(
            decode_via_for_each(&full),
            CommandDecoder::new(&full).and_then(CommandDecoder::decode_all)
        );

        // 2. Exact equivalence on every truncation prefix.
        for len in 0..full.len() {
            let slice = &full[..len];
            let streamed = decode_via_for_each(slice);
            let owned = CommandDecoder::new(slice).and_then(CommandDecoder::decode_all);
            assert_eq!(streamed, owned, "mismatch at truncated len {len}");
            assert!(streamed.is_err());
        }

        // 3. Trailing bytes rejected identically.
        let mut trailing = full.clone();
        trailing.extend_from_slice(&[0xAA, 0xBB]);
        assert_eq!(
            decode_via_for_each(&trailing),
            Err(DecodeError::TrailingBytes(2))
        );
        assert_eq!(
            decode_via_for_each(&trailing),
            CommandDecoder::new(&trailing).and_then(CommandDecoder::decode_all)
        );

        // 4. Deterministic property/fuzz sweep over bit-flips, splices, and random noise.
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut next_u64 = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };

        for _ in 0..300 {
            let mut mutated = full.clone();
            let flips = (next_u64() as usize % 5) + 1;
            for _ in 0..flips {
                let idx = next_u64() as usize % mutated.len();
                mutated[idx] ^= 1u8 << (next_u64() as usize % 8);
            }
            if next_u64() & 1 == 0 {
                let trunc = next_u64() as usize % (mutated.len() + 1);
                mutated.truncate(trunc);
            }
            assert_eq!(
                decode_via_for_each(&mutated),
                CommandDecoder::new(&mutated).and_then(CommandDecoder::decode_all)
            );
        }

        for _ in 0..100 {
            let len = next_u64() as usize % 128;
            let noise: Vec<u8> = (0..len).map(|_| next_u64() as u8).collect();
            assert_eq!(
                decode_via_for_each(&noise),
                CommandDecoder::new(&noise).and_then(CommandDecoder::decode_all)
            );
        }
    }
}
