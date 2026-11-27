//! Real `tsc` + Node runtime verification tests for TypeScript Hybrid FFI codegen
//! (`targets/typescript/hybrid`) and Web Component codegen (`targets/typescript/web_component`)
//! backed by the static `@nve/host` runtime (`packages/core/host-ts`).

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::specs::counter_card_export_spec;
use nve_codegen::{
    DefaultValue, EventBubbling, EventSpec, ExportBackend, HybridTarget, NveExportSpec,
    ObjectField, PropSpec, PropType, export_web_component, generate_all_abi_files,
    generate_hybrid_target, parse_hybrid_rust_spec,
};

const REALISTIC_HYBRID_SPEC: &str = include_str!("fixtures/realistic_hybrid.nve.rs");

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .expect("workspace root 3 levels above CARGO_MANIFEST_DIR")
        .to_path_buf()
}

fn temp_test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "nve_hybrid_ts_runtime_test_{}_{}",
        tag,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp test dir");
    dir
}

/// Materializes a self-contained offline `@nve/host` package under `dir/host_ts_pkg` (for Node's
/// native TypeScript type-stripping, which refuses `.ts` paths under `node_modules/`) AND under
/// `dir/node_modules/@nve/host` (for `tsc` `NodeNext` module resolution), plus a tiny ESM resolve
/// hook (`dir/resolve_host.mjs`) mapping `'@nve/host'` to `dir/host_ts_pkg/src/index.ts`.
fn prepare_offline_host_ts_pkg(dir: &Path) -> PathBuf {
    let src = workspace_root().join("packages/core/host-ts");
    let abi_files = generate_all_abi_files(nve_components::tables::CORE_TABLE_DESCS)
        .expect("generate host ABI files");

    for dst in [dir.join("node_modules/@nve/host"), dir.join("host_ts_pkg")] {
        common::copy_tree(&src.join("src"), &dst.join("src"));
        std::fs::copy(src.join("package.json"), dst.join("package.json"))
            .expect("copy host-ts package.json");
        for file in &abi_files {
            if let Some(rel) = file.relative_path.strip_prefix("packages/core/host-ts/") {
                let out = dst.join(rel);
                if let Some(parent) = out.parent() {
                    std::fs::create_dir_all(parent).expect("create host-ts generated dir");
                }
                std::fs::write(out, &file.content).expect("write host-ts generated file");
            }
        }
    }

    let hook = dir.join("resolve_host.mjs");
    std::fs::write(
        &hook,
        r#"import { registerHooks } from 'node:module';
const target = new URL('./host_ts_pkg/src/index.ts', import.meta.url).href;
registerHooks({
  resolve(specifier, context, nextResolve) {
    if (specifier === '@nve/host') return { url: target, shortCircuit: true };
    return nextResolve(specifier, context);
  },
});
"#,
    )
    .expect("write resolve_host.mjs");
    hook
}

