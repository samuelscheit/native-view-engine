// Generated flat `u32`-word `nve.res.draw_list` codec (`nve_draw_list.generated.ts`):
// golden word layout, all 8 opcodes, subarray non-zero `byteOffset`, strict validation
// (including negative radius/width & non-4B lengths), and `MAX_CLIP_DEPTH = 64` balancing.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { Key64 } from '@nve/host';
import {
  NVE_DRAW_LIST_HEADER_SIZE,
  NVE_DRAW_LIST_KIND,
  NVE_DRAW_LIST_MAX_CLIP_DEPTH,
  NVE_DRAW_LIST_MAX_DRAW_OPS,
  NVE_DRAW_LIST_MAX_POLYLINE_POINTS,
  NveDrawListError,
  NveDrawListOpcode,
  decodeDrawList,
  validateDrawList,
  type NveDrawListSink,
} from '@nve/host/draw-list';

function f32Bits(v: number): number {
  const buf = new ArrayBuffer(4);
  const view = new DataView(buf);
  view.setFloat32(0, v, true);
  return view.getUint32(0, true);
}

function packWords(words: readonly number[]): Uint8Array {
  const out = new Uint8Array(words.length * 4);
  const view = new DataView(out.buffer);
  for (let i = 0; i < words.length; i++) {
    view.setUint32(i * 4, words[i]! >>> 0, true);
  }
  return out;
}

function packRgba(r: number, g: number, b: number, a = 255): number {
  return ((r & 0xff) | ((g & 0xff) << 8) | ((b & 0xff) << 16) | ((a & 0xff) << 24)) >>> 0;
}

class RecordingSink implements NveDrawListSink {
  readonly ops: string[] = [];
  onClear(color: number): void {
    this.ops.push(`clear:${color}`);
  }
  onFillRect(x: number, y: number, w: number, h: number, color: number): void {
    this.ops.push(`rect:${x},${y},${w},${h},${color}`);
  }
  onFillRoundedRect(x: number, y: number, w: number, h: number, radius: number, color: number): void {
    this.ops.push(`rrect:${x},${y},${w},${h},${radius},${color}`);
  }
  onFillCircle(cx: number, cy: number, radius: number, color: number): void {
    this.ops.push(`circle:${cx},${cy},${radius},${color}`);
  }
  onStrokeLine(x0: number, y0: number, x1: number, y1: number, width: number, color: number): void {
    this.ops.push(`line:${x0},${y0},${x1},${y1},${width},${color}`);
  }
  onStrokePolylineStart(count: number, width: number, color: number): void {
    this.ops.push(`poly:${count},${width},${color}`);
  }
  onStrokePolylinePoint(index: number, x: number, y: number): void {
    this.ops.push(`pt${index}:${x},${y}`);
  }
  onStrokePolylineEnd(): void {
    this.ops.push('polyEnd');
  }
  onClipRect(x: number, y: number, w: number, h: number): void {
    this.ops.push(`clip:${x},${y},${w},${h}`);
  }
  onRestoreClip(): void {
    this.ops.push('restore');
  }
}

function buildAll8Ops(): Uint8Array {
  const c1 = packRgba(1, 2, 3, 4);
  const white = packRgba(255, 255, 255, 255);
  return packWords([
    8,
    NveDrawListOpcode.Clear, c1,
    NveDrawListOpcode.FillRect, f32Bits(1), f32Bits(2), f32Bits(3), f32Bits(4), white,
    NveDrawListOpcode.FillRoundedRect, f32Bits(0), f32Bits(0), f32Bits(10), f32Bits(10), f32Bits(2.5), white,
    NveDrawListOpcode.FillCircle, f32Bits(5), f32Bits(6), f32Bits(3), white,
    NveDrawListOpcode.StrokeLine, f32Bits(0), f32Bits(0), f32Bits(3), f32Bits(4), f32Bits(2), white,
    NveDrawListOpcode.StrokePolyline, f32Bits(1.5), white, 3,
    f32Bits(0), f32Bits(0), f32Bits(5), f32Bits(7.5), f32Bits(-2), f32Bits(12),
    NveDrawListOpcode.ClipRect, f32Bits(1), f32Bits(2), f32Bits(30), f32Bits(40),
    NveDrawListOpcode.RestoreClip,
  ]);
}

