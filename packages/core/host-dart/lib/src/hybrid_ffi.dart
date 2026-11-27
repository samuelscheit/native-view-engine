/// Native `dart:ffi` hybrid vtable typedefs, chunked bump arena, and owned-bytes helpers shared
/// by every `nve-codegen` generated Dart hybrid client.
///
/// Zero external package dependencies (uses libc `calloc` / `free` directly via `dart:ffi`).
/// Pure-Dart web-safe types live in `package:nve_host/nve_host.dart`.
library;

import 'dart:ffi' as ffi;
import 'dart:typed_data';

import 'bin_codec.dart';
import 'generated/nve_abi_ffi.generated.dart';
import 'hybrid_runtime.dart';
import 'key64.dart';
import 'native_heap.dart';
import 'utf8.dart';

/// Owned heap byte buffer returned across hybrid C-ABI vtables and released via `free_bytes`.
final class HybridOwnedBytes extends ffi.Struct {
  external ffi.Pointer<ffi.Uint8> ptr;

  @ffi.Size()
  external int len;

  @ffi.Size()
  external int cap;
}

/// Dart signature matching [NveGetInterfaceNative] (`nve_get_interface`).
typedef HybridGetInterfaceDart = ffi.Pointer<ffi.Void> Function(int interfaceId);

/// Native signature of a hybrid vtable `create()` function.
typedef HybridCreateNative = ffi.Pointer<ffi.Void> Function();

/// Dart signature of a hybrid vtable `create()` function.
typedef HybridCreateDart = ffi.Pointer<ffi.Void> Function();

/// Native signature of a hybrid vtable `destroy(handle)` function.
typedef HybridDestroyNative = ffi.Void Function(ffi.Pointer<ffi.Void> handle);

/// Dart signature of a hybrid vtable `destroy(handle)` function.
typedef HybridDestroyDart = void Function(ffi.Pointer<ffi.Void> handle);

/// Native signature of a hybrid vtable `alloc_bytes(len)` function.
typedef HybridAllocBytesNative = HybridOwnedBytes Function(ffi.Size len);

/// Native signature of a hybrid vtable `free_bytes(buf)` function.
typedef HybridFreeBytesNative = ffi.Void Function(HybridOwnedBytes buf);

/// Dart signature of a hybrid vtable `free_bytes(buf)` function.
typedef HybridFreeBytesDart = void Function(HybridOwnedBytes buf);

/// Native signature of a hybrid view vtable `set_props_direct(handle, entries, count)` function.
typedef HybridSetPropsDirectNative = ffi.Int32 Function(
  ffi.Pointer<ffi.Void> handle,
  ffi.Pointer<NveMapEntry> entries,
  ffi.Uint32 count,
);

/// Dart signature of a hybrid view vtable `set_props_direct(handle, entries, count)` function.
typedef HybridSetPropsDirectDart = int Function(
  ffi.Pointer<ffi.Void> handle,
  ffi.Pointer<NveMapEntry> entries,
  int count,
);

/// Chunked, zero-initialized native bump arena backing synchronous `dart:ffi` hybrid calls.
///
/// Use [HybridArena.using] so every slab is deterministically released even when an encoding or
/// native call throws. Earlier chunks never move until [dispose], so interior pointers inside
/// `NveValue` trees and `NveMapEntry[]` slices remain stable for the entire call.
final class HybridArena {
  HybridArena({int initialChunkBytes = 4096}) : _chunkSize = _validateInitialChunk(initialChunkBytes);

  static const int _minChunkBytes = 256;
  static const int _maxDoublingChunkBytes = 16 * 1024 * 1024;
  static const int _maxAllocBytes = 0x3FFFFFFF;

  static int _validateInitialChunk(int initialChunkBytes) {
    RangeError.checkValueInInterval(initialChunkBytes, 0, _maxAllocBytes, 'initialChunkBytes');
    return initialChunkBytes < _minChunkBytes ? _minChunkBytes : initialChunkBytes;
  }

