//! Cross-language verification tests for single-source wire codecs (`EventRing`/`NVEV`,
//! `NK1` keydown, `NVE_DRAW1` vector canvas, and canonical 64-bit FNV-1a).

use std::path::{Path, PathBuf};
use std::process::Command;

use nve_abi::events::{
    KEYDOWN_FLAG_ALT, KEYDOWN_FLAG_CTRL, KEYDOWN_FLAG_META, KEYDOWN_FLAG_SHIFT,
};
use nve_abi::{
    decode_events, decode_keydown_bytes, encode_events, encode_keydown_bytes, DecodedKeyDown,
    EventMask, EventPayload, NativeEvent, NativeEventKind, KEYDOWN_WIRE_MAGIC,
};
use nve_codegen::hybrid::{fnv1a_64, FNV1A_64_OFFSET_BASIS};
use nve_codegen::wire_codecs as wc;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn temp_test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nve_wire_codecs_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn assert_decoded_cross_language_event_ring(raw: &[u8]) {
    let decoded = decode_events(raw).expect("Rust nve_abi::decode_events must decode host EventRing");
    let expected = vec![
        NativeEvent {
            surface_id: 1,
            node_id: 10,
            timestamp_us: 1000,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point { x: 12.5, y: 34.25 },
        },
        NativeEvent {
            surface_id: 1,
            node_id: 11,
            timestamp_us: 2000,
            kind: NativeEventKind::LongPress,
            payload: EventPayload::Point { x: 5.0, y: 6.0 },
        },
        NativeEvent {
            surface_id: 1,
            node_id: 12,
            timestamp_us: 3000,
            kind: NativeEventKind::Scroll,
            payload: EventPayload::Scroll {
                x: 1.0,
                y: 2.0,
                content_width: 300.0,
                content_height: 900.0,
            },
        },
        NativeEvent {
            surface_id: 1,
            node_id: 13,
            timestamp_us: 4000,
            kind: NativeEventKind::TextChange,
            payload: EventPayload::Text("hello".into()),
        },
        NativeEvent {
            surface_id: 1,
            node_id: 14,
            timestamp_us: 5000,
            kind: NativeEventKind::Layout,
            payload: EventPayload::Layout {
                x: 0.0,
                y: 8.0,
                width: 320.0,
                height: 48.0,
            },
        },
        NativeEvent {
            surface_id: 1,
            node_id: 15,
            timestamp_us: 6000,
            kind: NativeEventKind::Focus,
            payload: EventPayload::Empty,
        },
        NativeEvent {
            surface_id: 1,
            node_id: 16,
            timestamp_us: 7000,
            kind: NativeEventKind::Custom,
            payload: EventPayload::Custom {
                name: "keydown".into(),
                data: encode_keydown_bytes("k", true, false, true, false),
            },
        },
    ];
    assert_eq!(decoded, expected);
    if let EventPayload::Custom { data, .. } = &decoded[6].payload {
        assert_eq!(
            decode_keydown_bytes(data),
            Some(DecodedKeyDown {
                key: "k".into(),
                meta: true,
                ctrl: false,
                shift: true,
                alt: false,
            })
        );
    }
}

