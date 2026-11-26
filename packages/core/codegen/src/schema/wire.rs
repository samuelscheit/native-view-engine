//! Declarative single-source specification for NVE cross-language wire codecs:
//! - Canonical 64-bit FNV-1a (`0xcbf29ce484222325` / `0x00000100000001b3`) + reference vectors
//! - Binary `EventRing` framing (`NVEV` / count header, 18-byte entry layout, `NativeEventKind`,
//!   `EventPayloadTag`, `EventMask`, typed key modifier bits)
//! - Binary `NVE_DRAW1` vector canvas stream constants, opcodes, fixed payload sizes, and header layout

/// Canonical 64-bit FNV-1a offset basis (`14695981039346656037`).
pub const FNV1A_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;

/// Canonical 64-bit FNV-1a prime (`1099511628211`).
pub const FNV1A_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Updates an in-progress 64-bit FNV-1a hash with `bytes`.
pub const fn fnv1a64_update(mut hash: u64, bytes: &[u8]) -> u64 {
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u64;
        hash = hash.wrapping_mul(FNV1A_PRIME);
        i += 1;
    }
    hash
}

/// Computes the canonical 64-bit FNV-1a hash of `bytes`.
pub const fn fnv1a64(bytes: &[u8]) -> u64 {
    fnv1a64_update(FNV1A_OFFSET_BASIS, bytes)
}

/// Computes the canonical 64-bit FNV-1a hash over the concatenation `a || b`.
pub const fn fnv1a64_pair(a: &[u8], b: &[u8]) -> u64 {
    fnv1a64_update(fnv1a64_update(FNV1A_OFFSET_BASIS, a), b)
}

/// Single reference test vector emitted into every target language to lock unsigned-byte
/// XOR and 64-bit wrapping multiplication semantics across C++, Swift, Dart, TS, and Java.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fnv1a64TestVector {
    pub label: &'static str,
    pub bytes: &'static [u8],
    pub expected: u64,
}

const BINARY_HIGH_BIT_VECTOR: &[u8] = &[0x00, 0x01, 0x7f, 0x80, 0xff];

/// Authoritative FNV-1a 64-bit test vectors computed directly from [`fnv1a64`].
pub const FNV1A64_TEST_VECTORS: &[Fnv1a64TestVector] = &[
    Fnv1a64TestVector {
        label: "empty",
        bytes: b"",
        expected: fnv1a64(b""),
    },
    Fnv1a64TestVector {
        label: "a",
        bytes: b"a",
        expected: fnv1a64(b"a"),
    },
    Fnv1a64TestVector {
        label: "foobar",
        bytes: b"foobar",
        expected: fnv1a64(b"foobar"),
    },
    Fnv1a64TestVector {
        label: "nvev",
        bytes: b"NVEV",
        expected: fnv1a64(b"NVEV"),
    },
    Fnv1a64TestVector {
        label: "nve_draw1",
        bytes: b"NVE_DRAW1",
        expected: fnv1a64(b"NVE_DRAW1"),
    },
    Fnv1a64TestVector {
        label: "high_bits",
        bytes: BINARY_HIGH_BIT_VECTOR,
        expected: fnv1a64(BINARY_HIGH_BIT_VECTOR),
    },
];

// ============================================================================
// EventRing (`NVEV` / event framing) — matches `packages/core/abi/src/events.rs`
// ============================================================================

/// 4-byte ASCII envelope magic identifying a framed `EventRing` stream (`NVEV`).
pub const EVENT_RING_MAGIC_BYTES: [u8; 4] = *b"NVEV";

/// Little-endian `u32` representation of `b"NVEV"` (`0x5645564E`).
pub const EVENT_RING_MAGIC: u32 = u32::from_le_bytes(EVENT_RING_MAGIC_BYTES);

/// Wire format version for framed `NVEV` `EventRing` headers.
pub const EVENT_RING_VERSION: u16 = 1;

/// Unframed `EventRing` count-prefix size in bytes (`count: u32 LE`), matching `encode_events`.
pub const EVENT_RING_COUNT_HEADER_SIZE: usize = 4;

