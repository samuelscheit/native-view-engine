use crate::{FrameId, NodeId, SurfaceId};

use super::types::{ComponentKind, LayoutProps, Opcode, RawPropValue, RawPropValueRef, StyleProps};
use super::{HEADER_SIZE, NVE_ABI_MAGIC, NVE_ABI_VERSION};

const DEFAULT_INITIAL_CAPACITY: usize = 16 * 1024;

/// Flat binary command buffer builder. Reuse across frames via `reset` for zero steady-state allocations.
pub struct CommandBufferBuilder {
    buf: Vec<u8>,
    surface_id: SurfaceId,
    frame_id: FrameId,
    op_count: u32,
}

impl CommandBufferBuilder {
    #[inline]
    pub fn new(surface_id: SurfaceId, frame_id: FrameId) -> Self {
        Self::with_capacity(surface_id, frame_id, DEFAULT_INITIAL_CAPACITY)
    }

    #[inline]
    pub fn with_capacity(surface_id: SurfaceId, frame_id: FrameId, capacity: usize) -> Self {
        let mut this = Self {
            buf: Vec::with_capacity(capacity.max(HEADER_SIZE)),
            surface_id,
            frame_id,
            op_count: 0,
        };
        this.write_header_placeholder();
        this
    }

    #[inline(always)]
    pub fn reset(&mut self, surface_id: SurfaceId, frame_id: FrameId) {
        self.buf.clear();
        self.surface_id = surface_id;
        self.frame_id = frame_id;
        self.op_count = 0;
        self.write_header_placeholder();
    }

    #[inline(always)]
    fn write_header_placeholder(&mut self) {
        let mut hdr = [0u8; HEADER_SIZE];
        hdr[0..4].copy_from_slice(&NVE_ABI_MAGIC.to_le_bytes());
        hdr[4..6].copy_from_slice(&NVE_ABI_VERSION.to_le_bytes());
        // hdr[6..8] is flags = 0
        hdr[8..12].copy_from_slice(&self.surface_id.to_le_bytes());
        hdr[12..20].copy_from_slice(&self.frame_id.to_le_bytes());
        // hdr[20..24] is op_count placeholder = 0
        self.buf.extend_from_slice(&hdr);
        debug_assert_eq!(self.buf.len(), HEADER_SIZE);
    }

    #[inline(always)]
    pub fn op_count(&self) -> u32 {
        self.op_count
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.op_count == 0
    }

    #[inline(always)]
    pub fn create_node(&mut self, node_id: NodeId, kind: ComponentKind, custom_name: Option<&str>) {
        let name_bytes = custom_name.unwrap_or("").as_bytes();
        let len = u16::try_from(name_bytes.len()).expect("custom component name too long");
        let nid = node_id.to_le_bytes();
        let kind_b = (kind as u16).to_le_bytes();
        let len_b = len.to_le_bytes();
        let frame = [
            Opcode::CreateNode as u8,
            nid[0],
            nid[1],
            nid[2],
            nid[3],
            kind_b[0],
            kind_b[1],
            len_b[0],
            len_b[1],
        ];
        if name_bytes.is_empty() {
            self.buf.extend_from_slice(&frame);
        } else {
            self.buf.reserve(frame.len() + name_bytes.len());
            self.buf.extend_from_slice(&frame);
            self.buf.extend_from_slice(name_bytes);
        }
        self.op_count += 1;
    }

    #[inline(always)]
    pub fn remove_node(&mut self, node_id: NodeId) {
        let nid = node_id.to_le_bytes();
        let frame = [Opcode::RemoveNode as u8, nid[0], nid[1], nid[2], nid[3]];
        self.buf.extend_from_slice(&frame);
        self.op_count += 1;
    }

    #[inline(always)]
    pub fn set_children(&mut self, parent_id: NodeId, children: &[NodeId]) {
        let len = u32::try_from(children.len()).expect("too many children");
        let pid = parent_id.to_le_bytes();
        let len_b = len.to_le_bytes();
        let header = [
            Opcode::SetChildren as u8,
            pid[0],
            pid[1],
            pid[2],
            pid[3],
            len_b[0],
            len_b[1],
            len_b[2],
            len_b[3],
        ];
        let payload_bytes = std::mem::size_of_val(children);
        self.buf.reserve(header.len() + payload_bytes);
        self.buf.extend_from_slice(&header);
        #[cfg(target_endian = "little")]
        {
            // SAFETY: `NodeId` (`u32`) is a plain integer with no padding bytes, and on
            // little-endian targets its in-memory representation matches `to_le_bytes()`.
            let raw = unsafe {
                std::slice::from_raw_parts(children.as_ptr().cast::<u8>(), payload_bytes)
            };
            self.buf.extend_from_slice(raw);
        }
        #[cfg(not(target_endian = "little"))]
        {
            for &child in children {
                self.buf.extend_from_slice(&child.to_le_bytes());
            }
        }
        self.op_count += 1;
    }

