/** Hybrid FFI prelude shared by every generated TypeScript / WASM hybrid client. */
import { BinReader, BinTag } from './bin_codec.ts';
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

  equals(other: DynamicValue): boolean {
    return this.data.length === other.data.length && this.data.every((b, i) => b === other.data[i]);
  }
}
