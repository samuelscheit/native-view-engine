# nve_host (Dart)

Zero-dependency pure-Dart NVE host runtime for the VM, Flutter, and Dart-web. It imports no
`dart:ffi` and no `dart:io`.

**64-bit rule:** every 64-bit value is a `Key64(lo, hi)` of unsigned 32-bit halves:

- XXH64, `i64` / `u64` wire fields, schema hashes, and frame ids all use it.
- All arithmetic stays below 2^53, and every bitwise result is masked to 32 bits.
- No integer literal exceeds 2^53 (`avoid_js_rounded_ints: error`).
- `ByteData.getInt64` / `getUint64` are never called; they throw on the web.

The same suite therefore passes bit-identically on the Dart VM and compiled to JavaScript.

| File | Contents |
|---|---|
| `src/key64.dart`, `src/u64_math.dart` | `Key64` with exact `int` conversion guards per platform, and the wrapping u64 math |
| `src/xxh64.dart` | `Xxh64Hasher`, `xxh64`, `constId`: canonical XXH64, bit-identical to Rust `nve-hash` |
| `src/bin_codec.dart` | `BinReader` / `BinWriter`, the `NveBinValue` format (`readI64Bits` exact, `readI64` guarded), `isValidUtf8` |
| `src/table_reader.dart` | `FieldKind`, `FieldSpec`, `LengthValue`, `Rgba`, `TableReader` |
| `src/scene_reader.dart` | `SceneReader` over slabs or over wasm32 linear memory via a generated `SceneLayout` |
| `src/hybrid_runtime.dart` | `HybridStatus`, `HostStatus`, `HybridException`, `DynamicValue` |

## Verify

```bash
cd packages/core/host-dart
dart pub get --offline && dart analyze --fatal-infos
dart run test/host_test.dart
dart compile js -O2 -o "$TMPDIR/nve_host_web.js" test/web_parity_entry.dart && node "$TMPDIR/nve_host_web.js"
```
