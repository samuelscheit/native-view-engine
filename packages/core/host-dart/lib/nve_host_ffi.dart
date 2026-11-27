/// Native-only `dart:ffi` structs, hybrid vtable typedefs, bump arena, and `NveValueTree` builder
/// for the NVE C ABI. Import [nve_host.dart] for everything that also runs on Dart-web.
library;

export 'src/generated/nve_abi_ffi.generated.dart';
export 'src/hybrid_ffi.dart' hide checkedNativeBytes;
export 'src/value_tree.dart';
