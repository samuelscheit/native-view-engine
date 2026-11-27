/**
 * Plain JavaScript values carried as `NveBinValue` (web-component props and events, devtools).
 *
 * Integers stay exact: a safe integer (excluding `-0`, which keeps its sign as `f64`) or `bigint`
 * in `i64` range is written as `i64`, and an `i64` reads back as a `number` only when it is a safe
 * integer (`-(2^53 - 1) ..= 2^53 - 1`), otherwise as a `bigint`. Any other number is an `f64`.
 */
import {
  BIN_VALUE_MAX_DEPTH,
  BinCodecError,
  BinReader,
  BinTag,
  BinWriter,
  writeI64Checked,
} from './bin_codec.ts';
import { Key64 } from './key64.ts';

/** A value `writeJsValue` accepts and `readJsValue` returns. */
export type JsBinValue =
  | null
  | boolean
  | number
  | bigint
  | string
  | Uint8Array
  | readonly JsBinValue[]
  | { readonly [key: string]: JsBinValue };

/**
 * Writes `value` (`undefined` as null; object entries whose value is `undefined` are skipped).
 * Throws `TypeError` for functions, symbols, and non-plain objects, `RangeError` for a `bigint`
 * outside `i64`, and `BinCodecError('maxDepth')` past `BIN_VALUE_MAX_DEPTH` nested containers.
 */
export function writeJsValue(w: BinWriter, value: unknown): void {
  writeAt(w, value, w.containerDepth);
}

function writeAt(w: BinWriter, value: unknown, depth: number): void {
  if (value === null || value === undefined) return w.writeNull();
  switch (typeof value) {
    case 'boolean':
      return w.writeBool(value);
    case 'number':
      return !Object.is(value, -0) && Number.isSafeInteger(value)
        ? w.writeI64Bits(Key64.fromSafeInteger(value))
        : w.writeF64(value);
    case 'bigint':
      return writeI64Checked(w, value);
    case 'string':
      return w.writeStr(value);
    case 'object':
      break;
    default:
      throw new TypeError(`cannot encode a ${typeof value} as NveBinValue`);
  }
  if (value instanceof Uint8Array) return w.writeBytes(value);
  if (depth >= BIN_VALUE_MAX_DEPTH) throw new BinCodecError('maxDepth', `depth ${depth}`);
  if (Array.isArray(value)) {
    const mark = w.beginArray(value.length);
    for (let i = 0; i < value.length; i += 1) writeAt(w, value[i], depth + 1);
    return w.end(mark);
  }
  const proto: unknown = Object.getPrototypeOf(value);
  if (proto !== Object.prototype && proto !== null) {
    throw new TypeError('only plain objects encode as NveBinValue maps');
  }
  const entries = Object.entries(value).filter(([, v]) => v !== undefined);
  const mark = w.beginMap(entries.length);
  for (const [k, v] of entries) {
    w.writeMapKey(k);
    writeAt(w, v, depth + 1);
  }
  w.end(mark);
}

/** Reads one value; throws `BinCodecError` on malformed input, depth overflow, or container slack. */
export function readJsValue(r: BinReader): JsBinValue {
  switch (r.peekTag()) {
    case BinTag.Null:
      r.readNull();
      return null;
    case BinTag.Bool:
      return r.readBool();
    case BinTag.I64: {
      const bits = r.readI64Bits();
      return bits.toSignedSafeInteger() ?? bits.toSignedBigInt();
    }
    case BinTag.F64:
      return r.readF64();
    case BinTag.Str:
      return r.readStr();
    case BinTag.Bytes:
      return r.readBytes().slice();
    case BinTag.Array:
      return r.readArray((count) => {
        const out: JsBinValue[] = [];
        for (let i = 0; i < count; i += 1) out.push(readJsValue(r));
        return out;
      });
    case BinTag.Map:
      return r.readMap((count) => {
        const out: Record<string, JsBinValue> = {};
        for (let i = 0; i < count; i += 1) {
          const key = r.readMapKey();
          const val = readJsValue(r);
          if (key === '__proto__') {
            Object.defineProperty(out, key, {
              value: val,
              enumerable: true,
              writable: true,
              configurable: true,
            });
          } else {
            out[key] = val;
          }
        }
        return out;
      });
  }
}

/** `value` as one standalone `NveBinValue`. */
export function encodeJsValue(value: unknown): Uint8Array {
  const w = new BinWriter();
  writeJsValue(w, value);
  return w.toBytes();
}

/** The single `NveBinValue` in `bytes`; throws `BinCodecError` on malformed or trailing bytes. */
export function decodeJsValue(bytes: Uint8Array): JsBinValue {
  const r = new BinReader(bytes);
  const value = readJsValue(r);
  r.requireEnd();
  return value;
}
