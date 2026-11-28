/// Validated `dart:ffi` binding of the single `NveRuntimeApi` vtable exported by `libnve_runtime`
/// (`nve_runtime_get_interface`), plus the error type shared by sessions.
library;

import 'dart:convert' show utf8;
import 'dart:ffi' as ffi;
import 'dart:io' show Platform;

import 'generated/nve_abi_ffi.generated.dart';
import 'generated/nve_runtime_abi.generated.dart';
import 'key64.dart';
import 'native_heap.dart';

/// Status-carrying failure of one `NveRuntimeApi` call (`message` includes `last_error`).
final class RuntimeException implements Exception {
  const RuntimeException(this.status, this.message);

  final NveRuntimeStatus status;
  final String message;

  @override
  String toString() => 'RuntimeException(${status.name}): $message';
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
        setTextMeasurer = vtable.ref.setTextMeasurer.asFunction(),
        attachAppInterface = vtable.ref.attachAppInterface.asFunction(),
        loadAppModule = vtable.ref.loadAppModule.asFunction(),
        setProps = vtable.ref.setProps.asFunction(),
        drainEmittedEvents = vtable.ref.drainEmittedEvents.asFunction(),
        snapshotState = vtable.ref.snapshotState.asFunction(),
        restoreState = vtable.ref.restoreState.asFunction(),
        saveStateFile = vtable.ref.saveStateFile.asFunction(),
        restoreStateFile = vtable.ref.restoreStateFile.asFunction(),
        clearStateFile = vtable.ref.clearStateFile.asFunction(),
        applyHotPatch = vtable.ref.applyHotPatch.asFunction(),
        resourcePayload = vtable.ref.resourcePayload.asFunction(),
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
        _lastError = vtable.ref.lastError.asFunction(),
        measureSurface = vtable.ref.measureSurface.asFunction(),
        setIntrinsicSizeInvalidatedCallback = vtable.ref.setIntrinsicSizeInvalidatedCallback.asFunction(),
        saveFrame0File = vtable.ref.saveFrame0File.asFunction(),
        bootFrame0File = vtable.ref.bootFrame0File.asFunction(),
        bootFrame0Bytes = vtable.ref.bootFrame0Bytes.asFunction(),
        setHostCallbacks = vtable.ref.setHostCallbacks.asFunction(),
        completeHostRequest = vtable.ref.completeHostRequest.asFunction(),
        pushHostEvent = vtable.ref.pushHostEvent.asFunction(),
        publishHostChannel = vtable.ref.publishHostChannel.asFunction();

  /// Resolves `nve_runtime_get_interface` from [library] and validates the returned vtable.
  ///
  /// Throws [ArgumentError] when the symbol is missing and [StateError] when the library speaks a
  /// different ABI version, vtable size, or schema hash, or leaves any slot NULL.
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
    if (api.structSize != ffi.sizeOf<NveRuntimeApi>()) {
      throw StateError('runtime vtable is ${api.structSize} bytes, binding expects ${ffi.sizeOf<NveRuntimeApi>()}');
    }
    final schema = key64FromBits(api.schemaHash);
    if (schema != nveRuntimeSchemaHash) {
      throw StateError('runtime schema $schema != $nveRuntimeSchemaHash (regenerate bindings)');
    }
    if (!runtimeApiSlotsComplete(vtable)) {
      throw StateError('runtime vtable leaves a slot NULL (of $nveRuntimeApiSlotCount)');
    }
    return RuntimeApi._(library, vtable);
  }

  /// Opens `libnve_runtime` at [path]; without one, the platform library name on the loader path,
  /// or the process image on iOS (where `libnve_runtime` is always statically linked).
  factory RuntimeApi.open([String? path]) => path == null && Platform.isIOS
      ? RuntimeApi.process()
      : RuntimeApi.fromLibrary(ffi.DynamicLibrary.open(path ?? defaultLibraryName));

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
  final NveRuntimeSetTextMeasurerDart setTextMeasurer;
  final NveRuntimeAttachAppInterfaceDart attachAppInterface;
  final NveRuntimeLoadAppModuleDart loadAppModule;
  final NveRuntimeSetPropsDart setProps;
  final NveRuntimeDrainEmittedEventsDart drainEmittedEvents;
  final NveRuntimeSnapshotStateDart snapshotState;
  final NveRuntimeRestoreStateDart restoreState;
  final NveRuntimeSaveStateFileDart saveStateFile;
  final NveRuntimeRestoreStateFileDart restoreStateFile;
  final NveRuntimeClearStateFileDart clearStateFile;
  final NveRuntimeApplyHotPatchDart applyHotPatch;
  final NveRuntimeResourcePayloadDart resourcePayload;
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
  final NveRuntimeMeasureSurfaceDart measureSurface;
  final NveRuntimeSetIntrinsicSizeInvalidatedCallbackDart setIntrinsicSizeInvalidatedCallback;
  final NveRuntimeSaveFrame0FileDart saveFrame0File;
  final NveRuntimeBootFrame0FileDart bootFrame0File;
  final NveRuntimeBootFrame0BytesDart bootFrame0Bytes;
  final NveRuntimeSetHostCallbacksDart setHostCallbacks;
  final NveRuntimeCompleteHostRequestDart completeHostRequest;
  final NveRuntimePushHostEventDart pushHostEvent;
  final NveRuntimePublishHostChannelDart publishHostChannel;

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
