/// The one libc heap binding of `nve_host`: zero-filled blocks for out-parameters, borrowed
/// argument bytes, and hybrid arenas (no third-party FFI package).
library;

import 'dart:ffi' as ffi;
import 'dart:io' show Platform;

/// Zero-filled libc heap blocks.
abstract final class NativeHeap {
  static final ffi.DynamicLibrary _libc =
      Platform.isWindows ? ffi.DynamicLibrary.open('ucrtbase.dll') : ffi.DynamicLibrary.process();
  static final ffi.Pointer<ffi.Void> Function(int, int) _calloc = _libc
      .lookupFunction<ffi.Pointer<ffi.Void> Function(ffi.Size, ffi.Size), ffi.Pointer<ffi.Void> Function(int, int)>(
          'calloc');
  static final void Function(ffi.Pointer<ffi.Void>) _free =
      _libc.lookupFunction<ffi.Void Function(ffi.Pointer<ffi.Void>), void Function(ffi.Pointer<ffi.Void>)>('free');

  /// [bytes] zeroed bytes (at least 1); throws [OutOfMemoryError] on allocation failure.
  static ffi.Pointer<T> zeroed<T extends ffi.NativeType>(int bytes) {
    final p = _calloc(1, bytes < 1 ? 1 : bytes);
    if (p.address == 0) {
      throw const OutOfMemoryError();
    }
    return p.cast<T>();
  }

  /// Frees a [zeroed] block (no-op for `nullptr`).
  static void free(ffi.Pointer<ffi.NativeType> p) {
    if (p.address != 0) {
      _free(p.cast<ffi.Void>());
    }
  }
}
