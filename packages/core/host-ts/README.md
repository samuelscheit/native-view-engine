# @nve/host (TypeScript)

Zero-dependency ESM TypeScript NVE host runtime. It uses only erasable syntax (`erasableSyntaxOnly`),
so Node runs the sources directly and any bundler can consume them.

**64-bit rule:**

- Keys and digests are `Key64(lo, hi)` of unsigned 32-bit halves. XXH64 runs on plain numbers with
  every intermediate below 2^53, so it needs no BigInt in the hot loop.
- An `i64` wire value is an exact `bigint` (`readI64`) or exact halves (`readI64Bits`).
- A 64-bit value is never passed through `Number(...)`. The ast-grep rule
  `no-unsafe-bigint64-to-number` enforces this.

Every index, count and offset must be an integer. `NaN`, `Infinity` and fractions throw instead of
slipping past a bounds check. This covers `Key64` halves, `BinWriter` capacity, and layout strides.
`FieldSpec` is a nominal class that is validated when constructed. A container mark is a branded
`BinMark` carrying the generation, so a stale or already-closed mark is rejected, and a rejected
write leaves no bytes.
Strings reject lone surrogates (`isWellFormed`), are encoded in place with `encodeInto`, and keep a
leading BOM.

| File | Contents |
|---|---|
| `src/key64.ts` | `Key64`, with `bigint` and safe-integer conversions that throw instead of rounding |
| `src/utf8.ts` | Strict `isValidUtf8`, BOM-preserving `UTF8_DECODER`, in-place code-point encoding |
| `src/xxh64.ts` | `Xxh64Hasher`, `xxh64`, `constId`: canonical XXH64, bit-identical to Rust `nve-hash` |
| `src/bin_codec.ts` | `BinReader` / `BinWriter`, the `NveBinValue` format, `BinMark` (generation-checked container marks) |
| `src/table_reader.ts` | `FieldKind`, `FieldSpec`, `decodeLength`, `rgba`, `TableReader` |
| `src/scene_reader.ts` | `SceneReader` over slabs or `SceneReader.fromLinearMemory(memory, ptr, layout, schema)` |
| `src/hybrid_runtime.ts` | `HybridStatus`, `HostStatus`, `HybridError`, `DynamicValue`, WASM linear-memory hybrid call helpers |
| `src/js_value.ts` | 53-bit-safe plain JS <-> `NveBinValue` codec (`encodeJsValue`, `decodeJsValue`, `writeJsValue`, `readJsValue`) |

## Generated data & consumption

`nve codegen abi` writes `src/generated/nve_abi.generated.ts` and `nve_tables.generated.ts` (exported from `index.ts`) and `test/hash_vectors.generated.ts` (all gitignored).

Generated `<stem>_hybrid.generated.ts` clients and `<nve-*>.generated.js` Web Components import `'@nve/host'`. Because this package exports `./src/index.ts` directly (erasable TypeScript, no committed `dist/` build artifacts), browsers and apps consume generated files through a TS-aware bundler (Vite, esbuild, Rollup, webpack) or workspace package resolver; Node runs them directly with native type-stripping when resolved outside `node_modules/`.

## Verify

`scripts/host-runtimes.sh ts` runs this suite. `scripts/check-all.sh` runs it too: C/C++ and TS in `--quick` mode, all five in a full run.

```bash
node --test packages/core/host-ts/test/*.test.ts
npx -y -p typescript tsc --noEmit -p packages/core/host-ts/tsconfig.json
```

The `tsc` check needs no install: `test/node_test.d.ts` declares the small `node:test` /
`node:assert` surface the suites use.
