//! Declarative single-source specification for NVE cross-language wire codecs:
//! - Binary `NVEV` v2 uniform 64-byte [`nve_abi::NveEvent`] + byte-arena batch framing
//! - Binary `NVE_DRAW1` vector canvas stream constants, opcodes, fixed payload sizes, and header layout

use nve_abi::{
    EventFlags, InteractionEvent, LayoutEvent, ScrollEvent, TextInputEvent, Xxh64Hasher,
    NVE_EVENT_INLINE_KEY_CAP, NVE_EVENT_SIZE, NVE_EVENT_WIRE_HEADER_SIZE, NVE_EVENT_WIRE_MAGIC,
    NVE_EVENT_WIRE_VERSION, NVE_TABLE_CUSTOM, NVE_TABLE_INTERACTION, NVE_TABLE_LAYOUT,
    NVE_TABLE_SCROLL, NVE_TABLE_TEXT_INPUT,
};

// ============================================================================
// Uniform 64-byte `NveEvent` batch wire format (`NVEV` v2)
// Matches `packages/core/abi/src/event/{mod,buf}.rs`
// ============================================================================

/// 4-byte ASCII envelope magic identifying an `NVEV` event batch frame.
pub const EVENT_WIRE_MAGIC_BYTES: [u8; 4] = NVE_EVENT_WIRE_MAGIC;

/// Little-endian `u32` representation of `b"NVEV"` (`0x5645564E`).
pub const EVENT_WIRE_MAGIC: u32 = u32::from_le_bytes(EVENT_WIRE_MAGIC_BYTES);

/// Wire format version for 64-byte `NveEvent` batch frames (`2`).
pub const EVENT_WIRE_VERSION: u16 = NVE_EVENT_WIRE_VERSION;

/// Fixed `NVEV` v2 header size in bytes (`magic[4] + version:u16 + event_count:u32 + bytes_len:u32`).
pub const EVENT_WIRE_HEADER_SIZE: usize = NVE_EVENT_WIRE_HEADER_SIZE;

/// Exact byte stride of one little-endian `NveEvent` record (64 bytes).
pub const EVENT_RECORD_SIZE: usize = NVE_EVENT_SIZE;

/// Maximum inline UTF-8 key bytes stored in `NveEvent::payload[2..6]`.
pub const EVENT_INLINE_KEY_CAP: usize = NVE_EVENT_INLINE_KEY_CAP;

/// Byte offsets within one 64-byte `NveEvent` record.
pub const EVENT_OFFSET_TIMESTAMP: usize = 0;
pub const EVENT_OFFSET_TABLE: usize = 8;
pub const EVENT_OFFSET_SURFACE: usize = 16;
pub const EVENT_OFFSET_NODE: usize = 20;
pub const EVENT_OFFSET_EVENT: usize = 24;
pub const EVENT_OFFSET_FLAGS: usize = 28;
pub const EVENT_OFFSET_PAYLOAD: usize = 32;
pub const EVENT_OFFSET_BYTES_OFFSET: usize = 56;
pub const EVENT_OFFSET_BYTES_LEN: usize = 60;

/// Canonical compile-time XXH64 `TableKey` values for core event-emitting tables.
pub const TABLE_KEY_INTERACTION: u64 = NVE_TABLE_INTERACTION.0;
pub const TABLE_KEY_TEXT_INPUT: u64 = NVE_TABLE_TEXT_INPUT.0;
pub const TABLE_KEY_SCROLL: u64 = NVE_TABLE_SCROLL.0;
pub const TABLE_KEY_LAYOUT: u64 = NVE_TABLE_LAYOUT.0;
pub const TABLE_KEY_CUSTOM: u64 = NVE_TABLE_CUSTOM.0;

