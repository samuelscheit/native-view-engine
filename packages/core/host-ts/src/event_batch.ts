/**
 * Uniform 64-byte `NveEvent` batch builder, `NVEV` v2 binary wire codec, and ingress-validated
 * zero-copy reader (byte-identical to Rust `packages/core/abi/src/event/{mod,buf,validated,wire}.rs`).
 *
 * Every 64-bit field (`timestamp`, `table`, `payloadU64`) stays exact via `Key64` `(lo, hi)` halves
 * (never rounded through `Number`). Strings reject lone surrogates on write and are validated
 * in-place on ingress without decoding.
 */
import {
  NVE_EVENT_BATCH_LAYOUT, NVE_EVENT_FLAG_BUTTON_MASK, NVE_EVENT_FLAG_BUTTON_SHIFT, NVE_EVENT_FLAG_CAPTURE,
  NVE_EVENT_FLAG_COMPOSING, NVE_EVENT_FLAG_CUSTOM_NAMED, NVE_EVENT_FLAG_INLINE_KEY_UTF8, NVE_EVENT_FLAG_KEY_SPLIT_BYTES,
  NVE_EVENT_FLAG_MOD_ALT, NVE_EVENT_FLAG_MOD_CTRL, NVE_EVENT_FLAG_MOD_META, NVE_EVENT_FLAG_MOD_REPEAT,
  NVE_EVENT_FLAG_MOD_SHIFT, NVE_EVENT_FLAG_NO_BUBBLE, NVE_EVENT_FLAG_UTF8_BYTES, NVE_EVENT_INLINE_KEY_CAP,
  NVE_EVENT_LAYOUT, NVE_EVENT_SIZE,
  NVE_INTERACTION_EVENT, NVE_LAYOUT_EVENT, NVE_SCROLL_EVENT, NVE_TABLE_CUSTOM, NVE_TABLE_INTERACTION, NVE_TABLE_LAYOUT,
  NVE_TABLE_SCROLL, NVE_TABLE_TEXT_INPUT, NVE_TEXT_INPUT_EVENT, RAW_EVENT_BYTES_LAYOUT,
} from './generated/nve_abi.generated.ts';
import type { NveInteractionEvent, NveTextInputEvent } from './generated/nve_abi.generated.ts';
import { Key64 } from './key64.ts';
import { NODE_INDEX_MASK, f32FromBits, f64FromBits, isIntIn } from './table_reader.ts';
import { UTF8_DECODER, isValidUtf8, requireWellFormed } from './utf8.ts';

/** Little-endian `"NVEV"` magic word for binary event batch frames. */
export const NVE_EVENT_WIRE_MAGIC = 0x5645_564e;
/** Wire format version for uniform 64-byte `NveEvent` batches (`NVEV` v2). */
export const NVE_EVENT_WIRE_VERSION = 2;
/** Fixed wire header size (`magic[4] + version:u16 + count:u32 + bytes_len:u32`). */
export const NVE_EVENT_WIRE_HEADER_SIZE = 14;
/** Combined keyboard/pointer modifier bitmask (`bits 0..=4`). */
export const NVE_EVENT_FLAG_MOD_MASK = 0x1f;

const INTERNAL_FRAMING_MASK =
  NVE_EVENT_FLAG_UTF8_BYTES | NVE_EVENT_FLAG_INLINE_KEY_UTF8 | NVE_EVENT_FLAG_CUSTOM_NAMED | NVE_EVENT_FLAG_KEY_SPLIT_BYTES;

const EV = NVE_EVENT_LAYOUT;
const RB = RAW_EVENT_BYTES_LAYOUT;
const BATCH = NVE_EVENT_BATCH_LAYOUT;
const INLINE_KEY_OFF = EV.payload + 8; // payload[2..6]
const UTF8_ENCODER = new TextEncoder();
const F32_SCRATCH = new DataView(new ArrayBuffer(4));
const EMPTY_BYTES = new Uint8Array(0);
const ZERO_PAYLOAD: readonly [number, number, number, number, number, number] = [0, 0, 0, 0, 0, 0];