test('generated DrawList decodes all 8 golden opcodes from subarray with non-zero byteOffset', () => {
  assert.ok(NVE_DRAW_LIST_KIND.equals(Key64.of('nve.res.draw_list')));
  assert.equal(NVE_DRAW_LIST_HEADER_SIZE, 4);
  assert.equal(NVE_DRAW_LIST_MAX_DRAW_OPS, 65536);
  assert.equal(NVE_DRAW_LIST_MAX_POLYLINE_POINTS, 16384);
  assert.equal(NVE_DRAW_LIST_MAX_CLIP_DEPTH, 64);

  const all8 = buildAll8Ops();
  assert.equal(all8.byteLength, 44 * 4);

  const padded = new Uint8Array(all8.byteLength + 7);
  padded.set(all8, 3);
  const view = padded.subarray(3, 3 + all8.byteLength);

  const sink = new RecordingSink();
  assert.equal(decodeDrawList(view, sink), null);
  assert.equal(sink.ops.length, 12);
  assert.equal(sink.ops[0], `clear:${packRgba(1, 2, 3, 4)}`);
  assert.equal(sink.ops[5], `poly:3,1.5,${packRgba(255, 255, 255, 255)}`);
  assert.equal(sink.ops[7], 'pt1:5,7.5');
  assert.equal(sink.ops[11], 'restore');

  // Empty stream, 0-pt polyline with -0.0 width, and negative rect coords are valid.
  assert.equal(validateDrawList(packWords([0])), null);
  assert.equal(validateDrawList(packWords([1, NveDrawListOpcode.StrokePolyline, f32Bits(-0), 0, 0])), null);
  assert.equal(
    validateDrawList(packWords([1, NveDrawListOpcode.FillRect, f32Bits(-1), f32Bits(-2), f32Bits(-3), f32Bits(-4), 0])),
    null,
  );
});

test('generated DrawList rejects truncated, trailing, oversized, non-finite, and negative payloads without callbacks', () => {
  const all8 = buildAll8Ops();
  for (let cut = 0; cut < all8.byteLength; cut++) {
    assert.equal(validateDrawList(all8.subarray(0, cut)), NveDrawListError.Truncated, `cut=${cut}`);
  }
  for (let extra = 1; extra <= 4; extra++) {
    const withExtra = new Uint8Array(all8.byteLength + extra);
    withExtra.set(all8, 0);
    assert.equal(validateDrawList(withExtra), NveDrawListError.TrailingBytes, `extra=${extra}`);
  }

  assert.equal(validateDrawList(packWords([NVE_DRAW_LIST_MAX_DRAW_OPS + 1])), NveDrawListError.OpCountExceedsLimit);
  assert.equal(validateDrawList(packWords([0xffff_ffff])), NveDrawListError.OpCountExceedsLimit);
  assert.equal(validateDrawList(packWords([1, 0])), NveDrawListError.UnknownOpcode);
  assert.equal(validateDrawList(packWords([1, 9])), NveDrawListError.UnknownOpcode);
  assert.equal(validateDrawList(packWords([1, 0x0001_0001])), NveDrawListError.UnknownOpcode);
  assert.equal(
    validateDrawList(packWords([1, NveDrawListOpcode.StrokePolyline, f32Bits(1), 0, NVE_DRAW_LIST_MAX_POLYLINE_POINTS + 1])),
    NveDrawListError.SliceLenExceedsLimit,
  );
  assert.equal(
    validateDrawList(packWords([1, NveDrawListOpcode.FillRect, f32Bits(Number.NaN), 0, 0, 0, 0])),
    NveDrawListError.NonFiniteFloat,
  );
  assert.equal(
    validateDrawList(packWords([1, NveDrawListOpcode.FillCircle, 0, 0, f32Bits(Number.NEGATIVE_INFINITY), 0])),
    NveDrawListError.NonFiniteFloat,
  );
  assert.equal(
    validateDrawList(packWords([1, NveDrawListOpcode.FillRoundedRect, 0, 0, f32Bits(10), f32Bits(10), f32Bits(-0.5), 0])),
    NveDrawListError.NegativeFloat,
  );
  assert.equal(
    validateDrawList(packWords([1, NveDrawListOpcode.FillCircle, 0, 0, f32Bits(-1), 0])),
    NveDrawListError.NegativeFloat,
  );
  assert.equal(
    validateDrawList(packWords([1, NveDrawListOpcode.StrokeLine, 0, 0, f32Bits(1), f32Bits(1), f32Bits(-0.25), 0])),
    NveDrawListError.NegativeFloat,
  );
  assert.equal(
    validateDrawList(packWords([1, NveDrawListOpcode.StrokePolyline, f32Bits(-2), 0, 0])),
    NveDrawListError.NegativeFloat,
  );

  // Retired legacy NVE_DRAW1 15-byte header rejected.
  const legacy = new TextEncoder().encode('NVE_DRAW1\x01\x00\x00\x00\x00\x00');
  assert.equal(validateDrawList(legacy), NveDrawListError.OpCountExceedsLimit);

  // Malformed second op invokes zero sink callbacks.
  const zero = new RecordingSink();
  const bad2 = packWords([2, NveDrawListOpcode.Clear, 1, NveDrawListOpcode.FillCircle, 0, 0, f32Bits(-1), 0]);
  assert.equal(decodeDrawList(bad2, zero), NveDrawListError.NegativeFloat);
  assert.equal(zero.ops.length, 0);
});

