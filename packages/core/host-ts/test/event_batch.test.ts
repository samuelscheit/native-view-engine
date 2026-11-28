import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  BinCodecError,
  EventBatchBuilder,
  EventBatchReader,
  EventError,
  Key64,
  NVE_EVENT_BATCH_LAYOUT,
  NVE_EVENT_FLAG_CAPTURE,
  NVE_EVENT_FLAG_COMPOSING,
  NVE_EVENT_FLAG_CUSTOM_NAMED,
  NVE_EVENT_FLAG_INLINE_KEY_UTF8,
  NVE_EVENT_FLAG_KEY_SPLIT_BYTES,
  NVE_EVENT_FLAG_MOD_ALT,
  NVE_EVENT_FLAG_MOD_CTRL,
  NVE_EVENT_FLAG_MOD_META,
  NVE_EVENT_FLAG_NO_BUBBLE,
  NVE_EVENT_FLAG_UTF8_BYTES,
  NVE_EVENT_LAYOUT,
  NVE_EVENT_SIZE,
  NVE_EVENT_WIRE_HEADER_SIZE,
  NVE_EVENT_WIRE_MAGIC,
  NVE_EVENT_WIRE_VERSION,
  NVE_INTERACTION_EVENT,
  NVE_LAYOUT_EVENT,
  NVE_SCROLL_EVENT,
  NVE_TABLE_CUSTOM,
  NVE_TABLE_INTERACTION,
  NVE_TABLE_LAYOUT,
  NVE_TABLE_SCROLL,
  NVE_TABLE_TEXT_INPUT,
  NVE_TEXT_INPUT_EVENT,
  RAW_EVENT_BYTES_LAYOUT,
  eventFlagsFromModifiers,
  eventFlagsWithPointerButton,
} from '../src/index.ts';
import type { EventErrorKind } from '../src/index.ts';

function expectEventError(kind: EventErrorKind, fn: () => unknown, what: string): void {
  assert.throws(fn, (e: unknown) => e instanceof EventError && e.kind === kind, what);
}

function buildRawWire(eventBytes: Uint8Array, arena: Uint8Array, version = NVE_EVENT_WIRE_VERSION): Uint8Array {
  const out = new Uint8Array(NVE_EVENT_WIRE_HEADER_SIZE + eventBytes.byteLength + arena.byteLength);
  const hdr = new DataView(out.buffer);
  hdr.setUint32(0, NVE_EVENT_WIRE_MAGIC, true);
  hdr.setUint16(4, version, true);
  hdr.setUint32(6, eventBytes.byteLength / NVE_EVENT_SIZE, true);
  hdr.setUint32(10, arena.byteLength, true);
  out.set(eventBytes, NVE_EVENT_WIRE_HEADER_SIZE);
  out.set(arena, NVE_EVENT_WIRE_HEADER_SIZE + eventBytes.byteLength);
  return out;
}

