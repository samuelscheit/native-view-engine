/// Owned `libnve_runtime` surface: lifecycle, viewport, app attach / NVE2 replay / pairing,
/// ticks, input events, hit-testing, scrolling, metrics, and scoped zero-copy scene reads.
///
/// Owner-thread affinity: create, use, and dispose a session on one isolate whose thread never
/// changes (the Flutter UI isolate, or a VM main isolate). The wake listener is the only entry
/// point the runtime may trigger from another thread; it is delivered asynchronously.
library;

import 'dart:ffi' as ffi;
import 'dart:typed_data';

import 'event_batch.dart';
import 'generated/nve_abi_const.generated.dart';
import 'generated/nve_abi_ffi.generated.dart';
import 'generated/nve_runtime_abi.generated.dart';
import 'hybrid_ffi.dart' show HybridArena, checkedNativeBytes;
import 'key64.dart';
import 'native_heap.dart';
import 'runtime_api.dart';
import 'scene_reader.dart';
import 'table_reader.dart';
import 'utf8.dart' show encodeUtf8CodePointAt, utf16UnitsAt, utf8EncodedLength;
import 'value_tree.dart' show NveValueTree;

/// Snapshot of `NveRuntimeMetrics`.
final class RuntimeMetrics {
  const RuntimeMetrics({
    required this.frameId,
    required this.reconcileUs,
    required this.layoutUs,
    required this.publishUs,
    required this.wireBytesRx,
    required this.wireBytesTx,
    required this.liveNodes,
    required this.dirtyNodes,
    required this.pairConnected,
  });

  final int frameId, reconcileUs, layoutUs, publishUs, wireBytesRx, wireBytesTx;
  final int liveNodes, dirtyNodes;
  final bool pairConnected;
}

/// Growable zero-filled native block reused across calls (never shrinks).
final class _NativeBlock {
  ffi.Pointer<ffi.Uint8> ptr = ffi.nullptr;
  int capacity = 0;

  ffi.Pointer<ffi.Uint8> reserve(int bytes) {
    if (bytes > capacity) {
      // Drop the old block first so a failed allocation never leaves a freed pointer behind
      // (a later `reserve` / `free` would free it twice).
      NativeHeap.free(ptr);
      ptr = ffi.nullptr;
      capacity = 0;
      final size = bytes < 256 ? 256 : bytes * 2;
      ptr = NativeHeap.zeroed<ffi.Uint8>(size);
      capacity = size;
    }
    return ptr;
  }

  void free() {
    NativeHeap.free(ptr);
    ptr = ffi.nullptr;
    capacity = 0;
  }
}

final class RuntimeSession {
  /// Creates a surface; throws [RuntimeException] when `create` returns NULL.
  factory RuntimeSession(
    RuntimeApi api, {
    int surfaceId = 1,
    double width = 800,
    double height = 600,
    double scaleFactor = 1,
    int flags = 0,
  }) {
    final config = NativeHeap.zeroed<NveRuntimeConfig>(ffi.sizeOf<NveRuntimeConfig>());
    try {
      config.ref
        ..surfaceId = surfaceId
        ..width = width
        ..height = height
        ..scaleFactor = scaleFactor
        ..flags = flags;
      final handle = api.create(config);
      if (handle.address == 0) {
        throw RuntimeException(NveRuntimeStatus.error, 'create: ${api.lastError()}');
      }
      return RuntimeSession._(api, handle, surfaceId);
    } finally {
      NativeHeap.free(config);
    }
  }

  RuntimeSession._(this.api, this._handle, this.surfaceId);

  final RuntimeApi api;
  final int surfaceId;
  NveRuntimeHandle _handle;
  ffi.NativeCallable<NveRuntimeWakeNative>? _wake;
  ffi.DynamicLibrary? _appLibrary;

  // One zeroed scratch block holds every fixed-size out-parameter / argument record (8-byte
  // aligned sub-blocks): one allocation per session, one free in [dispose].
  static int _align8(int n) => (n + 7) & ~7;
  static final int _atMetrics = _align8(ffi.sizeOf<NveSceneView>());
  static final int _atStrs = _atMetrics + _align8(ffi.sizeOf<NveRuntimeMetrics>());
  static final int _atBuffer = _atStrs + _align8(2 * ffi.sizeOf<NveStrSlice>());
  static final int _atBatch = _atBuffer + _align8(ffi.sizeOf<NveBufferSlice>());
  static final int _atU64 = _atBatch + _align8(ffi.sizeOf<NveEventBatch>());
  static final int _atF32 = _atU64 + 8;
  static final int _atU32 = _atF32 + 8;
  static final int _scratchBytes = _atU32 + 8;

