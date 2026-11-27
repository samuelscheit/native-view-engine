# NveHost (Swift)

Zero-dependency SwiftPM package: the static NVE host runtime for Apple platforms. It builds in Swift 6
language mode, so strict concurrency is on.

| File | Contents |
|---|---|
| `Xxh64.swift` | `Xxh64` / `Xxh64Hasher`: canonical XXH64, bit-identical to Rust `nve-hash`; an allocation-free value type |
| `Key64.swift` | `Key64`: 64-bit identity key with `(lo, hi)` halves |
| `BinCodec.swift` | `BinReader` / `BinWriter` (typed `throws(BinError)`), the `NveBinValue` format, `Utf8.isValid` |
| `TableReader.swift` | `FieldKind`, `FieldSpec`, `LengthValue`, `Rgba`, `StrSpan`, `TableReader`, `TableViewABI` |
| `RuntimeSceneReader.swift` | `RuntimeSceneReader<View: RuntimeSceneViewABI>` over `libnve_runtime`'s `NveRuntimeSceneView` (`NveNodeRow` / `NveLayoutRect` / slabs / tables), typed `NodeId`: validates once (null, address-space fit, alignment, UTF-8, tables), then O(1) bounds-checked, allocation-free reads |
| `HybridRuntime.swift` | `HybridStatus`, `HostStatus`, `HybridError`, `DynamicValue` |

The generated C structs conform with empty extensions, because Swift imports their fields with exactly
the protocol's names and types (implicitly unwrapped pointers):

```swift
extension NveTableView: TableViewABI {}
extension NveNodeRow: NodeRowABI {}
extension NveLayoutRect: LayoutRectABI {}
extension NveRuntimeSceneView: RuntimeSceneViewABI {}
var raw = NveRuntimeSceneView()
guard api.pointee.scene_acquire(handle, &raw) == 0 else { return }
defer { _ = api.pointee.scene_release(handle) }
let scene = try RuntimeSceneReader.open(raw, expectedSchema: Key64(NVE_RUNTIME_SCHEMA_HASH))
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

## Verify

`scripts/host-runtimes.sh swift` runs this suite. `scripts/check-all.sh` runs it too: C/C++ and TS in `--quick` mode, all five in a full run.

```bash
swift test --package-path packages/core/host-swift --scratch-path "$TMPDIR/nve-host-swift" -Xswiftc -warnings-as-errors
```