/// Framed `NVEV` envelope header size in bytes (`magic[4] + version: u16 LE + count: u32 LE`).
pub const EVENT_RING_FRAMED_HEADER_SIZE: usize = 10;

/// Fixed byte length of each `EventRing` entry prefix preceding the payload:
/// `surface_id(u32) + node_id(u32) + timestamp_us(u64) + kind(u8) + payload_tag(u8)`.
pub const EVENT_ENTRY_HEADER_SIZE: usize = 18;
pub const EVENT_ENTRY_SURFACE_ID_OFFSET: usize = 0;
pub const EVENT_ENTRY_NODE_ID_OFFSET: usize = 4;
pub const EVENT_ENTRY_TIMESTAMP_US_OFFSET: usize = 8;
pub const EVENT_ENTRY_KIND_OFFSET: usize = 16;
pub const EVENT_ENTRY_PAYLOAD_TAG_OFFSET: usize = 17;

/// Named discriminant entry for wire enums (`NativeEventKind`, `EventPayloadTag`, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WireEnumVariant {
    pub pascal: &'static str,
    pub camel: &'static str,
    pub screaming: &'static str,
    pub value: u8,
}

/// Authoritative `NativeEventKind` discriminants (must match `nve_abi::NativeEventKind`).
pub const EVENT_KINDS: &[WireEnumVariant] = &[
    WireEnumVariant {
        pascal: "Press",
        camel: "press",
        screaming: "PRESS",
        value: 1,
    },
    WireEnumVariant {
        pascal: "LongPress",
        camel: "longPress",
        screaming: "LONG_PRESS",
        value: 2,
    },
    WireEnumVariant {
        pascal: "Scroll",
        camel: "scroll",
        screaming: "SCROLL",
        value: 3,
    },
    WireEnumVariant {
        pascal: "TextChange",
        camel: "textChange",
        screaming: "TEXT_CHANGE",
        value: 4,
    },
    WireEnumVariant {
        pascal: "Focus",
        camel: "focus",
        screaming: "FOCUS",
        value: 5,
    },
    WireEnumVariant {
        pascal: "Blur",
        camel: "blur",
        screaming: "BLUR",
        value: 6,
    },
    WireEnumVariant {
        pascal: "Layout",
        camel: "layout",
        screaming: "LAYOUT",
        value: 7,
    },
    WireEnumVariant {
        pascal: "Custom",
        camel: "custom",
        screaming: "CUSTOM",
        value: 8,
    },
    WireEnumVariant {
        pascal: "CompositionStart",
        camel: "compositionStart",
        screaming: "COMPOSITION_START",
        value: 9,
    },
    WireEnumVariant {
        pascal: "CompositionUpdate",
        camel: "compositionUpdate",
        screaming: "COMPOSITION_UPDATE",
        value: 10,
    },
    WireEnumVariant {
        pascal: "CompositionEnd",
        camel: "compositionEnd",
        screaming: "COMPOSITION_END",
        value: 11,
    },
    WireEnumVariant {
        pascal: "KeyDown",
        camel: "keyDown",
        screaming: "KEY_DOWN",
        value: 12,
    },
    WireEnumVariant {
        pascal: "KeyUp",
        camel: "keyUp",
        screaming: "KEY_UP",
        value: 13,
    },
    WireEnumVariant {
        pascal: "Submit",
        camel: "submit",
        screaming: "SUBMIT",
        value: 14,
    },
    WireEnumVariant {
        pascal: "PointerDown",
        camel: "pointerDown",
        screaming: "POINTER_DOWN",
        value: 15,
    },
    WireEnumVariant {
        pascal: "PointerMove",
        camel: "pointerMove",
        screaming: "POINTER_MOVE",
        value: 16,
    },
    WireEnumVariant {
        pascal: "PointerUp",
        camel: "pointerUp",
        screaming: "POINTER_UP",
        value: 17,
    },
    WireEnumVariant {
        pascal: "PointerEnter",
        camel: "pointerEnter",
        screaming: "POINTER_ENTER",
        value: 18,
    },
    WireEnumVariant {
        pascal: "PointerLeave",
        camel: "pointerLeave",
        screaming: "POINTER_LEAVE",
        value: 19,
    },
    WireEnumVariant {
        pascal: "Wheel",
        camel: "wheel",
        screaming: "WHEEL",
        value: 20,
    },
];