export type EventErrorKind =
  | 'truncated' | 'invalidMagic' | 'unsupportedVersion' | 'trailingBytes' | 'spanOutOfBounds'
  | 'invalidUtf8' | 'invalidInlineKeyLen' | 'invalidSplitKeyLen' | 'invalidCustomNameLen' | 'conflictingFlags' | 'overflow';

export class EventError extends Error {
  readonly kind: EventErrorKind;
  constructor(kind: EventErrorKind, detail: string) {
    super(`${kind}: ${detail}`);
    this.name = 'EventError';
    this.kind = kind;
  }
}

/** Builds keyboard/pointer modifier bits for `NveEvent::flags`. */
export function eventFlagsFromModifiers(meta: boolean, ctrl: boolean, shift: boolean, alt: boolean, repeat = false): number {
  return (
    (meta ? NVE_EVENT_FLAG_MOD_META : 0) | (ctrl ? NVE_EVENT_FLAG_MOD_CTRL : 0) |
    (shift ? NVE_EVENT_FLAG_MOD_SHIFT : 0) | (alt ? NVE_EVENT_FLAG_MOD_ALT : 0) | (repeat ? NVE_EVENT_FLAG_MOD_REPEAT : 0)
  );
}

/** Sets pointer button index (`0..=7`) inside `NveEvent::flags`. */
export function eventFlagsWithPointerButton(flags: number, button: number): number {
  return ((flags & ~NVE_EVENT_FLAG_BUTTON_MASK) | ((button & 0x07) << NVE_EVENT_FLAG_BUTTON_SHIFT)) >>> 0;
}

function f32ToBits(v: number): number {
  F32_SCRATCH.setFloat32(0, v, true);
  return F32_SCRATCH.getUint32(0, true);
}

function canonicalHandle(raw: number): number {
  return (raw & NODE_INDEX_MASK) === 0 ? 0 : raw >>> 0;
}

function validateOneEvent(index: number, events: Uint8Array, evView: DataView, arena: Uint8Array): void {
  const base = index * NVE_EVENT_SIZE;
  const off = evView.getUint32(base + EV.bytes + RB.offset, true);
  const len = evView.getUint32(base + EV.bytes + RB.len, true);
  const end = off + len;
  if (end > arena.byteLength) {
    throw new EventError('spanOutOfBounds', `event ${index} span [${off}, +${len}) outside arena (${arena.byteLength})`);
  }
  const flags = evView.getUint32(base + EV.flags, true);
  const hasUtf8 = (flags & NVE_EVENT_FLAG_UTF8_BYTES) !== 0;
  const hasInlineKey = (flags & NVE_EVENT_FLAG_INLINE_KEY_UTF8) !== 0;
  const hasSplitKey = (flags & NVE_EVENT_FLAG_KEY_SPLIT_BYTES) !== 0;
  const hasCustom = (flags & NVE_EVENT_FLAG_CUSTOM_NAMED) !== 0;
  if ((hasInlineKey && hasSplitKey) || (hasCustom && (hasInlineKey || hasSplitKey))) {
    throw new EventError('conflictingFlags', `event ${index} flags 0x${flags.toString(16)}`);
  }
  if (hasUtf8 && !isValidUtf8(arena, off, end)) throw new EventError('invalidUtf8', `event ${index} bytes`);
  if (hasInlineKey) {
    const klen = evView.getUint32(base + EV.payload + 4, true);
    if (klen > NVE_EVENT_INLINE_KEY_CAP) throw new EventError('invalidInlineKeyLen', `event ${index} inline key len ${klen}`);
    if (!isValidUtf8(events, base + INLINE_KEY_OFF, base + INLINE_KEY_OFF + klen)) {
      throw new EventError('invalidUtf8', `event ${index} inline key`);
    }
  }
  if (hasSplitKey) {
    const klen = evView.getUint32(base + EV.payload + 4, true);
    if (klen > len) throw new EventError('invalidSplitKeyLen', `event ${index} split key len ${klen} > ${len}`);
    if (hasUtf8) {
      if (klen < len && (arena[off + klen]! & 0xc0) === 0x80) {
        throw new EventError('invalidUtf8', `event ${index} split key cuts a UTF-8 code point`);
      }
    } else if (!isValidUtf8(arena, off, off + klen) || !isValidUtf8(arena, off + klen, end)) {
      throw new EventError('invalidUtf8', `event ${index} split key/code`);
    }
  }
  if (hasCustom) {
    const nlen = evView.getUint32(base + EV.payload, true);
    if (nlen > len) throw new EventError('invalidCustomNameLen', `event ${index} custom name len ${nlen} > ${len}`);
    if (!isValidUtf8(arena, off, off + nlen)) throw new EventError('invalidUtf8', `event ${index} custom name`);
  }
}