  final ffi.Pointer<ffi.Uint8> _scratch = NativeHeap.zeroed<ffi.Uint8>(_scratchBytes);
  late final ffi.Pointer<NveSceneView> _view = _scratch.cast();
  late final ffi.Pointer<NveRuntimeMetrics> _metrics = (_scratch + _atMetrics).cast();
  late final ffi.Pointer<NveStrSlice> _strs = (_scratch + _atStrs).cast();
  late final ffi.Pointer<NveBufferSlice> _buffer = (_scratch + _atBuffer).cast();
  late final ffi.Pointer<NveEventBatch> _batch = (_scratch + _atBatch).cast();
  late final ffi.Pointer<ffi.Uint64> _outU64 = (_scratch + _atU64).cast();
  late final ffi.Pointer<ffi.Float> _outF32 = (_scratch + _atF32).cast();
  late final ffi.Pointer<ffi.Uint32> _outU32 = (_scratch + _atU32).cast();
  final _NativeBlock _bytes = _NativeBlock();
  final _NativeBlock _events = _NativeBlock();
  bool _inScene = false;

  /// Validated reader of the last acquired view and the view bytes it was built from: an
  /// unchanged publication (same frame id, slab pointers, and counts) reuses it, so repeated
  /// paints / hit tests of one frame validate and allocate nothing.
  SceneReader? _reader;
  final Uint8List _readerView = Uint8List(ffi.sizeOf<NveSceneView>());

  bool get isDisposed => _handle.address == 0;

  NveRuntimeHandle get _live {
    if (_handle.address == 0) {
      throw StateError('RuntimeSession($surfaceId) is disposed');
    }
    return _handle;
  }

  /// Registers [onWake] (delivered asynchronously on this isolate) or clears it with `null`.
  ///
  /// The runtime swaps sinks atomically and waits for an in-flight wake of the old one, so the
  /// previous callable closes only after the swap succeeded; a failed swap keeps it installed.
  void setWakeListener(void Function()? onWake) {
    final handle = _live;
    final next =
        onWake == null ? null : ffi.NativeCallable<NveRuntimeWakeNative>.listener((ffi.Pointer<ffi.Void> _) => onWake());
    try {
      api.check(api.setWakeCallback(handle, next?.nativeFunction ?? ffi.nullptr, ffi.nullptr), 'set_wake_callback');
    } on Object {
      next?.close();
      rethrow;
    }
    _wake?.close();
    _wake = next;
  }

  void setViewport(double width, double height, double scaleFactor) =>
      api.check(api.setViewport(_live, width, height, scaleFactor), 'set_viewport');

  /// Attaches the module behind [getInterface] (a `nve_get_interface` symbol); replaces any app.
  void attachApp(ffi.Pointer<ffi.NativeFunction<NveGetInterfaceNative>> getInterface, {ffi.DynamicLibrary? keepAlive}) {
    api.check(api.attachAppInterface(_live, getInterface), 'attach_app_interface');
    _appLibrary = keepAlive;
  }

  /// Attaches `nve_get_interface` (or [symbol]) exported by [module].
  void attachAppLibrary(ffi.DynamicLibrary module, {String symbol = 'nve_get_interface'}) =>
      attachApp(lookupAppInterface(module, symbol: symbol), keepAlive: module);

  /// Loads the app cdylib at [path] from a private staged copy and attaches it (`load_app_module`;
  /// a rebuilt file always runs new code). [carryState] moves the running app's dev state into
  /// the new one. A failure leaves the running app untouched.
  void loadAppModule(String path, {bool carryState = false}) {
    final handle = _live;
    api.check(api.loadAppModule(handle, _utf8Args(path).$1, carryState ? 1 : 0), 'load_app_module');
    _appLibrary = null;
  }

  /// Detaches the app (the scene keeps its last frame until new content arrives).
  void detachApp() {
    api.check(api.attachAppInterface(_live, ffi.nullptr), 'attach_app_interface(NULL)');
    _appLibrary = null;
  }