/// Payload tag descriptor for `EventPayload` wire framing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventPayloadTagSpec {
    pub pascal: &'static str,
    pub camel: &'static str,
    pub screaming: &'static str,
    pub tag: u8,
    /// Fixed byte size after the tag byte, or `None` for length-prefixed payloads.
    pub fixed_payload_bytes: Option<usize>,
}

/// Authoritative `EventPayload` wire tags (must match `nve_abi::encode_events`).
pub const EVENT_PAYLOAD_TAGS: &[EventPayloadTagSpec] = &[
    EventPayloadTagSpec {
        pascal: "Empty",
        camel: "empty",
        screaming: "EMPTY",
        tag: 0,
        fixed_payload_bytes: Some(0),
    },
    EventPayloadTagSpec {
        pascal: "Point",
        camel: "point",
        screaming: "POINT",
        tag: 1,
        fixed_payload_bytes: Some(8),
    },
    EventPayloadTagSpec {
        pascal: "Scroll",
        camel: "scroll",
        screaming: "SCROLL",
        tag: 2,
        fixed_payload_bytes: Some(16),
    },
    EventPayloadTagSpec {
        pascal: "Text",
        camel: "text",
        screaming: "TEXT",
        tag: 3,
        fixed_payload_bytes: None,
    },
    EventPayloadTagSpec {
        pascal: "Layout",
        camel: "layout",
        screaming: "LAYOUT",
        tag: 4,
        fixed_payload_bytes: Some(16),
    },
    EventPayloadTagSpec {
        pascal: "Custom",
        camel: "custom",
        screaming: "CUSTOM",
        tag: 5,
        fixed_payload_bytes: None,
    },
    EventPayloadTagSpec {
        pascal: "Key",
        camel: "key",
        screaming: "KEY",
        tag: 6,
        fixed_payload_bytes: None,
    },
    EventPayloadTagSpec {
        pascal: "Pointer",
        camel: "pointer",
        screaming: "POINTER",
        tag: 7,
        fixed_payload_bytes: Some(10),
    },
    EventPayloadTagSpec {
        pascal: "Wheel",
        camel: "wheel",
        screaming: "WHEEL",
        tag: 8,
        fixed_payload_bytes: Some(9),
    },
];

/// Named bitflag entry for `EventMask`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WireBitflag {
    pub pascal: &'static str,
    pub camel: &'static str,
    pub screaming: &'static str,
    pub value: u64,
}

