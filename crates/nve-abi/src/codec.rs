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

    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let tag = r.read_u8()?;
        let val = r.read_f32()?;
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

    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            top: r.read_f32()?,
            right: r.read_f32()?,
            bottom: r.read_f32()?,
            left: r.read_f32()?,
        })
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
        self.buf.push(Opcode::SetRawProp as u8);
        self.buf.extend_from_slice(&node_id.to_le_bytes());
        let key_bytes = key.as_bytes();
        let key_len = u16::try_from(key_bytes.len()).expect("prop key too long");
        self.buf.extend_from_slice(&key_len.to_le_bytes());
        self.buf.extend_from_slice(key_bytes);
        match value {
            RawPropValue::Null => self.buf.push(0),
            RawPropValue::Bool(b) => {
                self.buf.push(1);
                self.buf.push(u8::from(*b));
            }
            RawPropValue::I64(v) => {
                self.buf.push(2);
                self.buf.extend_from_slice(&v.to_le_bytes());
            }
            RawPropValue::F64(v) => {
                self.buf.push(3);
                self.buf.extend_from_slice(&v.to_le_bytes());
            }
            RawPropValue::String(s) => {
                self.buf.push(4);
                let b = s.as_bytes();
                self.buf.extend_from_slice(&(b.len() as u32).to_le_bytes());
                self.buf.extend_from_slice(b);
            }
            RawPropValue::Bytes(b) => {
                self.buf.push(5);
                self.buf.extend_from_slice(&(b.len() as u32).to_le_bytes());
                self.buf.extend_from_slice(b);
            }
        }
        self.op_count += 1;
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
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.pos)
    }

    fn read_bytes(&mut self, len: usize) -> Result<&'a [u8], DecodeError> {
        if self.remaining() < len {
            return Err(DecodeError::Truncated);
        }
        let slice = &self.bytes[self.pos..self.pos + len];
        self.pos += len;
        Ok(slice)
    }

    fn read_u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.read_bytes(1)?[0])
    }

    fn read_u16(&mut self) -> Result<u16, DecodeError> {
        let b: [u8; 2] = self.read_bytes(2)?.try_into().unwrap();
        Ok(u16::from_le_bytes(b))
    }

    fn read_u32(&mut self) -> Result<u32, DecodeError> {
        let b: [u8; 4] = self.read_bytes(4)?.try_into().unwrap();
        Ok(u32::from_le_bytes(b))
    }

    fn read_i32(&mut self) -> Result<i32, DecodeError> {
        let b: [u8; 4] = self.read_bytes(4)?.try_into().unwrap();
        Ok(i32::from_le_bytes(b))
    }

    fn read_u64(&mut self) -> Result<u64, DecodeError> {
        let b: [u8; 8] = self.read_bytes(8)?.try_into().unwrap();
        Ok(u64::from_le_bytes(b))
    }

    fn read_i64(&mut self) -> Result<i64, DecodeError> {
        let b: [u8; 8] = self.read_bytes(8)?.try_into().unwrap();
        Ok(i64::from_le_bytes(b))
    }

    fn read_f32(&mut self) -> Result<f32, DecodeError> {
        let b: [u8; 4] = self.read_bytes(4)?.try_into().unwrap();
        Ok(f32::from_le_bytes(b))
    }

    fn read_f64(&mut self) -> Result<f64, DecodeError> {
        let b: [u8; 8] = self.read_bytes(8)?.try_into().unwrap();
        Ok(f64::from_le_bytes(b))
    }

    fn read_str(&mut self, len: usize) -> Result<String, DecodeError> {
        let raw = self.read_bytes(len)?;
        std::str::from_utf8(raw)
            .map(ToOwned::to_owned)
            .map_err(|_| DecodeError::InvalidUtf8)
    }
}

pub struct CommandDecoder<'a> {
    header: CommandBufferHeader,
    reader: Reader<'a>,
    decoded: u32,
}