/** One ingress-validated 64-byte `NveEvent` borrowing its companion slice from `EventBatchReader`. */
export class EventView {
  private readonly events: Uint8Array;
  private readonly view: DataView;
  private readonly base: number;
  private readonly arena: Uint8Array;

  /** @internal */
  constructor(events: Uint8Array, view: DataView, base: number, arena: Uint8Array) {
    this.events = events;
    this.view = view;
    this.base = base;
    this.arena = arena;
  }

  get timestamp(): Key64 {
    return new Key64(this.view.getUint32(this.base + EV.timestamp, true), this.view.getUint32(this.base + EV.timestamp + 4, true));
  }
  get table(): Key64 {
    return new Key64(this.view.getUint32(this.base + EV.table, true), this.view.getUint32(this.base + EV.table + 4, true));
  }
  get surface(): number { return canonicalHandle(this.view.getUint32(this.base + EV.surface, true)); }
  get node(): number { return canonicalHandle(this.view.getUint32(this.base + EV.node, true)); }
  get event(): number { return this.view.getUint32(this.base + EV.event, true); }
  get flags(): number { return this.view.getUint32(this.base + EV.flags, true); }

  payloadU32(idx: number): number {
    if (!isIntIn(idx, 0, 5)) throw new RangeError(`payload index ${idx} outside 0..=5`);
    return this.view.getUint32(this.base + EV.payload + idx * 4, true);
  }
  payloadF32(idx: number): number { return f32FromBits(this.payloadU32(idx)); }
  payloadU64(idx: number): Key64 {
    if (!isIntIn(idx, 0, 4)) throw new RangeError(`payload u64 index ${idx} outside 0..=4`);
    return new Key64(this.payloadU32(idx), this.payloadU32(idx + 1));
  }
  payloadF64(idx: number): number {
    if (!isIntIn(idx, 0, 4)) throw new RangeError(`payload f64 index ${idx} outside 0..=4`);
    return f64FromBits(this.payloadU32(idx), this.payloadU32(idx + 1));
  }

  get x(): number { return this.payloadF32(0); }
  get y(): number { return this.payloadF32(1); }
  get deltaX(): number { return this.payloadF32(0); }
  get deltaY(): number { return this.payloadF32(1); }
  get width(): number { return this.payloadF32(2); }
  get height(): number { return this.payloadF32(3); }
  get pointerId(): number { return this.payloadU32(2); }
  get keyCode(): number { return this.payloadU32(0); }

  get meta(): boolean { return (this.flags & NVE_EVENT_FLAG_MOD_META) !== 0; }
  get ctrl(): boolean { return (this.flags & NVE_EVENT_FLAG_MOD_CTRL) !== 0; }
  get shift(): boolean { return (this.flags & NVE_EVENT_FLAG_MOD_SHIFT) !== 0; }
  get alt(): boolean { return (this.flags & NVE_EVENT_FLAG_MOD_ALT) !== 0; }
  get repeat(): boolean { return (this.flags & NVE_EVENT_FLAG_MOD_REPEAT) !== 0; }
  get hasPrimaryModifier(): boolean { return this.meta || this.ctrl; }
  get pointerButton(): number { return (this.flags & NVE_EVENT_FLAG_BUTTON_MASK) >>> NVE_EVENT_FLAG_BUTTON_SHIFT; }
  get isCapture(): boolean { return (this.flags & NVE_EVENT_FLAG_CAPTURE) !== 0; }
  get isComposing(): boolean { return (this.flags & NVE_EVENT_FLAG_COMPOSING) !== 0; }
  get bubbles(): boolean { return (this.flags & NVE_EVENT_FLAG_NO_BUBBLE) === 0; }
  get isCustom(): boolean { return this.table.equals(NVE_TABLE_CUSTOM) && (this.flags & NVE_EVENT_FLAG_CUSTOM_NAMED) !== 0; }

