/// Zero-dependency NVE host runtime: XXH64 / Key64 on 32-bit halves, the `NveBinValue` codec,
/// and word-table / scene readers. Pure Dart (no `dart:ffi`, no `dart:io`), bit-identical on the
/// Dart VM and on Dart-web.
library;

export 'src/bin_codec.dart';
export 'src/hybrid_runtime.dart';
export 'src/key64.dart' show Key64;
export 'src/scene_reader.dart';
export 'src/table_reader.dart';
export 'src/xxh64.dart';