impl<'a> CommandDecoder<'a> {
    pub fn new(bytes: &'a [u8]) -> Result<Self, DecodeError> {
        if bytes.len() < HEADER_SIZE {
            return Err(DecodeError::Truncated);
        }
        let mut reader = Reader::new(bytes);
        let magic = reader.read_u32()?;
        if magic != NVE_ABI_MAGIC {
            return Err(DecodeError::InvalidMagic(magic));
        }
        let version = reader.read_u16()?;
        if version != NVE_ABI_VERSION {
            return Err(DecodeError::UnsupportedVersion(version));
        }
        let flags = reader.read_u16()?;
        let surface_id = reader.read_u32()?;
        let frame_id = reader.read_u64()?;
        let op_count = reader.read_u32()?;
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

    pub fn header(&self) -> CommandBufferHeader {
        self.header
    }

    pub fn decode_all(mut self) -> Result<(CommandBufferHeader, Vec<Command>), DecodeError> {
        let initial_cap = (self.header.op_count as usize)
            .min(self.reader.remaining())
            .min(4096);
        let mut cmds = Vec::with_capacity(initial_cap);
        while let Some(cmd) = self.next_command()? {
            cmds.push(cmd);
        }
        if self.reader.remaining() != 0 {
            return Err(DecodeError::TrailingBytes(self.reader.remaining()));
        }
        Ok((self.header, cmds))
    }

    pub fn next_command(&mut self) -> Result<Option<Command>, DecodeError> {
        if self.decoded >= self.header.op_count {
            return Ok(None);
        }
        let op = Opcode::from_u8(self.reader.read_u8()?)?;
        let cmd = match op {
            Opcode::CreateNode => {
                let node_id = self.reader.read_u32()?;
                let kind = ComponentKind::from_u16(self.reader.read_u16()?)?;
                let name_len = self.reader.read_u16()? as usize;
                let custom_name = if name_len == 0 {
                    None
                } else {
                    Some(self.reader.read_str(name_len)?)
                };
                Command::CreateNode {
                    node_id,
                    kind,
                    custom_name,
                }
            }
            Opcode::RemoveNode => Command::RemoveNode {
                node_id: self.reader.read_u32()?,
            },
            Opcode::SetChildren => {
                let parent_id = self.reader.read_u32()?;
                let count = self.reader.read_u32()? as usize;
                let mut children = Vec::with_capacity(count.min(4096));
                for _ in 0..count {
                    children.push(self.reader.read_u32()?);
                }
                Command::SetChildren {
                    parent_id,
                    children,
                }
            }
            Opcode::SetText => {
                let node_id = self.reader.read_u32()?;
                let len = self.reader.read_u32()? as usize;
                let text = self.reader.read_str(len)?;
                Command::SetText { node_id, text }
            }
            Opcode::SetLayout => {
                let node_id = self.reader.read_u32()?;
                let layout = LayoutProps {
                    width: Dimension::decode(&mut self.reader)?,
                    height: Dimension::decode(&mut self.reader)?,
                    min_width: Dimension::decode(&mut self.reader)?,
                    min_height: Dimension::decode(&mut self.reader)?,
                    max_width: Dimension::decode(&mut self.reader)?,
                    max_height: Dimension::decode(&mut self.reader)?,
                    flex_direction: FlexDirection::from_u8(self.reader.read_u8()?)?,
                    justify_content: FlexJustify::from_u8(self.reader.read_u8()?)?,
                    align_items: FlexAlign::from_u8(self.reader.read_u8()?)?,
                    flex_grow: self.reader.read_f32()?,
                    flex_shrink: self.reader.read_f32()?,
                    gap: self.reader.read_f32()?,
                    padding: EdgeInsets::decode(&mut self.reader)?,
                    margin: EdgeInsets::decode(&mut self.reader)?,
                };
                Command::SetLayout { node_id, layout }
            }
            Opcode::SetStyle => {
                let node_id = self.reader.read_u32()?;
                let style = StyleProps {
                    background: ColorRgba::from_u32(self.reader.read_u32()?),
                    text_color: ColorRgba::from_u32(self.reader.read_u32()?),
                    border_color: ColorRgba::from_u32(self.reader.read_u32()?),
                    border_width: self.reader.read_f32()?,
                    border_radius: self.reader.read_f32()?,
                    opacity: self.reader.read_f32()?,
                    font_size: self.reader.read_f32()?,
                    z_index: self.reader.read_i32()?,
                };
                Command::SetStyle { node_id, style }
            }
            Opcode::SetRawProp => {
                let node_id = self.reader.read_u32()?;
                let key_len = self.reader.read_u16()? as usize;
                let key = self.reader.read_str(key_len)?;
                let val_tag = self.reader.read_u8()?;
                let value = match val_tag {
                    0 => RawPropValue::Null,
                    1 => RawPropValue::Bool(self.reader.read_u8()? != 0),
                    2 => RawPropValue::I64(self.reader.read_i64()?),
                    3 => RawPropValue::F64(self.reader.read_f64()?),
                    4 => {
                        let len = self.reader.read_u32()? as usize;
                        RawPropValue::String(self.reader.read_str(len)?)
                    }
                    5 => {
                        let len = self.reader.read_u32()? as usize;
                        RawPropValue::Bytes(self.reader.read_bytes(len)?.to_vec())
                    }
                    other => return Err(DecodeError::InvalidEnum("RawPropValue", other as u32)),
                };
                Command::SetRawProp {
                    node_id,
                    key,
                    value,
                }
            }
            Opcode::SetEventMask => {
                let node_id = self.reader.read_u32()?;
                let mask = self.reader.read_u64()?;
                Command::SetEventMask { node_id, mask }
            }
            Opcode::DispatchCommand => {
                let node_id = self.reader.read_u32()?;
                let name_len = self.reader.read_u16()? as usize;
                let command_name = self.reader.read_str(name_len)?;
                let payload_len = self.reader.read_u32()? as usize;
                let payload = self.reader.read_bytes(payload_len)?.to_vec();
                Command::DispatchCommand {
                    node_id,
                    command_name,
                    payload,
                }
            }
            Opcode::Commit => Command::Commit {
                sync_mount: self.reader.read_u8()? != 0,
            },
        };
        self.decoded += 1;
        Ok(Some(cmd))
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
}