  /** Zero-copy view of this event's companion byte slice in the batch arena. */
  bytes(): Uint8Array {
    const off = this.view.getUint32(this.base + EV.bytes + RB.offset, true);
    const len = this.view.getUint32(this.base + EV.bytes + RB.len, true);
    return this.arena.subarray(off, off + len);
  }

  /** Pre-validated UTF-8 text payload (`""` when not flagged `UTF8_BYTES` or when `KEY_SPLIT_BYTES`). */
  text(): string {
    const f = this.flags;
    return (f & NVE_EVENT_FLAG_UTF8_BYTES) !== 0 && (f & NVE_EVENT_FLAG_KEY_SPLIT_BYTES) === 0
      ? UTF8_DECODER.decode(this.bytes())
      : '';
  }

  /** W3C key string (`<= 16 B` inline fast path or `KEY_SPLIT_BYTES` arena). */
  key(): string {
    const f = this.flags;
    if ((f & NVE_EVENT_FLAG_INLINE_KEY_UTF8) !== 0) {
      return UTF8_DECODER.decode(this.events.subarray(this.base + INLINE_KEY_OFF, this.base + INLINE_KEY_OFF + this.payloadU32(1)));
    }
    if ((f & NVE_EVENT_FLAG_KEY_SPLIT_BYTES) !== 0) return UTF8_DECODER.decode(this.bytes().subarray(0, this.payloadU32(1)));
    return '';
  }

  /** Physical key code string. */
  code(): string {
    const f = this.flags;
    if ((f & NVE_EVENT_FLAG_INLINE_KEY_UTF8) !== 0 && (f & NVE_EVENT_FLAG_UTF8_BYTES) !== 0) return UTF8_DECODER.decode(this.bytes());
    if ((f & NVE_EVENT_FLAG_KEY_SPLIT_BYTES) !== 0) return UTF8_DECODER.decode(this.bytes().subarray(this.payloadU32(1)));
    return '';
  }

  /** Custom event name (`NVE_TABLE_CUSTOM` + `CUSTOM_NAMED`). */
  customName(): string {
    return (this.flags & NVE_EVENT_FLAG_CUSTOM_NAMED) !== 0 ? UTF8_DECODER.decode(this.bytes().subarray(0, this.payloadU32(0))) : '';
  }

  /** Raw custom event payload bytes following the name prefix. */
  customData(): Uint8Array {
    return (this.flags & NVE_EVENT_FLAG_CUSTOM_NAMED) !== 0 ? this.bytes().subarray(this.payloadU32(0)) : this.bytes();
  }
}

/** Ingress-validated zero-copy view over a uniform 64-byte `NveEvent` batch + companion byte arena. */
export class EventBatchReader {
  readonly length: number;
  readonly eventsBytes: Uint8Array;
  readonly arenaBytes: Uint8Array;
  private readonly view: DataView;

  private constructor(eventsBytes: Uint8Array, arenaBytes: Uint8Array, count: number) {
    this.eventsBytes = eventsBytes;
    this.arenaBytes = arenaBytes;
    this.length = count;
    this.view = new DataView(eventsBytes.buffer, eventsBytes.byteOffset, eventsBytes.byteLength);
  }

  /** @internal O(1) sealed view used by `EventBatchBuilder`. */
  static fromSealed(eventsBytes: Uint8Array, arenaBytes: Uint8Array, count: number): EventBatchReader {
    return new EventBatchReader(eventsBytes, arenaBytes, count);
  }

  /** Validates `(eventsBytes, arenaBytes)` in a single linear pass. */
  static validate(eventsBytes: Uint8Array, arenaBytes: Uint8Array): EventBatchReader {
    if (eventsBytes.byteLength % NVE_EVENT_SIZE !== 0) {
      throw new EventError('truncated', `events length ${eventsBytes.byteLength} not a multiple of ${NVE_EVENT_SIZE}`);
    }
    const count = eventsBytes.byteLength / NVE_EVENT_SIZE;
    const view = new DataView(eventsBytes.buffer, eventsBytes.byteOffset, eventsBytes.byteLength);
    for (let i = 0; i < count; i++) validateOneEvent(i, eventsBytes, view, arenaBytes);
    return new EventBatchReader(eventsBytes, arenaBytes, count);
  }