  final int _chunkSize;
  final List<ffi.Pointer<ffi.Uint8>> _chunks = <ffi.Pointer<ffi.Uint8>>[];
  ffi.Pointer<ffi.Uint8> _current = ffi.nullptr;
  int _capacity = 0;
  int _offset = 0;
  bool _disposed = false;

  /// Number of live native heap slabs currently owned by this arena (0 after [dispose]).
  int get allocatedChunkCount => _chunks.length;

  /// Whether [dispose] has already released all native slabs.
  bool get isDisposed => _disposed;

  /// Runs [body] synchronously with a scoped [HybridArena] and frees all native slabs in `finally`.
  ///
  /// Throws [StateError] if [body] returns a [Future] (the arena is disposed before async work completes).
  static T using<T>(T Function(HybridArena arena) body, {int initialChunkBytes = 4096}) {
    final arena = HybridArena(initialChunkBytes: initialChunkBytes);
    try {
      final result = body(arena);
      if (result is Future<Object?>) {
        throw StateError('HybridArena.using is synchronous; do not return a Future');
      }
      return result;
    } finally {
      arena.dispose();
    }
  }

  void _ensureLive() {
    if (_disposed) {
      throw StateError('HybridArena has already been disposed');
    }
  }

  /// Allocates [byteCount] zero-initialized bytes aligned to [align] (power of two in `1..=8`).
  ffi.Pointer<ffi.Uint8> allocBytes(int byteCount, {int align = 8}) {
    _ensureLive();
    RangeError.checkNotNegative(byteCount, 'byteCount');
    if (align <= 0 || align > 8 || (align & (align - 1)) != 0) {
      throw RangeError.value(align, 'align', 'must be a power of two in 1..=8');
    }
    if (byteCount > _maxAllocBytes) {
      throw BinCodecException(BinErrorKind.tooLarge, 'arena allocation $byteCount bytes');
    }
    final size = byteCount == 0 ? 1 : byteCount;
    final aligned = (_offset + (align - 1)) & ~(align - 1);
    if (_current != ffi.nullptr && aligned + size <= _capacity) {
      final ptr = ffi.Pointer<ffi.Uint8>.fromAddress(_current.address + aligned);
      _offset = aligned + size;
      return ptr;
    }
    final int doubled;
    if (_capacity <= 0) {
      doubled = _chunkSize;
    } else {
      final twice = _capacity * 2;
      doubled = twice > _maxDoublingChunkBytes ? _maxDoublingChunkBytes : twice;
    }
    final nextCap = size > doubled ? size : doubled;
    final chunk = NativeHeap.zeroed<ffi.Uint8>(nextCap);
    _chunks.add(chunk);
    _current = chunk;
    _capacity = nextCap;
    _offset = size;
    return chunk;
  }

  /// Allocates a single zero-initialized `ffi.Struct` slot of [byteSize] bytes.
  ffi.Pointer<T> allocStruct<T extends ffi.Struct>(int byteSize) =>
      allocBytes(byteSize, align: 8).cast<T>();

  /// Allocates a contiguous zero-initialized array of [count] `ffi.Struct` items.
  ffi.Pointer<T> allocStructArray<T extends ffi.Struct>(int elementSize, int count) {
    _ensureLive();
    RangeError.checkNotNegative(count, 'count');
    if (count == 0) {
      return ffi.nullptr.cast<T>();
    }
    if (elementSize <= 0 || count > _maxAllocBytes ~/ elementSize) {
      throw BinCodecException(BinErrorKind.tooLarge, 'array $count * $elementSize');
    }
    return allocBytes(elementSize * count, align: 8).cast<T>();
  }

