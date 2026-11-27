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
    } else if common::tool_available("npx") {
        let mut c = Command::new("npx");
        c.args(["-y", "-p", "typescript@5.9", "tsc", "--noEmit", "-p"])
            .arg(tsconfig);
        c
    } else {
        assert!(
            std::env::var_os("NVE_REQUIRE_TSC").is_none() && std::env::var_os("CI").is_none(),
            "`tsc` (or `npx`) is required (NVE_REQUIRE_TSC / CI) but neither was found"
        );
        eprintln!("SKIP: neither `tsc` nor `npx` found");
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

    assert!(src.contains("import * as _nve from '@nve/host';"));
    assert!(src.contains("new _nve.Key64("));
    assert!(src.contains("DynamicValue"));
    assert!(src.contains("_nve.wasmCallBin("));
    assert!(src.contains("_nve.wasmSetPropsBin("));
    assert!(src.contains("_nve.wasmCreateHandle("));

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
fn generated_web_component_reuses_host_ts_and_emits_typed_defaults() {
    let mut spec = counter_card_export_spec(ExportBackend::Skia);
    spec.props.push(PropSpec {
        name: "scale".to_owned(),
        prop_type: PropType::Double,
        optional: true,
        nullable: false,
        default_value: Some(DefaultValue::Int(1)),
        doc: None,
    });
    spec.props.push(PropSpec {
        name: "hugeMax".to_owned(),
        prop_type: PropType::Mixed,
        optional: true,
        nullable: false,
        default_value: Some(DefaultValue::Int(i64::MAX)),
        doc: None,
    });
    spec.props.push(PropSpec {
        name: "hugeMin".to_owned(),
        prop_type: PropType::Mixed,
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
    // `Double` with `DefaultValue::Int` emits a JS float number literal, never a BigInt `...n`.
    assert!(js.contains("'scale': 1.0,"));
    // `Mixed` with `DefaultValue::Int` outside ±(2^53 - 1) emits an exact BigInt literal.
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

    // Out-of-range Int32 / IntEnum defaults and integer defaults on non-numeric props are rejected
    // rather than silently emitting `bigint` (`...n`) on `number`/non-`Mixed` props.
    for bad_prop in [
        PropSpec {
            name: "badI32".to_owned(),
            prop_type: PropType::Int32,
            optional: true,
            nullable: false,
            default_value: Some(DefaultValue::Int(i64::MAX)),
            doc: None,
        },
        PropSpec {
            name: "badI32Edge".to_owned(),
            prop_type: PropType::Int32,
            optional: true,
            nullable: false,
            default_value: Some(DefaultValue::Int(i64::from(i32::MAX) + 1)),
            doc: None,
        },
        PropSpec {
            name: "badEnumRange".to_owned(),
            prop_type: PropType::IntEnum { variants: vec![0, 1] },
            optional: true,
            nullable: false,
            default_value: Some(DefaultValue::Int(i64::MAX)),
            doc: None,
        },
        PropSpec {
            name: "badEnumVariant".to_owned(),
            prop_type: PropType::IntEnum { variants: vec![0, 1] },
            optional: true,
            nullable: false,
            default_value: Some(DefaultValue::Int(2)),
            doc: None,
        },
        PropSpec {
            name: "badStrInt".to_owned(),
            prop_type: PropType::String,
            optional: true,
            nullable: false,
            default_value: Some(DefaultValue::Int(1)),
            doc: None,
        },
    ] {
        let mut bad_spec = NveExportSpec::new("BadWidget", ExportBackend::Skia);
        bad_spec.props.push(bad_prop);
        assert!(
            export_web_component(&bad_spec).is_err(),
            "expected validation error for invalid default on {}",
            bad_spec.props[0].name
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

    // 1. Generate realistic spec + edge-case specs (enum-only, struct-only, empty-props view with
    //    strict-mode reserved & helper-shadowing params, sibling Vec + FixedArray struct & method).
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
    fn ping(
        &self,
        default: i32,
        class: bool,
        arguments: u32,
        eval: String,
        undefined: Option<i32>,
        wasm_call_bin: i32,
        read_int_in_range: u16,
    ) -> Result<i32, String>;
}
"#,
        r#"
#[nve_hybrid_struct]
pub struct SiblingArrays {
    pub nums: Vec<i32>,
    pub tags: Vec<String>,
    pub coords: [u32; 3],
    pub flags: [bool; 2],
}

#[nve_hybrid_module(name = "Batcher")]
pub trait BatcherSpec {
    fn combine(
        &self,
        left: Vec<i32>,
        right: Vec<String>,
        coords: [u32; 3],
        flags: [bool; 2],
    ) -> SiblingArrays;
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

    // 2. End-to-end TypeScript driver exercising every codec path, sibling Vec/FixedArray block
    //    scoping, strict-mode reserved params (`arguments`, `eval`) + helper-shadowing params
    //    (`wasm_call_bin`, `read_int_in_range`), WASM memory grow, `HybridType::Value` tag
    //    preservation, `Option<Struct>`, `dispose()` idempotence, and zero WASM memory leaks.
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
import {
  BatcherHybridClient,
  decodeSiblingArrays,
  encodeSiblingArrays,
} from './batcher_hybrid.generated.ts';

// --- Direct struct, enum, and sibling Vec/FixedArray codec checks ---
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

// Sibling Vec + FixedArray struct round-trip and FixedArray length validation on encode & decode
{
  const sample = {
    nums: [1, -2, 3],
    tags: ['a', 'b'],
    coords: [10, 20, 30],
    flags: [true, false],
  };
  const w = new BinWriter();
  encodeSiblingArrays(w, sample);
  const r = new BinReader(w.bytes);
  assert.deepEqual(decodeSiblingArrays(r), sample);
  r.requireEnd();

  // Wrong FixedArray length throws RangeError on encode
  assert.throws(
    () => encodeSiblingArrays(new BinWriter(), { ...sample, coords: [1, 2] }),
    RangeError,
  );

  // Wrong FixedArray wire length throws BinCodecError('truncated') on decode
  const badW = new BinWriter();
  const m = badW.beginMap(4);
  badW.writeMapKey('nums');
  const a1 = badW.beginArray(0);
  badW.end(a1);
  badW.writeMapKey('tags');
  const a2 = badW.beginArray(0);
  badW.end(a2);
  badW.writeMapKey('coords');
  const a3 = badW.beginArray(2); // expected 3!
  badW.writeI64(1n);
  badW.writeI64(2n);
  badW.end(a3);
  badW.writeMapKey('flags');
  const a4 = badW.beginArray(2);
  badW.writeBool(true);
  badW.writeBool(false);
  badW.end(a4);
  badW.end(m);
  assert.throws(
    () => decodeSiblingArrays(new BinReader(badW.bytes)),
    (e: unknown) => e instanceof BinCodecError && e.kind === 'truncated',
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

let cryptoDestroyCalls = 0;
const cryptoWasm: CryptoWasmExports = {
  memory,
  nve_hybrid_alloc: wasmAlloc,
  nve_hybrid_free: wasmFree,
  nve_hybrid_crypto_schema_hash: () => CryptoHybridClient.SCHEMA_HASH.toSignedBigInt(),
  nve_hybrid_crypto_create: () => 11,
  nve_hybrid_crypto_destroy: (h) => {
    assert.equal(h, 11);
    cryptoDestroyCalls += 1;
  },
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
    r.requireEnd();
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
assert.deepEqual(crypto.hashWith(HashAlgorithm.Sha256, Uint8Array.of(1, 2, 3)), {
  algorithm: HashAlgorithm.Sha256,
  byteLen: 3,
  verified: true,
  anchor: { x: 1.5, y: -2.5 },
});
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
crypto.dispose(); // idempotent: never double-destroys
assert.equal(cryptoDestroyCalls, 1);
assert.throws(
  () => crypto.version(),
  (e: unknown) => e instanceof HybridError && e.status === HybridStatus.NullHandle,
);
assert.equal(live.size, 0, 'zero WASM memory leaks across all Crypto calls');

// --- VideoSurface view: audit `setProps` `HybridType::Value` tag preservation & Option<Struct> ---
let storedMeta: DynamicValue = DynamicValue.NULL;
let lastPropTags = new Map<string, BinTag>();
let lastFocusPoint: { x: number; y: number } | null | undefined;
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
    lastFocusPoint = undefined;
    r.readMap((count) => {
      for (let i = 0; i < count; i += 1) {
        const key = r.readMapKey();
        lastPropTags.set(key, r.peekTag());
        if (key === 'metadata') {
          storedMeta = DynamicValue.read(r);
        } else if (key === 'focus_point') {
          lastFocusPoint = r.readNull() ? null : decodeFramePoint(r);
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
  focusPoint: { x: 7.5, y: -3.25 },
  drmKey: Uint8Array.of(0xaa, 0xbb),
  chapters: [10, 20],
  metadata: meta,
});
assert.equal(lastPropTags.get('metadata'), BinTag.Map, 'setProps metadata must preserve true NveValue Map tag');
assert.equal(lastPropTags.get('poster_url'), BinTag.Null, 'explicit null optional prop must emit Null');
assert.equal(lastPropTags.get('focus_point'), BinTag.Map, 'Some(FramePoint) prop encodes as Map');
assert.deepEqual(lastFocusPoint, { x: 7.5, y: -3.25 });
assert.equal(lastPropTags.get('drm_key'), BinTag.Bytes);
assert.ok(video.currentMetadata().equals(meta));

// Explicit null clears Option<FramePoint>; omitting it skips the map key
video.setProps({
  sourceUrl: 'https://nve.dev/v.mp4',
  focusPoint: null,
  drmKey: new Uint8Array(0),
  chapters: [],
  metadata: DynamicValue.NULL,
});
assert.equal(lastPropTags.get('focus_point'), BinTag.Null);
assert.equal(lastFocusPoint, null);

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
  assert.equal(lastPropTags.has('focus_point'), false, 'omitted optional prop must not be emitted');
  assert.equal(lastPropTags.get('metadata'), expectedTag);
  assert.ok(video.currentMetadata().equals(sample));
}

video.seek(9876n);
assert.deepEqual(video.captureFrame({ x: 1, y: 2 }), Uint8Array.of(0xca, 0xfe));
video.dispose();
assert.equal(live.size, 0, 'zero WASM memory leaks after VideoSurface calls');

// --- EmptyView with strict-mode reserved (`arguments`, `eval`) & helper-shadowing params ---
const emptyClient = new EmptyViewHybridClient({
  memory,
  nve_hybrid_alloc: wasmAlloc,
  nve_hybrid_free: wasmFree,
  nve_hybrid_empty_view_schema_hash: () => EmptyViewHybridClient.SCHEMA_HASH.toSignedBigInt(),
  nve_hybrid_empty_view_create: () => 33,
  nve_hybrid_empty_view_destroy: () => {},
  nve_hybrid_empty_view_set_props_bin: () => HybridStatus.Ok,
  nve_hybrid_empty_view_ping_bin: (_h, reqPtr, reqLen, olp) => {
    const r = new BinReader(readWasmReq(reqPtr, reqLen));
    const sum = r.readArray((n) => {
      assert.equal(n, 7);
      const a = Number(r.readI64());
      const b = r.readBool();
      const c = Number(r.readI64());
      const d = r.readStr();
      const u = r.readNull() ? 0 : Number(r.readI64());
      const e = Number(r.readI64());
      const f = Number(r.readI64());
      return a + (b ? 1 : 0) + c + d.length + u + e + f;
    });
    r.requireEnd();
    const w = new BinWriter();
    w.writeBool(true);
    w.writeI64(BigInt(sum));
    return writeWasmResp(olp, w.bytes);
  },
});
emptyClient.setProps({});
assert.equal(emptyClient.ping(1, true, 2, 'ok', 5, 10, 20), 41);
assert.equal(emptyClient.ping(1, true, 2, 'ok', null, 10, 20), 36);
emptyClient.dispose();

// --- Batcher module with sibling Vec + FixedArray parameters ---
const batcher = new BatcherHybridClient({
  memory,
  nve_hybrid_alloc: wasmAlloc,
  nve_hybrid_free: wasmFree,
  nve_hybrid_batcher_schema_hash: () => BatcherHybridClient.SCHEMA_HASH.toSignedBigInt(),
  nve_hybrid_batcher_create: () => 44,
  nve_hybrid_batcher_destroy: () => {},
  nve_hybrid_batcher_combine_bin: (_h, reqPtr, reqLen, olp) => {
    const r = new BinReader(readWasmReq(reqPtr, reqLen));
    const out = r.readArray((n) => {
      assert.equal(n, 4);
      const nums = r.readArray((c) => Array.from({ length: c }, () => Number(r.readI64())));
      const tags = r.readArray((c) => Array.from({ length: c }, () => r.readStr()));
      const coords = r.readArray((c) => Array.from({ length: c }, () => Number(r.readI64())));
      const flags = r.readArray((c) => Array.from({ length: c }, () => r.readBool()));
      return { nums, tags, coords, flags };
    });
    r.requireEnd();
    const w = new BinWriter();
    encodeSiblingArrays(w, out);
    return writeWasmResp(olp, w.bytes);
  },
});
assert.deepEqual(batcher.combine([4, 5], ['hi'], [1, 2, 3], [false, true]), {
  nums: [4, 5],
  tags: ['hi'],
  coords: [1, 2, 3],
  flags: [false, true],
});
batcher.dispose();
assert.equal(live.size, 0, 'zero WASM memory leaks after all calls');
"#,
    )
    .expect("write e2e_driver.ts");

    // 3. Strict `tsconfig.json` matching `host-ts` + run `tsc --noEmit`!
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
            name: "scale".to_owned(),
            prop_type: PropType::Double,
            optional: true,
            nullable: false,
            default_value: Some(DefaultValue::Int(1)),
            doc: None,
        },
        PropSpec {
            name: "maxTxId".to_owned(),
            prop_type: PropType::Mixed,
            optional: true,
            nullable: false,
            default_value: Some(DefaultValue::Int(i64::MAX)),
            doc: None,
        },
        PropSpec {
            name: "enabled".to_owned(),
            prop_type: PropType::Bool,
            optional: true,
            nullable: false,
            default_value: Some(DefaultValue::Bool(true)),
            doc: None,
        },
        PropSpec {
            name: "note".to_owned(),
            prop_type: PropType::String,
            optional: true,
            nullable: false,
            default_value: Some(DefaultValue::Null),
            doc: None,
        },
        PropSpec {
            name: "subtitle".to_owned(),
            prop_type: PropType::String,
            optional: true,
            nullable: false,
            default_value: None,
            doc: None,
        },
        PropSpec {
            name: "badge".to_owned(),
            prop_type: PropType::String,
            optional: true,
            nullable: true,
            default_value: None,
            doc: None,
        },
        PropSpec {
            name: "meta".to_owned(),
            prop_type: PropType::Object {
                fields: vec![ObjectField {
                    name: "tag".to_owned(),
                    field_type: PropType::String,
                    optional: false,
                    nullable: true,
                }],
            },
            optional: false,
            nullable: false,
            default_value: None,
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

    // Verify generated `.d.ts` type-checks cleanly under exactOptionalPropertyTypes +
    // verbatimModuleSyntax + erasableSyntaxOnly across optional, defaulted, null-defaulted,
    // nullable, and Object-with-nullable-field props!
    std::fs::write(
        dir.join("check_dts.ts"),
        r#"
import type {
  LedgerWidgetElement,
  LedgerWidgetEventMap,
  LedgerWidgetProps,
} from './nve-ledger-widget.generated.d.ts';

export type TickDetail = LedgerWidgetEventMap['ledger-tick'];
export function checkProps(el: LedgerWidgetElement): LedgerWidgetProps {
  const s: string = el.title;
  const sc: number = el.scale;
  const en: boolean = el.enabled;
  const n: string | null = el.note;
  const sub: string | undefined = el.subtitle;
  const b: string | null | undefined = el.badge;
  const tag: string | null = el.meta.tag;
  el.subtitle = undefined;
  el.badge = null;
  el.note = null;
  return { title: s, scale: sc, enabled: en, note: n, subtitle: sub, badge: b, meta: { tag } };
}
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
    "exactOptionalPropertyTypes": true,
    "verbatimModuleSyntax": true,
    "erasableSyntaxOnly": true,
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
assert.equal(typeof decodedProps.scale, 'number');
assert.equal(decodedProps.scale, 1);
assert.equal(decodedProps.maxTxId, 9223372036854775807n);
assert.equal(decodedProps.note, null);

// Removing a defaulted attribute restores its default instead of `undefined`
el.attributeChangedCallback('title', 'Ledger', 'Custom');
assert.equal(el.title, 'Custom');
el.attributeChangedCallback('title', 'Custom', null);
assert.equal(el.title, 'Ledger');
el.attributeChangedCallback('scale', '1', '2.5');
assert.equal(el.scale, 2.5);
el.attributeChangedCallback('scale', '2.5', null);
assert.equal(el.scale, 1.0);
// Removing a `Bool` attribute with `default = true` intentionally restores `true`
assert.equal(el.enabled, true);
el.attributeChangedCallback('enabled', null, 'false');
assert.equal(el.enabled, false);
el.attributeChangedCallback('enabled', 'false', null);
assert.equal(el.enabled, true);
// Removing a non-defaulted optional attribute sets `undefined`
el.attributeChangedCallback('subtitle', null, 'sub');
assert.equal(el.subtitle, 'sub');
el.attributeChangedCallback('subtitle', 'sub', null);
assert.equal(el.subtitle, undefined);

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
