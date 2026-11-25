use crate::{FrameId, NodeId, SurfaceId};

use super::types::{ComponentKind, LayoutProps, Opcode, RawPropValue, RawPropValueRef, StyleProps};
use super::{HEADER_SIZE, NVE_ABI_MAGIC, NVE_ABI_VERSION};

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