  /// Encodes [value] as strict UTF-8 directly into this arena (zero intermediate list) and writes
  /// the resulting slice into [target]. Throws [BinCodecException] (`invalidUtf8`) on a lone
  /// UTF-16 surrogate.
  void writeStrSlice(NveStrSlice target, String value) {
    _ensureLive();
    if (value.isEmpty) {
      target
        ..ptr = ffi.nullptr
        ..len = 0;
      return;
    }
    final byteLen = utf8EncodedLength(value);
    final buf = allocBytes(byteLen, align: 1);
    final out = buf.asTypedList(byteLen);
    var pos = 0;
    for (var i = 0; i < value.length; i += utf16UnitsAt(value, i)) {
      pos = encodeUtf8CodePointAt(value, i, out, pos);
    }
    target
      ..ptr = buf
      ..len = byteLen;
  }

  /// Copies pre-validated UTF-8 [utf8Bytes] into this arena and writes the slice into [target].
  void writeUtf8BytesSlice(NveStrSlice target, List<int> utf8Bytes) {
    _ensureLive();
    if (utf8Bytes.isEmpty) {
      target
        ..ptr = ffi.nullptr
        ..len = 0;
      return;
    }
    final buf = allocBytes(utf8Bytes.length, align: 1);
    buf.asTypedList(utf8Bytes.length).setAll(0, utf8Bytes);
    target
      ..ptr = buf
      ..len = utf8Bytes.length;
  }

  /// Copies raw [bytes] into this arena and writes the slice into [target].
  void writeBytesSlice(NveBufferSlice target, List<int> bytes) {
    _ensureLive();
    if (bytes.isEmpty) {
      target
        ..ptr = ffi.nullptr
        ..len = 0;
      return;
    }
    final buf = allocBytes(bytes.length, align: 1);
    buf.asTypedList(bytes.length).setAll(0, bytes);
    target
      ..ptr = buf
      ..len = bytes.length;
  }

  /// Returns a borrowed [NveStrSlice] containing strict UTF-8 bytes of [value].
  /// An empty string returns a zero-initialized struct without allocating a native slab.
  NveStrSlice strSlice(String value) {
    _ensureLive();
    if (value.isEmpty) {
      return ffi.Struct.create<NveStrSlice>();
    }
    final slot = allocStruct<NveStrSlice>(ffi.sizeOf<NveStrSlice>());
    writeStrSlice(slot.ref, value);
    return slot.ref;
  }

  /// Returns a borrowed [NveBufferSlice] containing a copy of [bytes].
  /// An empty list returns a zero-initialized struct without allocating a native slab.
  NveBufferSlice bufferSlice(List<int> bytes) {
    _ensureLive();
    if (bytes.isEmpty) {
      return ffi.Struct.create<NveBufferSlice>();
    }
    final slot = allocStruct<NveBufferSlice>(ffi.sizeOf<NveBufferSlice>());
    writeBytesSlice(slot.ref, bytes);
    return slot.ref;
  }

  /// Releases every native slab owned by this arena. Idempotent.
  void dispose() {
    if (_disposed) {
      return;
    }
    _disposed = true;
    for (final chunk in _chunks) {
      NativeHeap.free(chunk);
    }
    _chunks.clear();
    _current = ffi.nullptr;
    _capacity = 0;
    _offset = 0;
  }
}

/// Package-internal validator for a `(ptr, len)` pair returned across the C ABI (guarding signed
/// `ffi.Size` and `(nullptr, len != 0)`), returning a zero-copy native view (or empty `Uint8List`).
Uint8List checkedNativeBytes(ffi.Pointer<ffi.Uint8> ptr, int len) {
  if (len < 0 || len > 0xFFFFFFFF) {
    throw BinCodecException(BinErrorKind.tooLarge, 'invalid native buffer length $len');
  }
  if (len == 0) {
    return Uint8List(0);
  }
  if (ptr == ffi.nullptr) {
    throw const BinCodecException(BinErrorKind.truncated, 'null native buffer pointer with non-zero length');
  }
  return ptr.asTypedList(len);
}