fn run_tsc_no_emit(tsconfig: &Path) {
    let mut cmd = if common::tool_available("tsc") {
        let mut c = Command::new("tsc");
        c.args(["--noEmit", "-p"]).arg(tsconfig);
        c
    } else if common::require_tool("npx") {
        let mut c = Command::new("npx");
        c.args(["-y", "-p", "typescript@5.9", "tsc", "--noEmit", "-p"])
            .arg(tsconfig);
        c
    } else {
        return;
    };
    let out = cmd.output().expect("run tsc --noEmit");
    assert!(
        out.status.success(),
        "tsc --noEmit failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn generated_ts_hybrid_reuses_host_ts_with_erasable_syntax_and_zero_duplication() {
    let spec = parse_hybrid_rust_spec(REALISTIC_HYBRID_SPEC).expect("parse realistic spec");
    let ts_out = generate_hybrid_target(&spec, HybridTarget::TypeScript).expect("generate TS");
    assert_eq!(ts_out.files.len(), 1);
    let src = &ts_out.files[0].content;

    assert!(src.contains("from '@nve/host';"));
    assert!(src.contains("new Key64("));
    assert!(src.contains("DynamicValue"));
    assert!(src.contains("wasmCallBin"));
    assert!(src.contains("wasmSetPropsBin"));
    assert!(src.contains("wasmCreateHandle"));

    // Must never duplicate static runtime codecs or emit non-erasable TypeScript syntax.
    for banned in [
        "class NveBinWriter",
        "class NveBinReader",
        "interface NveHybridDynamicValue",
        "interface NveHybridWasmBaseExports",
        "export enum ",
        "constructor(private ",
        "JSON.parse",
        "JSON.stringify",
        "getBigInt64",
        "getBigUint64",
    ] {
        assert!(
            !src.contains(banned),
            "generated TS hybrid must not contain `{banned}`"
        );
    }

    // `HybridType::Value` must write raw NveBinValue tags via `.writeTo(...)`, never `writeBytes(...)`.
    assert!(src.contains("props.metadata.writeTo(_nve_w);"));
    assert!(src.contains("meta.writeTo(_nve_w);"));
    assert!(!src.contains("writeBytes(props.metadata"));
    assert!(!src.contains("writeBytes(meta"));
}

#[test]
fn generated_web_component_reuses_host_ts_and_preserves_64_bit_defaults() {
    let mut spec = counter_card_export_spec(ExportBackend::Skia);
    spec.props.push(PropSpec {
        name: "hugeMax".to_owned(),
        prop_type: PropType::Double,
        optional: true,
        nullable: false,
        default_value: Some(DefaultValue::Int(i64::MAX)),
        doc: None,
    });
    spec.props.push(PropSpec {
        name: "hugeMin".to_owned(),
        prop_type: PropType::Double,
        optional: true,
        nullable: false,
        default_value: Some(DefaultValue::Int(i64::MIN)),
        doc: None,
    });

    let out = export_web_component(&spec).expect("export_web_component");
    let js = &out.element_js.content;

    assert!(js.contains("decodeJsValue as decodeNveBinValue"));
    assert!(js.contains("encodeJsValue as encodeNveBinValue"));
    assert!(js.contains("from '@nve/host';"));
    assert!(js.contains("'hugeMax': 9223372036854775807n,"));
    assert!(js.contains("'hugeMin': -9223372036854775808n,"));

    for banned in [
        "function _encodeNveBinValue",
        "function encodeNveBinValue",
        "function decodeNveBinValue",
        "getBigInt64",
        "Number(dv.getBigInt64",
    ] {
        assert!(
            !js.contains(banned),
            "generated web component JS must not contain `{banned}`"
        );
    }
}

#[test]
fn real_tsc_and_node_verify_generated_ts_hybrid_runtime_end_to_end() {
    if !common::require_tool("node") {
        return;
    }

    let dir = temp_test_dir("hybrid_e2e");
    let hook = prepare_offline_host_ts_pkg(&dir);
    std::fs::write(dir.join("package.json"), r#"{"type": "module"}"#).expect("write package.json");

    // 1. Generate realistic spec + edge-case specs (enum-only, struct-only, empty-props view, keyword params).
    let realistic = parse_hybrid_rust_spec(REALISTIC_HYBRID_SPEC).expect("parse realistic spec");
    generate_hybrid_target(&realistic, HybridTarget::TypeScript)
        .expect("generate realistic TS")
        .write_to_dir(&dir)
        .expect("write realistic TS");

    let edge_specs = [
        r#"
#[nve_hybrid_enum]
#[repr(u32)]
pub enum OnlyEnum { Alpha = 0, Beta = 7 }
"#,
        r#"
#[nve_hybrid_struct]
pub struct OnlyStruct {
    pub flag: bool,
    pub tiny: u8,
    pub small: u16,
    pub big_u: u64,
    pub big_i: i64,
    pub payload: nve_abi::NveValue,
}
"#,
        r#"
#[nve_hybrid_view(name = "EmptyView")]
pub trait EmptyViewSpec {
    #[method]
    fn ping(&self, default: i32, class: bool, in_val: u32) -> Result<(), String>;
}
"#,
    ];
    for src in edge_specs {
        let parsed = parse_hybrid_rust_spec(src).expect("parse edge spec");
        generate_hybrid_target(&parsed, HybridTarget::TypeScript)
            .expect("generate edge TS")
            .write_to_dir(&dir)
            .expect("write edge TS");
    }

    // 2. Write an end-to-end TypeScript driver exercising every codec path, WASM memory grow,
    //    `HybridType::Value` raw tag preservation in both methods and `setProps`, u64 unsignedness,
    //    nested `Vec<Vec<i32>>` (`Cursor`), error propagation, and zero WASM memory leaks.
    let driver = dir.join("e2e_driver.ts");
    std::fs::write(
        &driver,
        r#"
import assert from 'node:assert/strict';
import {
  BinCodecError,
  BinReader,
  BinTag,
  BinWriter,
  DynamicValue,
  HybridError,
  HybridStatus,
  encodeJsValue,
} from '@nve/host';
import {
  CryptoHybridClient,
  HashAlgorithm,
  VideoQuality,
  VideoSurfaceHybridClient,
  decodeCursor,
  decodeFramePoint,
  decodeHashAlgorithm,
  decodeSignedPayload,
  encodeCursor,
  encodeFramePoint,
  encodeSignedPayload,
} from './crypto_hybrid.generated.ts';
import type {
  CryptoWasmExports,
  Cursor,
  SignedPayload,
  VideoSurfaceWasmExports,
} from './crypto_hybrid.generated.ts';
import { OnlyEnum, decodeOnlyEnum } from './only_enum_hybrid.generated.ts';
import { decodeOnlyStruct, encodeOnlyStruct } from './only_struct_hybrid.generated.ts';
import { EmptyViewHybridClient } from './empty_view_hybrid.generated.ts';

// --- Direct struct & enum codec checks ---
{
  const w = new BinWriter();
  w.writeI64(BigInt(OnlyEnum.Beta));
  assert.equal(decodeOnlyEnum(new BinReader(w.bytes)), OnlyEnum.Beta);

  const badEnum = new BinWriter();
  badEnum.writeI64(99n);
  assert.throws(
    () => decodeHashAlgorithm(new BinReader(badEnum.bytes)),
    (e: unknown) => e instanceof HybridError && e.status === HybridStatus.InvalidArg,
  );
}

// u64::MAX and i64::MIN round-trip losslessly (u64 stays unsigned bigint, never -1n) + DynamicValue tag check
{
  const dyn = DynamicValue.of(encodeJsValue({ k: [1, '🚀', null], big: 2n ** 60n }));
  const w = new BinWriter();
  encodeOnlyStruct(w, {
    flag: true,
    tiny: 255,
    small: 65535,
    bigU: 0xffff_ffff_ffff_ffffn,
    bigI: -(2n ** 63n),
    payload: dyn,
  });
  // Inspect wire bytes directly: `payload` field must have BinTag.Map (7), NOT BinTag.Bytes (5).
  const inspect = new BinReader(w.bytes);
  let sawPayloadTag: BinTag | null = null;
  inspect.readMap((count) => {
    for (let i = 0; i < count; i += 1) {
      const k = inspect.readMapKey();
      if (k === 'payload') sawPayloadTag = inspect.peekTag();
      inspect.skipValue();
    }
  });
  assert.equal(sawPayloadTag, BinTag.Map, 'HybridType::Value struct field must preserve true NveValue tag');

  const decoded = decodeOnlyStruct(new BinReader(w.bytes));
  assert.equal(decoded.bigU, 0xffff_ffff_ffff_ffffn);
  assert.equal(decoded.bigI, -(2n ** 63n));
  assert.ok(decoded.payload.equals(dyn));

  // Missing required field must throw BinCodecError('truncated')
  const partial = new BinWriter();
  const pm = partial.beginMap(1);
  partial.writeMapKey('x');
  partial.writeF64(1.0);
  partial.end(pm);
  assert.throws(
    () => decodeFramePoint(new BinReader(partial.bytes)),
    (e: unknown) => e instanceof BinCodecError && e.kind === 'truncated',
  );
}

// Cursor: field names `w`, `r`, `mark`, `count`, `tag`, and nested `Vec<Vec<i32>>` (zero shadowing/TDZ)
{
  const cursor: Cursor = {
    mark: 1,
    w: Uint8Array.of(10, 20, 30),
    r: 'hello 🚀',
    count: 42,
    tag: -7,
    grid: [
      [1, 2, 3],
      [4, 5, 6],
    ],
  };
  const bw = new BinWriter();
  encodeCursor(bw, cursor);
  const br = new BinReader(bw.bytes);
  const out = decodeCursor(br);
  br.requireEnd();
  assert.deepEqual(out, cursor);
}

// --- Full mock WASM linear-memory round-trip with leak tracking and memory.grow() ---
const memory = new WebAssembly.Memory({ initial: 1, maximum: 8 });
let bump = 32;
const live = new Map<number, number>();
let growCounter = 0;

function wasmAlloc(len: number): number {
  const ptr = bump;
  bump += (len + 7) & ~7;
  live.set(ptr, len);
  return ptr;
}

function wasmFree(ptr: number, len: number): void {
  assert.equal(live.get(ptr), len, `WASM free mismatch at ptr=${ptr}`);
  live.delete(ptr);
}

function readWasmReq(ptr: number, len: number): Uint8Array {
  return new Uint8Array(memory.buffer, ptr, len).slice();
}

function writeWasmResp(outLenPtr: number, bytes: Uint8Array, growFirst = false): number {
  if (growFirst) {
    memory.grow(1);
    growCounter += 1;
  }
  const ptr = wasmAlloc(bytes.byteLength);
  new Uint8Array(memory.buffer, ptr, bytes.byteLength).set(bytes);
  new DataView(memory.buffer).setUint32(outLenPtr, bytes.byteLength, true);
  return ptr;
}

const cryptoWasm: CryptoWasmExports = {
  memory,
  nve_hybrid_alloc: wasmAlloc,
  nve_hybrid_free: wasmFree,
  // Return signed i64 just like WASM JS-BigInt integration does when bit 63 may be set.
  nve_hybrid_crypto_schema_hash: () => CryptoHybridClient.SCHEMA_HASH.toSignedBigInt(),
  nve_hybrid_crypto_create: () => 11,
  nve_hybrid_crypto_destroy: (h) => assert.equal(h, 11),
  nve_hybrid_crypto_version_bin: (_h, reqPtr, reqLen, outLenPtr) => {
    const r = new BinReader(readWasmReq(reqPtr, reqLen));
    r.readArray((n) => assert.equal(n, 0));
    r.requireEnd();
    const w = new BinWriter();
    w.writeStr('1.0.0-ts');
    return writeWasmResp(outLenPtr, w.bytes, true);
  },
  nve_hybrid_crypto_add_bin: (_h, reqPtr, reqLen, outLenPtr) => {
    const r = new BinReader(readWasmReq(reqPtr, reqLen));
    const sum = r.readArray((n) => {
      assert.equal(n, 2);
      return r.readI64() + r.readI64();
    });
    r.requireEnd();
    const w = new BinWriter();
    w.writeI64(sum);
    return writeWasmResp(outLenPtr, w.bytes);
  },
  nve_hybrid_crypto_blake3_bin: (_h, reqPtr, reqLen, outLenPtr) => {
    const r = new BinReader(readWasmReq(reqPtr, reqLen));
    const data = r.readArray(() => r.readBytes().slice());
    const w = new BinWriter();
    w.writeBytes(data.map((b) => b ^ 0x5a));
    return writeWasmResp(outLenPtr, w.bytes);
  },
  nve_hybrid_crypto_hash_with_bin: (_h, reqPtr, reqLen, outLenPtr) => {
    const r = new BinReader(readWasmReq(reqPtr, reqLen));
    const [algo, data] = r.readArray(() => [decodeHashAlgorithm(r), r.readBytes().slice()] as const);
    const w = new BinWriter();
    const m = w.beginMap(4);
    w.writeMapKey('algorithm');
    w.writeI64(BigInt(algo));
    w.writeMapKey('byte_len');
    w.writeI64(BigInt(data.length));
    w.writeMapKey('verified');
    w.writeBool(true);
    w.writeMapKey('anchor');
    encodeFramePoint(w, { x: 1.5, y: -2.5 });
    w.end(m);
    return writeWasmResp(outLenPtr, w.bytes);
  },
  nve_hybrid_crypto_verify_payload_bin: (_h, reqPtr, reqLen, outLenPtr) => {
    const r = new BinReader(readWasmReq(reqPtr, reqLen));
    const payload = r.readArray(() => decodeSignedPayload(r));
    const w = new BinWriter();
    if (payload.label === '') {
      w.writeBool(false);
      w.writeStr('missing payload label');
    } else {
      w.writeBool(true);
      w.writeBool(payload.nonce === 0xffff_ffff_ffff_fffen);
    }
    return writeWasmResp(outLenPtr, w.bytes);
  },
  nve_hybrid_crypto_find_nonce_bin: (_h, reqPtr, reqLen, outLenPtr) => {
    const r = new BinReader(readWasmReq(reqPtr, reqLen));
    const [label, hint] = r.readArray(() => {
      const l = r.readStr();
      const h = r.readNull() ? null : r.readI64Bits().toBigInt();
      return [l, h] as const;
    });
    const w = new BinWriter();
    if (hint === null) w.writeNull();
    else w.writeI64(hint + BigInt(label.length));
    return writeWasmResp(outLenPtr, w.bytes);
  },
  nve_hybrid_crypto_lookup_payload_bin: (_h, reqPtr, reqLen, outLenPtr) => {
    const r = new BinReader(readWasmReq(reqPtr, reqLen));
    const id = r.readArray(() => Number(r.readI64()));
    const w = new BinWriter();
    if (id === 0) {
      w.writeNull();
    } else {
      encodeSignedPayload(w, {
        header: { algorithm: HashAlgorithm.Blake3, byteLen: id, verified: true, anchor: { x: 1, y: 2 } },
        label: `rec-${id}`,
        digest: Uint8Array.of(0xde, 0xad),
        tags: ['a', 'b'],
        nonce: 42n,
      });
    }
    return writeWasmResp(outLenPtr, w.bytes);
  },
  nve_hybrid_crypto_batch_sign_bin: (_h, reqPtr, reqLen, outLenPtr) => {
    const r = new BinReader(readWasmReq(reqPtr, reqLen));
    const [inputs, algo] = r.readArray(() => {
      const arr = r.readArray((n) => {
        const out: string[] = [];
        for (let i = 0; i < n; i += 1) out.push(r.readStr());
        return out;
      });
      return [arr, decodeHashAlgorithm(r)] as const;
    });
    const w = new BinWriter();
    w.writeBool(true);
    const am = w.beginArray(inputs.length);
    for (const label of inputs) {
      encodeSignedPayload(w, {
        header: { algorithm: algo, byteLen: label.length, verified: true, anchor: { x: 0, y: 0 } },
        label,
        digest: Uint8Array.of(1),
        tags: [],
        nonce: null,
      });
    }
    w.end(am);
    return writeWasmResp(outLenPtr, w.bytes);
  },
  nve_hybrid_crypto_inspect_dynamic_bin: (_h, reqPtr, reqLen, outLenPtr) => {
    const r = new BinReader(readWasmReq(reqPtr, reqLen));
    const raw = r.readArray((n) => {
      assert.equal(n, 1);
      // CRITICAL AUDIT: `meta` must arrive as its true NveValue tag (Map=7), NOT Bytes(5)!
      assert.equal(r.peekTag(), BinTag.Map, 'inspectDynamic param must not be double-wrapped in Bytes');
      return r.readCheckedValue().slice();
    });
    r.requireEnd();
    return writeWasmResp(outLenPtr, raw);
  },
  nve_hybrid_crypto_transpose_bin: (_h, reqPtr, reqLen, outLenPtr) => {
    const r = new BinReader(readWasmReq(reqPtr, reqLen));
    const c = r.readArray(() => decodeCursor(r));
    const cols = c.grid[0]?.length ?? 0;
    const grid = Array.from({ length: cols }, (_, col) => c.grid.map((row) => row[col]!));
    const w = new BinWriter();
    encodeCursor(w, {
      mark: c.mark + 1,
      w: c.w.slice().reverse(),
      r: c.r === null ? null : c.r.toUpperCase(),
      count: c.count * 2,
      tag: c.tag === null ? null : -c.tag,
      grid,
    });
    return writeWasmResp(outLenPtr, w.bytes);
  },
  nve_hybrid_crypto_reset_cache_bin: (_h, _reqPtr, _reqLen, outLenPtr) => {
    const w = new BinWriter();
    w.writeBool(true);
    return writeWasmResp(outLenPtr, w.bytes);
  },
};

const crypto = new CryptoHybridClient(cryptoWasm);
assert.equal(crypto.version(), '1.0.0-ts');
assert.equal(growCounter, 1, 'memory.grow happened cleanly inside WASM call');
assert.equal(crypto.add(2n ** 53n, 5n), 2n ** 53n + 5n);
assert.deepEqual(crypto.blake3(Uint8Array.of(0x10, 0x20)), Uint8Array.of(0x4a, 0x7a));
assert.equal(crypto.findNonce('abc', 0xffff_ffff_ffff_fff0n), 0xffff_ffff_ffff_fff3n);
assert.equal(crypto.findNonce('abc', null), null);

const goodPayload: SignedPayload = {
  header: { algorithm: HashAlgorithm.Sha256, byteLen: 4, verified: true, anchor: { x: 1, y: 2 } },
  label: 'ok',
  digest: Uint8Array.of(1, 2),
  tags: ['x'],
  nonce: 0xffff_ffff_ffff_fffen,
};
assert.equal(crypto.verifyPayload(goodPayload), true);
assert.throws(
  () => crypto.verifyPayload({ ...goodPayload, label: '' }),
  (e: unknown) => e instanceof HybridError && e.status === HybridStatus.Err && e.message.includes('missing payload label'),
);
assert.equal(crypto.lookupPayload(0), null);
assert.equal(crypto.lookupPayload(3)?.label, 'rec-3');
assert.equal(crypto.batchSign(['one', 'two'], HashAlgorithm.Sha512).length, 2);
crypto.resetCache();

const meta = DynamicValue.of(encodeJsValue({ k: [1, 'x', null], f: 2.5, big: 2n ** 60n }));
assert.ok(crypto.inspectDynamic(meta).equals(meta));

const transposed = crypto.transpose({
  mark: 1,
  w: Uint8Array.of(1, 2, 3),
  r: 'ab',
  count: 4,
  tag: 9,
  grid: [
    [1, 2, 3],
    [4, 5, 6],
  ],
});
assert.deepEqual(transposed, {
  mark: 2,
  w: Uint8Array.of(3, 2, 1),
  r: 'AB',
  count: 8,
  tag: -9,
  grid: [
    [1, 4],
    [2, 5],
    [3, 6],
  ],
});

crypto.dispose();
assert.throws(
  () => crypto.version(),
  (e: unknown) => e instanceof HybridError && e.status === HybridStatus.NullHandle,
);
assert.equal(live.size, 0, 'zero WASM memory leaks across all Crypto calls');

// --- VideoSurface view: audit `setProps` `HybridType::Value` tag preservation ---
let storedMeta: DynamicValue = DynamicValue.NULL;
let lastPropTags = new Map<string, BinTag>();
const videoWasm: VideoSurfaceWasmExports = {
  memory,
  nve_hybrid_alloc: wasmAlloc,
  nve_hybrid_free: wasmFree,
  nve_hybrid_video_surface_schema_hash: () => VideoSurfaceHybridClient.SCHEMA_HASH.toSignedBigInt(),
  nve_hybrid_video_surface_create: () => 22,
  nve_hybrid_video_surface_destroy: (h) => assert.equal(h, 22),
  nve_hybrid_video_surface_set_props_bin: (_h, ptr, len) => {
    const r = new BinReader(readWasmReq(ptr, len));
    lastPropTags = new Map<string, BinTag>();
    r.readMap((count) => {
      for (let i = 0; i < count; i += 1) {
        const key = r.readMapKey();
        lastPropTags.set(key, r.peekTag());
        if (key === 'metadata') {
          storedMeta = DynamicValue.read(r);
        } else {
          r.skipValue();
        }
      }
    });
    r.requireEnd();
    return HybridStatus.Ok;
  },
  nve_hybrid_video_surface_seek_bin: (_h, reqPtr, reqLen, _outLenPtr) => {
    const r = new BinReader(readWasmReq(reqPtr, reqLen));
    assert.equal(r.readArray(() => r.readI64Bits().toBigInt()), 9876n);
    return 0;
  },
  nve_hybrid_video_surface_capture_frame_bin: (_h, _reqPtr, _reqLen, outLenPtr) => {
    const w = new BinWriter();
    w.writeBool(true);
    w.writeBytes(Uint8Array.of(0xca, 0xfe));
    return writeWasmResp(outLenPtr, w.bytes);
  },
  nve_hybrid_video_surface_current_metadata_bin: (_h, _reqPtr, _reqLen, outLenPtr) => {
    return writeWasmResp(outLenPtr, storedMeta.bytes);
  },
};

const video = new VideoSurfaceHybridClient(videoWasm);
video.setProps({
  sourceUrl: 'https://nve.dev/v.mp4',
  muted: true,
  quality: VideoQuality.Uhd4k,
  posterUrl: null,
  drmKey: Uint8Array.of(0xaa, 0xbb),
  chapters: [10, 20],
  metadata: meta,
});
// CRITICAL AUDIT: `metadata` prop must arrive with tag BinTag.Map (7), NOT BinTag.Bytes (5)!
assert.equal(lastPropTags.get('metadata'), BinTag.Map, 'setProps metadata must preserve true NveValue Map tag');
assert.equal(lastPropTags.get('poster_url'), BinTag.Null, 'explicit null optional prop must emit Null');
assert.equal(lastPropTags.has('focus_point'), false, 'omitted optional prop must not be emitted');
assert.equal(lastPropTags.get('drm_key'), BinTag.Bytes);
assert.ok(video.currentMetadata().equals(meta));

// Also verify scalar DynamicValue tags (I64, Str, Null) stay true tags in setProps (never Bytes)
for (const [sample, expectedTag] of [
  [DynamicValue.of(encodeJsValue(42)), BinTag.I64],
  [DynamicValue.of(encodeJsValue('raw-str')), BinTag.Str],
  [DynamicValue.NULL, BinTag.Null],
] as const) {
  video.setProps({
    sourceUrl: 'https://nve.dev/v.mp4',
    drmKey: new Uint8Array(0),
    chapters: [],
    metadata: sample,
  });
  assert.equal(lastPropTags.get('metadata'), expectedTag);
  assert.ok(video.currentMetadata().equals(sample));
}

video.seek(9876n);
assert.deepEqual(video.captureFrame({ x: 1, y: 2 }), Uint8Array.of(0xca, 0xfe));
video.dispose();
assert.equal(live.size, 0, 'zero WASM memory leaks after VideoSurface calls');

// --- EmptyView with reserved-keyword parameter names ---
const emptyClient = new EmptyViewHybridClient({
  memory,
  nve_hybrid_alloc: wasmAlloc,
  nve_hybrid_free: wasmFree,
  nve_hybrid_empty_view_schema_hash: () => EmptyViewHybridClient.SCHEMA_HASH.toSignedBigInt(),
  nve_hybrid_empty_view_create: () => 33,
  nve_hybrid_empty_view_destroy: () => {},
  nve_hybrid_empty_view_set_props_bin: () => HybridStatus.Ok,
  nve_hybrid_empty_view_ping_bin: (_h, _rp, _rl, olp) => {
    const w = new BinWriter();
    w.writeBool(true);
    return writeWasmResp(olp, w.bytes);
  },
});
emptyClient.setProps({});
emptyClient.ping(1, true, 2);
emptyClient.dispose();
assert.equal(live.size, 0, 'zero WASM memory leaks after EmptyView calls');
"#,
    )
    .expect("write e2e_driver.ts");

    // 3. Copy host-ts's strict `tsconfig.json` settings + `node_test.d.ts` ambient decls and run `tsc --noEmit`!
    std::fs::write(
        dir.join("node_ambient.d.ts"),
        r#"
declare module 'node:assert/strict' {
  interface Assert {
    (value: unknown, message?: string): asserts value;
    equal<T>(actual: unknown, expected: T, message?: string): asserts actual is T;
    deepEqual<T>(actual: unknown, expected: T, message?: string): asserts actual is T;
    ok(value: unknown, message?: string): asserts value;
    throws(fn: () => unknown, expected?: ((err: unknown) => boolean) | (abstract new (...args: never[]) => Error), message?: string): void;
  }
  const assert: Assert;
  export default assert;
}
"#,
    )
    .expect("write node_ambient.d.ts");

    let tsconfig = dir.join("tsconfig.json");
    std::fs::write(
        &tsconfig,
        r#"{
  "compilerOptions": {
    "target": "ES2022",
    "module": "NodeNext",
    "moduleResolution": "NodeNext",
    "lib": ["ES2024", "DOM"],
    "types": [],
    "strict": true,
    "noEmit": true,
    "erasableSyntaxOnly": true,
    "verbatimModuleSyntax": true,
    "allowImportingTsExtensions": true,
    "noUncheckedIndexedAccess": true,
    "exactOptionalPropertyTypes": true,
    "noImplicitOverride": true,
    "noFallthroughCasesInSwitch": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "isolatedModules": true,
    "skipLibCheck": false
  },
  "include": ["./*.ts"]
}"#,
    )
    .expect("write tsconfig.json");

    run_tsc_no_emit(&tsconfig);

    // 4. Execute the generated TypeScript directly with `node`!
    let node_run = Command::new("node")
        .arg("--import")
        .arg(&hook)
        .arg(&driver)
        .output()
        .expect("run node on generated TS hybrid e2e driver");
    assert!(
        node_run.status.success(),
        "Node execution of generated TS hybrid failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&node_run.stdout),
        String::from_utf8_lossy(&node_run.stderr)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn real_node_and_tsc_verify_web_component_53_bit_safe_codec() {
    if !common::require_tool("node") {
        return;
    }

    let dir = temp_test_dir("web_component_53bit");
    let hook = prepare_offline_host_ts_pkg(&dir);
    std::fs::write(dir.join("package.json"), r#"{"type": "module"}"#).expect("write package.json");

    let mut spec = NveExportSpec::new("LedgerWidget", ExportBackend::Skia);
    spec.props = vec![
        PropSpec {
            name: "title".to_owned(),
            prop_type: PropType::String,
            optional: true,
            nullable: false,
            default_value: Some(DefaultValue::String("Ledger".to_owned())),
            doc: None,
        },
        PropSpec {
            name: "maxTxId".to_owned(),
            prop_type: PropType::Double,
            optional: true,
            nullable: false,
            default_value: Some(DefaultValue::Int(i64::MAX)),
            doc: None,
        },
    ];
    spec.events = vec![EventSpec {
        name: "onLedgerTick".to_owned(),
        bubbling: EventBubbling::Bubbling,
        optional: true,
        paper_top_level_name: None,
        payload: vec![ObjectField {
            name: "txId".to_owned(),
            field_type: PropType::Double,
            optional: false,
            nullable: false,
        }],
        payload_type_name: None,
        doc: None,
    }];

    let out = export_web_component(&spec).expect("export_web_component");
    std::fs::write(dir.join(&out.element_js.relative_path), &out.element_js.content)
        .expect("write web component js");
    std::fs::write(dir.join(&out.types_dts.relative_path), &out.types_dts.content)
        .expect("write web component dts");

    // Verify generated `.d.ts` type-checks cleanly.
    std::fs::write(
        dir.join("check_dts.ts"),
        r#"
import type { LedgerWidgetElement, LedgerWidgetEventMap } from './nve-ledger-widget.generated.d.ts';
export type TickDetail = LedgerWidgetEventMap['ledger-tick'];
export type Elem = LedgerWidgetElement;
"#,
    )
    .expect("write check_dts.ts");
    let tsconfig = dir.join("tsconfig.json");
    std::fs::write(
        &tsconfig,
        r#"{
  "compilerOptions": {
    "target": "ES2022",
    "module": "NodeNext",
    "moduleResolution": "NodeNext",
    "lib": ["ES2024", "DOM"],
    "types": [],
    "strict": true,
    "noEmit": true,
    "skipLibCheck": false
  },
  "include": ["./check_dts.ts", "./nve-ledger-widget.generated.d.ts"]
}"#,
    )
    .expect("write web component tsconfig");
    run_tsc_no_emit(&tsconfig);

    // Execute generated `.generated.js` under Node with a minimal DOM shim and assert 64-bit exactness!
    let runner = dir.join("run_web_component.mjs");
    std::fs::write(
        &runner,
        r#"
import assert from 'node:assert/strict';
import { BinCodecError, decodeJsValue, encodeJsValue } from '@nve/host';

const registry = new Map();
const dispatched = [];

globalThis.document = {
  createElement: () => ({ id: '', style: {} }),
};
globalThis.CustomEvent = class CustomEvent {
  constructor(type, init = {}) {
    this.type = type;
    this.detail = init.detail;
    this.bubbles = Boolean(init.bubbles);
    this.composed = Boolean(init.composed);
  }
};
globalThis.HTMLElement = class HTMLElement {
  attachShadow() {
    return { appendChild() {} };
  }
  getAttribute() {
    return null;
  }
  dispatchEvent(ev) {
    dispatched.push(ev);
    return true;
  }
};
globalThis.customElements = {
  get: (name) => registry.get(name),
  define: (name, cls) => registry.set(name, cls),
};

const { LedgerWidgetElement } = await import('./nve-ledger-widget.generated.js');
assert.equal(registry.get('nve-ledger-widget'), LedgerWidgetElement);

const el = new LedgerWidgetElement();
// Verify 64-bit default literal survived as exact bigint and encodes losslessly through `@nve/host`!
let lastPropsBin = null;
el._surface = {
  set_props_binary(bytes) {
    lastPropsBin = bytes;
  },
};
el._syncProps();
assert.ok(lastPropsBin instanceof Uint8Array);
const decodedProps = decodeJsValue(lastPropsBin);
assert.equal(decodedProps.title, 'Ledger');
assert.equal(decodedProps.maxTxId, 9223372036854775807n);

// Verify native event decoding preserves 64-bit integers beyond ±(2^53 - 1) without truncation!
const payload = {
  safeMax: Number.MAX_SAFE_INTEGER,
  safeMin: Number.MIN_SAFE_INTEGER,
  over53: 2n ** 53n,
  over53Plus1: 2n ** 53n + 1n,
  underNeg53: -(2n ** 53n),
  i64Min: -(2n ** 63n),
  i64Max: 2n ** 63n - 1n,
  negZero: -0,
  emoji: '🚀',
  raw: Uint8Array.of(1, 2, 3),
};
el._dispatchNativeEvent('onLedgerTick', encodeJsValue(payload));
assert.equal(dispatched.length, 1);
const ev = dispatched[0];
assert.equal(ev.type, 'ledger-tick');
assert.equal(ev.bubbles, true);
assert.equal(typeof ev.detail.safeMax, 'number');
assert.equal(ev.detail.safeMax, Number.MAX_SAFE_INTEGER);
assert.equal(typeof ev.detail.safeMin, 'number');
assert.equal(ev.detail.safeMin, Number.MIN_SAFE_INTEGER);
assert.equal(typeof ev.detail.over53, 'bigint');
assert.equal(ev.detail.over53, 2n ** 53n);
assert.equal(ev.detail.over53Plus1, 2n ** 53n + 1n);
assert.equal(ev.detail.underNeg53, -(2n ** 53n));
assert.equal(ev.detail.i64Min, -(2n ** 63n));
assert.equal(ev.detail.i64Max, 2n ** 63n - 1n);
assert.ok(Object.is(ev.detail.negZero, -0));
assert.equal(ev.detail.emoji, '🚀');
assert.deepEqual(ev.detail.raw, Uint8Array.of(1, 2, 3));

// Malformed trailing-bytes event payload must throw BinCodecError instead of silently decoding
const validBytes = encodeJsValue(1);
const badBytes = new Uint8Array(validBytes.length + 1);
badBytes.set(validBytes);
assert.throws(
  () => el._dispatchNativeEvent('onLedgerTick', badBytes),
  (e) => e instanceof BinCodecError && e.kind === 'trailingBytes',
);
"#,
    )
    .expect("write run_web_component.mjs");

    let node_run = Command::new("node")
        .arg("--import")
        .arg(&hook)
        .arg(&runner)
        .output()
        .expect("run node web component 53-bit test");
    assert!(
        node_run.status.success(),
        "Node execution of generated web component failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&node_run.stdout),
        String::from_utf8_lossy(&node_run.stderr)
    );

    let _ = std::fs::remove_dir_all(&dir);
}
