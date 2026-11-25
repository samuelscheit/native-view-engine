//! Single-source cross-language wire codec generator for:
//! - Canonical 64-bit FNV-1a (`0xcbf29ce484222325` / `0x00000100000001b3`)
//! - Binary `EventRing` (`NVEV` / count framing, `NativeEventKind`, `EventPayloadTag`, `EventMask`)
//! - Compact binary `NK1` keydown wire framing
//! - Binary `NVE_DRAW1` vector canvas opcodes, constants, and header decoders
//!
//! Emits 5 checked-in host artifacts following the repository naming invariants:
//! 1. `cxx/nve-fabric-bridge/include/nve_wire_codecs.generated.hpp`
//! 2. `hosts/swiftui-kit/Sources/NveSwiftUI/NveWireCodecs.generated.swift`
//! 3. `hosts/flutter-plugin/lib/src/nve_wire_codecs.generated.dart`
//! 4. `hosts/web-shell/nve_wire_codecs.generated.ts`
//! 5. `hosts/android-shell/android-app/src/dev/nve/android/generated/NveWireCodecs.java`

pub mod emit_cpp;
pub mod emit_dart;
pub mod emit_java;
pub mod emit_swift;
pub mod emit_ts;
pub mod spec;

pub use emit_cpp::generate_cpp_wire_codecs;
pub use emit_dart::generate_dart_wire_codecs;
pub use emit_java::generate_java_wire_codecs;
pub use emit_swift::generate_swift_wire_codecs;
pub use emit_ts::generate_ts_wire_codecs;
pub use spec::{
    fnv1a64, fnv1a64_pair, fnv1a64_update, wire_codecs_schema_hash, DrawOpcodeSpec,
    EventPayloadTagSpec, Fnv1a64TestVector, WireBitflag, WireEnumVariant, DRAW_LIST_ATTR_PREFIX,
    DRAW_LIST_HEADER_SIZE, DRAW_LIST_MAGIC, DRAW_LIST_MAX_CLIP_DEPTH, DRAW_LIST_MAX_OPS,
    DRAW_LIST_MAX_POLYLINE_POINTS, DRAW_LIST_OP_COUNT_OFFSET, DRAW_LIST_VERSION,
    DRAW_LIST_VERSION_OFFSET, DRAW_OPCODES, DRAW_OP_CLEAR, DRAW_OP_CLIP_RECT, DRAW_OP_FILL_CIRCLE,
    DRAW_OP_FILL_RECT, DRAW_OP_FILL_ROUNDED_RECT, DRAW_OP_POLYLINE_HEADER_BYTES,
    DRAW_OP_POLYLINE_POINT_STRIDE, DRAW_OP_RESTORE_CLIP, DRAW_OP_STROKE_LINE,
    DRAW_OP_STROKE_POLYLINE, EVENT_ENTRY_HEADER_SIZE, EVENT_ENTRY_KIND_OFFSET,
    EVENT_ENTRY_NODE_ID_OFFSET, EVENT_ENTRY_PAYLOAD_TAG_OFFSET, EVENT_ENTRY_SURFACE_ID_OFFSET,
    EVENT_ENTRY_TIMESTAMP_US_OFFSET, EVENT_KINDS, EVENT_MASKS, EVENT_PAYLOAD_TAGS,
    EVENT_RING_COUNT_HEADER_SIZE, EVENT_RING_FRAMED_HEADER_SIZE, EVENT_RING_MAGIC,
    EVENT_RING_MAGIC_BYTES, EVENT_RING_VERSION, FNV1A64_TEST_VECTORS, FNV1A_OFFSET_BASIS,
    FNV1A_PRIME, KEYDOWN_FLAG_ALT, KEYDOWN_FLAG_CTRL, KEYDOWN_FLAG_META, KEYDOWN_FLAG_SHIFT,
    KEYDOWN_HEADER_SIZE, KEYDOWN_MIN_LEN, KEYDOWN_WIRE_MAGIC, NVE_WIRE_CODECS_SCHEMA_HASH,
};

use crate::ir::GeneratedFile;

/// Generates all 5 checked-in cross-language wire codec artifacts keyed by workspace-relative path.
pub fn generate_all_wire_codec_files() -> Vec<GeneratedFile> {
    vec![
        GeneratedFile {
            relative_path: "cxx/nve-fabric-bridge/include/nve_wire_codecs.generated.hpp".to_owned(),
            content: generate_cpp_wire_codecs(),
        },
        GeneratedFile {
            relative_path: "hosts/swiftui-kit/Sources/NveSwiftUI/NveWireCodecs.generated.swift"
                .to_owned(),
            content: generate_swift_wire_codecs(),
        },
        GeneratedFile {
            relative_path: "hosts/flutter-plugin/lib/src/nve_wire_codecs.generated.dart".to_owned(),
            content: generate_dart_wire_codecs(),
        },
        GeneratedFile {
            relative_path: "hosts/web-shell/nve_wire_codecs.generated.ts".to_owned(),
            content: generate_ts_wire_codecs(),
        },
        GeneratedFile {
            relative_path:
                "hosts/android-shell/android-app/src/dev/nve/android/generated/NveWireCodecs.java"
                    .to_owned(),
            content: generate_java_wire_codecs(),
        },
    ]
}