test('EventBatchBuilder + NVEV v2 wire round-trip (inline/split keys, custom, raw bytes, 64-bit timestamps)', () => {
  const b = new EventBatchBuilder(2, 16); // forces buffer growth
  const surface = 1;
  const node = ((2 << 24) | 7) >>> 0;
  const tsBig = new Key64(0x1234_5678, 0xf00d_cafe); // > 2^53

  b.pushPress(surface, node, new Key64(10, 0), 12.5, 24.5);
  b.pushLongPress(surface, node, new Key64(15, 0), 3.0, 4.0);
  b.pushFocus(surface, node, new Key64(16, 0));
  b.pushBlur(surface, node, new Key64(17, 0));
  b.pushScroll(surface, node, new Key64(20, 0), 1.0, 2.0, 300.0, 900.0);
  b.pushTextChange(surface, node, new Key64(30, 0), 'hello 你好');
  b.pushSubmit(surface, node, new Key64(40, 0), 'submitted');
  b.pushComposition(surface, node, new Key64(50, 0), NVE_TEXT_INPUT_EVENT.COMPOSITION_UPDATE, 'nihao');
  b.pushLayout(surface, node, new Key64(60, 0), 4.0, 8.0, 120.0, 48.0);
  // Short key (<= 16 B) uses inline UTF-8 fast path.
  b.pushKeyDown(
    surface,
    node,
    new Key64(70, 0),
    'Enter',
    'Enter',
    eventFlagsFromModifiers(true, false, true, false, false) | NVE_EVENT_FLAG_COMPOSING,
    13,
  );
  // Exact 16-byte key also fits inline.
  const key16 = '0123456789abcdef';
  b.pushKeyUp(surface, node, new Key64(75, 0), key16, 'KeyA', 0, 65);
  // Long key (> 16 B) spills into KEY_SPLIT_BYTES arena.
  const longKey = 'DeadCombinedVeryLongKeyName🚀';
  b.pushKeyDown(surface, node, new Key64(80, 0), longKey, 'Unidentified', NVE_EVENT_FLAG_MOD_CTRL, 229);
  b.pushPointer(
    surface,
    node,
    new Key64(90, 0),
    NVE_INTERACTION_EVENT.POINTER_DOWN,
    11.0,
    22.0,
    3,
    eventFlagsWithPointerButton(NVE_EVENT_FLAG_CAPTURE, 2),
  );
  b.pushWheel(surface, node, new Key64(100, 0), -3.0, 12.0, NVE_EVENT_FLAG_MOD_ALT);
  b.pushCustom(surface, node, tsBig, 'analytics.click', Uint8Array.of(1, 2, 3, 4));
  b.pushCustom(surface, node, new Key64(110, 0), 'close', undefined, NVE_EVENT_FLAG_NO_BUBBLE | NVE_EVENT_FLAG_UTF8_BYTES);
  const modKey = Key64.of('acme.map_view');
  b.pushTableEvent(
    modKey,
    surface,
    node,
    new Key64(120, 0),
    3,
    NVE_EVENT_FLAG_UTF8_BYTES, // internal framing bit must be stripped for raw bytes!
    [42, 0x89ab_cdef, 0x0123_4567, 0, 0, 0],
    Uint8Array.of(0xff, 0xfe),
  );

  assert.equal(b.length, 17);
  const wire = b.encodeWire();
  assert.equal(wire.byteLength, NVE_EVENT_WIRE_HEADER_SIZE + 17 * NVE_EVENT_SIZE + b.bytesLen);

  // Both sealed O(1) reader and wire-decoded reader agree completely.
  for (const view of [b.toReader(), EventBatchReader.decodeWire(wire)]) {
    assert.equal(view.length, 17);
    assert.equal([...view].length, 17);

    const e0 = view.get(0);
    assert.ok(e0.table.equals(NVE_TABLE_INTERACTION));
    assert.equal(e0.event, NVE_INTERACTION_EVENT.PRESS);
    assert.equal(e0.surface, surface);
    assert.equal(e0.node, node);
    assert.deepEqual([e0.x, e0.y], [12.5, 24.5]);

    const e4 = view.get(4);
    assert.ok(e4.table.equals(NVE_TABLE_SCROLL));
    assert.equal(e4.event, NVE_SCROLL_EVENT.SCROLL);
    assert.deepEqual([e4.x, e4.y, e4.width, e4.height], [1.0, 2.0, 300.0, 900.0]);

    const e5 = view.get(5);
    assert.ok(e5.table.equals(NVE_TABLE_TEXT_INPUT));
    assert.equal(e5.event, NVE_TEXT_INPUT_EVENT.CHANGE);
    assert.equal(e5.text(), 'hello 你好');

    const e8 = view.get(8);
    assert.ok(e8.table.equals(NVE_TABLE_LAYOUT));
    assert.equal(e8.event, NVE_LAYOUT_EVENT.LAYOUT);
    assert.deepEqual([e8.x, e8.y, e8.width, e8.height], [4.0, 8.0, 120.0, 48.0]);

    const e9 = view.get(9);
    assert.ok((e9.flags & NVE_EVENT_FLAG_INLINE_KEY_UTF8) !== 0);
    assert.equal(e9.key(), 'Enter');
    assert.equal(e9.code(), 'Enter');
    assert.ok(e9.meta && e9.shift && !e9.ctrl && e9.hasPrimaryModifier && e9.isComposing);
    assert.equal(e9.keyCode, 13);

    const e10 = view.get(10);
    assert.ok((e10.flags & NVE_EVENT_FLAG_INLINE_KEY_UTF8) !== 0);
    assert.equal(e10.key(), key16);
    assert.equal(e10.code(), 'KeyA');

    const e11 = view.get(11);
    assert.ok((e11.flags & NVE_EVENT_FLAG_KEY_SPLIT_BYTES) !== 0);
    assert.equal(e11.key(), longKey);
    assert.equal(e11.code(), 'Unidentified');
    assert.equal(e11.text(), '', 'KEY_SPLIT_BYTES is never exposed as text()');
    assert.ok(e11.ctrl && e11.hasPrimaryModifier);

    const e12 = view.get(12);
    assert.equal(e12.pointerId, 3);
    assert.equal(e12.pointerButton, 2);
    assert.ok(e12.isCapture);

    const e13 = view.get(13);
    assert.deepEqual([e13.deltaX, e13.deltaY], [-3.0, 12.0]);
    assert.ok(e13.alt);

    const e14 = view.get(14);
    assert.ok(e14.table.equals(NVE_TABLE_CUSTOM));
    assert.ok(e14.isCustom);
    assert.ok(e14.bubbles);
    assert.equal(e14.timestamp.toBigInt(), tsBig.toBigInt(), '64-bit timestamp exact');
    assert.equal(e14.customName(), 'analytics.click');
    assert.deepEqual(e14.customData(), Uint8Array.of(1, 2, 3, 4));

    const e15 = view.get(15);
    assert.ok(e15.table.equals(NVE_TABLE_CUSTOM));
    assert.ok(e15.isCustom);
    assert.equal(e15.bubbles, false);
    assert.equal(e15.flags, (NVE_EVENT_FLAG_NO_BUBBLE | NVE_EVENT_FLAG_CUSTOM_NAMED) >>> 0);
    assert.equal(e15.customName(), 'close');
    assert.equal(e15.customData().byteLength, 0);

    const e16 = view.get(16);
    assert.ok(e16.table.equals(modKey));
    assert.equal(e16.event, 3);
    assert.deepEqual(e16.bytes(), Uint8Array.of(0xff, 0xfe));
    assert.equal(e16.text(), '', 'invalid UTF-8 never exposed as text()');
    assert.equal(e16.payloadU64(1).toBigInt(), 0x0123_4567_89ab_cdefn);
  }

  // clear() reuses builder cleanly
  b.clear();
  assert.ok(b.isEmpty && b.bytesLen === 0);
  assert.equal(EventBatchReader.decodeWire(b.encodeWire()).length, 0);
});

