/// Pure-Dart (zero `package:flutter` imports) runtime barrel used internally by `nve_flutter`.
///
/// Re-exports:
/// - Core C-ABI FFI structs and hybrid runtime from `package:nve_host/nve_host_ffi.dart`
/// - Flutter-plugin binary `NveBinValue` codec (`NveBinWriter`, `NveBinReader`, `NveBinValueView`)
/// - Direct FFI bump-arena props & event writers (`NveDirectEncodable`, `NveDirectPropsBuilder`,
///   `NveDirectPropsWriter`, `NveDirectEventWriter`, `NveNativeArena`)
///
/// Generated `nve-codegen` hybrid Dart clients import `package:nve_host/nve_host.dart` and
/// `package:nve_host/nve_host_ffi.dart` directly.
library nve_hybrid_runtime;

export 'package:nve_host/nve_host_ffi.dart';
export 'src/direct_ffi.dart';
export 'src/nve_bin_codec.dart';
export 'src/nve_bin_value.dart';