  /// Library kept alive for the attached app, if any.
  ffi.DynamicLibrary? get appLibrary => _appLibrary;

  /// Applies one `NVE2` store wire frame (replay / custom transports; rejected while an app or
  /// pair session is attached).
  void submitNve2(Uint8List frame) => api.check(api.submitNve2(_live, _borrow(frame)), 'submit_nve2');

  /// Forwards embedding-host [props] to the attached app (`set_props`, one `NveMapEntry` per
  /// key); `true` when the app re-renders. Throws [RuntimeException] with
  /// [NveRuntimeStatus.unsupported] when the app exposes no component interface.
  bool setProps(Map<String, Object?> props) {
    final handle = _live;
    return HybridArena.using((arena) {
      final entries = props.isEmpty
          ? ffi.Pointer<NveMapEntry>.fromAddress(0)
          : arena.allocStructArray<NveMapEntry>(ffi.sizeOf<NveMapEntry>(), props.length);
      var i = 0;
      for (final MapEntry(:key, :value) in props.entries) {
        final entry = entries[i++];
        arena.writeStrSlice(entry.key, key);
        NveValueTree.writeObject(arena, entry.value, value);
      }
      api.check(api.setProps(handle, entries, props.length, _outU32), 'set_props');
      return _outU32.value != 0;
    });
  }

  /// Drains the events the app emitted since the last drain (`drain_emitted_events`), decoded
  /// into owned [RuntimeEvent]s. Empty when the app exposes no component interface.
  List<RuntimeEvent> drainEmittedEvents() {
    final status = api.drainEmittedEvents(_live, _batch);
    if (status == NveRuntimeStatus.unsupported.code) {
      return const <RuntimeEvent>[];
    }
    api.check(status, 'drain_emitted_events');
    final batch = _batch.ref;
    if (batch.eventCount == 0) {
      return const <RuntimeEvent>[];
    }
    if (batch.events.address == 0) {
      throw RuntimeException(NveRuntimeStatus.error, 'drain_emitted_events: NULL batch with ${batch.eventCount} events');
    }
    if (batch.eventCount > 0xFFFFFFFF ~/ nveEventSize) {
      throw RuntimeException(NveRuntimeStatus.error, 'drain_emitted_events: eventCount ${batch.eventCount} overflows 32 bits');
    }
    final records = batch.events.cast<ffi.Uint8>().asTypedList(batch.eventCount * nveEventSize);
    final arena = checkedNativeBytes(batch.bytes, batch.bytesLen);
    return EventBatch.validateAndDecode(records, arena);
  }

  /// Canonical payload bytes and kind of published resource [resourceId] (`resource_payload`).
  /// The bytes borrow runtime memory: valid only inside the [withScene] read that fetched them.
  /// `null` for `0`, an id outside `u32`, or an id the current scene does not reference.
  (Key64, Uint8List)? resourcePayload(int resourceId) {
    if (!_inScene) {
      throw StateError('resourcePayload outside withScene');
    }
    if (resourceId <= 0 || resourceId > 0xFFFFFFFF) {
      return null;
    }
    final status = api.resourcePayload(_handle, resourceId, _outU64, _buffer);
    if (status == NveRuntimeStatus.notFound.code) {
      return null;
    }
    api.check(status, 'resource_payload');
    return (key64FromBits(_outU64.value), checkedNativeBytes(_buffer.ref.ptr, _buffer.ref.len));
  }

  /// Owned copy of the app's state snapshot (`snapshot_state`, dev interface).
  Uint8List snapshotState() {
    api.check(api.snapshotState(_live, _buffer), 'snapshot_state');
    return Uint8List.fromList(checkedNativeBytes(_buffer.ref.ptr, _buffer.ref.len));
  }

  /// Restores a [snapshotState] result and schedules a re-render (`restore_state`).
  void restoreState(Uint8List state) => api.check(api.restoreState(_live, _borrow(state)), 'restore_state');

  /// Writes the app's state snapshot to [path] as a crash-atomic `StateFile` tagged
  /// [schemaVersion] (`save_state_file`); `false` when the app has no state.
  bool saveStateFile(String path, int schemaVersion) =>
      _found(api.saveStateFile(_live, _utf8Args(path).$1, schemaVersion), 'save_state_file');