/// Authoritative `EventMask` bitflags (must match `nve_abi::EventMask`).
pub const EVENT_MASKS: &[WireBitflag] = &[
    WireBitflag {
        pascal: "None",
        camel: "none",
        screaming: "NONE",
        value: 0,
    },
    WireBitflag {
        pascal: "Press",
        camel: "press",
        screaming: "PRESS",
        value: 1 << 0,
    },
    WireBitflag {
        pascal: "LongPress",
        camel: "longPress",
        screaming: "LONG_PRESS",
        value: 1 << 1,
    },
    WireBitflag {
        pascal: "Scroll",
        camel: "scroll",
        screaming: "SCROLL",
        value: 1 << 2,
    },
    WireBitflag {
        pascal: "TextChange",
        camel: "textChange",
        screaming: "TEXT_CHANGE",
        value: 1 << 3,
    },
    WireBitflag {
        pascal: "Focus",
        camel: "focus",
        screaming: "FOCUS",
        value: 1 << 4,
    },
    WireBitflag {
        pascal: "Blur",
        camel: "blur",
        screaming: "BLUR",
        value: 1 << 5,
    },
    WireBitflag {
        pascal: "Layout",
        camel: "layout",
        screaming: "LAYOUT",
        value: 1 << 6,
    },
    WireBitflag {
        pascal: "Custom",
        camel: "custom",
        screaming: "CUSTOM",
        value: 1 << 7,
    },
    WireBitflag {
        pascal: "Composition",
        camel: "composition",
        screaming: "COMPOSITION",
        value: 1 << 8,
    },
    WireBitflag {
        pascal: "KeyDown",
        camel: "keyDown",
        screaming: "KEY_DOWN",
        value: 1 << 9,
    },
    WireBitflag {
        pascal: "KeyUp",
        camel: "keyUp",
        screaming: "KEY_UP",
        value: 1 << 10,
    },
    WireBitflag {
        pascal: "Submit",
        camel: "submit",
        screaming: "SUBMIT",
        value: 1 << 11,
    },
    WireBitflag {
        pascal: "PointerDown",
        camel: "pointerDown",
        screaming: "POINTER_DOWN",
        value: 1 << 12,
    },
    WireBitflag {
        pascal: "PointerMove",
        camel: "pointerMove",
        screaming: "POINTER_MOVE",
        value: 1 << 13,
    },
    WireBitflag {
        pascal: "PointerUp",
        camel: "pointerUp",
        screaming: "POINTER_UP",
        value: 1 << 14,
    },
    WireBitflag {
        pascal: "PointerEnter",
        camel: "pointerEnter",
        screaming: "POINTER_ENTER",
        value: 1 << 15,
    },
    WireBitflag {
        pascal: "PointerLeave",
        camel: "pointerLeave",
        screaming: "POINTER_LEAVE",
        value: 1 << 16,
    },
    WireBitflag {
        pascal: "Wheel",
        camel: "wheel",
        screaming: "WHEEL",
        value: 1 << 17,
    },
];

/// Typed keyboard modifier bitflags shared with `NveDirectEvent::modifiers`.
pub const KEY_MOD_META: u8 = 1 << 0;
pub const KEY_MOD_CTRL: u8 = 1 << 1;
pub const KEY_MOD_SHIFT: u8 = 1 << 2;
pub const KEY_MOD_ALT: u8 = 1 << 3;
pub const KEY_MOD_REPEAT: u8 = 1 << 4;

// ============================================================================
// `NVE_DRAW1` vector canvas stream — matches `packages/core/components/src/canvas_ir`
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

/// Declarative `NVE_DRAW1` opcode constants (matches `canvas_ir::DrawOpcode`).
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

/// Authoritative `NVE_DRAW1` opcode table (must match `packages/core/components/src/canvas_ir`).
pub const DRAW_OPCODES: &[DrawOpcodeSpec] = &[
    DrawOpcodeSpec {
        pascal: "Clear",
        camel: "clear",
        screaming: "CLEAR",
        opcode: DRAW_OP_CLEAR,
        fixed_payload_bytes: Some(4),
    },
    DrawOpcodeSpec {
        pascal: "FillRect",
        camel: "fillRect",
        screaming: "FILL_RECT",
        opcode: DRAW_OP_FILL_RECT,
        fixed_payload_bytes: Some(20),
    },
    DrawOpcodeSpec {
        pascal: "FillRoundedRect",
        camel: "fillRoundedRect",
        screaming: "FILL_ROUNDED_RECT",
        opcode: DRAW_OP_FILL_ROUNDED_RECT,
        fixed_payload_bytes: Some(24),
    },
    DrawOpcodeSpec {
        pascal: "FillCircle",
        camel: "fillCircle",
        screaming: "FILL_CIRCLE",
        opcode: DRAW_OP_FILL_CIRCLE,
        fixed_payload_bytes: Some(16),
    },
    DrawOpcodeSpec {
        pascal: "StrokeLine",
        camel: "strokeLine",
        screaming: "STROKE_LINE",
        opcode: DRAW_OP_STROKE_LINE,
        fixed_payload_bytes: Some(24),
    },
    DrawOpcodeSpec {
        pascal: "StrokePolyline",
        camel: "strokePolyline",
        screaming: "STROKE_POLYLINE",
        opcode: DRAW_OP_STROKE_POLYLINE,
        fixed_payload_bytes: None,
    },
    DrawOpcodeSpec {
        pascal: "ClipRect",
        camel: "clipRect",
        screaming: "CLIP_RECT",
        opcode: DRAW_OP_CLIP_RECT,
        fixed_payload_bytes: Some(16),
    },
    DrawOpcodeSpec {
        pascal: "RestoreClip",
        camel: "restoreClip",
        screaming: "RESTORE_CLIP",
        opcode: DRAW_OP_RESTORE_CLIP,
        fixed_payload_bytes: Some(0),
    },
];

