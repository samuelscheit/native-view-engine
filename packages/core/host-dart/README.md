# nve_host (Dart)

Zero-dependency pure-Dart NVE host runtime for the VM, Flutter, and Dart-web. It imports no
`dart:ffi` and no `dart:io`.

**64-bit rule:** every 64-bit value is a `Key64(lo, hi)` of unsigned 32-bit halves:

- XXH64, `i64` / `u64` wire fields, schema hashes, and frame ids all use it.
- All arithmetic stays below 2^53, and every bitwise result is masked to 32 bits.
- No integer literal exceeds 2^53 (`avoid_js_rounded_ints: error`).
- `ByteData.getInt64` / `getUint64` are never called; they throw on the web.

The same suite therefore passes bit-identically on the Dart VM and compiled to JavaScript.

Strings follow Rust `&str` semantics:

- A leading U+FEFF is preserved. `dart:convert` would strip it; `decodeUtf8PreserveBom` restores it.
- A lone surrogate is rejected with `invalidUtf8`; it is never encoded as U+FFFD.
- Writers and `constId` encode in place without building a temporary list.

Counts are checked by division, never by multiplying, so a hostile value cannot wrap the 64-bit VM `int`.
Container marks are `BinMark` extension types holding the generation and offset, so a stale or
already-closed mark is rejected, and a rejected write leaves no bytes.

| File | Contents |
|---|---|
| `src/key64.dart`, `src/u64_math.dart` | `Key64` with exact `int` conversion guards per platform, and the wrapping u64 math |
| `src/utf8.dart` | Strict `isValidUtf8`, `decodeUtf8PreserveBom`, in-place encoding that rejects lone surrogates |
| `src/xxh64.dart` | `Xxh64Hasher`, `xxh64`, `constId`: canonical XXH64, bit-identical to Rust `nve-hash` |
| `src/bin_codec.dart` | `BinReader` / `BinWriter`, the `NveBinValue` format (`readI64Bits` exact, `readI64` guarded), `isValidUtf8` |
| `src/table_reader.dart` | `FieldKind`, `FieldSpec`, `LengthValue`, `Rgba`, `TableReader` |
| `src/scene_reader.dart` | `SceneReader` over slabs or over wasm32 linear memory via a generated `SceneLayout` |
| `src/hybrid_runtime.dart` | `HybridStatus`, `HostStatus`, `HybridException`, `DynamicValue` |

## Verify

`scripts/host-runtimes.sh dart` runs this suite. `scripts/check-all.sh` runs it too: C/C++ and TS in `--quick` mode, all five in a full run.

```bash
cd packages/core/host-dart
dart pub get --offline && dart analyze --fatal-infos
dart run test/host_test.dart
dart compile js -O2 -o "$TMPDIR/nve_host_web.js" test/web_parity_entry.dart && node "$TMPDIR/nve_host_web.js"
```