  /// Restores a [saveStateFile] file (`restore_state_file`); `false` when it is missing, corrupt,
  /// or of an older schema (the runtime quarantines / rotates those).
  bool restoreStateFile(String path, int schemaVersion) =>
      _found(api.restoreStateFile(_live, _utf8Args(path).$1, schemaVersion), 'restore_state_file');

  /// Deletes a state file for a cold start (`clear_state_file`; already absent is fine).
  void clearStateFile(String path) => api.check(api.clearStateFile(_live, _utf8Args(path).$1), 'clear_state_file');

  bool _found(int status, String what) {
    if (status == NveRuntimeStatus.notFound.code) {
      return false;
    }
    api.check(status, what);
    return true;
  }

  /// Validates and forwards one `NVP1` hot patch (`apply_hot_patch`); `true` when it changed the app.
  bool applyHotPatch(Uint8List patch) {
    api.check(api.applyHotPatch(_live, _borrow(patch), _outU32), 'apply_hot_patch');
    return _outU32.value != 0;
  }

  /// Copies [bytes] into the reusable native block, described by [_buffer].
  NveBufferSlice _borrow(Uint8List bytes) {
    final block = _bytes.reserve(bytes.length);
    block.asTypedList(bytes.length).setAll(0, bytes);
    return _buffer.ref
      ..ptr = block
      ..len = bytes.length;
  }

  /// Advances one frame by [dtSeconds] (drains app async work and pair frames); `true` when a new
  /// scene was published.
  bool tick(double dtSeconds) {
    api.check(api.tick(_live, dtSeconds, _outU32), 'tick');
    return _outU32.value != 0;
  }

  /// Dispatches [events] in order as one uniform 64-byte `NveEventBatch`; `true` when the app
  /// requested a re-render.
  bool dispatchEvents(List<RuntimeEvent> events) {
    final handle = _live;
    if (events.isEmpty) {
      return false;
    }
    return _dispatchBatch(handle, EventBatch.encode(events, defaultSurfaceId: surfaceId));
  }

  /// Dispatches an already-encoded [EventBatch] directly across `dispatch_events`.
  bool dispatchEventBatch(EventBatch batch) => _dispatchBatch(_live, batch);

  bool _dispatchBatch(NveRuntimeHandle handle, EventBatch batch) {
    if (batch.eventCount == 0) {
      return false;
    }
    final evBuf = _events.reserve(batch.records.length);
    evBuf.asTypedList(batch.records.length).setAll(0, batch.records);
    final byteBuf = batch.bytes.isEmpty ? ffi.nullptr : _bytes.reserve(batch.bytes.length);
    if (batch.bytes.isNotEmpty) {
      byteBuf.asTypedList(batch.bytes.length).setAll(0, batch.bytes);
    }
    _batch.ref
      ..events = evBuf.cast<NveEvent>()
      ..bytes = byteBuf
      ..eventCount = batch.eventCount
      ..bytesLen = batch.bytes.length;
    api.check(api.dispatchEvents(handle, _batch.ref, _outU32), 'dispatch_events');
    return _outU32.value != 0;
  }

  /// Raw `NodeId` of the deepest event target under `(x, y)`, or `null`.
  int? hitTest(double x, double y) {
    api.check(api.hitTest(_live, x, y, _outU32), 'hit_test');
    final id = _outU32.value;
    return id == 0 ? null : id;
  }

  /// Scrolls container [nodeId] by `(dx, dy)`; returns its clamped offset.
  (double x, double y) scrollBy(int nodeId, double dx, double dy) {
    api.check(api.scrollBy(_live, nodeId, dx, dy, _outF32, _outF32 + 1), 'scroll_by');
    return (_outF32[0], _outF32[1]);
  }

  RuntimeMetrics metrics() {
    api.check(api.getMetrics(_live, _metrics), 'get_metrics');
    final m = _metrics.ref;
    return RuntimeMetrics(
      frameId: m.frameId,
      reconcileUs: m.reconcileUs,
      layoutUs: m.layoutUs,
      publishUs: m.publishUs,
      wireBytesRx: m.wireBytesRx,
      wireBytesTx: m.wireBytesTx,
      liveNodes: m.liveNodes,
      dirtyNodes: m.dirtyNodes,
      pairConnected: m.pairConnected != 0,
    );
  }

