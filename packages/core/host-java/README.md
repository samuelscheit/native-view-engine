# dev.nve.host (Java)

Zero-dependency Java 11 runtime (`dev.nve.host`) for Android and the JVM. It holds the static half of
every Java / Kotlin host binding.

| Class | Contents |
|---|---|
| `Xxh64`, `Key64` | Canonical XXH64 (unsigned 64-bit in a `long`), bit-identical to Rust `nve-hash`, plus `Key64` halves |
| `BinReader`, `BinWriter`, `BinTag`, `BinCodecException`, `Utf8` | Bounds-checked LE primitives, the `NveBinValue` format, strict UTF-8 |
| `FieldKind`, `FieldSpec`, `Length`, `Rgba`, `TableReader` | Reusable, allocation-free word-table reader over direct `ByteBuffer`s |
| `RuntimeSceneReader`, `SceneException` | Reusable `NveRuntimeSceneView` frame reader over JNI direct buffers (32 B `NveNodeRow`s, 16 B `NveLayoutRect`s, text/children slabs, dirty/removed deltas, published tables, `scene_epoch`, `dirty_resources`). `attach` validates schema, slab sizes, dirty indexes, and table slots once before mutating (a rejected frame leaves the reader detached); the packed `attach(long[] header, ByteBuffer[] slabs)` takes a whole frame in one JNI crossing (`H_*` / `S_*` layout). Pooled, allocation-free in steady state. |
| `NativeRuntime`, `RuntimeSurface`, `EventRingWriter`, `ImageSource` | JNI binding of `libnve_runtime` (implemented by `hosts/android-shell`; every call returns a non-negative `STATUS_*`, values via out arrays), the owner-thread surface (frame consumer, re-entrancy guard: in-frame `send` / `scrollBy` are deferred past scene release, structural calls throw, reader detached before release), the allocation-free `EventRing` writer (byte-checked against the Rust golden `EventRingVectors`), and the `nve.res.image_src` codec |
| `SceneSemantics`, `AndroidKeys` | Shared host semantics: node kind from table presence, pressability, inherited `nve.typography` (cycle-capped ancestor walk); Android key codes → W3C `key` / `code` |
| `HybridRuntime` | `HybridStatus`, `HostStatus`, `HybridException`, `checkSchema`, `DynamicValue`, `directBuffer` |

Generated Kotlin / Java hybrid clients (`nve codegen hybrid --target kotlin`) import this runtime and
never copy it: structs encode through `BinWriter` (`beginMap` / `end`) and decode through `BinReader`
(`beginMap` / `beginArray` return a `ReadMark` that bounds every read until `end(mark)`, which also
rejects unread entries);
JNI arguments cross as `BinWriter.toDirectBuffer()` / `HybridRuntime.directBuffer` (capacity == size,
little-endian), `NveValue` fields as `DynamicValue` (`read` validates, `writeTo` appends without a
copy), and every result is checked with `BinReader.expectEnd()`.

- Strings are encoded straight into the writer's buffer, and so is the input to `Xxh64.constId` /
  `Hasher.updateUtf8`. A lone surrogate is rejected with `INVALID_UTF8`; it is never replaced with `?`.
- `FieldSpec` rejects invalid specs at construction. Its checks subtract instead of add, so `int`
  overflow cannot slip a spec through.
- Container marks are `long` values carrying the writer's generation, so `end` rejects a stale or
  already-closed mark.
- A rejected or oversized write leaves no bytes.
- Validation does not allocate. The suite proves this with the thread allocation counter.

`attach` sets the passed buffers to little-endian and reads relative to their current position.

## Generated data

`nve codegen abi` writes `dev.nve.host.generated.{NveAbi,NveTables,NveRuntimeAbi,NveTableBits,NveModuleTables}` and the test-only `HashVectors` / `EventRingVectors` (gitignored). `GeneratedTest` and `EventRingTest` check them against this runtime.

## Verify

`scripts/host-runtimes.sh java` runs this suite. `scripts/check-all.sh` runs it too: C/C++ and TS in `--quick` mode, all five in a full run.

```bash
O=$(mktemp -d) && javac --release 11 -Xlint:all -Werror -d "$O" $(find packages/core/host-java/src -name '*.java') \
  && java -cp "$O" dev.nve.host.HostTestMain
# or, with Gradle 8 on JDK 17/21 (the suite is a plain main, so it runs offline):
hosts/compose-kit/gradlew -p packages/core/host-java --offline check
```

Consumers wire the package in with `includeBuild("../../packages/core/host-java")` (Gradle) or by
adding `packages/core/host-java/src/main/java` to the `javac` source path.
