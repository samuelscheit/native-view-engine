# dev.nve.host (Java)

Zero-dependency Java 11 runtime (`dev.nve.host`) for Android and the JVM. It holds the static half of
every Java / Kotlin host binding.

| Class | Contents |
|---|---|
| `Xxh64`, `Key64` | Canonical XXH64 (unsigned 64-bit in a `long`), bit-identical to Rust `nve-hash`, plus `Key64` halves |
| `BinReader`, `BinWriter`, `BinTag`, `BinCodecException`, `Utf8` | Bounds-checked LE primitives, the `NveBinValue` format, strict UTF-8 |
| `FieldKind`, `FieldSpec`, `Length`, `Rgba`, `TableReader` | Reusable, allocation-free word-table reader over direct `ByteBuffer`s |
| `SceneReader`, `SceneException` | Reusable `NveSceneView` frame reader. `attach` validates once; node fields are read by generated offsets. |
| `HybridRuntime` | `HybridStatus`, `HostStatus`, `HybridException`, `checkSchema`, `DynamicValue` |

`attach` sets the passed buffers to little-endian and reads relative to their current position.

## Verify

```bash
O=$(mktemp -d) && javac --release 11 -Xlint:all -Werror -d "$O" $(find packages/core/host-java/src -name '*.java') \
  && java -cp "$O" dev.nve.host.HostTestMain
# or, with Gradle 8 on JDK 17/21 (the suite is a plain main, so it runs offline):
hosts/compose-kit/gradlew -p packages/core/host-java --offline check
```

Consumers wire the package in with `includeBuild("../../packages/core/host-java")` (Gradle) or by
adding `packages/core/host-java/src/main/java` to the `javac` source path.
