# @nve/host (TypeScript)

Zero-dependency ESM TypeScript NVE host runtime. It uses only erasable syntax (`erasableSyntaxOnly`),
so Node runs the sources directly and any bundler can consume them.

**64-bit rule:**

- Keys and digests are `Key64(lo, hi)` of unsigned 32-bit halves. XXH64 runs on plain numbers with
  every intermediate below 2^53, so it needs no BigInt in the hot loop.
- An `i64` wire value is an exact `bigint` (`readI64`) or exact halves (`readI64Bits`).
- A 64-bit value is never passed through `Number(...)`. The ast-grep rule
  `no-unsafe-bigint64-to-number` enforces this.

| File | Contents |
|---|---|
| `src/key64.ts` | `Key64`, with `bigint` and safe-integer conversions that throw instead of rounding |
| `src/xxh64.ts` | `Xxh64Hasher`, `xxh64`, `constId`: canonical XXH64, bit-identical to Rust `nve-hash` |
| `src/bin_codec.ts` | `BinReader` / `BinWriter`, the `NveBinValue` format, `isValidUtf8`, a strict BOM-preserving `UTF8_DECODER` |
| `src/table_reader.ts` | `FieldKind`, `FieldSpec`, `decodeLength`, `rgba`, `TableReader` |
| `src/scene_reader.ts` | `SceneReader` over slabs or `SceneReader.fromLinearMemory(memory, ptr, layout, schema)` |
| `src/hybrid_runtime.ts` | `HybridStatus`, `HostStatus`, `HybridError`, `DynamicValue` |

## Verify

```bash
node --test packages/core/host-ts/test/*.test.ts
npx -y -p typescript tsc --noEmit -p packages/core/host-ts/tsconfig.json
```

The `tsc` check needs no install: `test/node_test.d.ts` declares the small `node:test` /
`node:assert` surface the suites use.