test('EventBatchReader.fromLinearMemory reads wasm32 NveEventBatch', () => {
  const b = new EventBatchBuilder();
  b.pushKeyDown(1, 7, new Key64(42, 0), 'Escape', 'Escape', NVE_EVENT_FLAG_MOD_META, 27);
  const memory = new ArrayBuffer(1024);
  const bytes = new Uint8Array(memory);
  const mem = new DataView(memory);
  const evPtr = 128;
  const arenaPtr = 256;
  const batchPtr = 64;
  bytes.set(b.eventsBytes, evPtr);
  bytes.set(b.arenaBytes, arenaPtr);
  mem.setUint32(batchPtr + NVE_EVENT_BATCH_LAYOUT.events, evPtr, true);
  mem.setUint32(batchPtr + NVE_EVENT_BATCH_LAYOUT.bytes, arenaPtr, true);
  mem.setUint32(batchPtr + NVE_EVENT_BATCH_LAYOUT.eventCount, b.length, true);
  mem.setUint32(batchPtr + NVE_EVENT_BATCH_LAYOUT.bytesLen, b.bytesLen, true);

  const view = EventBatchReader.fromLinearMemory(memory, batchPtr);
  assert.equal(view.length, 1);
  assert.equal(view.get(0).key(), 'Escape');
  assert.equal(view.get(0).code(), 'Escape');
  assert.ok(view.get(0).meta);

  // Null pointers with non-zero counts are rejected
  mem.setUint32(batchPtr + NVE_EVENT_BATCH_LAYOUT.events, 0, true);
  expectEventError('truncated', () => EventBatchReader.fromLinearMemory(memory, batchPtr), 'null events ptr');
  mem.setUint32(batchPtr + NVE_EVENT_BATCH_LAYOUT.events, evPtr, true);
  mem.setUint32(batchPtr + NVE_EVENT_BATCH_LAYOUT.bytes, 0, true);
  expectEventError('truncated', () => EventBatchReader.fromLinearMemory(memory, batchPtr), 'null bytes ptr');
});

