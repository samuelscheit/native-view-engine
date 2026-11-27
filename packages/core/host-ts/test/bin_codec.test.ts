import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  BIN_VALUE_MAX_DEPTH,
  BinCodecError,
  BinReader,
  BinTag,
  BinWriter,
  DynamicValue,
  HostStatus,
  HybridError,
  HybridStatus,
  Key64,
  isValidUtf8,
} from '../src/index.ts';
import type { BinErrorKind } from '../src/index.ts';
import { GOLDEN_BIN_VALUE as GOLDEN } from './golden.ts';

function expectKind(kind: BinErrorKind, fn: () => unknown, what: string): void {
  assert.throws(fn, (e: unknown) => e instanceof BinCodecError && e.kind === kind, what);
}

function nested(depth: number): Uint8Array {
  const w = new BinWriter();
  const marks: number[] = [];
  for (let i = 0; i < depth; i++) marks.push(w.beginArray(1));
  w.writeNull();
  for (const m of marks.reverse()) w.end(m);
  return w.toBytes();
}

test('golden NveBinValue encodes byte-identically and decodes', () => {
  const w = new BinWriter(4); // forces growth
  const map = w.beginMap(1);
  w.writeMapKey('k');
  const arr = w.beginArray(6);
  w.writeI64(1n);
  w.writeStr('é');
  w.writeNull();
  w.writeBool(true);
  w.writeF64(1.5);
  w.writeBytes(Uint8Array.of(0xff));
  w.end(arr);
  w.end(map);
  assert.deepEqual(w.toBytes(), GOLDEN);
  BinReader.validate(GOLDEN);
  const r = new BinReader(GOLDEN);
  assert.equal(r.beginMap(), 1);
  assert.equal(r.readMapKey(), 'k');
  assert.equal(r.beginArray(), 6);
  assert.equal(r.readI64(), 1n);
  assert.equal(r.readStr(), 'é');
  assert.equal(r.readNull(), true);
  assert.equal(r.readNull(), false);
  assert.equal(r.readBool(), true);
  assert.equal(r.readF64(), 1.5);
  assert.deepEqual(r.readBytes(), Uint8Array.of(0xff));
  assert.ok(r.atEnd);
});

test('primitives and exact 64-bit values beyond 2^53', () => {
  const w = new BinWriter();
  w.u8(0xaa);
  w.u16(0xbeef);
  w.u32(0xdead_beef);
  w.u64(new Key64(0x89ab_cdef, 0x0123_4567));
  w.i32(-2);
  w.f32(-0);
  w.f64(Infinity);
  w.writeI64(-7n);
  w.writeI64(-(2n ** 63n));
  w.writeI64(2n ** 63n - 1n);
  w.writeI64Bits(new Key64(0x0000_0001, 0x0020_0000)); // 2^53 + 1
  w.writeF64(NaN);
  const r = new BinReader(w.bytes);
  assert.equal(r.u8(), 0xaa);
  assert.equal(r.u16(), 0xbeef);
  assert.equal(r.u32(), 0xdead_beef);
  assert.equal(r.u64().toBigInt(), 0x0123_4567_89ab_cdefn);
  assert.equal(r.i32(), -2);
  assert.ok(Object.is(r.f32(), -0));
  assert.equal(r.f64(), Infinity);
  assert.equal(r.readF64(), -7, 'readF64 widens i64');
  assert.equal(r.readI64(), -(2n ** 63n));
  assert.equal(r.readI64(), 2n ** 63n - 1n);
  assert.equal(r.readI64(), 2n ** 53n + 1n, 'not rounded to 2^53');
  assert.ok(Number.isNaN(r.readF64()));
  expectKind('truncated', () => r.u8(), 'read past end');
  expectKind('truncated', () => new BinReader(new Uint8Array(2)).bytesView(-1), 'negative take');
  // A subarray with a non-zero byteOffset reads relative to its own start.
  const backing = Uint8Array.of(9, 9, BinTag.Null, 9);
  assert.equal(new BinReader(backing.subarray(2, 3)).readNull(), true);
});

