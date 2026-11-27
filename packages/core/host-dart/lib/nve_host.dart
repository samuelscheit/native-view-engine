/// Zero-dependency NVE host runtime: XXH64 / Key64 on 32-bit halves, the `NveBinValue` codec,
/// word-table / scene readers, and the generated web-safe ABI constants and table specs. Pure Dart
/// (no `dart:ffi`, no `dart:io`), bit-identical on the Dart VM and on Dart-web. Native `dart:ffi`
/// structs live in `nve_host_ffi.dart`.
library;

export 'src/bin_codec.dart';
export 'src/generated/nve_abi_const.generated.dart';
export 'src/generated/nve_tables.generated.dart';
export 'src/hybrid_runtime.dart';
export 'src/key64.dart' show Key64;
export 'src/scene_reader.dart';
export 'src/table_reader.dart';
export 'src/utf8.dart' show decodeUtf8PreserveBom, isValidUtf8, utf8EncodedLength;
export 'src/xxh64.dart';