  /// Connects the embedded pairing client to [uri] (`nve://host:port` / `quic://…`). On success
  /// the runtime has atomically replaced (and dropped) any attached local app, so its library is
  /// released too; a failure leaves the app running.
  void pairConnect(String uri, {String token = ''}) {
    final handle = _live;
    final (u, t) = _utf8Args(uri, token);
    api.check(api.pairConnect(handle, u, t), 'pair_connect');
    _appLibrary = null;
  }

  /// UTF-8 copies of [a] and [b] in the reusable byte block, as the two scratch `NveStrSlice`s
  /// (valid until the next call that reuses the block).
  (NveStrSlice, NveStrSlice) _utf8Args(String a, [String b = '']) {
    final la = utf8EncodedLength(a);
    final lb = utf8EncodedLength(b);
    final bytes = _bytes.reserve(la + lb);
    final view = bytes.asTypedList(la + lb);
    _encodeUtf8(b, view, _encodeUtf8(a, view, 0));
    final sa = (_strs + 0).ref
      ..ptr = la == 0 ? ffi.nullptr : bytes
      ..len = la;
    final sb = (_strs + 1).ref
      ..ptr = lb == 0 ? ffi.nullptr : bytes + la
      ..len = lb;
    return (sa, sb);
  }

  void pairDisconnect() => api.check(api.pairDisconnect(_live), 'pair_disconnect');

  bool get isPairConnected => api.pairIsConnected(_live) != 0;

  /// Runs [read] over the current published scene between `scene_acquire` / `scene_release`.
  ///
  /// The reader borrows runtime memory: it (and every list it returns) MUST NOT escape [read].
  /// Not re-entrant: runtime writes (`tick`, events, viewport) inside [read] fail with `busy`.
  T withScene<T>(T Function(SceneReader scene) read) {
    final handle = _live;
    if (_inScene) {
      throw StateError('withScene is not re-entrant');
    }
    api.check(api.sceneAcquire(handle, _view), 'scene_acquire');
    _inScene = true;
    try {
      return read(_currentReader());
    } finally {
      _inScene = false;
      api.check(api.sceneRelease(handle), 'scene_release');
    }
  }

  SceneReader _currentReader() {
    final bytes = _view.cast<ffi.Uint8>().asTypedList(_readerView.length);
    final cached = _reader;
    if (cached != null && _sameBytes(bytes, _readerView)) {
      return cached;
    }
    _reader = null;
    final reader = readScene(_view.ref);
    _readerView.setAll(0, bytes);
    return _reader = reader;
  }

  static bool _sameBytes(Uint8List a, Uint8List b) {
    for (var i = 0; i < a.length; i++) {
      if (a[i] != b[i]) {
        return false;
      }
    }
    return true;
  }

  /// Destroys the surface and frees every native block (idempotent). Throws [StateError] inside
  /// [withScene]: the borrowed scene and the view block must outlive the read.
  void dispose() {
    if (_inScene) {
      throw StateError('RuntimeSession.dispose inside withScene');
    }
    if (_handle.address == 0) {
      return;
    }
    _reader = null;
    // Unregister first, so no new wake targets the listener; `destroy` then drains in-flight
    // wake callbacks before freeing, so only after it can the native trampoline close without a
    // worker thread invoking a closed callable.
    if (_wake != null) {
      api.setWakeCallback(_handle, ffi.nullptr, ffi.nullptr);
    }
    api.destroy(_handle);
    _handle = ffi.nullptr;
    _wake?.close();
    _wake = null;
    _appLibrary = null;
    NativeHeap.free(_scratch);
    _bytes.free();
    _events.free();
  }
}

/// Encodes [s] into `out[o ..]` in place (lone surrogates throw); returns the end offset.
int _encodeUtf8(String s, Uint8List out, int o) {
  for (var i = 0; i < s.length; i += utf16UnitsAt(s, i)) {
    o = encodeUtf8CodePointAt(s, i, out, o);
  }
  return o;
}

/// Natural alignment of `NveTableView` (`u32` fields + pointers: 8 on 64-bit, 4 on 32-bit).
final int _tableViewAlign = ffi.sizeOf<ffi.Pointer<ffi.Void>>();

