/// Validated `dart:ffi` binding of the single `NveRuntimeApi` vtable exported by `libnve_runtime`
/// (`nve_runtime_get_interface`), plus the native heap and error type shared by sessions.
library;

import 'dart:convert' show utf8;
import 'dart:ffi' as ffi;
import 'dart:io' show Platform;

import 'key64.dart';
import 'nve_core_abi.generated.dart';
import 'nve_runtime_abi.generated.dart';

/// Status-carrying failure of one `NveRuntimeApi` call (`message` includes `last_error`).
final class RuntimeException implements Exception {
  const RuntimeException(this.status, this.message);

  final NveRuntimeStatus status;
  final String message;

  @override
  String toString() => 'RuntimeException(${status.name}): $message';
}

/// Zero-filled libc heap blocks for out-parameters and borrowed argument bytes.
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

  static void free(ffi.Pointer<ffi.NativeType> p) {
    if (p.address != 0) {
      _free(p.cast<ffi.Void>());
    }
  }
}

/// Splits a VM `int` holding raw `u64` bits into exact `Key64` halves.
Key64 key64FromBits(int bits) => Key64(bits & 0xFFFFFFFF, (bits >>> 32) & 0xFFFFFFFF);

/// One loaded, version- and schema-checked `NveRuntimeApi` with every slot bound once.
final class RuntimeApi {
  RuntimeApi._(this.library, this.vtable)
      : create = vtable.ref.create.asFunction(),
        destroy = vtable.ref.destroy.asFunction(),
        setWakeCallback = vtable.ref.setWakeCallback.asFunction(),
        setViewport = vtable.ref.setViewport.asFunction(),
        attachAppInterface = vtable.ref.attachAppInterface.asFunction(),
        submitNve2 = vtable.ref.submitNve2.asFunction(),
        tick = vtable.ref.tick.asFunction(),
        dispatchEvents = vtable.ref.dispatchEvents.asFunction(),
        hitTest = vtable.ref.hitTest.asFunction(),
        scrollBy = vtable.ref.scrollBy.asFunction(),
        sceneAcquire = vtable.ref.sceneAcquire.asFunction(),
        sceneRelease = vtable.ref.sceneRelease.asFunction(),
        getMetrics = vtable.ref.getMetrics.asFunction(),
        pairConnect = vtable.ref.pairConnect.asFunction(),
        pairDisconnect = vtable.ref.pairDisconnect.asFunction(),
        pairIsConnected = vtable.ref.pairIsConnected.asFunction(),
        _lastError = vtable.ref.lastError.asFunction();

  /// Resolves `nve_runtime_get_interface` from [library] and validates the returned vtable.
  ///
  /// Throws [ArgumentError] when the symbol is missing and [StateError] when the library speaks a
  /// different ABI version, vtable size, or schema hash.
  factory RuntimeApi.fromLibrary(ffi.DynamicLibrary library) {
    final getInterface = library.lookupFunction<NveRuntimeGetInterfaceNative, NveRuntimeGetInterfaceDart>(
      'nve_runtime_get_interface',
    );
    final vtable = getInterface(nveRuntimeAbiVersion);
    if (vtable.address == 0) {
      throw StateError('nve_runtime_get_interface($nveRuntimeAbiVersion) returned NULL (ABI version mismatch)');
    }
    final api = vtable.ref;
    if (api.abiVersion != nveRuntimeAbiVersion) {
      throw StateError('runtime ABI version ${api.abiVersion} != $nveRuntimeAbiVersion');
    }
    if (api.structSize < ffi.sizeOf<NveRuntimeApi>()) {
      throw StateError('runtime vtable is ${api.structSize} bytes, binding needs ${ffi.sizeOf<NveRuntimeApi>()}');
    }
    final schema = key64FromBits(api.schemaHash);
    if (schema != nveRuntimeSchemaHash) {
      throw StateError('runtime schema $schema != $nveRuntimeSchemaHash (regenerate bindings)');
    }
    return RuntimeApi._(library, vtable);
  }

  /// Opens `libnve_runtime` at [path] (default: the platform library name on the loader path).
  factory RuntimeApi.open([String? path]) => RuntimeApi.fromLibrary(ffi.DynamicLibrary.open(path ?? defaultLibraryName));

  /// Uses a `libnve_runtime` statically linked into the running process (iOS, CMake hosts).
  factory RuntimeApi.process() => RuntimeApi.fromLibrary(ffi.DynamicLibrary.process());

  /// `libnve_runtime.dylib` / `libnve_runtime.so` / `nve_runtime.dll`.
  static String get defaultLibraryName => Platform.isWindows
      ? 'nve_runtime.dll'
      : Platform.isMacOS || Platform.isIOS
          ? 'libnve_runtime.dylib'
          : 'libnve_runtime.so';

  final ffi.DynamicLibrary library;
  final ffi.Pointer<NveRuntimeApi> vtable;

  final NveRuntimeCreateDart create;
  final NveRuntimeDestroyDart destroy;
  final NveRuntimeSetWakeCallbackDart setWakeCallback;
  final NveRuntimeSetViewportDart setViewport;
  final NveRuntimeAttachAppInterfaceDart attachAppInterface;
  final NveRuntimeSubmitNve2Dart submitNve2;
  final NveRuntimeTickDart tick;
  final NveRuntimeDispatchEventsDart dispatchEvents;
  final NveRuntimeHitTestDart hitTest;
  final NveRuntimeScrollByDart scrollBy;
  final NveRuntimeSceneAcquireDart sceneAcquire;
  final NveRuntimeSceneReleaseDart sceneRelease;
  final NveRuntimeGetMetricsDart getMetrics;
  final NveRuntimePairConnectDart pairConnect;
  final NveRuntimePairDisconnectDart pairDisconnect;
  final NveRuntimePairIsConnectedDart pairIsConnected;
  final NveRuntimeLastErrorDart _lastError;

  /// Thread-local diagnostic text of the last failing call on this thread (`''` when none).
  String lastError() {
    final needed = _lastError(ffi.nullptr, 0);
    if (needed == 0) {
      return '';
    }
    // Returns the full byte length; copies at most `cap - 1` bytes plus a NUL.
    final buf = NativeHeap.zeroed<ffi.Uint8>(needed + 1);
    try {
      final total = _lastError(buf, needed + 1);
      return utf8.decode(buf.asTypedList(total < needed ? total : needed), allowMalformed: true);
    } finally {
      NativeHeap.free(buf);
    }
  }

  /// Throws [RuntimeException] unless [status] is `NVE_RUNTIME_OK`.
  void check(int status, String what) {
    if (status != NveRuntimeStatus.ok.code) {
      throw RuntimeException(NveRuntimeStatus.fromCode(status) ?? NveRuntimeStatus.error, '$what: ${lastError()}');
    }
  }
}

/// Resolves a module's `nve_get_interface` (or [symbol]) for `RuntimeSession.attachApp`.
ffi.Pointer<ffi.NativeFunction<NveGetInterfaceNative>> lookupAppInterface(
  ffi.DynamicLibrary module, {
  String symbol = 'nve_get_interface',
}) =>
    module.lookup<ffi.NativeFunction<NveGetInterfaceNative>>(symbol);
