import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  BIN_VALUE_MAX_DEPTH,
  BinCodecError,
  BinReader,
  BinTag,
  BinWriter,
  DynamicValue,
  HybridError,
  HybridStatus,
  Key64,
  decodeJsValue,
  encodeJsValue,
  readIntInRange,
  readJsValue,
  readResultOk,
  wasmCallBin,
  wasmCopyIn,
  wasmCreateHandle,
  wasmSetPropsBin,
  writeI64Checked,
  writeIntInRange,
  writeJsValue,
  writeU64Checked,
} from '../src/index.ts';
import type { BinErrorKind, BinMark, HybridWasmExports } from '../src/index.ts';
import { GOLDEN_BIN_VALUE as GOLDEN } from './golden.ts';

function expectKind(kind: BinErrorKind, fn: () => unknown, what: string): void {
  assert.throws(fn, (e: unknown) => e instanceof BinCodecError && e.kind === kind, what);
}

function expectHybridStatus(status: HybridStatus, fn: () => unknown, what: string): void {
  assert.throws(fn, (e: unknown) => e instanceof HybridError && e.status === status, what);
}

function nestedArray(depth: number, leaf: unknown = null): unknown {
  let v: unknown = leaf;
  for (let i = 0; i < depth; i += 1) v = [v];
  return v;
}

function nestedMap(depth: number, leaf: unknown = null): unknown {
  let v: unknown = leaf;
  for (let i = 0; i < depth; i += 1) v = { k: v };
  return v;
}

function rawNestedArrays(depth: number): Uint8Array {
  const w = new BinWriter();
  const marks: BinMark[] = [];
  for (let i = 0; i < depth; i += 1) marks.push(w.beginArray(1));
  w.writeNull();
  for (const m of marks.reverse()) w.end(m);
  return w.toBytes();
}

test('js_value: 53-bit safe integer and bigint boundaries never silently truncate', () => {
  const safeCases: readonly [number | bigint, number][] = [
    [0, 0],
    [0n, 0],
    [1, 1],
    [-1, -1],
    [Number.MAX_SAFE_INTEGER, Number.MAX_SAFE_INTEGER],
    [Number.MIN_SAFE_INTEGER, Number.MIN_SAFE_INTEGER],
    [BigInt(Number.MAX_SAFE_INTEGER), Number.MAX_SAFE_INTEGER],
    [BigInt(Number.MIN_SAFE_INTEGER), Number.MIN_SAFE_INTEGER],
  ];
  for (const [input, expected] of safeCases) {
    const encoded = encodeJsValue(input);
    assert.equal(encoded[0], BinTag.I64, `tag for ${String(input)}`);
    const decoded = decodeJsValue(encoded);
    assert.equal(typeof decoded, 'number', `typeof decoded ${String(input)}`);
    assert.ok(Object.is(decoded, expected), `value for ${String(input)}`);
  }

  // ±2^53 and beyond stay exact `bigint` on decode (never rounded through `Number`).
  const bigCases: readonly bigint[] = [
    2n ** 53n,
    -(2n ** 53n),
    2n ** 53n + 1n,
    -(2n ** 53n) - 1n,
    2n ** 62n + 123456789n,
    -(2n ** 63n),
    2n ** 63n - 1n,
  ];
  for (const input of bigCases) {
    const encoded = encodeJsValue(input);
    assert.equal(encoded[0], BinTag.I64, `tag for ${input}`);
    const decoded = decodeJsValue(encoded);
    assert.equal(typeof decoded, 'bigint', `typeof decoded ${input}`);
    assert.equal(decoded, input, `exact bigint ${input}`);
  }

  // A JS `number` equal to 2^53 is NOT a safe integer -> encoded as F64 and round-trips as number.
  const unsafeNum = 2 ** 53;
  const unsafeBytes = encodeJsValue(unsafeNum);
  assert.equal(unsafeBytes[0], BinTag.F64);
  assert.equal(decodeJsValue(unsafeBytes), unsafeNum);
});

