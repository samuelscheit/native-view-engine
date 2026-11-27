/** Hybrid FFI prelude shared by every generated TypeScript / WASM hybrid client. */
import { BIN_VALUE_MAX_DEPTH, BinCodecError, BinReader, BinTag, BinWriter } from './bin_codec.ts';
import { Key64 } from './key64.ts';

/** Status of a fallible hybrid FFI trampoline (Rust `NveHybridStatus`, `#[repr(i32)]`). */
export const HybridStatus = { Ok: 0, Err: 1, InvalidArg: 2, SchemaMismatch: 3, NullHandle: 4 } as const;
export type HybridStatus = (typeof HybridStatus)[keyof typeof HybridStatus];

/** Status of `NveHostCallbacks::invoke` / `channel_publish` (Rust `NveHostStatus`, `#[repr(i32)]`). */
export const HostStatus = {
  Ok: 0,
  Pending: 1,
  BufferTooSmall: -1,
  Unsupported: -2,
  PermissionDenied: -3,
  InvalidArg: -4,
  NotFound: -5,
  Cancelled: -6,
  IoError: -7,
  InternalError: -8,
} as const;
export type HostStatus = (typeof HostStatus)[keyof typeof HostStatus];

/** A hybrid call failed: bad status, schema mismatch, or a remote error message. */
export class HybridError extends Error {
  readonly status: HybridStatus;

  constructor(status: HybridStatus, detail: string) {
    super(`hybrid status ${status}: ${detail}`);
    this.name = 'HybridError';
    this.status = status;
  }

  /** Throws `SchemaMismatch` unless a peer's schema hash equals the generated one. */
  static checkSchema(expected: Key64, actual: Key64): void {
    if (!expected.equals(actual)) throw new HybridError(HybridStatus.SchemaMismatch, `expected ${expected}, got ${actual}`);
  }
}

/** Owned dynamic value in the compact `NveBinValue` encoding; always exactly one valid value. */
export class DynamicValue {
  static readonly NULL = new DynamicValue(Uint8Array.of(BinTag.Null), 0);

  private readonly data: Uint8Array;
  private readonly maxDepth: number;

  private constructor(data: Uint8Array, maxDepth: number) {
    this.data = data;
    this.maxDepth = maxDepth;
  }

  /** Validated copy of `bytes`; throws `BinCodecError` when invalid. */
  static of(bytes: Uint8Array): DynamicValue {
    const maxDepth = BinReader.validate(bytes);
    return new DynamicValue(bytes.slice(), maxDepth);
  }

  /** Reads, validates, and copies the next value from `r`. */
  static read(r: BinReader): DynamicValue {
    const start = r.position;
    const maxDepth = r.checkValue();
    return new DynamicValue(r.bytes.slice(start, r.position), maxDepth);
  }

  /** Copy of the encoded bytes. */
  get bytes(): Uint8Array {
    return this.data.slice();
  }

  get tag(): BinTag {
    return this.data[0] as BinTag;
  }

  get isNull(): boolean {
    return this.tag === BinTag.Null;
  }

  /** Maximum container nesting depth contained inside this value (`0` for scalars). */
  get containerDepth(): number {
    return this.maxDepth;
  }

  reader(): BinReader {
    return new BinReader(this.data);
  }

  /**
   * Writes this already-validated `NveBinValue` directly into `w` without copying.
   * Rejects writes whose combined nesting depth would exceed `BIN_VALUE_MAX_DEPTH`.
   */
  writeTo(w: BinWriter): void {
    if (this.maxDepth > 0 && w.containerDepth + this.maxDepth > BIN_VALUE_MAX_DEPTH) {
      throw new BinCodecError('maxDepth', `depth ${w.containerDepth + this.maxDepth}`);
    }
    w.raw(this.data);
  }

  equals(other: DynamicValue): boolean {
    return this.data.length === other.data.length && this.data.every((b, i) => b === other.data[i]);
  }
}

/** Base linear-memory exports every hybrid WASM module provides. */
export interface HybridWasmExports {
  readonly memory: WebAssembly.Memory;
  nve_hybrid_alloc(len: number): number;
  nve_hybrid_free(ptr: number, len: number): void;
}

