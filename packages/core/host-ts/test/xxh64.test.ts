import assert from 'node:assert/strict';
import { test } from 'node:test';

import { BinCodecError, Key64, Xxh64Hasher, constId, xxh64, xxh64Primes } from '../src/index.ts';
import { VECTORS, seq, WIRE_SEED } from './golden.ts';

test('golden vectors match Rust nve-hash (one-shot and every chunking)', () => {
  for (const [label, bytes, seed, expected] of VECTORS) {
    assert.equal(xxh64(bytes, seed).toHex(), expected.toHex(), label);
    for (const step of [1, 3, 7, 13, 24, 31, 32, 33, 64, 100]) {
      const h = new Xxh64Hasher(seed);
      for (let at = 0; at < bytes.length; at += step) h.update(bytes.subarray(at, at + step));
      assert.equal(h.digest().toHex(), expected.toHex(), `${label} step ${step}`);
    }
  }
});

test('BigInt cross-check of digests above 2^53', () => {
  const d = xxh64(seq(101), WIRE_SEED);
  assert.equal(d.toBigInt(), 0x277e_092a_8c3d_b1bcn);
  assert.equal(Key64.fromBigInt(d.toBigInt()).toHex(), d.toHex());
  assert.equal(xxh64Primes()[0].toBigInt(), 0x9e37_79b1_85eb_ca87n);
});

test('typed feeds and non-consuming digest', () => {
  const h = new Xxh64Hasher(new Key64(7, 0)).updateU32(0x0403_0201);
  assert.equal(h.digest().toHex(), h.digest().toHex());
  h.updateU64(new Key64(0x0807_0605, 0x0c0b_0a09));
  assert.equal(h.digest().toHex(), xxh64(Uint8Array.of(1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12), new Key64(7, 0)).toHex());
});

test('Key64 halves, conversions and 53-bit guards', () => {
  const k = Key64.of('nve.visual');
  assert.equal(k.toHex(), '33e73115985a4fd6');
  assert.equal(constId('abc').toHex(), '44bc2cf5ad770999');
  assert.ok(new Key64(-1, -1).equals(new Key64(0xffff_ffff, 0xffff_ffff)), 'int32 patterns normalize');
  assert.equal(new Key64(-1, -1).toSignedBigInt(), -1n);
  assert.equal(Key64.fromBigInt(-1n).toHex(), 'ffffffffffffffff');
  assert.equal(Key64.fromBigInt(1n << 64n).toHex(), '0000000000000000', 'wraps mod 2^64');
  assert.equal(new Key64(0xffff_ffff, 0x1f_ffff).toSafeInteger(), Number.MAX_SAFE_INTEGER);
  assert.throws(() => new Key64(0, 0x20_0000).toSafeInteger(), RangeError);
  assert.equal(Key64.fromSafeInteger(-Number.MAX_SAFE_INTEGER).toSignedBigInt(), -BigInt(Number.MAX_SAFE_INTEGER));
  assert.equal(Key64.fromSafeInteger(Number.MAX_SAFE_INTEGER).toBigInt(), BigInt(Number.MAX_SAFE_INTEGER));
  assert.throws(() => Key64.fromSafeInteger(2 ** 53), RangeError);
  assert.throws(() => Key64.fromSafeInteger(1.5), RangeError);
  assert.equal(new Key64(0xffff_ffff, 0x7fff_ffff).toSignedNumber(), 2 ** 63, 'i64::MAX rounds like `as f64`');
  assert.equal(new Key64(0, 0x8000_0000).toSignedNumber(), -(2 ** 63));
  const le = new Uint8Array(9);
  k.writeLE(le, 1);
  assert.ok(Key64.readLE(le, 1).equals(k));
  assert.equal(le[1], 0xd6);
  assert.throws(() => Key64.readLE(le, 2), RangeError);
  assert.equal(String(k), 'Key64(0x33e73115985a4fd6)');
});

test('constId encodes in place and rejects lone surrogates; update ranges are checked', () => {
  assert.equal(constId('héllo wörld — 日本語 🚀').toHex(), '72ac442ff213f592');
  for (const lone of ['\ud800', 'x\udc00', '\ud83d', '\ude80\ud83d']) {
    assert.throws(() => constId(lone), (e: unknown) => e instanceof BinCodecError && e.kind === 'invalidUtf8');
  }
  const bytes = seq(101);
  assert.equal(new Xxh64Hasher(WIRE_SEED).update(bytes, 0, 50).update(bytes, 50, 101).digest().toHex(), '277e092a8c3db1bc');
  for (const [a, b] of [[NaN, 1], [0, 0.5], [2, 1], [0, 102], [-1, 1]] as const) {
    assert.throws(() => new Xxh64Hasher().update(bytes, a, b), RangeError, `range ${a}..${b}`);
  }
});