  /** Decodes and validates an `NVEV` v2 binary wire frame (zero-copy subarrays into `raw`). */
  static decodeWire(raw: Uint8Array): EventBatchReader {
    if (raw.byteLength < NVE_EVENT_WIRE_HEADER_SIZE) {
      throw new EventError('truncated', `header needs ${NVE_EVENT_WIRE_HEADER_SIZE} bytes, have ${raw.byteLength}`);
    }
    const hdr = new DataView(raw.buffer, raw.byteOffset, NVE_EVENT_WIRE_HEADER_SIZE);
    const magic = hdr.getUint32(0, true);
    if (magic !== NVE_EVENT_WIRE_MAGIC) throw new EventError('invalidMagic', `got 0x${magic.toString(16)}`);
    const version = hdr.getUint16(4, true);
    if (version !== NVE_EVENT_WIRE_VERSION) throw new EventError('unsupportedVersion', `version ${version} (expected ${NVE_EVENT_WIRE_VERSION})`);
    const count = hdr.getUint32(6, true);
    const bytesLen = hdr.getUint32(10, true);
    const evBytes = count * NVE_EVENT_SIZE;
    const expected = NVE_EVENT_WIRE_HEADER_SIZE + evBytes + bytesLen;
    if (raw.byteLength < expected) throw new EventError('truncated', `need ${expected} bytes, have ${raw.byteLength}`);
    if (raw.byteLength > expected) throw new EventError('trailingBytes', `${raw.byteLength - expected} trailing bytes`);
    return EventBatchReader.validate(
      raw.subarray(NVE_EVENT_WIRE_HEADER_SIZE, NVE_EVENT_WIRE_HEADER_SIZE + evBytes),
      raw.subarray(NVE_EVENT_WIRE_HEADER_SIZE + evBytes, expected),
    );
  }

  /** Reads and validates an `NveEventBatch` from wasm32 linear memory (zero copy). */
  static fromLinearMemory(memory: ArrayBuffer, batchPtr: number): EventBatchReader {
    if (!isIntIn(batchPtr, 0, memory.byteLength - BATCH.size)) {
      throw new EventError('truncated', `batch at ${batchPtr} outside memory (${memory.byteLength})`);
    }
    const mem = new DataView(memory);
    const evPtr = mem.getUint32(batchPtr + BATCH.events, true);
    const bytesPtr = mem.getUint32(batchPtr + BATCH.bytes, true);
    const count = mem.getUint32(batchPtr + BATCH.eventCount, true);
    const bytesLen = mem.getUint32(batchPtr + BATCH.bytesLen, true);
    const evBytes = count * NVE_EVENT_SIZE;
    if (evBytes > 0 && (evPtr === 0 || evPtr + evBytes > memory.byteLength)) {
      throw new EventError('truncated', `events [${evPtr}, +${evBytes}) outside memory (${memory.byteLength})`);
    }
    if (bytesLen > 0 && (bytesPtr === 0 || bytesPtr + bytesLen > memory.byteLength)) {
      throw new EventError('truncated', `bytes [${bytesPtr}, +${bytesLen}) outside memory (${memory.byteLength})`);
    }
    return EventBatchReader.validate(
      evBytes === 0 ? EMPTY_BYTES : new Uint8Array(memory, evPtr, evBytes),
      bytesLen === 0 ? EMPTY_BYTES : new Uint8Array(memory, bytesPtr, bytesLen),
    );
  }

  get isEmpty(): boolean { return this.length === 0; }

  get(index: number): EventView {
    if (!isIntIn(index, 0, this.length - 1)) throw new RangeError(`event ${index} outside ${this.length}`);
    return new EventView(this.eventsBytes, this.view, index * NVE_EVENT_SIZE, this.arenaBytes);
  }

  *[Symbol.iterator](): IterableIterator<EventView> {
    for (let i = 0; i < this.length; i++) yield new EventView(this.eventsBytes, this.view, i * NVE_EVENT_SIZE, this.arenaBytes);
  }
}

/**
 * Reusable owned builder of uniform 64-byte `NveEvent` records + companion byte arena.
 * Seals all `EventBatchReader` invariants by construction; `clear()` keeps backing capacity.
 */