const U64_MAX = 2n ** 64n - 1n;

/** Writes a safe integer checked against `[min, max]` as an `NveBinValue` `i64`. */
export function writeIntInRange(w: BinWriter, value: number, min: number, max: number): void {
  if (!(Number.isSafeInteger(min) && Number.isSafeInteger(max) && min <= max)) {
    throw new RangeError(`invalid safe-integer bounds [${min}, ${max}]`);
  }
  if (!(Number.isSafeInteger(value) && value >= min && value <= max)) {
    throw new RangeError(`integer ${value} outside [${min}, ${max}]`);
  }
  w.writeI64Bits(Key64.fromSafeInteger(value));
}

/** Writes an unsigned `u64` `bigint`, rejecting values outside `[0, 2^64 - 1]`. */
export function writeU64Checked(w: BinWriter, value: bigint): void {
  if (value < 0n || value > U64_MAX) throw new RangeError(`${value} is outside u64`);
  w.writeI64(value);
}

/** Reads an `i64` tag and narrows to a JS `number` only within safe bounds `[min, max]` (zero `BigInt` on fast path). */
export function readIntInRange(r: BinReader, min: number, max: number): number {
  if (!(Number.isSafeInteger(min) && Number.isSafeInteger(max) && min <= max)) {
    throw new RangeError(`invalid safe-integer bounds [${min}, ${max}]`);
  }
  const bits = r.readI64Bits();
  const v = bits.toSignedSafeInteger();
  if (v === null || v < min || v > max) {
    throw new RangeError(`i64 ${v ?? bits.toSignedBigInt()} outside [${min}, ${max}]`);
  }
  return v;
}

/** Reads the leading `Bool` tag of a `Result` payload; throws `HybridError(Err, msg)` on `false`. */
export function readResultOk(r: BinReader): void {
  if (!r.readBool()) {
    const msg = r.readStr();
    r.requireEnd();
    throw new HybridError(HybridStatus.Err, msg);
  }
}

function toHybridStatus(code: number): HybridStatus {
  switch (code) {
    case HybridStatus.Ok:
    case HybridStatus.Err:
    case HybridStatus.InvalidArg:
    case HybridStatus.SchemaMismatch:
    case HybridStatus.NullHandle:
      return code;
    default:
      return HybridStatus.Err;
  }
}

/** Verifies the peer's schema hash and creates a non-null WASM hybrid instance handle. */
export function wasmCreateHandle(
  name: string,
  expectedSchema: Key64,
  actualSchema: bigint,
  create: () => number,
): number {
  HybridError.checkSchema(expectedSchema, Key64.fromBigInt(actualSchema));
  const handle = create() >>> 0;
  if (handle === 0) throw new HybridError(HybridStatus.NullHandle, `failed to create ${name} hybrid instance`);
  return handle;
}

/**
 * Allocates `bytes.byteLength` bytes in WASM linear memory and copies `bytes` into it (`0` when empty).
 * Validates `u32` length before allocation, snapshots views aliasing `wasm.memory.buffer` so
 * `memory.grow()` cannot detach the source, and never passes corrupt/out-of-bounds pointers to `free`.
 */
export function wasmCopyIn(wasm: HybridWasmExports, bytes: Uint8Array): number {
  const len = bytes.byteLength;
  if (len === 0) return 0;
  if (len > 0xffff_ffff) throw new RangeError(`wasm payload length ${len} exceeds u32`);
  const src = bytes.buffer === wasm.memory.buffer ? bytes.slice() : bytes;
  const ptr = wasm.nve_hybrid_alloc(len) >>> 0;
  if (ptr === 0) throw new HybridError(HybridStatus.Err, `wasm alloc(${len}) returned null`);
  const mem = wasm.memory.buffer;
  if (ptr + len > mem.byteLength) {
    throw new HybridError(HybridStatus.Err, `wasm alloc(${len}) at ${ptr} outside memory (${mem.byteLength})`);
  }
  try {
    new Uint8Array(mem, ptr, len).set(src);
  } catch (e) {
    wasm.nve_hybrid_free(ptr, len);
    throw e;
  }
  return ptr;
}