test('js_value: i64 bounds reject out-of-range bigint cleanly', () => {
  for (const bad of [2n ** 63n, -(2n ** 63n) - 1n, 2n ** 64n, 1n << 128n, -(1n << 128n)]) {
    assert.throws(() => encodeJsValue(bad), RangeError, `out-of-range bigint ${bad}`);
  }
});

test('js_value: special floats (-0, NaN, ±Infinity), strings, emoji, BOM, bytes, undefined', () => {
  // +0 -> I64(+0), -0 -> F64(-0) preserving sign bit.
  const posZero = encodeJsValue(0);
  assert.equal(posZero[0], BinTag.I64);
  assert.ok(Object.is(decodeJsValue(posZero), 0));

  const negZero = encodeJsValue(-0);
  assert.equal(negZero[0], BinTag.F64);
  assert.ok(Object.is(decodeJsValue(negZero), -0));

  const nanVal = decodeJsValue(encodeJsValue(NaN));
  assert.equal(typeof nanVal, 'number');
  assert.ok(Number.isNaN(nanVal));

  assert.equal(decodeJsValue(encodeJsValue(Infinity)), Infinity);
  assert.equal(decodeJsValue(encodeJsValue(-Infinity)), -Infinity);
  assert.equal(decodeJsValue(encodeJsValue(1.5)), 1.5);
  assert.equal(decodeJsValue(encodeJsValue(-3.25)), -3.25);

  // Null, undefined, booleans
  assert.equal(decodeJsValue(encodeJsValue(null)), null);
  assert.equal(decodeJsValue(encodeJsValue(undefined)), null);
  assert.equal(decodeJsValue(encodeJsValue(true)), true);
  assert.equal(decodeJsValue(encodeJsValue(false)), false);

  // UTF-8 emoji, astral code points, BOM preservation, lone surrogate rejection
  for (const s of ['', 'ascii', 'café', '🚀🎉', '👨‍👩‍👧‍👦', '\u{10ffff}', '\ufeffleading-bom']) {
    assert.equal(decodeJsValue(encodeJsValue(s)), s, `string ${JSON.stringify(s)}`);
  }
  expectKind('invalidUtf8', () => encodeJsValue('\ud800'), 'lone high surrogate');
  expectKind('invalidUtf8', () => encodeJsValue({ ['\udc00']: 1 }), 'lone surrogate map key');

  // Uint8Array including subarray with non-zero byteOffset and isolated copy on decode
  const backing = Uint8Array.of(0x00, 0xde, 0xad, 0xbe, 0xef, 0x99);
  const view = backing.subarray(1, 5);
  const encodedBytes = encodeJsValue(view);
  const decodedBytes = decodeJsValue(encodedBytes);
  assert.ok(decodedBytes instanceof Uint8Array);
  assert.deepEqual(decodedBytes, Uint8Array.of(0xde, 0xad, 0xbe, 0xef));
  encodedBytes.fill(0);
  assert.deepEqual(decodedBytes, Uint8Array.of(0xde, 0xad, 0xbe, 0xef), 'decoded bytes must not alias input');
  assert.deepEqual(decodeJsValue(encodeJsValue(new Uint8Array(0))), new Uint8Array(0));
});