export class EventBatchBuilder {
  private evBuf: Uint8Array;
  private evView: DataView;
  private count = 0;
  private arenaBuf: Uint8Array;
  private arenaLen = 0;

  constructor(eventCapacity = 8, byteCapacity = 128) {
    if (!(Number.isSafeInteger(eventCapacity) && eventCapacity >= 0 && Number.isSafeInteger(byteCapacity) && byteCapacity >= 0)) {
      throw new RangeError(`invalid capacities (${eventCapacity}, ${byteCapacity})`);
    }
    this.evBuf = new Uint8Array(Math.max(1, eventCapacity) * NVE_EVENT_SIZE);
    this.evView = new DataView(this.evBuf.buffer);
    this.arenaBuf = new Uint8Array(Math.max(16, byteCapacity));
  }

  get length(): number { return this.count; }
  get isEmpty(): boolean { return this.count === 0; }
  get bytesLen(): number { return this.arenaLen; }
  get eventsBytes(): Uint8Array { return this.evBuf.subarray(0, this.count * NVE_EVENT_SIZE); }
  get arenaBytes(): Uint8Array { return this.arenaBuf.subarray(0, this.arenaLen); }

  clear(): void {
    this.count = 0;
    this.arenaLen = 0;
  }

  /** O(1) sealed zero-copy reader over the events built so far. */
  toReader(): EventBatchReader {
    return EventBatchReader.fromSealed(this.eventsBytes, this.arenaBytes, this.count);
  }

  /** Encodes the batch as a standalone little-endian `NVEV` v2 wire frame. */
  encodeWire(): Uint8Array {
    const evBytes = this.count * NVE_EVENT_SIZE;
    const out = new Uint8Array(NVE_EVENT_WIRE_HEADER_SIZE + evBytes + this.arenaLen);
    const hdr = new DataView(out.buffer);
    hdr.setUint32(0, NVE_EVENT_WIRE_MAGIC, true);
    hdr.setUint16(4, NVE_EVENT_WIRE_VERSION, true);
    hdr.setUint32(6, this.count >>> 0, true);
    hdr.setUint32(10, this.arenaLen >>> 0, true);
    out.set(this.eventsBytes, NVE_EVENT_WIRE_HEADER_SIZE);
    out.set(this.arenaBytes, NVE_EVENT_WIRE_HEADER_SIZE + evBytes);
    return out;
  }

  pushTableEvent(
    table: Key64, surface: number, node: number, timestamp: Key64, eventId: number, flags: number,
    payload: readonly [number, number, number, number, number, number] = ZERO_PAYLOAD, data: Uint8Array = EMPTY_BYTES,
  ): void {
    this.pushSealedPair(table, surface, node, timestamp, eventId, (flags & ~INTERNAL_FRAMING_MASK) >>> 0, payload, data, EMPTY_BYTES);
  }

  pushEventText(
    table: Key64, surface: number, node: number, timestamp: Key64, eventId: number, flags: number,
    payload: readonly [number, number, number, number, number, number], text: string,
  ): void {
    requireWellFormed(text);
    const encoded = text.length === 0 ? EMPTY_BYTES : UTF8_ENCODER.encode(text);
    this.pushSealedPair(table, surface, node, timestamp, eventId, ((flags & ~INTERNAL_FRAMING_MASK) | NVE_EVENT_FLAG_UTF8_BYTES) >>> 0, payload, encoded, EMPTY_BYTES);
  }

  pushPress(surface: number, node: number, timestamp: Key64, x: number, y: number): void {
    this.pushTableEvent(NVE_TABLE_INTERACTION, surface, node, timestamp, NVE_INTERACTION_EVENT.PRESS, 0, [f32ToBits(x), f32ToBits(y), 0, 0, 0, 0]);
  }
  pushLongPress(surface: number, node: number, timestamp: Key64, x: number, y: number): void {
    this.pushTableEvent(NVE_TABLE_INTERACTION, surface, node, timestamp, NVE_INTERACTION_EVENT.LONG_PRESS, 0, [f32ToBits(x), f32ToBits(y), 0, 0, 0, 0]);
  }
  pushFocus(surface: number, node: number, timestamp: Key64): void {
    this.pushTableEvent(NVE_TABLE_INTERACTION, surface, node, timestamp, NVE_INTERACTION_EVENT.FOCUS, 0);
  }
  pushBlur(surface: number, node: number, timestamp: Key64): void {
    this.pushTableEvent(NVE_TABLE_INTERACTION, surface, node, timestamp, NVE_INTERACTION_EVENT.BLUR, 0);
  }

