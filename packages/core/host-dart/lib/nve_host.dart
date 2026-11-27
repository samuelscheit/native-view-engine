/// Zero-dependency NVE host runtime: XXH64 / Key64 on 32-bit halves, the `NveBinValue` codec,
/// word-table / scene readers, the runtime scene reader, and the generated core table layouts.
/// Pure Dart (no `dart:ffi`, no `dart:io`), bit-identical on the Dart VM and on Dart-web; the
/// native `libnve_runtime` session lives in `nve_native.dart`.
library;

export 'src/bin_codec.dart';
export 'src/bin_dynamic.dart';
export 'src/hybrid_runtime.dart';
export 'src/key64.dart' show Key64;
export 'src/nve_tables.generated.dart';
export 'src/runtime_scene_reader.dart';
export 'src/scene_reader.dart';
export 'src/table_reader.dart';
export 'src/utf8.dart' show decodeUtf8PreserveBom, isValidUtf8, utf8EncodedLength;
export 'src/xxh64.dart';