test('js_value: nested arrays, maps, golden parity, sparse arrays, __proto__, null-proto', () => {
  // Golden vector parity with Rust BinValueView
  const goldenDecoded = decodeJsValue(GOLDEN);
  assert.deepEqual(goldenDecoded, { k: [1, 'é', null, true, 1.5, Uint8Array.of(0xff)] });
  assert.deepEqual(encodeJsValue(goldenDecoded), GOLDEN);

  // Nested structures, undefined map entries skipped, sparse array holes -> null
  const complex = {
    a: [1, 2n ** 60n, { nested: ['🚀', true, false, null] }],
    skipped: undefined,
    bytes: Uint8Array.of(7, 8),
  };
  assert.deepEqual(decodeJsValue(encodeJsValue(complex)), {
    a: [1, 2n ** 60n, { nested: ['🚀', true, false, null] }],
    bytes: Uint8Array.of(7, 8),
  });

  // eslint-disable-next-line no-sparse-arrays
  const sparse = [1, , 3];
  assert.deepEqual(decodeJsValue(encodeJsValue(sparse)), [1, null, 3]);

  // Null-prototype plain object is accepted
  const nullProto = Object.create(null) as Record<string, unknown>;
  nullProto['x'] = 42;
  assert.deepEqual(decodeJsValue(encodeJsValue(nullProto)), { x: 42 });

  // __proto__ map key decodes as own data property without polluting Object.prototype
  const protoInput: Record<string, unknown> = {};
  Object.defineProperty(protoInput, '__proto__', {
    value: { polluted: true },
    enumerable: true,
    writable: true,
    configurable: true,
  });
  const protoDecoded = decodeJsValue(encodeJsValue(protoInput)) as Record<string, unknown>;
  assert.equal(Object.getPrototypeOf(protoDecoded), Object.prototype);
  assert.equal(({} as Record<string, unknown>)['polluted'], undefined);
  assert.ok(Object.prototype.hasOwnProperty.call(protoDecoded, '__proto__'));
  assert.deepEqual(protoDecoded['__proto__'], { polluted: true });
});

test('js_value: non-plain objects, functions, and symbols throw TypeError', () => {
  class Custom {
    x = 1;
  }
  const nonPlain: unknown[] = [
    new Date(),
    /abc/,
    new Map(),
    new Set(),
    new Error('boom'),
    new Int32Array(2),
    new DataView(new ArrayBuffer(4)),
    new Custom(),
    Object(1),
    Object('str'),
    Object(true),
    () => 1,
    Symbol('sym'),
  ];
  for (const bad of nonPlain) {
    assert.throws(() => encodeJsValue(bad), TypeError, `reject ${Object.prototype.toString.call(bad)}`);
  }
});

test('js_value: maxDepth nesting limit and hostile cyclic objects', () => {
  // Exact max depth succeeds for both arrays and maps (encode + decode)
  const arr64 = encodeJsValue(nestedArray(BIN_VALUE_MAX_DEPTH, 7));
  BinReader.validate(arr64);
  assert.ok(Array.isArray(decodeJsValue(arr64)));

  const map64 = encodeJsValue(nestedMap(BIN_VALUE_MAX_DEPTH, 'ok'));
  BinReader.validate(map64);
  assert.equal(typeof decodeJsValue(map64), 'object');

  // Depth 65 fails on encode and decode
  expectKind('maxDepth', () => encodeJsValue(nestedArray(BIN_VALUE_MAX_DEPTH + 1)), 'encode array depth 65');
  expectKind('maxDepth', () => encodeJsValue(nestedMap(BIN_VALUE_MAX_DEPTH + 1)), 'encode map depth 65');
  expectKind('maxDepth', () => decodeJsValue(rawNestedArrays(BIN_VALUE_MAX_DEPTH + 1)), 'decode array depth 65');

  // Hostile cyclic objects & arrays fail with maxDepth instead of RangeError stack overflow
  const cyclicObj: Record<string, unknown> = {};
  cyclicObj['self'] = cyclicObj;
  expectKind('maxDepth', () => encodeJsValue(cyclicObj), 'cyclic object');

  const cyclicArr: unknown[] = [];
  cyclicArr.push(cyclicArr);
  expectKind('maxDepth', () => encodeJsValue(cyclicArr), 'cyclic array');
});