    #[inline(always)]
    pub fn set_text(&mut self, node_id: NodeId, text: &str) {
        let bytes = text.as_bytes();
        let len = u32::try_from(bytes.len()).expect("text too long");
        let nid = node_id.to_le_bytes();
        let len_b = len.to_le_bytes();
        let header = [
            Opcode::SetText as u8,
            nid[0],
            nid[1],
            nid[2],
            nid[3],
            len_b[0],
            len_b[1],
            len_b[2],
            len_b[3],
        ];
        self.buf.reserve(header.len() + bytes.len());
        self.buf.extend_from_slice(&header);
        self.buf.extend_from_slice(bytes);
        self.op_count += 1;
    }

    #[inline(always)]
    pub fn set_layout(&mut self, node_id: NodeId, layout: &LayoutProps) {
        let mut frame = [0u8; 82];
        frame[0] = Opcode::SetLayout as u8;
        frame[1..5].copy_from_slice(&node_id.to_le_bytes());
        layout
            .width
            .encode((&mut frame[5..10]).try_into().unwrap());
        layout
            .height
            .encode((&mut frame[10..15]).try_into().unwrap());
        layout
            .min_width
            .encode((&mut frame[15..20]).try_into().unwrap());
        layout
            .min_height
            .encode((&mut frame[20..25]).try_into().unwrap());
        layout
            .max_width
            .encode((&mut frame[25..30]).try_into().unwrap());
        layout
            .max_height
            .encode((&mut frame[30..35]).try_into().unwrap());
        frame[35] = layout.flex_direction as u8;
        frame[36] = layout.justify_content as u8;
        frame[37] = layout.align_items as u8;
        frame[38..42].copy_from_slice(&layout.flex_grow.to_le_bytes());
        frame[42..46].copy_from_slice(&layout.flex_shrink.to_le_bytes());
        frame[46..50].copy_from_slice(&layout.gap.to_le_bytes());
        layout
            .padding
            .encode((&mut frame[50..66]).try_into().unwrap());
        layout
            .margin
            .encode((&mut frame[66..82]).try_into().unwrap());
        self.buf.extend_from_slice(&frame);
        self.op_count += 1;
    }

    #[inline(always)]
    pub fn set_style(&mut self, node_id: NodeId, style: &StyleProps) {
        let mut frame = [0u8; 37];
        frame[0] = Opcode::SetStyle as u8;
        frame[1..5].copy_from_slice(&node_id.to_le_bytes());
        frame[5..9].copy_from_slice(&style.background.to_u32().to_le_bytes());
        frame[9..13].copy_from_slice(&style.text_color.to_u32().to_le_bytes());
        frame[13..17].copy_from_slice(&style.border_color.to_u32().to_le_bytes());
        frame[17..21].copy_from_slice(&style.border_width.to_le_bytes());
        frame[21..25].copy_from_slice(&style.border_radius.to_le_bytes());
        frame[25..29].copy_from_slice(&style.opacity.to_le_bytes());
        frame[29..33].copy_from_slice(&style.font_size.to_le_bytes());
        frame[33..37].copy_from_slice(&style.z_index.to_le_bytes());
        self.buf.extend_from_slice(&frame);
        self.op_count += 1;
    }