/** Sends a binary-encoded prop map to a WASM hybrid view instance. */
export function wasmSetPropsBin(
  wasm: HybridWasmExports,
  name: string,
  handle: number,
  payload: Uint8Array,
  setPropsFn: (handle: number, ptr: number, len: number) => number,
): void {
  if (handle === 0) throw new HybridError(HybridStatus.NullHandle, `${name} client is disposed`);
  // Cache length before `wasmCopyIn` so `memory.grow()` detaching an aliased view cannot zero it.
  const payloadLen = payload.byteLength;
  const ptr = wasmCopyIn(wasm, payload);
  try {
    const status = setPropsFn(handle, ptr, payloadLen);
    if (status !== HybridStatus.Ok) {
      throw new HybridError(toHybridStatus(status), `setProps failed on ${name} (status ${status})`);
    }
  } finally {
    if (ptr !== 0) wasm.nve_hybrid_free(ptr, payloadLen);
  }
}

/** Invokes a binary WASM hybrid method, copying any response bytes out before freeing them. */
export function wasmCallBin(
  wasm: HybridWasmExports,
  name: string,
  handle: number,
  req: Uint8Array,
  callFn: (handle: number, reqPtr: number, reqLen: number, outLenPtr: number) => number,
): Uint8Array {
  if (handle === 0) throw new HybridError(HybridStatus.NullHandle, `${name} client is disposed`);
  // Cache length before `wasmCopyIn` so `memory.grow()` detaching an aliased view cannot zero it.
  const reqLen = req.byteLength;
  const reqPtr = wasmCopyIn(wasm, req);
  let outLenPtr = 0;
  try {
    const rawOutLenPtr = wasm.nve_hybrid_alloc(4) >>> 0;
    if (rawOutLenPtr === 0 || rawOutLenPtr + 4 > wasm.memory.buffer.byteLength) {
      throw new HybridError(
        HybridStatus.Err,
        `wasm alloc(4) for outLen returned invalid ptr ${rawOutLenPtr} (memory ${wasm.memory.buffer.byteLength}) on ${name}`,
      );
    }
    // Register for `finally` cleanup only after verifying `rawOutLenPtr` is in linear-memory bounds.
    outLenPtr = rawOutLenPtr;
    new DataView(wasm.memory.buffer).setUint32(outLenPtr, 0, true);
    const respPtr = callFn(handle, reqPtr, reqLen, outLenPtr) >>> 0;
    // Re-read `wasm.memory.buffer` after the call in case WASM grew linear memory.
    const mem = wasm.memory.buffer;
    const respLen = new DataView(mem).getUint32(outLenPtr, true);
    if (respPtr === 0) {
      if (respLen > 0) {
        throw new HybridError(HybridStatus.Err, `wasm returned null response pointer with length ${respLen} on ${name}`);
      }
      return new Uint8Array(0);
    }
    if (respLen === 0) {
      // Zero-length allocations must never be passed to Rust's `dealloc` (size 0 is UB); still guard bounds.
      if (respPtr > mem.byteLength) {
        throw new HybridError(
          HybridStatus.Err,
          `wasm response ptr ${respPtr} (+0) outside memory (${mem.byteLength}) on ${name}`,
        );
      }
      return new Uint8Array(0);
    }
    // Validate bounds BEFORE entering `finally` so corrupt pointers are never passed to `nve_hybrid_free`.
    if (respPtr + respLen > mem.byteLength) {
      throw new HybridError(
        HybridStatus.Err,
        `wasm response [${respPtr}, +${respLen}) outside memory (${mem.byteLength})`,
      );
    }
    try {
      return new Uint8Array(mem, respPtr, respLen).slice();
    } finally {
      wasm.nve_hybrid_free(respPtr, respLen);
    }
  } finally {
    if (reqPtr !== 0) wasm.nve_hybrid_free(reqPtr, reqLen);
    if (outLenPtr !== 0) wasm.nve_hybrid_free(outLenPtr, 4);
  }
}