test('js_value: trailingBytes, nested container slack, and container escape are rejected', () => {
  // Top-level trailing bytes
  const valid = encodeJsValue(42);
  const withTrail = new Uint8Array(valid.length + 1);
  withTrail.set(valid);
  expectKind('trailingBytes', () => decodeJsValue(withTrail), 'top-level trailing byte');

  // Top-level container slack
  expectKind('trailingBytes', () => decodeJsValue(Uint8Array.of(6, 6, 0, 0, 0, 1, 0, 0, 0, 0, 0)), 'array slack');

  // Nested array slack disguised inside outer array (must fail even though total length matches!)
  // Outer array: count=1, payload=15; inner array: payload=6, count=1, elem=Null(0), slack=Null(0)
  const nestedSlack = Uint8Array.of(
    6, 15, 0, 0, 0, 1, 0, 0, 0,
    6, 6, 0, 0, 0, 1, 0, 0, 0, 0, 0,
  );
  expectKind('trailingBytes', () => decodeJsValue(nestedSlack), 'nested array slack');

  // Nested map slack disguised inside outer array:
  // Outer array: count=1, payload=14 (4 count + 10 inner map); inner map: payload=5, count=0, slack=Null(0)
  const nestedMapSlack = Uint8Array.of(
    6, 14, 0, 0, 0, 1, 0, 0, 0,
    7, 5, 0, 0, 0, 0, 0, 0, 0, 0,
  );
  expectKind('trailingBytes', () => decodeJsValue(nestedMapSlack), 'nested map slack');

  // Container escape: inner array claims payload=5 (1 byte after count), holds a 9-byte I64
  const escape = Uint8Array.of(6, 5, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0);
  expectKind('truncated', () => decodeJsValue(escape), 'container overrun');

  // Streaming `writeJsValue` / `readJsValue` compose cleanly on one writer/reader
  const w = new BinWriter();
  writeJsValue(w, 'first');
  writeJsValue(w, 2n ** 60n);
  const r = new BinReader(w.bytes);
  assert.equal(readJsValue(r), 'first');
  assert.equal(readJsValue(r), 2n ** 60n);
  r.requireEnd();
});

