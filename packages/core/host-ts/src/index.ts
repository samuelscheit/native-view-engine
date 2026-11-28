/**
 * Zero-dependency NVE host runtime: 53-bit-safe XXH64 / `Key64` on `(lo, hi)` u32 halves, the
 * `NveBinValue` codec, and word-table / scene readers. Erasable TypeScript, runnable directly by
 * Node's type stripping and by any bundler.
 */
export * from './bin_codec.ts';
export * from './event_batch.ts';
export * from './generated/nve_abi.generated.ts';
export * from './generated/nve_tables.generated.ts';
export * from './hybrid_runtime.ts';
export * from './js_value.ts';
export * from './key64.ts';
export * from './scene_reader.ts';
export * from './table_reader.ts';
export * from './utf8.ts';
export * from './xxh64.ts';