/// Wraps a borrowed `NveSceneView` as a validated [SceneReader] (zero copy).
///
/// Throws [SceneException] for null slabs with non-zero counts, invalid table word counts, or
/// any [SceneReader] validation failure.
SceneReader readScene(NveSceneView v) {
  // A non-empty slab must be non-NULL, aligned for its element type, and its byte size must fit
  // the ABI's 32-bit lengths: a hostile `count * stride` never reaches `asTypedList`.
  ffi.Pointer<T> require<T extends ffi.NativeType>(ffi.Pointer<T> p, int count, int elemBytes, String what,
      {int align = 4}) {
    if (count > 0 && p.address == 0) {
      throw SceneException(SceneErrorKind.missingSlab, '$what is NULL with count $count');
    }
    if (p.address % align != 0) {
      throw SceneException(SceneErrorKind.missingSlab, '$what is not $align-byte aligned');
    }
    if (count > 0xFFFFFFFF ~/ elemBytes) {
      throw SceneException(SceneErrorKind.missingSlab, '$what: $count x $elemBytes bytes exceeds 32 bits');
    }
    return p;
  }

  final cap = v.nodeCapacity;
  final rows = cap == 0
      ? Uint32List(0)
      : require(v.rows, cap, nodeRowWords * 4, 'rows').cast<ffi.Uint32>().asTypedList(cap * nodeRowWords);
  final rects = cap == 0
      ? Float32List(0)
      : require(v.rects, cap, layoutRectFloats * 4, 'rects').cast<ffi.Float>().asTypedList(cap * layoutRectFloats);
  Uint32List u32s(ffi.Pointer<ffi.Uint32> p, int n, String what, [int words = 1]) =>
      n == 0 ? Uint32List(0) : require(p, n, words * 4, what).asTypedList(n * words);

  final tables = List<TableReader>.generate(v.tableCount, (i) {
    final t = (require(v.tables, v.tableCount, ffi.sizeOf<NveTableView>(), 'tables', align: _tableViewAlign) + i).ref;
    if (t.words < 1 || t.words > maxTableWords) {
      throw SceneException(SceneErrorKind.invalidTable, 'table $i: words ${t.words} outside 1..=$maxTableWords');
    }
    try {
      return TableReader(
        key: Key64(t.keyLo, t.keyHi),
        slot: t.slot,
        words: t.words,
        rowCount: t.rowCount,
        rows: ByteData.sublistView(u32s(t.rows, t.rowCount, 'table $i rows', t.words)),
        owners: t.owners.address == 0 ? null : ByteData.sublistView(u32s(t.owners, t.rowCount, 'table $i owners')),
        dirtyRows: ByteData.sublistView(u32s(t.dirtyRows, t.dirtyCount, 'table $i dirtyRows')),
        dirtyCount: t.dirtyCount,
        rowIndexDir: ByteData.sublistView(u32s(t.rowIndexDir, t.rowIndexDirLen, 'table $i rowIndexDir')),
        rowIndexPages: ByteData.sublistView(
            u32s(t.rowIndexPages, t.rowIndexPageCount, 'table $i rowIndexPages', nveRowIndexPageLen)),
      );
    } on ArgumentError catch (e) {
      throw SceneException(SceneErrorKind.invalidTable, 'table $i: ${e.message}');
    }
  }, growable: false);

  return SceneReader(
    expectedSchema: nveRuntimeSchemaHash,
    schemaHash: key64FromBits(v.schemaHash),
    frameId: key64FromBits(v.frameId),
    nodeCapacity: cap,
    rows: rows,
    rects: rects,
    textSlab:
        v.textSlabLen == 0 ? Uint8List(0) : require(v.textSlab, v.textSlabLen, 1, 'textSlab', align: 1).asTypedList(v.textSlabLen),
    childrenSlab: u32s(v.childrenSlab, v.childrenSlabLen, 'childrenSlab'),
    dirtyIndices: u32s(v.dirtyIndices, v.dirtyCount, 'dirtyIndices'),
    removedIds: u32s(v.removedIds, v.removedCount, 'removedIds'),
    tables: tables,
    scrollOffsets: v.scrollCount == 0
        ? null
        : require(v.scrollOffsets, v.scrollCount, scrollOffsetWords * 4, 'scrollOffsets')
            .cast<ffi.Uint32>()
            .asTypedList(v.scrollCount * scrollOffsetWords),
    sceneEpoch: v.sceneEpoch,
    dirtyResources: u32s(v.dirtyResources, v.dirtyResourceCount, 'dirtyResources'),
    movedIndices: u32s(v.movedIndices, v.movedCount, 'movedIndices'),
  );
}
