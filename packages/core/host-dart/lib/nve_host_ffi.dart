/// Native-only `dart:ffi` structs, hybrid vtable typedefs, bump arena, and `NveValueTree` builder
/// for the NVE C ABI. Import [nve_host.dart] for everything that also runs on Dart-web.
library;

export 'src/generated/nve_abi_ffi.generated.dart';
export 'src/generated/nve_runtime_abi.generated.dart';
export 'src/hybrid_ffi.dart' hide checkedNativeBytes;
export 'src/native_heap.dart';
export 'src/runtime_api.dart';
export 'src/runtime_session.dart';
export 'src/value_tree.dart';
