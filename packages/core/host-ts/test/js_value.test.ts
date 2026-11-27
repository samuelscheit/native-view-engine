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

function rawNestedMaps(depth: number): Uint8Array {
  const w = new BinWriter();
  const marks: BinMark[] = [];
  for (let i = 0; i < depth; i += 1) {
    marks.push(w.beginMap(1));
    w.writeMapKey('k');
  }
  w.writeNull();
  for (const m of marks.reverse()) w.end(m);
  return w.toBytes();
}

test('js_value: Key64.toSignedSafeInteger and 53-bit boundaries never silently truncate', () => {
  // Direct zero-BigInt safe-integer boundary checks on Key64
  assert.equal(Key64.fromBigInt(0n).toSignedSafeInteger(), 0);
  assert.equal(Key64.fromBigInt(-1n).toSignedSafeInteger(), -1);
  assert.equal(Key64.fromBigInt(-(2n ** 32n)).toSignedSafeInteger(), -(2 ** 32));
  assert.equal(Key64.fromBigInt(BigInt(Number.MAX_SAFE_INTEGER)).toSignedSafeInteger(), Number.MAX_SAFE_INTEGER);
  assert.equal(Key64.fromBigInt(BigInt(Number.MIN_SAFE_INTEGER)).toSignedSafeInteger(), Number.MIN_SAFE_INTEGER);
  assert.equal(Key64.fromBigInt(2n ** 53n).toSignedSafeInteger(), null);
  assert.equal(Key64.fromBigInt(-(2n ** 53n)).toSignedSafeInteger(), null);
  assert.equal(Key64.fromBigInt(2n ** 53n + 1n).toSignedSafeInteger(), null);
  assert.equal(Key64.fromBigInt(-(2n ** 53n) - 1n).toSignedSafeInteger(), null);
  assert.equal(Key64.fromBigInt(-(2n ** 63n)).toSignedSafeInteger(), null);
  assert.equal(Key64.fromBigInt(2n ** 63n - 1n).toSignedSafeInteger(), null);

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

  // Standalone empty containers
  assert.deepEqual(decodeJsValue(encodeJsValue([])), []);
  assert.deepEqual(decodeJsValue(encodeJsValue({})), {});

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

test('js_value: nested containers, golden parity, sparse arrays, duplicate keys, __proto__', () => {
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

  // Duplicate wire map keys (including duplicate '__proto__'): last-write-wins, zero prototype pollution
  const dupW = new BinWriter();
  const dupM = dupW.beginMap(4);
  dupW.writeMapKey('a');
  dupW.writeI64(1n);
  dupW.writeMapKey('a');
  dupW.writeI64(2n);
  dupW.writeMapKey('__proto__');
  dupW.writeStr('first');
  dupW.writeMapKey('__proto__');
  dupW.writeStr('second');
  dupW.end(dupM);
  const dupDecoded = decodeJsValue(dupW.bytes) as Record<string, unknown>;
  assert.equal(Object.getPrototypeOf(dupDecoded), Object.prototype);
  assert.equal(dupDecoded['a'], 2);
  assert.equal(dupDecoded['__proto__'], 'second');
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

test('js_value: maxDepth nesting limit (arrays, maps, cyclic objects, embedded DynamicValue)', () => {
  // Exact max depth succeeds for both arrays and maps (encode + decode)
  const arr64 = encodeJsValue(nestedArray(BIN_VALUE_MAX_DEPTH, 7));
  BinReader.validate(arr64);
  assert.ok(Array.isArray(decodeJsValue(arr64)));

  const map64 = encodeJsValue(nestedMap(BIN_VALUE_MAX_DEPTH, 'ok'));
  BinReader.validate(map64);
  assert.equal(typeof decodeJsValue(map64), 'object');

  // Depth 65 fails on encode and decode for both arrays and maps
  expectKind('maxDepth', () => encodeJsValue(nestedArray(BIN_VALUE_MAX_DEPTH + 1)), 'encode array depth 65');
  expectKind('maxDepth', () => encodeJsValue(nestedMap(BIN_VALUE_MAX_DEPTH + 1)), 'encode map depth 65');
  expectKind('maxDepth', () => decodeJsValue(rawNestedArrays(BIN_VALUE_MAX_DEPTH + 1)), 'decode array depth 65');
  expectKind('maxDepth', () => decodeJsValue(rawNestedMaps(BIN_VALUE_MAX_DEPTH + 1)), 'decode map depth 65');

  // Hostile cyclic objects & arrays fail with maxDepth instead of RangeError stack overflow
  const cyclicObj: Record<string, unknown> = {};
  cyclicObj['self'] = cyclicObj;
  expectKind('maxDepth', () => encodeJsValue(cyclicObj), 'cyclic object');

  const cyclicArr: unknown[] = [];
  cyclicArr.push(cyclicArr);
  expectKind('maxDepth', () => encodeJsValue(cyclicArr), 'cyclic array');

  // Embedded DynamicValue depth check on writeTo(w)
  const dv64 = DynamicValue.of(arr64);
  assert.equal(dv64.containerDepth, BIN_VALUE_MAX_DEPTH);
  const dv63 = DynamicValue.of(encodeJsValue(nestedArray(BIN_VALUE_MAX_DEPTH - 1, 1)));
  assert.equal(dv63.containerDepth, BIN_VALUE_MAX_DEPTH - 1);

  const wReject = new BinWriter();
  wReject.beginMap(1);
  wReject.writeMapKey('v');
  const lenBefore = wReject.length;
  expectKind('maxDepth', () => dv64.writeTo(wReject), 'depth-64 DynamicValue inside open map exceeds maxDepth');
  assert.equal(wReject.length, lenBefore, 'rejected writeTo writes nothing');

  const wOk = new BinWriter();
  const mOk = wOk.beginMap(1);
  wOk.writeMapKey('v');
  dv63.writeTo(wOk);
  wOk.end(mOk);
  BinReader.validate(wOk.bytes);
  assert.equal(wOk.containerDepth, 0);

  // Depth-64 container holding a scalar DynamicValue (containerDepth 0) succeeds
  const wScalar = new BinWriter();
  const marks: BinMark[] = [];
  for (let i = 0; i < BIN_VALUE_MAX_DEPTH; i += 1) marks.push(wScalar.beginArray(1));
  DynamicValue.NULL.writeTo(wScalar);
  for (const m of marks.reverse()) wScalar.end(m);
  BinReader.validate(wScalar.bytes);

  // `writeJsValue` respects enclosing `BinWriter.containerDepth` (no silent depth-65 output)
  const wJsReject = new BinWriter();
  wJsReject.beginMap(1);
  wJsReject.writeMapKey('deep');
  expectKind(
    'maxDepth',
    () => writeJsValue(wJsReject, nestedArray(BIN_VALUE_MAX_DEPTH, 1)),
    'writeJsValue depth-64 inside open map exceeds maxDepth',
  );
  const wJsOk = new BinWriter();
  const mJsOk = wJsOk.beginMap(1);
  wJsOk.writeMapKey('ok');
  writeJsValue(wJsOk, nestedArray(BIN_VALUE_MAX_DEPTH - 1, 1));
  wJsOk.end(mJsOk);
  assert.equal(BinReader.validate(wJsOk.bytes), BIN_VALUE_MAX_DEPTH);

  const wJsLeaf = new BinWriter();
  const leafMarks: BinMark[] = [];
  for (let i = 0; i < BIN_VALUE_MAX_DEPTH; i += 1) leafMarks.push(wJsLeaf.beginArray(1));
  expectKind('maxDepth', () => writeJsValue(wJsLeaf, []), 'empty array at containerDepth 64 rejected');
  writeJsValue(wJsLeaf, 42);
  for (const m of leafMarks.reverse()) wJsLeaf.end(m);
  assert.equal(BinReader.validate(wJsLeaf.bytes), BIN_VALUE_MAX_DEPTH);
});

test('js_value: trailingBytes, nested container slack, container escape, and EOF-before-maxDepth', () => {
  // Top-level trailing bytes
  const valid = encodeJsValue(42);
  const withTrail = new Uint8Array(valid.length + 1);
  withTrail.set(valid);
  expectKind('trailingBytes', () => decodeJsValue(withTrail), 'top-level trailing byte');

  // Top-level container slack
  expectKind('trailingBytes', () => decodeJsValue(Uint8Array.of(6, 6, 0, 0, 0, 1, 0, 0, 0, 0, 0)), 'array slack');

  // Nested array slack disguised inside outer array (must fail even though total length matches!)
  const nestedSlack = Uint8Array.of(
    6, 15, 0, 0, 0, 1, 0, 0, 0,
    6, 6, 0, 0, 0, 1, 0, 0, 0, 0, 0,
  );
  expectKind('trailingBytes', () => decodeJsValue(nestedSlack), 'nested array slack');

  // Nested map slack disguised inside outer array
  const nestedMapSlack = Uint8Array.of(
    6, 14, 0, 0, 0, 1, 0, 0, 0,
    7, 5, 0, 0, 0, 0, 0, 0, 0, 0,
  );
  expectKind('trailingBytes', () => decodeJsValue(nestedMapSlack), 'nested map slack');

  // Container escape: inner array claims payload=5 (1 byte after count), holds a 9-byte I64
  const escape = Uint8Array.of(6, 5, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0);
  expectKind('truncated', () => decodeJsValue(escape), 'container overrun');

  // At depth 64, an unexpected tag / EOF reports unexpectedTag / truncated (not maxDepth)
  const arr64 = encodeJsValue(nestedArray(BIN_VALUE_MAX_DEPTH, null));
  const r64 = new BinReader(arr64);
  const descend = (remaining: number): void => {
    if (remaining === 0) {
      expectKind('unexpectedTag', () => r64.readArray(() => null), 'wrong tag at depth 64');
      assert.equal(r64.readNull(), true);
      expectKind('truncated', () => r64.readArray(() => null), 'EOF at depth 64');
      return;
    }
    r64.readArray(() => descend(remaining - 1));
  };
  descend(BIN_VALUE_MAX_DEPTH);

  // Streaming `writeJsValue` / `readJsValue` compose cleanly on one writer/reader
  const w = new BinWriter();
  writeJsValue(w, 'first');
  writeJsValue(w, 2n ** 60n);
  const r = new BinReader(w.bytes);
  assert.equal(readJsValue(r), 'first');
  assert.equal(readJsValue(r), 2n ** 60n);
  r.requireEnd();
});

test('hybrid_runtime: DynamicValue.read/writeTo, checked ints, readResultOk, and WASM call guards', () => {
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
  assert.equal(dvOut.containerDepth, 2);
  r.requireEnd();

  // Range rejections on write & read
  assert.throws(() => writeIntInRange(new BinWriter(), 256, 0, 255), RangeError);
  assert.throws(() => writeIntInRange(new BinWriter(), 1.5, 0, 255), RangeError);
  assert.throws(() => writeIntInRange(new BinWriter(), NaN, 0, 255), RangeError);
  assert.throws(() => writeIntInRange(new BinWriter(), 0, 10, 5), RangeError);
  assert.throws(() => writeI64Checked(new BinWriter(), 2n ** 63n), RangeError);
  assert.throws(() => writeU64Checked(new BinWriter(), -1n), RangeError);
  assert.throws(() => writeU64Checked(new BinWriter(), 2n ** 64n), RangeError);
  assert.throws(() => readIntInRange(new BinReader(encodeJsValue(300)), 0, 255), RangeError);
  assert.throws(() => readIntInRange(new BinReader(encodeJsValue(0)), 0, 2 ** 53), RangeError);

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

  const errTrailW = new BinWriter();
  errTrailW.writeBool(false);
  errTrailW.writeStr('bad input');
  errTrailW.writeNull();
  expectKind('trailingBytes', () => readResultOk(new BinReader(errTrailW.bytes)), 'Result Err trailing bytes');

  // Mock WASM linear memory with bump allocator + live-allocation leak tracking + memory.grow()
  const memory = new WebAssembly.Memory({ initial: 1, maximum: 8 });
  let bump = 16;
  const live = new Map<number, number>();
  const freedPtrs: number[] = [];
  let allocCount = 0;
  const wasm: HybridWasmExports = {
    memory,
    nve_hybrid_alloc(len: number): number {
      allocCount += 1;
      const ptr = bump;
      bump += (len + 7) & ~7;
      live.set(ptr, len);
      return ptr;
    },
    nve_hybrid_free(ptr: number, len: number): void {
      freedPtrs.push(ptr);
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

  // wasmCallBin across `memory.grow(1)` with zero leaks
  const resp = wasmCallBin(wasm, 'Demo', handle, Uint8Array.of(1, 2, 3), (_h, reqPtr, reqLen, outLenPtr) => {
    assert.deepEqual(new Uint8Array(wasm.memory.buffer, reqPtr, reqLen), Uint8Array.of(1, 2, 3));
    wasm.memory.grow(1);
    const out = Uint8Array.of(9, 8, 7, 6);
    const respPtr = wasm.nve_hybrid_alloc(out.length);
    new Uint8Array(wasm.memory.buffer, respPtr, out.length).set(out);
    new DataView(wasm.memory.buffer).setUint32(outLenPtr, out.length, true);
    return respPtr;
  });
  assert.deepEqual(resp, Uint8Array.of(9, 8, 7, 6));
  assert.equal(live.size, 0, 'zero WASM memory leaks after call');

  // wasmCopyIn with source bytes aliasing wasm.memory.buffer while alloc grows linear memory
  const aliasedOffset = 1024;
  new Uint8Array(wasm.memory.buffer, aliasedOffset, 4).set(Uint8Array.of(0xaa, 0xbb, 0xcc, 0xdd));
  const aliasedView = new Uint8Array(wasm.memory.buffer, aliasedOffset, 4);
  const growOnAllocWasm: HybridWasmExports = {
    memory: wasm.memory,
    nve_hybrid_alloc(len: number): number {
      wasm.memory.grow(1); // detaches aliasedView's underlying ArrayBuffer!
      return wasm.nve_hybrid_alloc(len);
    },
    nve_hybrid_free: wasm.nve_hybrid_free,
  };
  const copiedPtr = wasmCopyIn(growOnAllocWasm, aliasedView);
  assert.deepEqual(new Uint8Array(wasm.memory.buffer, copiedPtr, 4), Uint8Array.of(0xaa, 0xbb, 0xcc, 0xdd));
  wasm.nve_hybrid_free(copiedPtr, 4);
  assert.equal(live.size, 0);

  // wasmSetPropsBin with an aliased wasm.memory.buffer view across memory.grow():
  // input view detaches (byteLength becomes 0), yet cached length is passed to setPropsFn and free!
  const aliasedPropsOff = 1040;
  new Uint8Array(wasm.memory.buffer, aliasedPropsOff, 4).set(Uint8Array.of(0x11, 0x22, 0x33, 0x44));
  const aliasedPropsView = new Uint8Array(wasm.memory.buffer, aliasedPropsOff, 4);
  let sawPropsLen = -1;
  let sawPropsBytes = new Uint8Array(0);
  wasmSetPropsBin(growOnAllocWasm, 'Demo', handle, aliasedPropsView, (_h, ptr, len) => {
    assert.equal(aliasedPropsView.byteLength, 0, 'aliased payload must be detached by memory.grow()');
    sawPropsLen = len;
    sawPropsBytes = new Uint8Array(wasm.memory.buffer, ptr, len).slice();
    return HybridStatus.Ok;
  });
  assert.equal(sawPropsLen, 4);
  assert.deepEqual(sawPropsBytes, Uint8Array.of(0x11, 0x22, 0x33, 0x44));
  assert.equal(live.size, 0, 'wasmSetPropsBin freed aliased payload with cached len 4');

  // wasmCallBin with an aliased wasm.memory.buffer view across memory.grow():
  // input view detaches (byteLength becomes 0), yet cached length is passed to callFn and free!
  const aliasedReqOff = 1056;
  new Uint8Array(wasm.memory.buffer, aliasedReqOff, 4).set(Uint8Array.of(0x55, 0x66, 0x77, 0x88));
  const aliasedReqView = new Uint8Array(wasm.memory.buffer, aliasedReqOff, 4);
  let sawReqLen = -1;
  const aliasedResp = wasmCallBin(
    growOnAllocWasm,
    'Demo',
    handle,
    aliasedReqView,
    (_h, reqPtr, reqLen, outLenPtr) => {
      assert.equal(aliasedReqView.byteLength, 0, 'aliased req must be detached by memory.grow()');
      sawReqLen = reqLen;
      assert.deepEqual(
        new Uint8Array(wasm.memory.buffer, reqPtr, reqLen),
        Uint8Array.of(0x55, 0x66, 0x77, 0x88),
      );
      const out = Uint8Array.of(0xfe, 0xed);
      const rp = wasm.nve_hybrid_alloc(out.length);
      new Uint8Array(wasm.memory.buffer, rp, out.length).set(out);
      new DataView(wasm.memory.buffer).setUint32(outLenPtr, out.length, true);
      return rp;
    },
  );
  assert.equal(sawReqLen, 4);
  assert.deepEqual(aliasedResp, Uint8Array.of(0xfe, 0xed));
  assert.equal(live.size, 0, 'wasmCallBin freed aliased req with cached len 4');

  // wasmCopyIn rejects oversized payload (> u32) BEFORE calling alloc
  let allocCalled = false;
  const neverAllocWasm: HybridWasmExports = {
    memory: wasm.memory,
    nve_hybrid_alloc: () => {
      allocCalled = true;
      return 16;
    },
    nve_hybrid_free: () => {},
  };
  assert.throws(() => wasmCopyIn(neverAllocWasm, { byteLength: 0x1_0000_0000 } as Uint8Array), RangeError);
  assert.equal(allocCalled, false);

  // wasmCopyIn OOB pointer from alloc throws HybridError and NEVER passes corrupt ptr to free
  let badFreeCalls = 0;
  const oobAllocWasm: HybridWasmExports = {
    memory: new WebAssembly.Memory({ initial: 1 }),
    nve_hybrid_alloc: () => 0x0fff_0000,
    nve_hybrid_free: () => {
      badFreeCalls += 1;
    },
  };
  expectHybridStatus(HybridStatus.Err, () => wasmCopyIn(oobAllocWasm, Uint8Array.of(1, 2)), 'OOB alloc ptr');
  assert.equal(badFreeCalls, 0, 'must never pass OOB alloc ptr to free');

  // wasmCopyIn frees valid ptr when .set(src) throws (non-vacuous: alloc runs and ptr is freed)
  const detached = new Uint8Array(new ArrayBuffer(4));
  structuredClone(detached.buffer, { transfer: [detached.buffer] });
  Object.defineProperty(detached, 'byteLength', { value: 4 });
  const allocsBefore = allocCount;
  const freesBefore = freedPtrs.length;
  assert.throws(() => wasmCopyIn(wasm, detached), TypeError);
  assert.equal(allocCount, allocsBefore + 1, 'must reach nve_hybrid_alloc before .set(src) throws');
  assert.equal(freedPtrs.length, freesBefore + 1, 'must free allocated ptr when .set(src) throws');
  assert.equal(live.size, 0, 'wasmCopyIn freed valid ptr after .set() threw');

  // wasmCallBin frees both reqPtr and outLenPtr when callFn throws
  assert.throws(
    () =>
      wasmCallBin(wasm, 'Demo', handle, Uint8Array.of(1, 2, 3), () => {
        throw new Error('boom inside wasm');
      }),
    (e: unknown) => e instanceof Error && e.message === 'boom inside wasm',
  );
  assert.equal(live.size, 0, 'zero WASM memory leaks when callFn throws');

  // wasmCallBin out-of-bounds respPtr throws HybridError, frees reqPtr & outLenPtr, NEVER frees OOB respPtr
  const corruptRespPtr = 0x0fff_0000;
  freedPtrs.length = 0;
  expectHybridStatus(
    HybridStatus.Err,
    () =>
      wasmCallBin(wasm, 'Demo', handle, Uint8Array.of(5), (_h, _rp, _rl, olp) => {
        new DataView(wasm.memory.buffer).setUint32(olp, 16, true);
        return corruptRespPtr;
      }),
    'OOB respPtr',
  );
  assert.equal(live.size, 0, 'reqPtr and outLenPtr freed on OOB respPtr');
  assert.ok(!freedPtrs.includes(corruptRespPtr), 'corrupt respPtr must never be passed to free');

  // wasmCallBin non-null respPtr with respLen === 0: in-bounds returns empty (no free(ptr,0)); OOB throws
  freedPtrs.length = 0;
  const emptyDangling = wasmCallBin(wasm, 'Demo', handle, Uint8Array.of(1), () => 1);
  assert.equal(emptyDangling.byteLength, 0);
  assert.ok(!freedPtrs.includes(1), 'zero-length dangling ptr must never be passed to free');
  expectHybridStatus(
    HybridStatus.Err,
    () => wasmCallBin(wasm, 'Demo', handle, Uint8Array.of(1), () => corruptRespPtr),
    'OOB respPtr with respLen 0',
  );
  assert.equal(live.size, 0);

  // wasmCallBin outLenPtr OOM and OOB both free reqPtr and never free invalid outLenPtr
  let outLenMode: 'oom' | 'oob' = 'oom';
  let allocStep = 0;
  let invalidFreeCount = 0;
  const badOutLenWasm: HybridWasmExports = {
    memory: wasm.memory,
    nve_hybrid_alloc(len: number): number {
      allocStep += 1;
      if (allocStep % 2 === 1) return wasm.nve_hybrid_alloc(len);
      return outLenMode === 'oom' ? 0 : 0x0fff_0000;
    },
    nve_hybrid_free(ptr: number, len: number): void {
      if (ptr === 0 || ptr === 0x0fff_0000) {
        invalidFreeCount += 1;
        return;
      }
      wasm.nve_hybrid_free(ptr, len);
    },
  };
  expectHybridStatus(
    HybridStatus.Err,
    () => wasmCallBin(badOutLenWasm, 'Demo', handle, Uint8Array.of(7), () => 0),
    'outLenPtr OOM',
  );
  outLenMode = 'oob';
  expectHybridStatus(
    HybridStatus.Err,
    () => wasmCallBin(badOutLenWasm, 'Demo', handle, Uint8Array.of(7), () => 0),
    'outLenPtr OOB',
  );
  assert.equal(live.size, 0, 'reqPtr freed on outLenPtr OOM and OOB');
  assert.equal(invalidFreeCount, 0, 'invalid outLenPtr never passed to free');

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

  // OOM copyIn throws HybridError without corrupting memory[0]
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
  // DynamicValue.read inside a depth-64 container rejects a 65th nested container
  const deepInner = rawNestedArrays(BIN_VALUE_MAX_DEPTH + 1);
  const deepReader = new BinReader(deepInner);
  expectKind(
    'maxDepth',
    () => deepReader.readArray(() => DynamicValue.read(deepReader)),
    'DynamicValue.read inherits reader container depth',
  );
  assert.equal(live.size, 0, 'zero WASM memory leaks');
});