    #[inline(always)]
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
    #[inline(always)]
    pub fn set_raw_prop_ref(&mut self, node_id: NodeId, key: &str, value: RawPropValueRef<'_>) {
        let key_bytes = key.as_bytes();
        let key_len = u16::try_from(key_bytes.len()).expect("prop key too long");
        let nid = node_id.to_le_bytes();
        let klen = key_len.to_le_bytes();
        let header = [
            Opcode::SetRawProp as u8,
            nid[0],
            nid[1],
            nid[2],
            nid[3],
            klen[0],
            klen[1],
        ];
        match value {
            RawPropValueRef::Null => {
                self.buf.reserve(header.len() + key_bytes.len() + 1);
                self.buf.extend_from_slice(&header);
                self.buf.extend_from_slice(key_bytes);
                self.buf.push(0);
            }
            RawPropValueRef::Bool(b) => {
                let tail = [1u8, u8::from(b)];
                self.buf.reserve(header.len() + key_bytes.len() + tail.len());
                self.buf.extend_from_slice(&header);
                self.buf.extend_from_slice(key_bytes);
                self.buf.extend_from_slice(&tail);
            }
            RawPropValueRef::I64(v) => {
                let vb = v.to_le_bytes();
                let tail = [2u8, vb[0], vb[1], vb[2], vb[3], vb[4], vb[5], vb[6], vb[7]];
                self.buf.reserve(header.len() + key_bytes.len() + tail.len());
                self.buf.extend_from_slice(&header);
                self.buf.extend_from_slice(key_bytes);
                self.buf.extend_from_slice(&tail);
            }
            RawPropValueRef::F64(v) => {
                let vb = v.to_le_bytes();
                let tail = [3u8, vb[0], vb[1], vb[2], vb[3], vb[4], vb[5], vb[6], vb[7]];
                self.buf.reserve(header.len() + key_bytes.len() + tail.len());
                self.buf.extend_from_slice(&header);
                self.buf.extend_from_slice(key_bytes);
                self.buf.extend_from_slice(&tail);
            }
            RawPropValueRef::String(s) => {
                let b = s.as_bytes();
                let len = u32::try_from(b.len()).expect("raw prop string too long");
                let lb = len.to_le_bytes();
                let tag_len = [4u8, lb[0], lb[1], lb[2], lb[3]];
                self.buf
                    .reserve(header.len() + key_bytes.len() + tag_len.len() + b.len());
                self.buf.extend_from_slice(&header);
                self.buf.extend_from_slice(key_bytes);
                self.buf.extend_from_slice(&tag_len);
                self.buf.extend_from_slice(b);
            }
            RawPropValueRef::Bytes(b) => {
                let len = u32::try_from(b.len()).expect("raw prop bytes too long");
                let lb = len.to_le_bytes();
                let tag_len = [5u8, lb[0], lb[1], lb[2], lb[3]];
                self.buf
                    .reserve(header.len() + key_bytes.len() + tag_len.len() + b.len());
                self.buf.extend_from_slice(&header);
                self.buf.extend_from_slice(key_bytes);
                self.buf.extend_from_slice(&tag_len);
                self.buf.extend_from_slice(b);
            }
        }
        self.op_count += 1;
    }

    /// Convenience helper to write a string raw property directly from `&str` with zero allocation.
    #[inline(always)]
    pub fn set_raw_prop_str(&mut self, node_id: NodeId, key: &str, value: &str) {
        self.set_raw_prop_ref(node_id, key, RawPropValueRef::String(value));
    }

    #[inline(always)]
    pub fn set_event_mask(&mut self, node_id: NodeId, mask: u64) {
        let nid = node_id.to_le_bytes();
        let m = mask.to_le_bytes();
        let frame = [
            Opcode::SetEventMask as u8,
            nid[0],
            nid[1],
            nid[2],
            nid[3],
            m[0],
            m[1],
            m[2],
            m[3],
            m[4],
            m[5],
            m[6],
            m[7],
        ];
        self.buf.extend_from_slice(&frame);
        self.op_count += 1;
    }

    #[inline]
    pub fn dispatch_command(&mut self, node_id: NodeId, command_name: &str, payload: &[u8]) {
        let name = command_name.as_bytes();
        let nid = node_id.to_le_bytes();
        let nlen = (name.len() as u16).to_le_bytes();
        let plen = (payload.len() as u32).to_le_bytes();
        let header = [
            Opcode::DispatchCommand as u8,
            nid[0],
            nid[1],
            nid[2],
            nid[3],
            nlen[0],
            nlen[1],
        ];
        self.buf
            .reserve(header.len() + name.len() + plen.len() + payload.len());
        self.buf.extend_from_slice(&header);
        self.buf.extend_from_slice(name);
        self.buf.extend_from_slice(&plen);
        self.buf.extend_from_slice(payload);
        self.op_count += 1;
    }

    #[inline(always)]
    pub fn commit(&mut self, sync_mount: bool) {
        let frame = [Opcode::Commit as u8, u8::from(sync_mount)];
        self.buf.extend_from_slice(&frame);
        self.op_count += 1;
    }

    /// Patches the op_count in the header and returns the finished byte slice.
    #[inline(always)]
    pub fn finish(&mut self) -> &[u8] {
        self.buf[20..24].copy_from_slice(&self.op_count.to_le_bytes());
        &self.buf
    }

    #[inline(always)]
    pub fn into_bytes(mut self) -> Vec<u8> {
        self.finish();
        self.buf
    }
}