test('generated DrawList enforces MAX_CLIP_DEPTH = 64 with overflow tracking and finally unwind', () => {
  const cap = NVE_DRAW_LIST_MAX_CLIP_DEPTH;
  const deepWords: number[] = [cap + 2 + 2 + 1];
  for (let i = 0; i < cap + 2; i++) {
    deepWords.push(NveDrawListOpcode.ClipRect, 0, 0, f32Bits(10), f32Bits(10));
  }
  deepWords.push(NveDrawListOpcode.RestoreClip, NveDrawListOpcode.RestoreClip, NveDrawListOpcode.Clear, 7);

  const sink = new RecordingSink();
  assert.equal(decodeDrawList(packWords(deepWords), sink), null);
  assert.equal(sink.ops.length, cap * 2 + 1);
  assert.equal(sink.ops[cap], 'clear:7');
  assert.ok(sink.ops.slice(0, cap).every((op) => op.startsWith('clip:')));
  assert.ok(sink.ops.slice(cap + 1).every((op) => op === 'restore'));

  // Throwing sink still unwinds already-opened clips via finally.
  class ThrowingSink extends RecordingSink {
    override onClipRect(x: number, y: number, w: number, h: number): void {
      super.onClipRect(x, y, w, h);
      if (this.ops.length === 2) {
        throw new Error('sink failed');
      }
    }
  }
  const twoClips = packWords([
    3,
    NveDrawListOpcode.RestoreClip,
    NveDrawListOpcode.ClipRect, f32Bits(1), f32Bits(2), f32Bits(3), f32Bits(4),
    NveDrawListOpcode.ClipRect, f32Bits(1), f32Bits(2), f32Bits(3), f32Bits(4),
  ]);
  const throwing = new ThrowingSink();
  assert.throws(
    () => decodeDrawList(twoClips, throwing),
    (err: unknown) => err instanceof Error && err.message === 'sink failed',
  );
  assert.deepEqual(throwing.ops, ['clip:1,2,3,4', 'clip:1,2,3,4', 'restore', 'restore']);

  // Throw while overflowed (depth == 64, overflow == 2): exactly 64 restores unwind, NOT 66.
  const overflowThrowWords: number[] = [cap + 2 + 1];
  for (let i = 0; i < cap + 2; i++) {
    overflowThrowWords.push(NveDrawListOpcode.ClipRect, 0, 0, f32Bits(10), f32Bits(10));
  }
  overflowThrowWords.push(NveDrawListOpcode.Clear, 99);
  class OverflowThrowingSink extends RecordingSink {
    override onClear(color: number): void {
      super.onClear(color);
      throw new Error('clear boom');
    }
  }
  const overflowThrowing = new OverflowThrowingSink();
  assert.throws(
    () => decodeDrawList(packWords(overflowThrowWords), overflowThrowing),
    (err: unknown) => err instanceof Error && err.message === 'clear boom',
  );
  assert.equal(overflowThrowing.ops.length, cap + 1 + cap);
  assert.equal(overflowThrowing.ops[cap], 'clear:99');
  assert.ok(overflowThrowing.ops.slice(0, cap).every((op) => op.startsWith('clip:')));
  assert.ok(overflowThrowing.ops.slice(cap + 1).every((op) => op === 'restore'));
});