  pushKey(surface: number, node: number, timestamp: Key64, event: NveInteractionEvent, key: string, code: string, flags: number, keyCode: number): void {
    requireWellFormed(key);
    requireWellFormed(code);
    const keyBytes = key.length === 0 ? EMPTY_BYTES : UTF8_ENCODER.encode(key);
    const codeBytes = code.length === 0 ? EMPTY_BYTES : UTF8_ENCODER.encode(code);
    const baseFlags = (flags & ~INTERNAL_FRAMING_MASK) >>> 0;
    if (keyBytes.length <= NVE_EVENT_INLINE_KEY_CAP) {
      const f = (baseFlags | NVE_EVENT_FLAG_INLINE_KEY_UTF8 | (codeBytes.length > 0 ? NVE_EVENT_FLAG_UTF8_BYTES : 0)) >>> 0;
      const base = this.pushSealedPair(NVE_TABLE_INTERACTION, surface, node, timestamp, event, f, [keyCode >>> 0, keyBytes.length, 0, 0, 0, 0], codeBytes, EMPTY_BYTES);
      this.evBuf.set(keyBytes, base + INLINE_KEY_OFF);
    } else {
      const f = (baseFlags | NVE_EVENT_FLAG_KEY_SPLIT_BYTES | NVE_EVENT_FLAG_UTF8_BYTES) >>> 0;
      this.pushSealedPair(NVE_TABLE_INTERACTION, surface, node, timestamp, event, f, [keyCode >>> 0, keyBytes.length, 0, 0, 0, 0], keyBytes, codeBytes);
    }
  }

  pushKeyDown(surface: number, node: number, timestamp: Key64, key: string, code: string, flags: number, keyCode: number): void {
    this.pushKey(surface, node, timestamp, NVE_INTERACTION_EVENT.KEY_DOWN, key, code, flags, keyCode);
  }
  pushKeyUp(surface: number, node: number, timestamp: Key64, key: string, code: string, flags: number, keyCode: number): void {
    this.pushKey(surface, node, timestamp, NVE_INTERACTION_EVENT.KEY_UP, key, code, flags, keyCode);
  }
  pushPointer(surface: number, node: number, timestamp: Key64, event: NveInteractionEvent, x: number, y: number, pointerId: number, flags: number): void {
    this.pushTableEvent(NVE_TABLE_INTERACTION, surface, node, timestamp, event, flags, [f32ToBits(x), f32ToBits(y), pointerId >>> 0, 0, 0, 0]);
  }
  pushWheel(surface: number, node: number, timestamp: Key64, deltaX: number, deltaY: number, flags: number): void {
    this.pushTableEvent(NVE_TABLE_INTERACTION, surface, node, timestamp, NVE_INTERACTION_EVENT.WHEEL, flags, [f32ToBits(deltaX), f32ToBits(deltaY), 0, 0, 0, 0]);
  }
  pushTextChange(surface: number, node: number, timestamp: Key64, text: string): void {
    this.pushEventText(NVE_TABLE_TEXT_INPUT, surface, node, timestamp, NVE_TEXT_INPUT_EVENT.CHANGE, 0, ZERO_PAYLOAD, text);
  }
  pushSubmit(surface: number, node: number, timestamp: Key64, text: string): void {
    this.pushEventText(NVE_TABLE_TEXT_INPUT, surface, node, timestamp, NVE_TEXT_INPUT_EVENT.SUBMIT, 0, ZERO_PAYLOAD, text);
  }
  pushComposition(surface: number, node: number, timestamp: Key64, event: NveTextInputEvent, text: string): void {
    this.pushEventText(NVE_TABLE_TEXT_INPUT, surface, node, timestamp, event, 0, ZERO_PAYLOAD, text);
  }
  pushScroll(surface: number, node: number, timestamp: Key64, x: number, y: number, contentWidth: number, contentHeight: number): void {
    this.pushTableEvent(NVE_TABLE_SCROLL, surface, node, timestamp, NVE_SCROLL_EVENT.SCROLL, 0, [f32ToBits(x), f32ToBits(y), f32ToBits(contentWidth), f32ToBits(contentHeight), 0, 0]);
  }
  pushLayout(surface: number, node: number, timestamp: Key64, x: number, y: number, width: number, height: number): void {
    this.pushTableEvent(NVE_TABLE_LAYOUT, surface, node, timestamp, NVE_LAYOUT_EVENT.LAYOUT, 0, [f32ToBits(x), f32ToBits(y), f32ToBits(width), f32ToBits(height), 0, 0]);
  }
  pushCustom(surface: number, node: number, timestamp: Key64, name: string, data: Uint8Array = EMPTY_BYTES, flags = 0): void {
    requireWellFormed(name);
    const nameBytes = name.length === 0 ? EMPTY_BYTES : UTF8_ENCODER.encode(name);
    const f = ((flags & ~INTERNAL_FRAMING_MASK) | NVE_EVENT_FLAG_CUSTOM_NAMED) >>> 0;
    this.pushSealedPair(NVE_TABLE_CUSTOM, surface, node, timestamp, 0, f, [nameBytes.length, 0, 0, 0, 0, 0], nameBytes, data);
  }

