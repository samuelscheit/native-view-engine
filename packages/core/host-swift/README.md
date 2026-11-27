# NveHost (Swift)

Zero-dependency SwiftPM package: the static NVE host runtime for Apple platforms. It builds in Swift 6
language mode, so strict concurrency is on.

| File | Contents |
|---|---|
| `Xxh64.swift` | `Xxh64` / `Xxh64Hasher`: canonical XXH64, bit-identical to Rust `nve-hash`; an allocation-free value type |
| `Key64.swift` | `Key64`: 64-bit identity key with `(lo, hi)` halves |
| `BinCodec.swift` | `BinReader` / `BinWriter` (typed `throws(BinError)`), the `NveBinValue` format, `Utf8.isValid` |
| `TableReader.swift` | `FieldKind`, `FieldSpec`, `LengthValue`, `Rgba`, `StrSpan`, `TableReader`, `TableViewABI` |
| `SceneReader.swift` | `SceneReader<View: SceneViewABI>`: validates once, then O(1) bounds-checked reads |
| `HybridRuntime.swift` | `HybridStatus`, `HostStatus`, `HybridError`, `HybridEnum`, `HybridInterfaceResolver`, `DynamicValue` |
| `NveHostAbi/NveValueTree.swift` | `NveValueTree`: the C `NveValue` graph one ABI call borrows (separate target: it imports `CNveAbi`) |

Generated hybrid Swift (`nve codegen hybrid <spec> --target swift`) copies no runtime: it imports `NveHost`
(codec, errors, enums, dynamic values) and `NveHostAbi`. `setProps` and `NveValue` parameters build an
`NveValueTree` that outlives the call: a `DynamicValue` becomes a lossless tree with its own tags
(`Null` ... `Map`, never wrapped as `Bytes`), while `Struct` / `Vec` props cross as one `NveBinValue` in
`Bytes`. The tree is a chunked bump arena (one allocation per chunk, never per node; scalar values
allocate nothing), and nothing it hands out moves before it is released.

The generated file also needs its C header, `<Stem>HybridAbi.generated.h`, with host-cxx's `include/`
(the `CNveAbi` module) on the header search path:

- **App target:** include the header from the bridging header.
- **SwiftPM:** packages have no bridging headers, so add a C target named `<Stem>HybridAbi` whose
  `include/` holds the header; the generated file imports it under `#if canImport(<Stem>HybridAbi)`.
  Several generated files can share one module: their constants live in `<Stem>HybridFile`.

The generated C structs conform with empty extensions, because Swift imports their fields with exactly
the protocol's names and types (implicitly unwrapped pointers):

```swift
extension NveTableView: TableViewABI {}
extension NveSceneView: SceneViewABI {}
let scene = try SceneReader.open(nve_scene_view(), expectedSchema: Key64(NVE_SCENE_SCHEMA_HASH))
```

- `FieldSpec.init` throws for an invalid spec, so every `FieldSpec` is well-formed.
- Wire lengths, offsets and counts are compared as `UInt64` before they become an `Int`, so hostile input
  cannot trap, including on 32-bit `arm64_32` watchOS. The suite compile-checks `arm64_32` when the
  watchOS SDK is present.
- While validating, reads stay inside the container being checked, and strings are validated in place
  without being decoded.
- Container marks carry the writer's generation, so `end` rejects a stale mark (from before
  `clear()`) and a container that is already closed.
- The allocation guard counts every malloc call through libmalloc's `malloc_logger` hook. It runs in
  the release suite (`swift test -c release`); in `-Onone` builds even trivial calls allocate.

`Tests/CNveHostTestABI` is a test-only C mirror of those structs. It proves this conformance in CI.

## Generated data

`nve codegen abi` writes `Sources/NveHost/NveTables.generated.swift` and `Tests/NveHostTests/HashVectors.generated.swift` (gitignored). The `CNveAbi` target's module map imports host-cxx's generated `nve_abi.generated.h` in place, so the C ABI exists once.

## Verify

`scripts/host-runtimes.sh swift` runs this suite. `scripts/check-all.sh` runs it too: C/C++ and TS in `--quick` mode, all five in a full run.
`cargo test -p nve-codegen --test hybrid_swift_runtime_tests` links generated Swift clients against both
modules and a live Rust provider, under AddressSanitizer.

```bash
swift test --package-path packages/core/host-swift --scratch-path "$TMPDIR/nve-host-swift" -Xswiftc -warnings-as-errors
```