test('adversarial NVEV v2 wire frames, validation errors, handle canonicalization, and lone surrogates', () => {
  const b = new EventBatchBuilder();
  b.pushPress(1, 1, new Key64(1, 0), 0, 0);
  const goodWire = b.encodeWire();

  expectEventError('truncated', () => EventBatchReader.decodeWire(goodWire.subarray(0, 10)), 'truncated header');
  const badMagic = goodWire.slice();
  badMagic[0]! ^= 0xff;
  expectEventError('invalidMagic', () => EventBatchReader.decodeWire(badMagic), 'bad magic');
  // NVEV v1 is rejected (v2 only)
  const v1 = goodWire.slice();
  v1[4] = 1;
  expectEventError('unsupportedVersion', () => EventBatchReader.decodeWire(v1), 'NVEV v1 rejected');
  const trailing = new Uint8Array(goodWire.length + 1);
  trailing.set(goodWire);
  expectEventError('trailingBytes', () => EventBatchReader.decodeWire(trailing), 'trailing byte');

  // Span out of bounds / u32 overflow
  const oob = new Uint8Array(NVE_EVENT_SIZE);
  const oobV = new DataView(oob.buffer);
  oobV.setUint32(NVE_EVENT_LAYOUT.bytes + RAW_EVENT_BYTES_LAYOUT.offset, 0xffff_ffff, true);
  oobV.setUint32(NVE_EVENT_LAYOUT.bytes + RAW_EVENT_BYTES_LAYOUT.len, 2, true);
  expectEventError('spanOutOfBounds', () => EventBatchReader.decodeWire(buildRawWire(oob, Uint8Array.of(1, 2, 3))), 'span overflow');

  // Invalid UTF-8 under UTF8_BYTES
  const badUtf8 = new Uint8Array(NVE_EVENT_SIZE);
  const badUtf8V = new DataView(badUtf8.buffer);
  badUtf8V.setUint32(NVE_EVENT_LAYOUT.flags, NVE_EVENT_FLAG_UTF8_BYTES, true);
  badUtf8V.setUint32(NVE_EVENT_LAYOUT.bytes + RAW_EVENT_BYTES_LAYOUT.len, 2, true);
  expectEventError('invalidUtf8', () => EventBatchReader.decodeWire(buildRawWire(badUtf8, Uint8Array.of(0xff, 0xfe))), 'bad utf8');

  // Inline key length > 16 and invalid inline UTF-8
  const badInlineLen = new Uint8Array(NVE_EVENT_SIZE);
  const badInlineLenV = new DataView(badInlineLen.buffer);
  badInlineLenV.setUint32(NVE_EVENT_LAYOUT.flags, NVE_EVENT_FLAG_INLINE_KEY_UTF8, true);
  badInlineLenV.setUint32(NVE_EVENT_LAYOUT.payload + 4, 17, true);
  expectEventError('invalidInlineKeyLen', () => EventBatchReader.decodeWire(buildRawWire(badInlineLen, new Uint8Array(0))), 'inline len 17');

  const badInlineUtf8 = new Uint8Array(NVE_EVENT_SIZE);
  const badInlineUtf8V = new DataView(badInlineUtf8.buffer);
  badInlineUtf8V.setUint32(NVE_EVENT_LAYOUT.flags, NVE_EVENT_FLAG_INLINE_KEY_UTF8, true);
  badInlineUtf8V.setUint32(NVE_EVENT_LAYOUT.payload + 4, 2, true);
  badInlineUtf8V.setUint32(NVE_EVENT_LAYOUT.payload + 8, 0x0000_feff, true);
  expectEventError('invalidUtf8', () => EventBatchReader.decodeWire(buildRawWire(badInlineUtf8, new Uint8Array(0))), 'inline bad utf8');

  // Conflicting framing flags
  const conflict = new Uint8Array(NVE_EVENT_SIZE);
  new DataView(conflict.buffer).setUint32(NVE_EVENT_LAYOUT.flags, NVE_EVENT_FLAG_INLINE_KEY_UTF8 | NVE_EVENT_FLAG_KEY_SPLIT_BYTES, true);
  expectEventError('conflictingFlags', () => EventBatchReader.decodeWire(buildRawWire(conflict, new Uint8Array(0))), 'conflicting flags');

  // Split key cutting inside multi-byte UTF-8 character (`🚀` = 4 bytes, split at byte 2)
  const splitMid = new Uint8Array(NVE_EVENT_SIZE);
  const splitMidV = new DataView(splitMid.buffer);
  splitMidV.setUint32(NVE_EVENT_LAYOUT.flags, NVE_EVENT_FLAG_KEY_SPLIT_BYTES | NVE_EVENT_FLAG_UTF8_BYTES, true);
  splitMidV.setUint32(NVE_EVENT_LAYOUT.payload + 4, 2, true);
  splitMidV.setUint32(NVE_EVENT_LAYOUT.bytes + RAW_EVENT_BYTES_LAYOUT.len, 4, true);
  expectEventError('invalidUtf8', () => EventBatchReader.decodeWire(buildRawWire(splitMid, new TextEncoder().encode('🚀'))), 'split mid codepoint');

  // Split key length exceeding event bytes
  const splitOob = new Uint8Array(NVE_EVENT_SIZE);
  const splitOobV = new DataView(splitOob.buffer);
  splitOobV.setUint32(NVE_EVENT_LAYOUT.flags, NVE_EVENT_FLAG_KEY_SPLIT_BYTES, true);
  splitOobV.setUint32(NVE_EVENT_LAYOUT.payload + 4, 9, true);
  splitOobV.setUint32(NVE_EVENT_LAYOUT.bytes + RAW_EVENT_BYTES_LAYOUT.len, 2, true);
  expectEventError('invalidSplitKeyLen', () => EventBatchReader.decodeWire(buildRawWire(splitOob, Uint8Array.of(97, 98))), 'split len > bytes');

  // Custom name length exceeding event bytes
  const customOob = new Uint8Array(NVE_EVENT_SIZE);
  const customOobV = new DataView(customOob.buffer);
  customOobV.setUint32(NVE_EVENT_LAYOUT.flags, NVE_EVENT_FLAG_CUSTOM_NAMED, true);
  customOobV.setUint32(NVE_EVENT_LAYOUT.payload, 10, true);
  customOobV.setUint32(NVE_EVENT_LAYOUT.bytes + RAW_EVENT_BYTES_LAYOUT.len, 2, true);
  expectEventError('invalidCustomNameLen', () => EventBatchReader.decodeWire(buildRawWire(customOob, Uint8Array.of(97, 98))), 'custom len > bytes');

  // Non-canonical index-0 handle bits (0x0500_0000) canonicalize to 0 on read
  const nonCanon = new Uint8Array(NVE_EVENT_SIZE);
  const nonCanonV = new DataView(nonCanon.buffer);
  nonCanonV.setUint32(NVE_EVENT_LAYOUT.surface, 0x0500_0000, true);
  nonCanonV.setUint32(NVE_EVENT_LAYOUT.node, 0x0500_0000, true);
  const rCanon = EventBatchReader.decodeWire(buildRawWire(nonCanon, new Uint8Array(0)));
  assert.equal(rCanon.get(0).surface, 0);
  assert.equal(rCanon.get(0).node, 0);

  // Lone surrogates throw BinCodecError('invalidUtf8') atomically without adding an event
  const atomic = new EventBatchBuilder();
  assert.throws(() => atomic.pushTextChange(1, 1, Key64.ZERO, '\ud800'), (e: unknown) => e instanceof BinCodecError && e.kind === 'invalidUtf8');
  assert.throws(() => atomic.pushKeyDown(1, 1, Key64.ZERO, '\udc00', 'KeyA', 0, 65), BinCodecError);
  assert.throws(() => atomic.pushCustom(1, 1, Key64.ZERO, 'bad\ud800'), BinCodecError);
  assert.equal(atomic.length, 0);
});
