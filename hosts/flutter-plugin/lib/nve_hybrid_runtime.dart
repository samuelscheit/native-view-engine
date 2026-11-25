/// Pure-Dart (zero `package:flutter` imports) runtime barrel shared by `nve_flutter`
/// and all `nve-codegen` generated hybrid `dart:ffi` bindings.
///
/// Provides one single source of truth for:
/// - Core C-ABI FFI structs (`NveStrSlice`, `NveBufferSlice`, `NveValue`, `NveMapEntry`, ...)
/// - Hybrid owned-bytes ABI struct (`NveHybridOwnedBytes`)
/// - Canonical binary `NveBinValue` codec (`NveBinWriter`, `NveBinReader`, `NveHybridDynamicValue`)
/// - Zero-JSON bump-arena direct props encoder (`NveDirectEncodable`, `NveDirectPropsBuilder`,
///   `NveDirectPropsWriter`, `NveNativeArena`)
library nve_hybrid_runtime;

import 'dart:ffi' as ffi;

export 'src/direct_ffi.dart';
export 'src/nve_bin_codec.dart';
export 'src/nve_bin_value.dart';
export 'src/nve_core_abi_gen.dart';

/// Owned heap byte buffer returned across hybrid C-ABI vtables and released via `free_bytes`.
final class NveHybridOwnedBytes extends ffi.Struct {
  external ffi.Pointer<ffi.Uint8> ptr;
  @ffi.Size()
  external int len;
  @ffi.Size()
  external int cap;
}
