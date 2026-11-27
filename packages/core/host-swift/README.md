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
| `HybridRuntime.swift` | `HybridStatus`, `HostStatus`, `HybridError`, `DynamicValue` |

The generated C structs conform with empty extensions, because Swift imports their fields with exactly
the protocol's names and types (implicitly unwrapped pointers):

```swift
extension NveTableView: TableViewABI {}
extension NveSceneView: SceneViewABI {}
let scene = try SceneReader.open(nve_scene_view(), expectedSchema: Key64(NVE_SCENE_SCHEMA_HASH))
```

`Tests/CNveHostTestABI` is a test-only C mirror of those structs. It proves this conformance in CI.

## Verify

```bash
swift test --package-path packages/core/host-swift --scratch-path "$TMPDIR/nve-host-swift" -Xswiftc -warnings-as-errors
```