test('hybrid_runtime: DynamicValue.read/writeTo, checked ints, and WASM call helpers', () => {
  const w = new BinWriter();
  writeIntInRange(w, 255, 0, 255);
  writeI64Checked(w, -(2n ** 63n));
  writeU64Checked(w, 2n ** 64n - 1n);
  const dv = DynamicValue.of(GOLDEN);
  dv.writeTo(w);

  const r = new BinReader(w.bytes);
  assert.equal(readIntInRange(r, 0, 255), 255);
  assert.equal(r.readI64(), -(2n ** 63n));
  assert.equal(r.readI64Bits().toBigInt(), 2n ** 64n - 1n);
  const dvOut = DynamicValue.read(r);
  assert.ok(dvOut.equals(dv));
  r.requireEnd();

  // Range rejections on write & read
  assert.throws(() => writeIntInRange(new BinWriter(), 256, 0, 255), RangeError);
  assert.throws(() => writeIntInRange(new BinWriter(), 1.5, 0, 255), RangeError);
  assert.throws(() => writeIntInRange(new BinWriter(), NaN, 0, 255), RangeError);
  assert.throws(() => writeI64Checked(new BinWriter(), 2n ** 63n), RangeError);
  assert.throws(() => writeU64Checked(new BinWriter(), -1n), RangeError);
  assert.throws(() => writeU64Checked(new BinWriter(), 2n ** 64n), RangeError);
  assert.throws(() => readIntInRange(new BinReader(encodeJsValue(300)), 0, 255), RangeError);

  // readResultOk on Ok, Err, and Err with trailing bytes
  const okW = new BinWriter();
  okW.writeBool(true);
  okW.writeStr('hi');
  const okR = new BinReader(okW.bytes);
  readResultOk(okR);
  assert.equal(okR.readStr(), 'hi');
  okR.requireEnd();

  const errW = new BinWriter();
  errW.writeBool(false);
  errW.writeStr('bad input');
  expectHybridStatus(HybridStatus.Err, () => readResultOk(new BinReader(errW.bytes)), 'Result Err');

  // Mock WASM linear memory with bump allocator + live-allocation leak tracking + memory.grow()
  const memory = new WebAssembly.Memory({ initial: 1, maximum: 4 });
  let bump = 16;
  const live = new Map<number, number>();
  const wasm: HybridWasmExports = {
    memory,
    nve_hybrid_alloc(len: number): number {
      const ptr = bump;
      bump += (len + 7) & ~7;
      live.set(ptr, len);
      return ptr;
    },
    nve_hybrid_free(ptr: number, len: number): void {
      assert.equal(live.get(ptr), len, `free mismatch at ${ptr}`);
      live.delete(ptr);
    },
  };

  // Schema check + handle creation (including bit-63 signed bigint from WASM i64)
  const expected = new Key64(0x1122_3344, 0xfedc_ba98);
  const signedFromWasm = expected.toSignedBigInt();
  assert.ok(signedFromWasm < 0n);
  const handle = wasmCreateHandle('Demo', expected, signedFromWasm, () => 7);
  assert.equal(handle, 7);
  expectHybridStatus(
    HybridStatus.SchemaMismatch,
    () => wasmCreateHandle('Demo', expected, 0n, () => 7),
    'schema mismatch',
  );
  expectHybridStatus(
    HybridStatus.NullHandle,
    () => wasmCreateHandle('Demo', expected, signedFromWasm, () => 0),
    'null handle',
  );

  // wasmCallBin across a `memory.grow(1)` (detaches the old ArrayBuffer!) with zero leaks
  const resp = wasmCallBin(wasm, 'Demo', handle, Uint8Array.of(1, 2, 3), (_h, reqPtr, reqLen, outLenPtr) => {
    assert.deepEqual(new Uint8Array(wasm.memory.buffer, reqPtr, reqLen), Uint8Array.of(1, 2, 3));
    wasm.memory.grow(1); // detaches previous wasm.memory.buffer!
    const out = Uint8Array.of(9, 8, 7, 6);
    const respPtr = wasm.nve_hybrid_alloc(out.length);
    new Uint8Array(wasm.memory.buffer, respPtr, out.length).set(out);
    new DataView(wasm.memory.buffer).setUint32(outLenPtr, out.length, true);
    return respPtr;
  });
  assert.deepEqual(resp, Uint8Array.of(9, 8, 7, 6));
  assert.equal(live.size, 0, 'zero WASM memory leaks after call');

  // Disposed handle rejected cleanly
  expectHybridStatus(
    HybridStatus.NullHandle,
    () => wasmCallBin(wasm, 'Demo', 0, new Uint8Array(0), () => 0),
    'disposed call',
  );
  expectHybridStatus(
    HybridStatus.NullHandle,
    () => wasmSetPropsBin(wasm, 'Demo', 0, new Uint8Array(0), () => 0),
    'disposed setProps',
  );

  // OOM alloc returning 0 throws HybridError without corrupting memory[0]
  const oomWasm: HybridWasmExports = {
    memory: new WebAssembly.Memory({ initial: 1 }),
    nve_hybrid_alloc: () => 0,
    nve_hybrid_free: () => {},
  };
  expectHybridStatus(HybridStatus.Err, () => wasmCopyIn(oomWasm, Uint8Array.of(1)), 'OOM copyIn');

  // Non-Ok setProps status throws HybridError and still frees payload
  expectHybridStatus(
    HybridStatus.InvalidArg,
    () => wasmSetPropsBin(wasm, 'Demo', handle, Uint8Array.of(0), () => HybridStatus.InvalidArg),
    'setProps error status',
  );
  // Null response pointer paired with non-zero outLen throws HybridError
  expectHybridStatus(
    HybridStatus.Err,
    () =>
      wasmCallBin(wasm, 'Demo', handle, Uint8Array.of(0), (_h, _rp, _rl, olp) => {
        new DataView(wasm.memory.buffer).setUint32(olp, 8, true);
        return 0;
      }),
    'null response pointer with nonzero length',
  );
  // Unsafe bounds rejected by readIntInRange / writeIntInRange
  assert.throws(() => readIntInRange(new BinReader(encodeJsValue(0)), 0, 2 ** 53), RangeError);
  assert.throws(() => writeIntInRange(new BinWriter(), 0, 10, 5), RangeError);
  // DynamicValue.read inside a depth-64 container rejects a 65th nested container
  const deepInner = rawNestedArrays(BIN_VALUE_MAX_DEPTH + 1);
  const deepReader = new BinReader(deepInner);
  expectKind(
    'maxDepth',
    () => deepReader.readArray(() => DynamicValue.read(deepReader)),
    'DynamicValue.read inherits reader container depth',
  );
  assert.equal(live.size, 0, 'zero WASM memory leaks after failed setProps');
});
