use super::commands::{ChildIdIter, Command, CommandRef};
use super::types::{
    ColorRgba, CommandBufferHeader, ComponentKind, DecodeError, Dimension, EdgeInsets, FlexAlign,
    FlexDirection, FlexJustify, LayoutProps, Opcode, RawPropValueRef, StyleProps,
};
use super::{HEADER_SIZE, NVE_ABI_MAGIC, NVE_ABI_VERSION};

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