/// Resolves [interfaceId] via `nve_get_interface` from [library], throwing [HybridException] if
/// the interface is absent.
ffi.Pointer<ffi.Void> hybridResolveInterface(
  ffi.DynamicLibrary library,
  int interfaceId,
  String name,
) {
  final getIface = library.lookupFunction<NveGetInterfaceNative, HybridGetInterfaceDart>('nve_get_interface');
  final raw = getIface(interfaceId);
  if (raw == ffi.nullptr) {
    final hex = interfaceId.toRadixString(16).toUpperCase().padLeft(8, '0');
    throw HybridException(HybridStatus.err, 'NVE hybrid interface $name (0x$hex) not found');
  }
  return raw;
}

/// Verifies a hybrid vtable's ABI version and 64-bit schema hash against generated constants.
void hybridVerifyVtable({
  required int expectedAbiVersion,
  required int actualAbiVersion,
  required int rawSchemaHash,
  required Key64 expectedSchemaHash,
  required String name,
}) {
  if (actualAbiVersion != expectedAbiVersion) {
    throw HybridException(
      HybridStatus.schemaMismatch,
      'NVE hybrid ABI version mismatch for $name: expected $expectedAbiVersion, got $actualAbiVersion',
    );
  }
  final actualSchema = Key64.fromInt(rawSchemaHash);
  if (expectedSchemaHash != actualSchema) {
    throw HybridException(
      HybridStatus.schemaMismatch,
      'NVE hybrid schema hash mismatch for $name: expected $expectedSchemaHash, got $actualSchema',
    );
  }
}

/// Decodes a peer-owned UTF-8 buffer (preserving leading U+FEFF) and always frees [raw].
String hybridTakeString(HybridOwnedBytes raw, HybridFreeBytesDart freeBytes) {
  try {
    final view = checkedNativeBytes(raw.ptr, raw.len);
    if (view.isEmpty) {
      return '';
    }
    if (!isValidUtf8(view)) {
      throw const BinCodecException(BinErrorKind.invalidUtf8, 'peer returned invalid UTF-8');
    }
    return decodeUtf8PreserveBom(view);
  } finally {
    if (raw.ptr != ffi.nullptr) {
      freeBytes(raw);
    }
  }
}

/// Copies a peer-owned byte buffer into Dart memory and always frees [raw].
Uint8List hybridTakeBytes(HybridOwnedBytes raw, HybridFreeBytesDart freeBytes) {
  try {
    final view = checkedNativeBytes(raw.ptr, raw.len);
    if (view.isEmpty) {
      return view;
    }
    return Uint8List.fromList(view);
  } finally {
    if (raw.ptr != ffi.nullptr) {
      freeBytes(raw);
    }
  }
}

/// Validates and copies a peer-owned `NveBinValue` buffer into a [DynamicValue] and always frees [raw].
DynamicValue hybridTakeDynamicValue(HybridOwnedBytes raw, HybridFreeBytesDart freeBytes) {
  try {
    final view = checkedNativeBytes(raw.ptr, raw.len);
    return DynamicValue(view);
  } finally {
    if (raw.ptr != ffi.nullptr) {
      freeBytes(raw);
    }
  }
}

/// Decodes a peer-owned `NveBinValue` buffer straight out of native memory (zero whole-buffer
/// copy), enforces [BinReader.requireDone], and always frees [raw].
///
/// **Lifetime contract:** The [BinReader] passed to [decode] borrows [raw]'s native memory, which
/// is freed by [freeBytes] immediately when [decode] returns or throws. Callers MUST NOT retain
/// zero-copy views (`reader.readBytes()`, `reader.take()`, `reader.readRawValue()`,
/// `reader.readStrUtf8()`) past [decode]; copy them into Dart memory (e.g.
/// `Uint8List.fromList(reader.readBytes())` or `DynamicValue.read(reader)`) before returning.
T hybridDecodeOwned<T>(
  HybridOwnedBytes raw,
  HybridFreeBytesDart freeBytes,
  T Function(BinReader reader) decode,
) {
  try {
    final view = checkedNativeBytes(raw.ptr, raw.len);
    final reader = BinReader(view);
    final result = decode(reader);
    reader.requireDone();
    return result;
  } finally {
    if (raw.ptr != ffi.nullptr) {
      freeBytes(raw);
    }
  }
}