test('rejects malformed input exactly like Rust', () => {
  expectKind('invalidBool', () => BinReader.validate(Uint8Array.of(1, 2)), 'bool 2');
  expectKind('invalidTag', () => BinReader.validate(Uint8Array.of(8)), 'tag 8');
  expectKind('invalidUtf8', () => BinReader.validate(Uint8Array.of(4, 2, 0, 0, 0, 0xc0, 0xaf)), 'overlong');
  expectKind('truncated', () => BinReader.validate(Uint8Array.of(4, 0xff, 0xff, 0xff, 0xff, 0x61)), 'huge len');
  expectKind('truncated', () => BinReader.validate(Uint8Array.of(6, 4, 0, 0, 0, 0xff, 0xff, 0xff, 0xff)), 'count bomb');
  expectKind('truncated', () => BinReader.validate(Uint8Array.of(7, 8, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0)), 'map bomb');
  expectKind('trailingBytes', () => BinReader.validate(Uint8Array.of(6, 6, 0, 0, 0, 1, 0, 0, 0, 0, 0)), 'slack');
  expectKind('trailingBytes', () => BinReader.validate(Uint8Array.of(0, 0)), 'two values');
  expectKind('truncated', () => BinReader.validate(Uint8Array.of(6, 5, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0)), 'overrun');
  expectKind('unexpectedTag', () => new BinReader(Uint8Array.of(0)).readI64(), 'wrong tag');
  BinReader.validate(nested(BIN_VALUE_MAX_DEPTH));
  expectKind('maxDepth', () => BinReader.validate(nested(BIN_VALUE_MAX_DEPTH + 1)), 'depth 65');
  // A leading BOM is data (Rust keeps it), not something to strip.
  const bom = new BinWriter();
  bom.writeStr('\ufeffx');
  assert.equal(new BinReader(bom.bytes).readStr(), '\ufeffx');
});

test('every truncated prefix fails; raw values skip exactly; fuzz never escapes', () => {
  for (let n = 0; n < GOLDEN.length; n++) {
    const prefix = GOLDEN.subarray(0, n);
    assert.throws(() => BinReader.validate(prefix), BinCodecError, `prefix ${n}`);
    assert.throws(() => new BinReader(prefix).skipValue(), BinCodecError, `skip prefix ${n}`);
  }
  const two = new Uint8Array(GOLDEN.length + 1);
  two.set(GOLDEN);
  const r = new BinReader(two);
  assert.equal(r.readRawValue().length, GOLDEN.length);
  assert.ok(r.readNull() && r.atEnd);

  let s = 0x9e37_79b9;
  const next = (): number => {
    s ^= s << 13;
    s ^= s >>> 17;
    s ^= s << 5;
    return s >>> 0;
  };
  for (let iter = 0; iter < 20000; iter++) {
    let buf: Uint8Array;
    if (iter % 2 === 0) {
      buf = GOLDEN.slice();
      buf[next() % buf.length] = next() & 0xff;
    } else {
      buf = Uint8Array.from({ length: next() % 48 }, () => next() % 9);
    }
    let valid = true;
    try {
      BinReader.validate(buf);
    } catch (e) {
      assert.ok(e instanceof BinCodecError, `only BinCodecError escapes: ${String(e)}`);
      valid = false;
    }
    try {
      const fr = new BinReader(buf);
      fr.readRawValue();
      if (valid) assert.ok(fr.atEnd);
    } catch (e) {
      assert.ok(e instanceof BinCodecError && !valid, `skip of valid input threw ${String(e)}`);
    }
  }
});