/// Computes the deterministic FNV-1a 64-bit schema hash covering every constant, enum,
/// layout offset, and opcode in this wire codec specification.
pub const fn wire_codecs_schema_hash() -> u64 {
    let mut h = FNV1A_OFFSET_BASIS;
    h = fnv1a64_update(h, b"nve.wire_codecs.v1");
    h = fnv1a64_update(h, &FNV1A_OFFSET_BASIS.to_le_bytes());
    h = fnv1a64_update(h, &FNV1A_PRIME.to_le_bytes());
    h = fnv1a64_update(h, &EVENT_RING_MAGIC_BYTES);
    h = fnv1a64_update(h, &EVENT_RING_VERSION.to_le_bytes());
    h = fnv1a64_update(h, &(EVENT_RING_COUNT_HEADER_SIZE as u64).to_le_bytes());
    h = fnv1a64_update(h, &(EVENT_RING_FRAMED_HEADER_SIZE as u64).to_le_bytes());
    h = fnv1a64_update(h, &(EVENT_ENTRY_HEADER_SIZE as u64).to_le_bytes());
    h = fnv1a64_update(h, &[KEY_MOD_META, KEY_MOD_CTRL, KEY_MOD_SHIFT, KEY_MOD_ALT, KEY_MOD_REPEAT]);
    h = fnv1a64_update(h, &DRAW_LIST_MAGIC);
    h = fnv1a64_update(h, &DRAW_LIST_VERSION.to_le_bytes());
    h = fnv1a64_update(h, &(DRAW_LIST_HEADER_SIZE as u64).to_le_bytes());
    h = fnv1a64_update(h, &(DRAW_LIST_MAX_OPS as u64).to_le_bytes());
    h = fnv1a64_update(h, &(DRAW_LIST_MAX_POLYLINE_POINTS as u64).to_le_bytes());
    h = fnv1a64_update(h, &(DRAW_LIST_MAX_CLIP_DEPTH as u64).to_le_bytes());
    h = fnv1a64_update(h, DRAW_LIST_ATTR_PREFIX.as_bytes());

    let mut i = 0;
    while i < EVENT_KINDS.len() {
        h = fnv1a64_update(h, b"|ek:");
        h = fnv1a64_update(h, EVENT_KINDS[i].screaming.as_bytes());
        h = fnv1a64_update(h, &[EVENT_KINDS[i].value]);
        i += 1;
    }
    i = 0;
    while i < EVENT_PAYLOAD_TAGS.len() {
        h = fnv1a64_update(h, b"|ep:");
        h = fnv1a64_update(h, EVENT_PAYLOAD_TAGS[i].screaming.as_bytes());
        h = fnv1a64_update(h, &[EVENT_PAYLOAD_TAGS[i].tag]);
        let sz = match EVENT_PAYLOAD_TAGS[i].fixed_payload_bytes {
            Some(n) => n as u64,
            None => u64::MAX,
        };
        h = fnv1a64_update(h, &sz.to_le_bytes());
        i += 1;
    }
    i = 0;
    while i < DRAW_OPCODES.len() {
        h = fnv1a64_update(h, b"|do:");
        h = fnv1a64_update(h, DRAW_OPCODES[i].screaming.as_bytes());
        h = fnv1a64_update(h, &[DRAW_OPCODES[i].opcode]);
        let sz = match DRAW_OPCODES[i].fixed_payload_bytes {
            Some(n) => n as u64,
            None => u64::MAX,
        };
        h = fnv1a64_update(h, &sz.to_le_bytes());
        i += 1;
    }
    h
}

/// Deterministic FNV-1a 64-bit schema hash of the wire codec spec.
pub const NVE_WIRE_CODECS_SCHEMA_HASH: u64 = wire_codecs_schema_hash();
