// `[T; N]` (`Array<T>`) over the generated TypeScript codecs: lossless round trips (nested and
// `N = 1` dimensions, absent options), `RangeError` on encoding a wrong length, and
// `truncated` / `tooLarge` on decoding one, inner rows included.
import assert from 'node:assert/strict';
import { BinCodecError, BinReader, BinWriter } from '@nve/host';
import type { BinErrorKind } from '@nve/host';
import {
  FixedKind,
  decodeFixedBundle,
  decodeFixedPod,
  encodeFixedBundle,
  encodeFixedPod,
  encodeFixedPoint,
} from './fixed_codec_hybrid.generated.ts';
import type { FixedBundle, FixedPod } from './fixed_codec_hybrid.generated.ts';

const bundle: FixedBundle = {
  label: 'fixed',
  coords: [1, 0xffff_ffff, 0, 42],
  points: [{ x: -7, y: 8 }, { x: 2147483647, y: -2147483648 }],
  matrix: [[1, 2], [3, 4], [5, 6]],
  tag: [9, 0, 255],
};
const flags = [Array<boolean>(33).fill(false), Array<boolean>(33).fill(false)];
flags[0]![0] = true;
flags[0]![32] = true;
flags[1]![5] = true;
const pod: FixedPod = {
  v: [1.5, -2, 0, 3.25],
  points: [{ x: 1, y: 2 }, { x: 3, y: 4 }],
  kinds: [FixedKind.Gamma, FixedKind.Beta],
  grid: [[10, 11], [12, 13], [14, 15]],
  flags,
  one: [21],
  cells: [[{ x: 5, y: 6 }], [{ x: 7, y: 8 }]],
};

for (const value of [bundle, { ...bundle, tag: null }]) {
  const w = new BinWriter();
  encodeFixedBundle(w, value);
  const r = new BinReader(w.bytes);
  assert.deepEqual(decodeFixedBundle(r), value);
  r.requireEnd();
}
{
  const w = new BinWriter();
  encodeFixedPod(w, pod);
  const r = new BinReader(w.bytes);
  assert.deepEqual(decodeFixedPod(r), pod);
  r.requireEnd();
}

assert.throws(() => encodeFixedBundle(new BinWriter(), { ...bundle, matrix: [[1, 2], [3], [5, 6]] }), RangeError);
assert.throws(() => encodeFixedPod(new BinWriter(), { ...pod, one: [] }), RangeError);
assert.throws(() => encodeFixedPod(new BinWriter(), { ...pod, cells: [[{ x: 0, y: 0 }], []] }), RangeError);

/** `bundle` encoded with a `matrix` whose row `i` has `lens[i]` integers. */
function matrixWith(lens: readonly number[]): Uint8Array {
  const w = new BinWriter();
  const map = w.beginMap(5);
  w.writeMapKey('label');
  w.writeStr(bundle.label);
  w.writeMapKey('coords');
  const coords = w.beginArray(4);
  for (const c of bundle.coords) w.writeI64(BigInt(c));
  w.end(coords);
  w.writeMapKey('points');
  const points = w.beginArray(2);
  for (const p of bundle.points) encodeFixedPoint(w, p);
  w.end(points);
  w.writeMapKey('tag');
  w.writeNull();
  w.writeMapKey('matrix');
  const rows = w.beginArray(lens.length);
  for (const len of lens) {
    const cols = w.beginArray(len);
    for (let i = 0; i < len; i += 1) w.writeI64(BigInt(i));
    w.end(cols);
  }
  w.end(rows);
  w.end(map);
  return w.bytes;
}

const cases: ReadonlyArray<readonly [readonly number[], BinErrorKind | null]> = [
  [[2, 2, 2], null],
  [[2, 1, 2], 'truncated'],
  [[2, 3, 2], 'tooLarge'],
  [[2, 2], 'truncated'],
  [[2, 2, 2, 2], 'tooLarge'],
];
for (const [lens, kind] of cases) {
  const decode = (): FixedBundle => decodeFixedBundle(new BinReader(matrixWith(lens)));
  if (kind === null) {
    assert.deepEqual(decode().matrix, [[0, 1], [0, 1], [0, 1]]);
  } else {
    assert.throws(decode, (e: unknown) => e instanceof BinCodecError && e.kind === kind);
  }
}

console.log('ts fixed array ok');