/// Canonical `NveEvent::flags` bit constants (sourced directly from `nve_abi::EventFlags`).
pub const EVENT_FLAG_MOD_META: u32 = EventFlags::MOD_META.raw();
pub const EVENT_FLAG_MOD_CTRL: u32 = EventFlags::MOD_CTRL.raw();
pub const EVENT_FLAG_MOD_SHIFT: u32 = EventFlags::MOD_SHIFT.raw();
pub const EVENT_FLAG_MOD_ALT: u32 = EventFlags::MOD_ALT.raw();
pub const EVENT_FLAG_MOD_REPEAT: u32 = EventFlags::MOD_REPEAT.raw();
pub const EVENT_FLAG_BUTTON_SHIFT: u32 = EventFlags::BUTTON_SHIFT;
pub const EVENT_FLAG_BUTTON_MASK: u32 = EventFlags::BUTTON_MASK.raw();
pub const EVENT_FLAG_CAPTURE: u32 = EventFlags::CAPTURE.raw();
pub const EVENT_FLAG_COMPOSING: u32 = EventFlags::COMPOSING.raw();
pub const EVENT_FLAG_UTF8_BYTES: u32 = EventFlags::UTF8_BYTES.raw();
pub const EVENT_FLAG_INLINE_KEY_UTF8: u32 = EventFlags::INLINE_KEY_UTF8.raw();
pub const EVENT_FLAG_CUSTOM_NAMED: u32 = EventFlags::CUSTOM_NAMED.raw();
pub const EVENT_FLAG_KEY_SPLIT_BYTES: u32 = EventFlags::KEY_SPLIT_BYTES.raw();

/// Named discriminant entry for per-table event enums.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WireEnumVariant {
    pub pascal: &'static str,
    pub camel: &'static str,
    pub screaming: &'static str,
    pub value: u8,
}

/// Authoritative `InteractionEvent` discriminants (must match `nve_abi::InteractionEvent`).
pub const INTERACTION_EVENTS: &[WireEnumVariant] = &[
    WireEnumVariant { pascal: "Press", camel: "press", screaming: "PRESS", value: InteractionEvent::Press as u8 },
    WireEnumVariant { pascal: "LongPress", camel: "longPress", screaming: "LONG_PRESS", value: InteractionEvent::LongPress as u8 },
    WireEnumVariant { pascal: "Focus", camel: "focus", screaming: "FOCUS", value: InteractionEvent::Focus as u8 },
    WireEnumVariant { pascal: "Blur", camel: "blur", screaming: "BLUR", value: InteractionEvent::Blur as u8 },
    WireEnumVariant { pascal: "KeyDown", camel: "keyDown", screaming: "KEY_DOWN", value: InteractionEvent::KeyDown as u8 },
    WireEnumVariant { pascal: "KeyUp", camel: "keyUp", screaming: "KEY_UP", value: InteractionEvent::KeyUp as u8 },
    WireEnumVariant { pascal: "PointerDown", camel: "pointerDown", screaming: "POINTER_DOWN", value: InteractionEvent::PointerDown as u8 },
    WireEnumVariant { pascal: "PointerMove", camel: "pointerMove", screaming: "POINTER_MOVE", value: InteractionEvent::PointerMove as u8 },
    WireEnumVariant { pascal: "PointerUp", camel: "pointerUp", screaming: "POINTER_UP", value: InteractionEvent::PointerUp as u8 },
    WireEnumVariant { pascal: "PointerEnter", camel: "pointerEnter", screaming: "POINTER_ENTER", value: InteractionEvent::PointerEnter as u8 },
    WireEnumVariant { pascal: "PointerLeave", camel: "pointerLeave", screaming: "POINTER_LEAVE", value: InteractionEvent::PointerLeave as u8 },
    WireEnumVariant { pascal: "Wheel", camel: "wheel", screaming: "WHEEL", value: InteractionEvent::Wheel as u8 },
];

/// Authoritative `TextInputEvent` discriminants (must match `nve_abi::TextInputEvent`).
pub const TEXT_INPUT_EVENTS: &[WireEnumVariant] = &[
    WireEnumVariant { pascal: "Change", camel: "change", screaming: "CHANGE", value: TextInputEvent::Change as u8 },
    WireEnumVariant { pascal: "Submit", camel: "submit", screaming: "SUBMIT", value: TextInputEvent::Submit as u8 },
    WireEnumVariant { pascal: "CompositionStart", camel: "compositionStart", screaming: "COMPOSITION_START", value: TextInputEvent::CompositionStart as u8 },
    WireEnumVariant { pascal: "CompositionUpdate", camel: "compositionUpdate", screaming: "COMPOSITION_UPDATE", value: TextInputEvent::CompositionUpdate as u8 },
    WireEnumVariant { pascal: "CompositionEnd", camel: "compositionEnd", screaming: "COMPOSITION_END", value: TextInputEvent::CompositionEnd as u8 },
];

/// Authoritative `ScrollEvent` discriminants (must match `nve_abi::ScrollEvent`).
pub const SCROLL_EVENTS: &[WireEnumVariant] = &[
    WireEnumVariant { pascal: "Scroll", camel: "scroll", screaming: "SCROLL", value: ScrollEvent::Scroll as u8 },
];