test('hybrid prelude', () => {
  assert.ok(DynamicValue.NULL.isNull);
  const v = DynamicValue.of(GOLDEN);
  assert.equal(v.tag, BinTag.Map);
  assert.ok(v.equals(DynamicValue.of(GOLDEN.slice())));
  v.bytes[0] = 0; // a copy: the value stays intact
  assert.equal(v.tag, BinTag.Map);
  expectKind('truncated', () => DynamicValue.of(GOLDEN.subarray(0, 10)), 'dynamic prefix');
  expectKind('truncated', () => DynamicValue.of(new Uint8Array(0)), 'dynamic empty');
  HybridError.checkSchema(new Key64(1, 2), new Key64(1, 2));
  assert.throws(
    () => HybridError.checkSchema(new Key64(1, 2), new Key64(1, 3)),
    (e: unknown) => e instanceof HybridError && e.status === HybridStatus.SchemaMismatch,
  );
  assert.equal(HostStatus.InternalError, -8);
});

test('strict UTF-8 validation', () => {
  assert.ok(isValidUtf8(new TextEncoder().encode('plain é — 🚀 \u{10ffff}')));
  for (const bad of [
    [0xc0, 0xaf], [0xe0, 0x80, 0xaf], [0xf0, 0x80, 0x80, 0xaf], [0xed, 0xa0, 0x80],
    [0xf4, 0x90, 0x80, 0x80], [0xe2, 0x80], [0xe2, 0x28, 0xa1], [0x80], [0xf5, 0x80, 0x80, 0x80],
  ]) {
    assert.ok(!isValidUtf8(Uint8Array.from(bad)), `invalid ${bad.join(',')}`);
  }
});

test('hardening: container limit, BOM, lone surrogates, stale marks, NaN counts', () => {
  expectKind('truncated', () => BinReader.validate(Uint8Array.of(6, 5, 0, 0, 0, 1, 0, 0, 0, 4, 2, 0, 0, 0, 0xc0, 0xaf)), 'escape container');
  expectKind('truncated', () => BinReader.validate(Uint8Array.of(6, 10, 0, 0, 0, 1, 0, 0, 0, 6, 9, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0)), 'nested escape');
  for (const s of ['\ufeff', '\ufeffx', '\ufeff\ufeffx']) {
    const w = new BinWriter();
    const m = w.beginMap(1);
    w.writeMapKey(s);
    w.writeStr(s);
    w.end(m);
    const r = new BinReader(w.bytes);
    assert.equal(r.beginMap(), 1);
    assert.equal(r.readMapKey(), s);
    assert.equal(r.readStr(), s);
  }
  for (const lone of ['\ud800', 'a\udc00', '\ud83d', '\ude80\ud83d']) {
    const w = new BinWriter();
    expectKind('invalidUtf8', () => w.writeMapKey(lone), 'key lone surrogate');
    expectKind('invalidUtf8', () => w.writeStr(lone), 'str lone surrogate');
    assert.equal(w.length, 0, 'nothing written (not even the tag)');
  }
  const w = new BinWriter();
  const mark = w.beginArray(1);
  w.writeNull();
  w.clear();
  expectKind('invalidMark', () => w.end(mark), 'stale mark');
  w.writeNull();
  expectKind('invalidMark', () => w.end(0), 'mark without container');
  expectKind('invalidMark', () => w.end(NaN), 'NaN mark');
  // NaN / fractional / Infinity counts never poison the cursor.
  const r = new BinReader(GOLDEN);
  for (const bad of [NaN, 0.5, Infinity, -1]) {
    expectKind('truncated', () => r.bytesView(bad), `bytesView ${bad}`);
    expectKind('truncated', () => r.utf8(bad), `utf8 ${bad}`);
  }
  assert.equal(r.position, 0, 'cursor untouched');
  BinReader.validate(GOLDEN.slice());
  assert.throws(() => isValidUtf8(GOLDEN, 0.5, 2), RangeError);
  assert.throws(() => isValidUtf8(GOLDEN, 2, 1), RangeError);
  assert.throws(() => isValidUtf8(GOLDEN, 0, GOLDEN.length + 1), RangeError);
});