#[test]
fn spec_matches_nve_abi_and_nve_components_authoritatively() {
    // 1. Canonical 64-bit FNV-1a constants & reference vectors
    assert_eq!(wc::FNV1A_OFFSET_BASIS, 0xcbf2_9ce4_8422_2325);
    assert_eq!(wc::FNV1A_PRIME, 0x0000_0100_0000_01b3);
    assert_eq!(wc::FNV1A_OFFSET_BASIS, FNV1A_64_OFFSET_BASIS);
    assert_eq!(wc::fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
    assert_eq!(wc::fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
    assert_eq!(wc::fnv1a64(b"foobar"), 0x8594_4171_f739_67e8);
    assert_eq!(wc::fnv1a64_pair(b"foo", b"bar"), wc::fnv1a64(b"foobar"));
    assert_eq!(
        wc::fnv1a64_update(wc::fnv1a64_update(wc::FNV1A_OFFSET_BASIS, b"NVE_"), b"DRAW1"),
        wc::fnv1a64(b"NVE_DRAW1")
    );
    for vec in wc::FNV1A64_TEST_VECTORS {
        assert_eq!(wc::fnv1a64(vec.bytes), vec.expected);
        assert_eq!(fnv1a_64(vec.bytes), vec.expected);
    }
    assert_eq!(wc::wire_codecs_schema_hash(), wc::NVE_WIRE_CODECS_SCHEMA_HASH);
    assert_ne!(wc::NVE_WIRE_CODECS_SCHEMA_HASH, 0);

    // 2. EventRing framing + NativeEventKind + EventMask + NK1 against `packages/core/abi`
    assert_eq!(&wc::EVENT_RING_MAGIC_BYTES, b"NVEV");
    assert_eq!(wc::EVENT_RING_MAGIC, u32::from_le_bytes(*b"NVEV"));
    assert_eq!(wc::EVENT_RING_VERSION, 1);
    assert_eq!(wc::EVENT_RING_COUNT_HEADER_SIZE, 4);
    assert_eq!(wc::EVENT_RING_FRAMED_HEADER_SIZE, 10);
    assert_eq!(wc::EVENT_ENTRY_HEADER_SIZE, 18);
    assert_eq!(wc::EVENT_ENTRY_SURFACE_ID_OFFSET, 0);
    assert_eq!(wc::EVENT_ENTRY_NODE_ID_OFFSET, 4);
    assert_eq!(wc::EVENT_ENTRY_TIMESTAMP_US_OFFSET, 8);
    assert_eq!(wc::EVENT_ENTRY_KIND_OFFSET, 16);
    assert_eq!(wc::EVENT_ENTRY_PAYLOAD_TAG_OFFSET, 17);

    let expected_kinds = [
        ("PRESS", NativeEventKind::Press as u8),
        ("LONG_PRESS", NativeEventKind::LongPress as u8),
        ("SCROLL", NativeEventKind::Scroll as u8),
        ("TEXT_CHANGE", NativeEventKind::TextChange as u8),
        ("FOCUS", NativeEventKind::Focus as u8),
        ("BLUR", NativeEventKind::Blur as u8),
        ("LAYOUT", NativeEventKind::Layout as u8),
        ("CUSTOM", NativeEventKind::Custom as u8),
    ];
    assert_eq!(wc::EVENT_KINDS.len(), expected_kinds.len());
    for (spec_v, (name, abi_val)) in wc::EVENT_KINDS.iter().zip(expected_kinds.iter()) {
        assert_eq!((spec_v.screaming, spec_v.value), (*name, *abi_val));
    }

    let expected_masks = [
        ("NONE", EventMask::NONE),
        ("PRESS", EventMask::PRESS),
        ("LONG_PRESS", EventMask::LONG_PRESS),
        ("SCROLL", EventMask::SCROLL),
        ("TEXT_CHANGE", EventMask::TEXT_CHANGE),
        ("FOCUS", EventMask::FOCUS),
        ("BLUR", EventMask::BLUR),
        ("LAYOUT", EventMask::LAYOUT),
        ("CUSTOM", EventMask::CUSTOM),
    ];
    assert_eq!(wc::EVENT_MASKS.len(), expected_masks.len());
    for (spec_m, (name, abi_val)) in wc::EVENT_MASKS.iter().zip(expected_masks.iter()) {
        assert_eq!((spec_m.screaming, spec_m.value), (*name, *abi_val));
    }

    assert_eq!(wc::KEYDOWN_WIRE_MAGIC, KEYDOWN_WIRE_MAGIC);
    assert_eq!(wc::KEYDOWN_FLAG_META, KEYDOWN_FLAG_META);
    assert_eq!(wc::KEYDOWN_FLAG_CTRL, KEYDOWN_FLAG_CTRL);
    assert_eq!(wc::KEYDOWN_FLAG_SHIFT, KEYDOWN_FLAG_SHIFT);
    assert_eq!(wc::KEYDOWN_FLAG_ALT, KEYDOWN_FLAG_ALT);

    // Cross-check binary layout of `encode_events` against spec offsets & payload tags.
    let single = encode_events(&[NativeEvent {
        surface_id: 0x1122_3344,
        node_id: 0x5566_7788,
        timestamp_us: 0x0102_0304_0506_0708,
        kind: NativeEventKind::Blur,
        payload: EventPayload::Empty,
    }]);
    assert_eq!(single.len(), wc::EVENT_RING_COUNT_HEADER_SIZE + wc::EVENT_ENTRY_HEADER_SIZE);
    let e = &single[wc::EVENT_RING_COUNT_HEADER_SIZE..];
    assert_eq!(u32::from_le_bytes(e[0..4].try_into().unwrap()), 0x1122_3344);
    assert_eq!(u32::from_le_bytes(e[4..8].try_into().unwrap()), 0x5566_7788);
    assert_eq!(u64::from_le_bytes(e[8..16].try_into().unwrap()), 0x0102_0304_0506_0708);
    assert_eq!(e[wc::EVENT_ENTRY_KIND_OFFSET], NativeEventKind::Blur as u8);
    assert_eq!(e[wc::EVENT_ENTRY_PAYLOAD_TAG_OFFSET], wc::EVENT_PAYLOAD_TAGS[0].tag);

    // 3. Cross-check `NVE_DRAW1` constants & opcodes directly against `packages/core/components`
    let canvas_types =
        std::fs::read_to_string(repo_root().join("packages/core/components/src/canvas_ir/types.rs"))
            .expect("read packages/core/components/src/canvas_ir/types.rs");
    assert_eq!(&wc::DRAW_LIST_MAGIC, b"NVE_DRAW1");
    assert!(canvas_types.contains("pub const NVE_DRAW_MAGIC: [u8; 9] = *b\"NVE_DRAW1\";"));
    assert!(canvas_types.contains(&format!("pub const NVE_DRAW_VERSION: u16 = {};", wc::DRAW_LIST_VERSION)));
    assert!(canvas_types.contains(&format!("pub const NVE_DRAW_HEADER_SIZE: usize = {};", wc::DRAW_LIST_HEADER_SIZE)));
    assert!(canvas_types.contains("pub const MAX_DRAW_OPS: usize = 65_536;"));
    assert!(canvas_types.contains("pub const MAX_POLYLINE_POINTS: usize = 16_384;"));
    assert_eq!((wc::DRAW_LIST_MAX_OPS, wc::DRAW_LIST_MAX_POLYLINE_POINTS, wc::DRAW_LIST_MAX_CLIP_DEPTH), (65_536, 16_384, 64));
    assert!(canvas_types.contains(&format!("pub const DRAW_LIST_ATTR_PREFIX: &str = \"{}\";", wc::DRAW_LIST_ATTR_PREFIX)));
    assert_eq!((wc::DRAW_LIST_VERSION_OFFSET, wc::DRAW_LIST_OP_COUNT_OFFSET), (9, 11));

    let expected_ops = [
        ("Clear", wc::DRAW_OP_CLEAR, 1u8),
        ("FillRect", wc::DRAW_OP_FILL_RECT, 2u8),
        ("FillRoundedRect", wc::DRAW_OP_FILL_ROUNDED_RECT, 3u8),
        ("FillCircle", wc::DRAW_OP_FILL_CIRCLE, 4u8),
        ("StrokeLine", wc::DRAW_OP_STROKE_LINE, 5u8),
        ("StrokePolyline", wc::DRAW_OP_STROKE_POLYLINE, 6u8),
        ("ClipRect", wc::DRAW_OP_CLIP_RECT, 7u8),
        ("RestoreClip", wc::DRAW_OP_RESTORE_CLIP, 8u8),
    ];
    assert_eq!(wc::DRAW_OPCODES.len(), expected_ops.len());
    for (spec_op, (pascal, const_val, num)) in wc::DRAW_OPCODES.iter().zip(expected_ops.iter()) {
        assert_eq!((spec_op.pascal, spec_op.opcode, *const_val), (*pascal, *num, *num));
        assert!(canvas_types.contains(&format!("    {pascal} = {num},")));
    }
}

#[test]
fn wire_codec_generated_files_match_checked_in_artifacts_and_naming_invariants() {
    let root = repo_root();
    let files = wc::generate_all_wire_codec_files();
    assert_eq!(files.len(), 5);
    assert_eq!(files, wc::generate_all_wire_codec_files());

    for file in &files {
        let fname = Path::new(&file.relative_path).file_name().and_then(|n| n.to_str()).unwrap();
        if file.relative_path.ends_with(".java") {
            assert!(file.relative_path.contains("/generated/"));
            assert!(file.content.contains("package dev.nve.android.generated;"));
        } else {
            assert!(fname.contains(".generated."), "missing .generated. in {}", file.relative_path);
        }
        for forbidden in ["serde_json", "JSONObject", "JSON.stringify", "jsonEncode"] {
            assert!(!file.content.contains(forbidden));
        }
        let on_disk = std::fs::read_to_string(root.join(&file.relative_path))
            .unwrap_or_else(|e| panic!("missing {}: {e}", file.relative_path))
            .replace("\r\n", "\n");
        assert_eq!(on_disk, file.content, "wire codec drift in {}", file.relative_path);
    }
}

#[test]
fn cpp20_wire_codecs_compile_and_execute_against_rust_reference() {
    let tmp = temp_test_dir("cpp");
    let include_dir = repo_root().join("cxx/nve-fabric-bridge/include");
    let out_bin = tmp.join("wire_check");
    let event_bin = tmp.join("events.bin");
    let driver = tmp.join("driver.cpp");

    std::fs::write(
        &driver,
        format!(
            r#"#include "nve_draw_list.hpp"
#include "nve_wire_codecs.generated.hpp"
#include <cassert>
#include <fstream>
#include <vector>
int main() {{
  using namespace nve::wire;
  assert(fnv1a64_str("foobar") == 0x85944171F73967E8ULL);
  EventRingWriter w(false);
  w.push_press(1, 10, 1000, 12.5f, 34.25f);
  w.push_long_press(1, 11, 2000, 5.0f, 6.0f);
  w.push_scroll(1, 12, 3000, 1.0f, 2.0f, 300.0f, 900.0f);
  assert(w.push_text_change(1, 13, 4000, "hello"));
  w.push_layout(1, 14, 5000, 0.0f, 8.0f, 320.0f, 48.0f);
  w.push_empty(1, 15, 6000, EventKind::Focus);
  assert(w.push_keydown(1, 16, 7000, "k", true, false, true, false));
  const auto bytes = w.finish();
  std::ofstream out("{}", std::ios::binary);
  out.write(reinterpret_cast<const char*>(bytes.data()), static_cast<std::streamsize>(bytes.size()));
  EventRingWriter framed(true);
  framed.push_empty(1, 2, 3, EventKind::Blur);
  const auto hdr = decode_event_ring_framed_header(framed.finish());
  assert(hdr.has_value() && hdr->version == kEventRingVersion && hdr->count == 1u);
  std::vector<uint8_t> draw(kDrawListMagic.begin(), kDrawListMagic.end());
  append_le_u16(draw, kDrawListVersion);
  append_le_u32(draw, 1u);
  draw.push_back(static_cast<uint8_t>(DrawOpcode::RestoreClip));
  assert(decode_draw_list_header(draw).ok());
  assert(draw_op_fixed_payload_bytes(DrawOpcode::FillRect) == 20u);
  assert(!draw_op_fixed_payload_bytes(DrawOpcode::StrokePolyline).has_value());
  assert(decode_draw_list_header(std::vector<uint8_t>(15, 0u)).status == DrawListHeaderStatus::BadMagic);
  return 0;
}}
"#,
            event_bin.display()
        ),
    )
    .unwrap();

    let compile = Command::new("clang++")
        .args(["-std=c++20", "-Wall", "-Wextra", "-Werror", "-Wpedantic"])
        .arg("-I")
        .arg(&include_dir)
        .arg(&driver)
        .arg("-o")
        .arg(&out_bin)
        .output()
        .expect("run clang++");
    assert!(compile.status.success(), "C++20 compile failed:\n{}", String::from_utf8_lossy(&compile.stderr));
    let run = Command::new(&out_bin).output().expect("run C++20 binary");
    assert!(run.status.success(), "C++20 run failed:\n{}", String::from_utf8_lossy(&run.stderr));
    assert_decoded_cross_language_event_ring(&std::fs::read(&event_bin).unwrap());
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn java11_wire_codecs_compile_and_execute_against_rust_reference() {
    let tmp = temp_test_dir("java");
    let classes_dir = tmp.join("classes");
    std::fs::create_dir_all(&classes_dir).unwrap();
    let event_bin = tmp.join("events.bin");
    let driver = tmp.join("WireCheckDriver.java");
    let java_src = repo_root()
        .join("hosts/android-shell/android-app/src/dev/nve/android/generated/NveWireCodecs.java");

    std::fs::write(
        &driver,
        format!(
            r#"import dev.nve.android.generated.NveWireCodecs;
import java.nio.file.*;
public final class WireCheckDriver {{
    public static void main(String[] args) throws Exception {{
        if (!NveWireCodecs.verifyFnv1a64TestVectors()) throw new AssertionError("FNV-1a64 failed");
        var w = new NveWireCodecs.EventRingWriter(false);
        w.pushPress(1, 10, 1000L, 12.5f, 34.25f);
        w.pushLongPress(1, 11, 2000L, 5.0f, 6.0f);
        w.pushScroll(1, 12, 3000L, 1.0f, 2.0f, 300.0f, 900.0f);
        w.pushTextChange(1, 13, 4000L, "hello");
        w.pushLayout(1, 14, 5000L, 0.0f, 8.0f, 320.0f, 48.0f);
        w.pushEmpty(1, 15, 6000L, NveWireCodecs.EVENT_KIND_FOCUS);
        if (!w.pushKeyDown(1, 16, 7000L, "k", true, false, true, false)) throw new AssertionError("kd");
        Files.write(Path.of("{}"), w.finish());
        var framed = new NveWireCodecs.EventRingWriter(true);
        framed.pushEmpty(1, 2, 3L, NveWireCodecs.EVENT_KIND_BLUR);
        var fhdr = NveWireCodecs.decodeEventRingFramedHeader(framed.finish());
        if (fhdr == null || fhdr.version != NveWireCodecs.EVENT_RING_VERSION || fhdr.count != 1L) throw new AssertionError("nvev");
        byte[] draw = new byte[16];
        System.arraycopy(NveWireCodecs.DRAW_LIST_MAGIC, 0, draw, 0, 9);
        draw[9] = 1; draw[11] = 1; draw[15] = (byte) NveWireCodecs.DRAW_OP_RESTORE_CLIP;
        if (!NveWireCodecs.decodeDrawListHeader(draw).isOk()) throw new AssertionError("draw");
        if (NveWireCodecs.drawOpFixedPayloadBytes(NveWireCodecs.DRAW_OP_FILL_RECT) != 20) throw new AssertionError("rect");
        if (NveWireCodecs.drawOpFixedPayloadBytes(NveWireCodecs.DRAW_OP_STROKE_POLYLINE) != -1) throw new AssertionError("poly");
    }}
}}
"#,
            event_bin.display()
        ),
    )
    .unwrap();

    let compile = Command::new("javac")
        .args(["--release", "11", "-Xlint:all", "-Werror", "-d"])
        .arg(&classes_dir)
        .arg(&java_src)
        .arg(&driver)
        .output()
        .expect("run javac");
    assert!(compile.status.success(), "Java 11 compile failed:\n{}", String::from_utf8_lossy(&compile.stderr));
    let run = Command::new("java").arg("-cp").arg(&classes_dir).arg("WireCheckDriver").output().expect("run java");
    assert!(run.status.success(), "Java run failed:\n{}", String::from_utf8_lossy(&run.stderr));
    assert_decoded_cross_language_event_ring(&std::fs::read(&event_bin).unwrap());
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn swift_wire_codecs_compile_and_execute_against_rust_reference() {
    let tmp = temp_test_dir("swift");
    let out_bin = tmp.join("wire_check_swift");
    let event_bin = tmp.join("events.bin");
    let main_swift = tmp.join("main.swift");
    let swift_src = repo_root().join("hosts/swiftui-kit/Sources/NveSwiftUI/NveWireCodecs.generated.swift");

    std::fs::write(
        &main_swift,
        format!(
            r#"import Foundation
precondition(NveWireCodecs.Fnv1a64.verifyTestVectors())
var w = NveWireCodecs.EventRingWriter(framed: false)
w.pushPress(surfaceId: 1, nodeId: 10, timestampUs: 1000, x: 12.5, y: 34.25)
w.pushLongPress(surfaceId: 1, nodeId: 11, timestampUs: 2000, x: 5.0, y: 6.0)
w.pushScroll(surfaceId: 1, nodeId: 12, timestampUs: 3000, x: 1.0, y: 2.0, contentWidth: 300.0, contentHeight: 900.0)
w.pushTextChange(surfaceId: 1, nodeId: 13, timestampUs: 4000, text: "hello")
w.pushLayout(surfaceId: 1, nodeId: 14, timestampUs: 5000, x: 0.0, y: 8.0, width: 320.0, height: 48.0)
w.pushEmpty(surfaceId: 1, nodeId: 15, timestampUs: 6000, kind: .focus)
precondition(w.pushKeyDown(surfaceId: 1, nodeId: 16, timestampUs: 7000, key: "k", meta: true, shift: true))
try w.finish().write(to: URL(fileURLWithPath: "{}"))
var framed = NveWireCodecs.EventRingWriter(framed: true)
framed.pushEmpty(surfaceId: 1, nodeId: 2, timestampUs: 3, kind: .blur)
let fhdr = NveWireCodecs.decodeEventRingFramedHeader(framed.finish())
precondition(fhdr?.version == NveWireCodecs.EventRing.version && fhdr?.count == 1)
var draw = Data(NveWireCodecs.DrawList.magic)
draw.append(contentsOf: [1, 0, 1, 0, 0, 0, NveWireCodecs.DrawOpcode.restoreClip.rawValue])
let dhdr = try NveWireCodecs.decodeDrawListHeader(draw)
precondition(dhdr.version == 1 && dhdr.opCount == 1)
precondition(NveWireCodecs.DrawOpcode.fillRect.fixedPayloadBytes == 20)
precondition(NveWireCodecs.DrawOpcode.strokePolyline.fixedPayloadBytes == nil)
"#,
            event_bin.display()
        ),
    )
    .unwrap();

    let compile = Command::new("swiftc")
        .arg("-warnings-as-errors")
        .arg(&swift_src)
        .arg(&main_swift)
        .arg("-o")
        .arg(&out_bin)
        .output()
        .expect("run swiftc");
    assert!(compile.status.success(), "Swift compile failed:\n{}", String::from_utf8_lossy(&compile.stderr));
    let run = Command::new(&out_bin).output().expect("run Swift binary");
    assert!(run.status.success(), "Swift run failed:\n{}", String::from_utf8_lossy(&run.stderr));
    assert_decoded_cross_language_event_ring(&std::fs::read(&event_bin).unwrap());
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn dart_wire_codecs_analyze_and_execute_against_rust_reference() {
    let tmp = temp_test_dir("dart");
    let event_bin = tmp.join("events.bin");
    let dart_copy = tmp.join("nve_wire_codecs.generated.dart");
    let driver = tmp.join("driver.dart");
    std::fs::copy(repo_root().join("hosts/flutter-plugin/lib/src/nve_wire_codecs.generated.dart"), &dart_copy).unwrap();

    std::fs::write(
        &driver,
        format!(
            r#"import 'dart:io';
import 'dart:typed_data';
import 'nve_wire_codecs.generated.dart';
void main() {{
  if (!nveWireVerifyFnv1a64TestVectors()) throw StateError('fnv1a64');
  final w = NveWireEventRingWriter(framed: false);
  w.pushPress(1, 10, 1000, 12.5, 34.25);
  w.pushLongPress(1, 11, 2000, 5.0, 6.0);
  w.pushScroll(1, 12, 3000, 1.0, 2.0, 300.0, 900.0);
  w.pushTextChange(1, 13, 4000, 'hello');
  w.pushLayout(1, 14, 5000, 0.0, 8.0, 320.0, 48.0);
  w.pushEmpty(1, 15, 6000, NveWireEventKind.focus);
  if (!w.pushKeyDown(1, 16, 7000, 'k', meta: true, shift: true)) throw StateError('kd');
  File('{}').writeAsBytesSync(w.finish());
  final framed = NveWireEventRingWriter(framed: true);
  framed.pushEmpty(1, 2, 3, NveWireEventKind.blur);
  final fhdr = nveWireDecodeEventRingFramedHeader(framed.finish());
  if (fhdr == null || fhdr.version != nveWireEventRingVersion || fhdr.count != 1) throw StateError('nvev');
  final draw = Uint8List(16);
  draw.setRange(0, 9, nveWireDrawListMagic);
  draw[9] = 1; draw[11] = 1; draw[15] = NveWireDrawOpcode.restoreClip;
  final dhdr = nveWireDecodeDrawListHeader(draw);
  if (!dhdr.isOk || dhdr.version != 1 || dhdr.opCount != 1) throw StateError('draw');
  if (NveWireDrawOpcode.fixedPayloadBytes(NveWireDrawOpcode.fillRect) != 20) throw StateError('rect');
  if (NveWireDrawOpcode.fixedPayloadBytes(NveWireDrawOpcode.strokePolyline) != null) throw StateError('poly');
}}
"#,
            event_bin.display()
        ),
    )
    .unwrap();

    let analyze = Command::new("dart").args(["analyze", "--fatal-infos"]).arg(&dart_copy).output().expect("dart analyze");
    assert!(analyze.status.success(), "dart analyze failed:\n{}{}", String::from_utf8_lossy(&analyze.stdout), String::from_utf8_lossy(&analyze.stderr));
    let run = Command::new("dart").arg("run").arg(&driver).output().expect("dart run");
    assert!(run.status.success(), "Dart run failed:\n{}{}", String::from_utf8_lossy(&run.stdout), String::from_utf8_lossy(&run.stderr));
    assert_decoded_cross_language_event_ring(&std::fs::read(&event_bin).unwrap());
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn typescript_wire_codecs_execute_and_verify_against_rust_reference() {
    let tmp = temp_test_dir("ts");
    let event_bin = tmp.join("events.bin");
    let driver = tmp.join("driver.mts");
    let ts_src = repo_root().join("hosts/web-shell/nve_wire_codecs.generated.ts");

    std::fs::write(
        &driver,
        format!(
            r#"import * as fs from 'node:fs';
import {{
  verifyFnv1a64TestVectors, NveWireEventRingWriter, NVE_WIRE_EVENT_KIND,
  NVE_EVENT_RING_VERSION, decodeEventRingFramedHeader, NVE_DRAW_LIST_MAGIC,
  NVE_DRAW_OP, decodeDrawListHeader, drawOpFixedPayloadBytes,
}} from '{}';
if (!verifyFnv1a64TestVectors()) throw new Error('fnv1a64');
const w = new NveWireEventRingWriter(false);
w.pushPress(1, 10, 1000n, 12.5, 34.25);
w.pushLongPress(1, 11, 2000n, 5.0, 6.0);
w.pushScroll(1, 12, 3000n, 1.0, 2.0, 300.0, 900.0);
w.pushTextChange(1, 13, 4000n, 'hello');
w.pushLayout(1, 14, 5000n, 0.0, 8.0, 320.0, 48.0);
w.pushEmpty(1, 15, 6000n, NVE_WIRE_EVENT_KIND.FOCUS);
if (!w.pushKeyDown(1, 16, 7000n, 'k', true, false, true, false)) throw new Error('kd');
fs.writeFileSync('{}', w.finish());
const framed = new NveWireEventRingWriter(true);
framed.pushEmpty(1, 2, 3n, NVE_WIRE_EVENT_KIND.BLUR);
const fhdr = decodeEventRingFramedHeader(framed.finish());
if (!fhdr || fhdr.version !== NVE_EVENT_RING_VERSION || fhdr.count !== 1) throw new Error('nvev');
const draw = new Uint8Array(16);
draw.set(NVE_DRAW_LIST_MAGIC, 0);
draw[9] = 1; draw[11] = 1; draw[15] = NVE_DRAW_OP.RESTORE_CLIP;
const dhdr = decodeDrawListHeader(draw);
if (dhdr.status !== 'ok' || dhdr.version !== 1 || dhdr.opCount !== 1) throw new Error('draw');
if (drawOpFixedPayloadBytes(NVE_DRAW_OP.FILL_RECT) !== 20) throw new Error('rect');
if (drawOpFixedPayloadBytes(NVE_DRAW_OP.STROKE_POLYLINE) !== null) throw new Error('poly');
"#,
            ts_src.display(),
            event_bin.display()
        ),
    )
    .unwrap();

    let run = Command::new("node").arg(&driver).output().expect("run node");
    assert!(run.status.success(), "TS run failed:\n{}{}", String::from_utf8_lossy(&run.stdout), String::from_utf8_lossy(&run.stderr));
    assert_decoded_cross_language_event_ring(&std::fs::read(&event_bin).unwrap());
    let _ = std::fs::remove_dir_all(&tmp);
}