/// Authoritative `LayoutEvent` discriminants (must match `nve_abi::LayoutEvent`).
pub const LAYOUT_EVENTS: &[WireEnumVariant] = &[
    WireEnumVariant { pascal: "Layout", camel: "layout", screaming: "LAYOUT", value: LayoutEvent::Layout as u8 },
];

// ============================================================================
// `NVE_DRAW1` vector canvas stream — matches `packages/modules/canvas/src/types.rs`
// ============================================================================

/// 9-byte ASCII magic header identifying a binary `NVE_DRAW1` vector draw-list stream.
pub const DRAW_LIST_MAGIC: [u8; 9] = *b"NVE_DRAW1";

/// Current binary format version of `NVE_DRAW1`.
pub const DRAW_LIST_VERSION: u16 = 1;

/// Fixed byte length of the `NVE_DRAW1` header (`magic[9] + version: u16 LE + op_count: u32 LE`).
pub const DRAW_LIST_HEADER_SIZE: usize = 15;
pub const DRAW_LIST_VERSION_OFFSET: usize = 9;
pub const DRAW_LIST_OP_COUNT_OFFSET: usize = 11;

/// Strict upper bound on operations per draw list.
pub const DRAW_LIST_MAX_OPS: usize = 65_536;

/// Strict upper bound on vertices inside a single `StrokePolyline` operation.
pub const DRAW_LIST_MAX_POLYLINE_POINTS: usize = 16_384;

/// Maximum nested `ClipRect` stack depth guarded during rendering.
pub const DRAW_LIST_MAX_CLIP_DEPTH: usize = 64;

/// Compact ASCII prefix used when transporting `NVE_DRAW1` bytes via UTF-8 string props.
pub const DRAW_LIST_ATTR_PREFIX: &str = "nve-draw1;b64,";

/// Byte stride of each `(x: f32, y: f32)` vertex inside `StrokePolyline`.
pub const DRAW_OP_POLYLINE_POINT_STRIDE: usize = 8;

/// Fixed prefix size (excluding 1-byte opcode) of `StrokePolyline`: `width(4) + color(4) + count(4)`.
pub const DRAW_OP_POLYLINE_HEADER_BYTES: usize = 12;

/// Declarative `NVE_DRAW1` opcode constants (matches `nve_canvas::DrawOpcode`).
pub const DRAW_OP_CLEAR: u8 = 1;
pub const DRAW_OP_FILL_RECT: u8 = 2;
pub const DRAW_OP_FILL_ROUNDED_RECT: u8 = 3;
pub const DRAW_OP_FILL_CIRCLE: u8 = 4;
pub const DRAW_OP_STROKE_LINE: u8 = 5;
pub const DRAW_OP_STROKE_POLYLINE: u8 = 6;
pub const DRAW_OP_CLIP_RECT: u8 = 7;
pub const DRAW_OP_RESTORE_CLIP: u8 = 8;

/// Descriptor for one `NVE_DRAW1` drawing operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrawOpcodeSpec {
    pub pascal: &'static str,
    pub camel: &'static str,
    pub screaming: &'static str,
    pub opcode: u8,
    /// Fixed payload byte length (excluding the 1-byte opcode tag), or `None` if variable-length.
    pub fixed_payload_bytes: Option<usize>,
}

/// Authoritative `NVE_DRAW1` opcode table (must match `packages/modules/canvas/src/types.rs`).
pub const DRAW_OPCODES: &[DrawOpcodeSpec] = &[
    DrawOpcodeSpec { pascal: "Clear", camel: "clear", screaming: "CLEAR", opcode: DRAW_OP_CLEAR, fixed_payload_bytes: Some(4) },
    DrawOpcodeSpec { pascal: "FillRect", camel: "fillRect", screaming: "FILL_RECT", opcode: DRAW_OP_FILL_RECT, fixed_payload_bytes: Some(20) },
    DrawOpcodeSpec { pascal: "FillRoundedRect", camel: "fillRoundedRect", screaming: "FILL_ROUNDED_RECT", opcode: DRAW_OP_FILL_ROUNDED_RECT, fixed_payload_bytes: Some(24) },
    DrawOpcodeSpec { pascal: "FillCircle", camel: "fillCircle", screaming: "FILL_CIRCLE", opcode: DRAW_OP_FILL_CIRCLE, fixed_payload_bytes: Some(16) },
    DrawOpcodeSpec { pascal: "StrokeLine", camel: "strokeLine", screaming: "STROKE_LINE", opcode: DRAW_OP_STROKE_LINE, fixed_payload_bytes: Some(24) },
    DrawOpcodeSpec { pascal: "StrokePolyline", camel: "strokePolyline", screaming: "STROKE_POLYLINE", opcode: DRAW_OP_STROKE_POLYLINE, fixed_payload_bytes: None },
    DrawOpcodeSpec { pascal: "ClipRect", camel: "clipRect", screaming: "CLIP_RECT", opcode: DRAW_OP_CLIP_RECT, fixed_payload_bytes: Some(16) },
    DrawOpcodeSpec { pascal: "RestoreClip", camel: "restoreClip", screaming: "RESTORE_CLIP", opcode: DRAW_OP_RESTORE_CLIP, fixed_payload_bytes: Some(0) },
];

