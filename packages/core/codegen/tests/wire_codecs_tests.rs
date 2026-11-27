//! Cross-language verification tests for single-source wire codecs (`NVEV` v2 64-byte `NveEvent`
//! batches + byte arena, typed `KeyDown`, and `NVE_DRAW1` vector canvas).

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use nve_abi::{
    EventBatchBuf, EventFlags, InteractionEvent, LayoutEvent, NodeId, ScrollEvent, SurfaceId,
    TextInputEvent, TimestampUs, NVE_EVENT_INLINE_KEY_CAP, NVE_EVENT_SIZE,
    NVE_EVENT_WIRE_HEADER_SIZE, NVE_EVENT_WIRE_MAGIC, NVE_EVENT_WIRE_VERSION, NVE_TABLE_CUSTOM,
    NVE_TABLE_INTERACTION, NVE_TABLE_LAYOUT, NVE_TABLE_SCROLL, NVE_TABLE_TEXT_INPUT,
};
use nve_codegen::schema::wire as wc;
use nve_codegen::targets::generate_all_wire_codec_files;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn temp_test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nve_wire_codecs_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn assert_decoded_cross_language_event_batch(raw: &[u8]) {
    let decoded = EventBatchBuf::decode_wire(raw)
        .expect("Rust EventBatchBuf::decode_wire must decode host NVEV v2 batch");
    let mut expected = EventBatchBuf::new();
    expected.push_press(SurfaceId::ROOT, NodeId::new(10, 0), TimestampUs(1000), 12.5, 34.25);
    expected.push_long_press(SurfaceId::ROOT, NodeId::new(11, 0), TimestampUs(2000), 5.0, 6.0);
    expected.push_scroll(
        SurfaceId::ROOT,
        NodeId::new(12, 0),
        TimestampUs(3000),
        1.0,
        2.0,
        300.0,
        900.0,
    );
    expected.push_text_change(SurfaceId::ROOT, NodeId::new(13, 0), TimestampUs(4000), "hello");
    expected.push_layout(
        SurfaceId::ROOT,
        NodeId::new(14, 0),
        TimestampUs(5000),
        0.0,
        8.0,
        320.0,
        48.0,
    );
    expected.push_focus(SurfaceId::ROOT, NodeId::new(15, 0), TimestampUs(6000));
    expected.push_key_down(
        SurfaceId::ROOT,
        NodeId::new(16, 0),
        TimestampUs(7000),
        "k",
        "",
        EventFlags::from_modifiers(true, false, true, false, false),
        0,
    );
    assert_eq!(decoded, expected);
}