  private pushSealedPair(
    table: Key64, surface: number, node: number, timestamp: Key64, eventId: number, flags: number,
    payload: readonly [number, number, number, number, number, number], first: Uint8Array, second: Uint8Array,
  ): number {
    if (this.count >= 0xffff_ffff) throw new EventError('overflow', 'event count exceeds u32');
    const totalBytes = first.byteLength + second.byteLength;
    if (this.arenaLen + totalBytes > 0xffff_ffff) throw new EventError('overflow', 'event arena exceeds u32');
    this.ensureEvents(this.count + 1);
    let off = 0;
    if (totalBytes > 0) {
      this.ensureArena(this.arenaLen + totalBytes);
      off = this.arenaLen;
      if (first.byteLength > 0) { this.arenaBuf.set(first, this.arenaLen); this.arenaLen += first.byteLength; }
      if (second.byteLength > 0) { this.arenaBuf.set(second, this.arenaLen); this.arenaLen += second.byteLength; }
    }
    const base = this.count * NVE_EVENT_SIZE;
    this.count += 1;
    const v = this.evView;
    v.setUint32(base + EV.timestamp, timestamp.lo, true);
    v.setUint32(base + EV.timestamp + 4, timestamp.hi, true);
    v.setUint32(base + EV.table, table.lo, true);
    v.setUint32(base + EV.table + 4, table.hi, true);
    v.setUint32(base + EV.surface, canonicalHandle(surface), true);
    v.setUint32(base + EV.node, canonicalHandle(node), true);
    v.setUint32(base + EV.event, eventId >>> 0, true);
    v.setUint32(base + EV.flags, flags >>> 0, true);
    for (let i = 0; i < 6; i++) v.setUint32(base + EV.payload + i * 4, payload[i]! >>> 0, true);
    v.setUint32(base + EV.bytes + RB.offset, off >>> 0, true);
    v.setUint32(base + EV.bytes + RB.len, totalBytes >>> 0, true);
    return base;
  }

  private ensureEvents(need: number): void {
    const needBytes = need * NVE_EVENT_SIZE;
    if (needBytes <= this.evBuf.byteLength) return;
    let next = this.evBuf.byteLength * 2;
    while (next < needBytes) next *= 2;
    const grown = new Uint8Array(next);
    grown.set(this.evBuf.subarray(0, this.count * NVE_EVENT_SIZE));
    this.evBuf = grown;
    this.evView = new DataView(grown.buffer);
  }

  private ensureArena(need: number): void {
    if (need <= this.arenaBuf.byteLength) return;
    let next = this.arenaBuf.byteLength * 2;
    while (next < need) next *= 2;
    const grown = new Uint8Array(next);
    grown.set(this.arenaBuf.subarray(0, this.arenaLen));
    this.arenaBuf = grown;
  }
}
