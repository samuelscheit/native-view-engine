/// Native-only (`dart:ffi`) NVE host bindings: generated Core / Runtime C ABI structs and the
/// owned `libnve_runtime` [RuntimeSession]. Never imported by `nve_host.dart`, so the pure library
/// stays Dart-web compatible.
library;

export 'nve_host.dart';
export 'src/native_values.dart';
export 'src/nve_core_abi.generated.dart';
export 'src/nve_runtime_abi.generated.dart';
export 'src/runtime_api.dart';
export 'src/runtime_session.dart';