#[test]
fn spec_matches_nve_abi_and_nve_canvas_authoritatively() {
    assert_eq!(wc::wire_codecs_schema_hash(), wc::NVE_WIRE_CODECS_SCHEMA_HASH);
    assert_ne!(wc::NVE_WIRE_CODECS_SCHEMA_HASH, 0);

    assert_eq!(wc::EVENT_WIRE_MAGIC_BYTES, NVE_EVENT_WIRE_MAGIC);
    assert_eq!(wc::EVENT_WIRE_MAGIC, u32::from_le_bytes(*b"NVEV"));
    assert_eq!(wc::EVENT_WIRE_VERSION, NVE_EVENT_WIRE_VERSION);
    assert_eq!(wc::EVENT_WIRE_HEADER_SIZE, NVE_EVENT_WIRE_HEADER_SIZE);
    assert_eq!(wc::EVENT_RECORD_SIZE, NVE_EVENT_SIZE);
    assert_eq!(wc::EVENT_INLINE_KEY_CAP, NVE_EVENT_INLINE_KEY_CAP);
    assert_eq!(wc::TABLE_KEY_INTERACTION, NVE_TABLE_INTERACTION.0);
    assert_eq!(wc::TABLE_KEY_TEXT_INPUT, NVE_TABLE_TEXT_INPUT.0);
    assert_eq!(wc::TABLE_KEY_SCROLL, NVE_TABLE_SCROLL.0);
    assert_eq!(wc::TABLE_KEY_LAYOUT, NVE_TABLE_LAYOUT.0);
    assert_eq!(wc::TABLE_KEY_CUSTOM, NVE_TABLE_CUSTOM.0);

    assert_eq!(wc::INTERACTION_EVENTS.len(), 12);
    assert_eq!(wc::INTERACTION_EVENTS[0].value, InteractionEvent::Press as u8);
    assert_eq!(wc::INTERACTION_EVENTS[11].value, InteractionEvent::Wheel as u8);
    assert_eq!(wc::TEXT_INPUT_EVENTS.len(), 5);
    assert_eq!(wc::TEXT_INPUT_EVENTS[0].value, TextInputEvent::Change as u8);
    assert_eq!(wc::TEXT_INPUT_EVENTS[4].value, TextInputEvent::CompositionEnd as u8);
    assert_eq!(wc::SCROLL_EVENTS.len(), 1);
    assert_eq!(wc::SCROLL_EVENTS[0].value, ScrollEvent::Scroll as u8);
    assert_eq!(wc::LAYOUT_EVENTS.len(), 1);
    assert_eq!(wc::LAYOUT_EVENTS[0].value, LayoutEvent::Layout as u8);

    // Cross-check binary layout of `EventBatchBuf::to_wire_bytes` against spec offsets.
    let mut single_buf = EventBatchBuf::new();
    single_buf.push_blur(
        SurfaceId::from_raw(0x1122_3344).unwrap(),
        NodeId::from_raw(0x5566_7788).unwrap(),
        TimestampUs(0x0102_0304_0506_0708),
    );
    let single = single_buf.to_wire_bytes();
    assert_eq!(single.len(), wc::EVENT_WIRE_HEADER_SIZE + wc::EVENT_RECORD_SIZE);
    let e = &single[wc::EVENT_WIRE_HEADER_SIZE..];
    assert_eq!(
        u64::from_le_bytes(e[wc::EVENT_OFFSET_TIMESTAMP..wc::EVENT_OFFSET_TIMESTAMP + 8].try_into().unwrap()),
        0x0102_0304_0506_0708
    );
    assert_eq!(
        u64::from_le_bytes(e[wc::EVENT_OFFSET_TABLE..wc::EVENT_OFFSET_TABLE + 8].try_into().unwrap()),
        NVE_TABLE_INTERACTION.0
    );
    assert_eq!(
        u32::from_le_bytes(e[wc::EVENT_OFFSET_SURFACE..wc::EVENT_OFFSET_SURFACE + 4].try_into().unwrap()),
        0x1122_3344
    );
    assert_eq!(
        u32::from_le_bytes(e[wc::EVENT_OFFSET_NODE..wc::EVENT_OFFSET_NODE + 4].try_into().unwrap()),
        0x5566_7788
    );
    assert_eq!(
        u32::from_le_bytes(e[wc::EVENT_OFFSET_EVENT..wc::EVENT_OFFSET_EVENT + 4].try_into().unwrap()),
        InteractionEvent::Blur as u32
    );

    // Cross-check `NVE_DRAW1` constants & opcodes directly against `packages/modules/canvas`
    let canvas_types =
        std::fs::read_to_string(repo_root().join("packages/modules/canvas/src/types.rs"))
            .expect("read packages/modules/canvas/src/types.rs");
    assert_eq!(&wc::DRAW_LIST_MAGIC, b"NVE_DRAW1");
    assert!(canvas_types.contains("pub const NVE_DRAW_MAGIC: [u8; 9] = *b\"NVE_DRAW1\";"));
    assert!(canvas_types.contains(&format!("pub const NVE_DRAW_VERSION: u16 = {};", wc::DRAW_LIST_VERSION)));
    assert!(canvas_types.contains(&format!("pub const NVE_DRAW_HEADER_SIZE: usize = {};", wc::DRAW_LIST_HEADER_SIZE)));
    assert_eq!(
        (wc::DRAW_LIST_MAX_OPS, wc::DRAW_LIST_MAX_POLYLINE_POINTS, wc::DRAW_LIST_MAX_CLIP_DEPTH),
        (65_536, 16_384, 64)
    );
    assert!(canvas_types.contains(&format!("pub const DRAW_LIST_ATTR_PREFIX: &str = \"{}\";", wc::DRAW_LIST_ATTR_PREFIX)));

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

fn generated_wire_file(suffix: &str) -> nve_codegen::GeneratedFile {
    generate_all_wire_codec_files()
        .into_iter()
        .find(|f| f.relative_path.ends_with(suffix))
        .expect("generated wire codec file should exist for suffix")
}

#[test]
fn wire_codec_generated_files_match_checked_in_artifacts_and_naming_invariants() {
    let root = repo_root();
    let files = generate_all_wire_codec_files();
    assert_eq!(files.len(), 5);
    assert_eq!(files, generate_all_wire_codec_files());

    for file in &files {
        for forbidden in ["serde_json", "JSONObject", "JSON.stringify", "jsonEncode"] {
            assert!(!file.content.contains(forbidden));
        }
    }
    common::assert_generated_file_naming_and_optional_disk_parity(&files, &root, false);
}

#[test]
fn cpp20_wire_codecs_compile_and_execute_against_rust_reference() {
    let tmp = temp_test_dir("cpp");
    common::write_generated_c_abi_headers(&tmp);
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
  EventBatchWriter w;
  w.push_press(1, 10, 1000, 12.5f, 34.25f);
  w.push_long_press(1, 11, 2000, 5.0f, 6.0f);
  w.push_scroll(1, 12, 3000, 1.0f, 2.0f, 300.0f, 900.0f);
  assert(w.push_text_change(1, 13, 4000, "hello"));
  w.push_layout(1, 14, 5000, 0.0f, 8.0f, 320.0f, 48.0f);
  w.push_focus(1, 15, 6000);
  assert(w.push_keydown(1, 16, 7000, "k", true, false, true, false));
  const auto bytes = w.finish();
  const auto hdr = decode_event_batch_header(bytes);
  assert(hdr.has_value() && hdr->version == kEventWireVersion && hdr->event_count == 7u && hdr->bytes_len == 5u);
  std::ofstream out("{}", std::ios::binary);
  out.write(reinterpret_cast<const char*>(bytes.data()), static_cast<std::streamsize>(bytes.size()));
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

    let compile = match Command::new("clang++")
        .args(["-std=c++20", "-Wall", "-Wextra", "-Werror", "-Wpedantic"])
        .arg("-I")
        .arg(&tmp)
        .arg("-I")
        .arg(&include_dir)
        .arg(&driver)
        .arg("-o")
        .arg(&out_bin)
        .output()
    {
        Ok(out) => out,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let _ = std::fs::remove_dir_all(&tmp);
            eprintln!("skipping C++20 wire codec test: clang++ not installed");
            return;
        }
        Err(e) => panic!("run clang++: {e}"),
    };
    assert!(compile.status.success(), "C++20 compile failed:\n{}", String::from_utf8_lossy(&compile.stderr));
    let run = Command::new(&out_bin).output().expect("run C++20 binary");
    assert!(run.status.success(), "C++20 run failed:\n{}", String::from_utf8_lossy(&run.stderr));
    assert_decoded_cross_language_event_batch(&std::fs::read(&event_bin).unwrap());
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn java11_wire_codecs_compile_and_execute_against_rust_reference() {
    let tmp = temp_test_dir("java");
    let classes_dir = tmp.join("classes");
    std::fs::create_dir_all(&classes_dir).unwrap();
    let event_bin = tmp.join("events.bin");
    let driver = tmp.join("WireCheckDriver.java");
    let java_dir = tmp.join("dev/nve/android/generated");
    std::fs::create_dir_all(&java_dir).unwrap();
    let java_src = java_dir.join("NveWireCodecs.java");
    std::fs::write(&java_src, generated_wire_file("NveWireCodecs.java").content).unwrap();

    std::fs::write(
        &driver,
        format!(
            r#"import dev.nve.android.generated.NveWireCodecs;
import java.nio.file.*;
public final class WireCheckDriver {{
    public static void main(String[] args) throws Exception {{
        var w = new NveWireCodecs.EventBatchWriter();
        w.pushPress(1, 10, 1000L, 12.5f, 34.25f);
        w.pushLongPress(1, 11, 2000L, 5.0f, 6.0f);
        w.pushScroll(1, 12, 3000L, 1.0f, 2.0f, 300.0f, 900.0f);
        w.pushTextChange(1, 13, 4000L, "hello");
        w.pushLayout(1, 14, 5000L, 0.0f, 8.0f, 320.0f, 48.0f);
        w.pushFocus(1, 15, 6000L);
        if (!w.pushKeyDown(1, 16, 7000L, "k", true, false, true, false)) throw new AssertionError("kd");
        byte[] finished = w.finish();
        var hdr = NveWireCodecs.decodeEventBatchHeader(finished);
        if (hdr == null || hdr.version != NveWireCodecs.EVENT_WIRE_VERSION || hdr.eventCount != 7L || hdr.bytesLen != 5L) throw new AssertionError("nvev");
        Files.write(Path.of("{}"), finished);
        byte[] draw = new byte[16];
        System.arraycopy(NveWireCodecs.DRAW_LIST_MAGIC, 0, draw, 0, 9);
        draw[9] = 1; draw[11] = 1; draw[15] = (byte) NveWireCodecs.DRAW_OP_RESTORE_CLIP;
        if (!NveWireCodecs.decodeDrawListHeader(draw).isOk()) throw new AssertionError("draw");
        if (NveWireCodecs.drawOpFixedPayloadBytes(NveWireCodecs.DRAW_OP_FILL_RECT) != 20) throw new AssertionError("rect");
        if (NveWireCodecs.drawOpFixedPayloadBytes(NveWireCodecs.DRAW_OP_STROKE_POLYLINE) != -1) throw new AssertionError("poly");
    }}
}}
"#,
            event_bin.to_string_lossy().replace('\\', "/")
        ),
    )
    .unwrap();

    let compile = match Command::new("javac")
        .args(["--release", "11", "-Xlint:all", "-Werror", "-d"])
        .arg(&classes_dir)
        .arg(&java_src)
        .arg(&driver)
        .output()
    {
        Ok(out) => out,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let _ = std::fs::remove_dir_all(&tmp);
            eprintln!("skipping Java 11 wire codec test: javac not installed");
            return;
        }
        Err(e) => panic!("run javac: {e}"),
    };
    assert!(compile.status.success(), "Java 11 compile failed:\n{}", String::from_utf8_lossy(&compile.stderr));
    let run = Command::new("java").arg("-cp").arg(&classes_dir).arg("WireCheckDriver").output().expect("run java");
    assert!(run.status.success(), "Java run failed:\n{}", String::from_utf8_lossy(&run.stderr));
    assert_decoded_cross_language_event_batch(&std::fs::read(&event_bin).unwrap());
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn swift_wire_codecs_compile_and_execute_against_rust_reference() {
    let tmp = temp_test_dir("swift");
    let out_bin = tmp.join("wire_check_swift");
    let event_bin = tmp.join("events.bin");
    let main_swift = tmp.join("main.swift");
    let swift_src = tmp.join("NveWireCodecs.generated.swift");
    std::fs::write(
        &swift_src,
        generated_wire_file("NveWireCodecs.generated.swift").content,
    )
    .unwrap();

    std::fs::write(
        &main_swift,
        format!(
            r#"import Foundation
var w = NveWireCodecs.EventBatchWriter()
w.pushPress(surfaceId: 1, nodeId: 10, timestampUs: 1000, x: 12.5, y: 34.25)
w.pushLongPress(surfaceId: 1, nodeId: 11, timestampUs: 2000, x: 5.0, y: 6.0)
w.pushScroll(surfaceId: 1, nodeId: 12, timestampUs: 3000, x: 1.0, y: 2.0, contentWidth: 300.0, contentHeight: 900.0)
w.pushTextChange(surfaceId: 1, nodeId: 13, timestampUs: 4000, text: "hello")
w.pushLayout(surfaceId: 1, nodeId: 14, timestampUs: 5000, x: 0.0, y: 8.0, width: 320.0, height: 48.0)
w.pushFocus(surfaceId: 1, nodeId: 15, timestampUs: 6000)
precondition(w.pushKeyDown(surfaceId: 1, nodeId: 16, timestampUs: 7000, key: "k", meta: true, shift: true))
let finished = w.finish()
let hdr = NveWireCodecs.decodeEventBatchHeader(finished)
precondition(hdr?.version == NveWireCodecs.EventWire.version && hdr?.eventCount == 7 && hdr?.bytesLen == 5)
try finished.write(to: URL(fileURLWithPath: "{}"))
var draw = Data(NveWireCodecs.DrawList.magic)
draw.append(contentsOf: [1, 0, 1, 0, 0, 0, NveWireCodecs.DrawOpcode.restoreClip.rawValue])
let dhdr = try NveWireCodecs.decodeDrawListHeader(draw)
precondition(dhdr.version == 1 && dhdr.opCount == 1)
precondition(NveWireCodecs.DrawOpcode.fillRect.fixedPayloadBytes == 20)
precondition(NveWireCodecs.DrawOpcode.strokePolyline.fixedPayloadBytes == nil)
"#,
            event_bin.to_string_lossy().replace('\\', "/")
        ),
    )
    .unwrap();

    let compile = match Command::new("swiftc")
        .arg("-warnings-as-errors")
        .arg(&swift_src)
        .arg(&main_swift)
        .arg("-o")
        .arg(&out_bin)
        .output()
    {
        Ok(out) => out,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let _ = std::fs::remove_dir_all(&tmp);
            eprintln!("skipping Swift wire codec test: swiftc not installed");
            return;
        }
        Err(e) => panic!("run swiftc: {e}"),
    };
    assert!(compile.status.success(), "Swift compile failed:\n{}", String::from_utf8_lossy(&compile.stderr));
    let run = Command::new(&out_bin).output().expect("run Swift binary");
    assert!(run.status.success(), "Swift run failed:\n{}", String::from_utf8_lossy(&run.stderr));
    assert_decoded_cross_language_event_batch(&std::fs::read(&event_bin).unwrap());
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn dart_wire_codecs_analyze_and_execute_against_rust_reference() {
    let tmp = temp_test_dir("dart");
    let event_bin = tmp.join("events.bin");
    let dart_copy = tmp.join("nve_wire_codecs.generated.dart");
    let driver = tmp.join("driver.dart");
    std::fs::write(
        &dart_copy,
        generated_wire_file("nve_wire_codecs.generated.dart").content,
    )
    .unwrap();

    std::fs::write(
        &driver,
        format!(
            r#"import 'dart:io';
import 'dart:typed_data';
import 'nve_wire_codecs.generated.dart';
void main() {{
  final w = NveWireEventBatchWriter();
  w.pushPress(1, 10, 1000, 12.5, 34.25);
  w.pushLongPress(1, 11, 2000, 5.0, 6.0);
  w.pushScroll(1, 12, 3000, 1.0, 2.0, 300.0, 900.0);
  w.pushTextChange(1, 13, 4000, 'hello');
  w.pushLayout(1, 14, 5000, 0.0, 8.0, 320.0, 48.0);
  w.pushFocus(1, 15, 6000);
  if (!w.pushKeyDown(1, 16, 7000, 'k', meta: true, shift: true)) throw StateError('kd');
  final finished = w.finish();
  final hdr = nveWireDecodeEventBatchHeader(finished);
  if (hdr == null || hdr.version != nveWireEventVersion || hdr.eventCount != 7 || hdr.bytesLen != 5) throw StateError('nvev');
  File('{}').writeAsBytesSync(finished);
  final draw = Uint8List(16);
  draw.setRange(0, 9, nveWireDrawListMagic);
  draw[9] = 1; draw[11] = 1; draw[15] = NveWireDrawOpcode.restoreClip;
  final dhdr = nveWireDecodeDrawListHeader(draw);
  if (!dhdr.isOk || dhdr.version != 1 || dhdr.opCount != 1) throw StateError('draw');
  if (NveWireDrawOpcode.fixedPayloadBytes(NveWireDrawOpcode.fillRect) != 20) throw StateError('rect');
  if (NveWireDrawOpcode.fixedPayloadBytes(NveWireDrawOpcode.strokePolyline) != null) throw StateError('poly');
}}
"#,
            event_bin.to_string_lossy().replace('\\', "/")
        ),
    )
    .unwrap();

    let analyze = match Command::new("dart").args(["analyze", "--fatal-infos"]).arg(&dart_copy).output() {
        Ok(out) => out,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let _ = std::fs::remove_dir_all(&tmp);
            eprintln!("skipping Dart wire codec test: dart not installed");
            return;
        }
        Err(e) => panic!("dart analyze: {e}"),
    };
    assert!(analyze.status.success(), "dart analyze failed:\n{}{}", String::from_utf8_lossy(&analyze.stdout), String::from_utf8_lossy(&analyze.stderr));
    let run = Command::new("dart").arg("run").arg(&driver).output().expect("dart run");
    assert!(run.status.success(), "Dart run failed:\n{}{}", String::from_utf8_lossy(&run.stdout), String::from_utf8_lossy(&run.stderr));
    assert_decoded_cross_language_event_batch(&std::fs::read(&event_bin).unwrap());
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn typescript_wire_codecs_execute_and_verify_against_rust_reference() {
    let tmp = temp_test_dir("ts");
    let event_bin = tmp.join("events.bin");
    let driver = tmp.join("driver.mts");
    let ts_src = tmp.join("nve_wire_codecs.generated.ts");
    std::fs::write(
        &ts_src,
        generated_wire_file("nve_wire_codecs.generated.ts").content,
    )
    .unwrap();

    let ts_import_url = {
        let s = ts_src.to_string_lossy().replace('\\', "/");
        if s.starts_with('/') {
            format!("file://{s}")
        } else {
            format!("file:///{s}")
        }
    };
    let event_bin_js = event_bin.to_string_lossy().replace('\\', "/");

    std::fs::write(
        &driver,
        format!(
            r#"import * as fs from 'node:fs';
import {{
  NveWireEventBatchWriter,
  NVE_EVENT_WIRE_VERSION, decodeEventBatchHeader, NVE_DRAW_LIST_MAGIC,
  NVE_DRAW_OP, decodeDrawListHeader, drawOpFixedPayloadBytes,
}} from '{}';
const w = new NveWireEventBatchWriter();
w.pushPress(1, 10, 1000n, 12.5, 34.25);
w.pushLongPress(1, 11, 2000n, 5.0, 6.0);
w.pushScroll(1, 12, 3000n, 1.0, 2.0, 300.0, 900.0);
w.pushTextChange(1, 13, 4000n, 'hello');
w.pushLayout(1, 14, 5000n, 0.0, 8.0, 320.0, 48.0);
w.pushFocus(1, 15, 6000n);
if (!w.pushKeyDown(1, 16, 7000n, 'k', true, false, true, false)) throw new Error('kd');
const finished = w.finish();
const hdr = decodeEventBatchHeader(finished);
if (!hdr || hdr.version !== NVE_EVENT_WIRE_VERSION || hdr.eventCount !== 7 || hdr.bytesLen !== 5) throw new Error('nvev');
fs.writeFileSync('{}', finished);
const draw = new Uint8Array(16);
draw.set(NVE_DRAW_LIST_MAGIC, 0);
draw[9] = 1; draw[11] = 1; draw[15] = NVE_DRAW_OP.RESTORE_CLIP;
const dhdr = decodeDrawListHeader(draw);
if (dhdr.status !== 'ok' || dhdr.version !== 1 || dhdr.opCount !== 1) throw new Error('draw');
if (drawOpFixedPayloadBytes(NVE_DRAW_OP.FILL_RECT) !== 20) throw new Error('rect');
if (drawOpFixedPayloadBytes(NVE_DRAW_OP.STROKE_POLYLINE) !== null) throw new Error('poly');
"#,
            ts_import_url,
            event_bin_js
        ),
    )
    .unwrap();

    let run = match Command::new("node").arg(&driver).output() {
        Ok(out) => out,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let _ = std::fs::remove_dir_all(&tmp);
            eprintln!("skipping TS wire codec test: node not installed");
            return;
        }
        Err(e) => panic!("run node: {e}"),
    };
    assert!(run.status.success(), "TS run failed:\n{}{}", String::from_utf8_lossy(&run.stdout), String::from_utf8_lossy(&run.stderr));
    assert_decoded_cross_language_event_batch(&std::fs::read(&event_bin).unwrap());
    let _ = std::fs::remove_dir_all(&tmp);
}
