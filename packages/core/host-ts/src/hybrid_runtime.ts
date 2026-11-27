/** Hybrid FFI prelude shared by every generated TypeScript / WASM hybrid client. */
import { BinReader, BinTag, BinWriter } from './bin_codec.ts';
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
  static readonly NULL = new DynamicValue(Uint8Array.of(BinTag.Null));

  private readonly data: Uint8Array;

  private constructor(data: Uint8Array) {
    this.data = data;
  }

  /** Validated copy of `bytes`; throws `BinCodecError` when invalid. */
  static of(bytes: Uint8Array): DynamicValue {
    BinReader.validate(bytes);
    return new DynamicValue(bytes.slice());
  }

  /** Reads, validates, and copies the next value from `r`. */
  static read(r: BinReader): DynamicValue {
    return new DynamicValue(r.readCheckedValue().slice());
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

  reader(): BinReader {
    return new BinReader(this.data);
  }

  /** Writes this already-validated `NveBinValue` directly into `w` without copying. */
  writeTo(w: BinWriter): void {
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

const I64_MIN = -(2n ** 63n);
const I64_MAX = 2n ** 63n - 1n;
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

/** Writes a signed `i64` `bigint`, rejecting values outside `[-2^63, 2^63 - 1]`. */
export function writeI64Checked(w: BinWriter, value: bigint): void {
  if (value < I64_MIN || value > I64_MAX) throw new RangeError(`${value} is outside i64`);
  w.writeI64(value);
}

/** Writes an unsigned `u64` `bigint`, rejecting values outside `[0, 2^64 - 1]`. */
export function writeU64Checked(w: BinWriter, value: bigint): void {
  if (value < 0n || value > U64_MAX) throw new RangeError(`${value} is outside u64`);
  w.writeI64(value);
}

/** Reads an `i64` tag and narrows to a JS `number` only within safe bounds `[min, max]`. */
export function readIntInRange(r: BinReader, min: number, max: number): number {
  if (!(Number.isSafeInteger(min) && Number.isSafeInteger(max) && min <= max)) {
    throw new RangeError(`invalid safe-integer bounds [${min}, ${max}]`);
  }
  const v = r.readI64();
  if (v < BigInt(min) || v > BigInt(max)) {
    throw new RangeError(`i64 ${v} outside [${min}, ${max}]`);
  }
  return Number(v);
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

/** Allocates `bytes.byteLength` bytes in WASM linear memory and copies `bytes` into it (`0` when empty). */
export function wasmCopyIn(wasm: HybridWasmExports, bytes: Uint8Array): number {
  const len = bytes.byteLength;
  if (len === 0) return 0;
  const ptr = wasm.nve_hybrid_alloc(len) >>> 0;
  if (ptr === 0) throw new HybridError(HybridStatus.Err, `wasm alloc(${len}) returned null`);
  const mem = wasm.memory.buffer;
  if (ptr + len > mem.byteLength) {
    wasm.nve_hybrid_free(ptr, len);
    throw new HybridError(HybridStatus.Err, `wasm alloc(${len}) at ${ptr} outside memory (${mem.byteLength})`);
  }
  new Uint8Array(mem, ptr, len).set(bytes);
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
  const ptr = wasmCopyIn(wasm, payload);
  try {
    const status = setPropsFn(handle, ptr, payload.byteLength);
    if (status !== HybridStatus.Ok) {
      throw new HybridError(toHybridStatus(status), `setProps failed on ${name} (status ${status})`);
    }
  } finally {
    if (ptr !== 0) wasm.nve_hybrid_free(ptr, payload.byteLength);
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
  const reqPtr = wasmCopyIn(wasm, req);
  let outLenPtr = 0;
  try {
    outLenPtr = wasm.nve_hybrid_alloc(4) >>> 0;
    if (outLenPtr === 0 || outLenPtr + 4 > wasm.memory.buffer.byteLength) {
      throw new HybridError(HybridStatus.Err, `wasm alloc(4) for outLen failed on ${name}`);
    }
    new DataView(wasm.memory.buffer).setUint32(outLenPtr, 0, true);
    const respPtr = callFn(handle, reqPtr, req.byteLength, outLenPtr) >>> 0;
    // Re-read `wasm.memory.buffer` after the call in case WASM grew linear memory.
    const mem = wasm.memory.buffer;
    const respLen = new DataView(mem).getUint32(outLenPtr, true);
    if (respPtr === 0) {
      if (respLen > 0) {
        throw new HybridError(HybridStatus.Err, `wasm returned null response pointer with length ${respLen} on ${name}`);
      }
      return new Uint8Array(0);
    }
    if (respLen === 0) return new Uint8Array(0);
    try {
      if (respPtr + respLen > mem.byteLength) {
        throw new HybridError(
          HybridStatus.Err,
          `wasm response [${respPtr}, +${respLen}) outside memory (${mem.byteLength})`,
        );
      }
      return new Uint8Array(mem, respPtr, respLen).slice();
    } finally {
      wasm.nve_hybrid_free(respPtr, respLen);
    }
  } finally {
    if (reqPtr !== 0) wasm.nve_hybrid_free(reqPtr, req.byteLength);
    if (outLenPtr !== 0) wasm.nve_hybrid_free(outLenPtr, 4);
  }
}