const fn hash_variants(h: &mut Xxh64Hasher, tag: &[u8], list: &[WireEnumVariant]) {
    let mut i = 0;
    while i < list.len() {
        h.update(tag);
        h.update(list[i].screaming.as_bytes());
        h.update(&[list[i].value]);
        i += 1;
    }
}

/// Computes the deterministic XXH64 schema hash covering every constant, enum,
/// layout offset, and opcode in this wire codec specification.
pub const fn wire_codecs_schema_hash() -> u64 {
    let mut h = Xxh64Hasher::new(0);
    h.update(b"nve.wire_codecs.v3");
    h.update(&EVENT_WIRE_MAGIC_BYTES);
    h.update(&EVENT_WIRE_VERSION.to_le_bytes());
    h.update_u64(EVENT_WIRE_HEADER_SIZE as u64);
    h.update_u64(EVENT_RECORD_SIZE as u64);
    h.update_u64(EVENT_INLINE_KEY_CAP as u64);
    h.update_u64(TABLE_KEY_INTERACTION);
    h.update_u64(TABLE_KEY_TEXT_INPUT);
    h.update_u64(TABLE_KEY_SCROLL);
    h.update_u64(TABLE_KEY_LAYOUT);
    h.update_u64(TABLE_KEY_CUSTOM);
    h.update_u64(EVENT_FLAG_MOD_META as u64);
    h.update_u64(EVENT_FLAG_MOD_CTRL as u64);
    h.update_u64(EVENT_FLAG_MOD_SHIFT as u64);
    h.update_u64(EVENT_FLAG_MOD_ALT as u64);
    h.update_u64(EVENT_FLAG_MOD_REPEAT as u64);
    h.update_u64(EVENT_FLAG_BUTTON_SHIFT as u64);
    h.update_u64(EVENT_FLAG_BUTTON_MASK as u64);
    h.update_u64(EVENT_FLAG_CAPTURE as u64);
    h.update_u64(EVENT_FLAG_COMPOSING as u64);
    h.update_u64(EVENT_FLAG_UTF8_BYTES as u64);
    h.update_u64(EVENT_FLAG_INLINE_KEY_UTF8 as u64);
    h.update_u64(EVENT_FLAG_CUSTOM_NAMED as u64);
    h.update_u64(EVENT_FLAG_KEY_SPLIT_BYTES as u64);
    hash_variants(&mut h, b"|ie:", INTERACTION_EVENTS);
    hash_variants(&mut h, b"|te:", TEXT_INPUT_EVENTS);
    hash_variants(&mut h, b"|se:", SCROLL_EVENTS);
    hash_variants(&mut h, b"|le:", LAYOUT_EVENTS);
    h.update(&DRAW_LIST_MAGIC);
    h.update(&DRAW_LIST_VERSION.to_le_bytes());
    h.update_u64(DRAW_LIST_HEADER_SIZE as u64);
    h.update_u64(DRAW_LIST_MAX_OPS as u64);
    h.update_u64(DRAW_LIST_MAX_POLYLINE_POINTS as u64);
    h.update_u64(DRAW_LIST_MAX_CLIP_DEPTH as u64);
    h.update(DRAW_LIST_ATTR_PREFIX.as_bytes());

    let mut i = 0;
    while i < DRAW_OPCODES.len() {
        h.update(b"|do:");
        h.update(DRAW_OPCODES[i].screaming.as_bytes());
        h.update(&[DRAW_OPCODES[i].opcode]);
        let sz = match DRAW_OPCODES[i].fixed_payload_bytes {
            Some(n) => n as u64,
            None => u64::MAX,
        };
        h.update(&sz.to_le_bytes());
        i += 1;
    }
    h.digest()
}

/// Deterministic XXH64 schema hash of the wire codec spec.
pub const NVE_WIRE_CODECS_SCHEMA_HASH: u64 = wire_codecs_schema_hash();
