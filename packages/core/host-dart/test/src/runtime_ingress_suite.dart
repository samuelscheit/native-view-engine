/// `dart:ffi` layout locks of the runtime scene ABI (sizes and field offsets mirror the Rust
/// `offset_of!` asserts) and untrusted `NveSceneView` ingress: every malformed view a
/// runtime could publish is rejected with a [SceneException] before any slab is viewed.
library;

import 'dart:ffi' as ffi;

import 'package:nve_host/nve_host.dart';
import 'package:nve_host/nve_host_ffi.dart';

import 'check.dart';

final int _ptr = ffi.sizeOf<ffi.Pointer<ffi.Void>>();

/// Byte offset of the field [write] stores [probe] into (found by scanning the zeroed struct).
int _offsetOf<T extends ffi.Struct>(int size, void Function(ffi.Pointer<T> p) write, int probeWidth) {
  final p = NativeHeap.zeroed<T>(size);
  try {
    write(p);
    final bytes = p.cast<ffi.Uint8>().asTypedList(size);
    final at = bytes.indexWhere((b) => b != 0);
    return at < 0 ? -1 : at - (at % probeWidth);
  } finally {
    NativeHeap.free(p);
  }
}

void runRuntimeIngress(Check c) {
  // ---- layout locks ---------------------------------------------------------------------------
  c.eq(ffi.sizeOf<NveNodeRow>(), nveNodeRowSize, 'NveNodeRow size');
  c.eq(nveNodeRowSize, nodeRowWords * 4, 'node row words');
  c.eq(ffi.sizeOf<NveLayoutRect>(), nveLayoutRectSize, 'NveLayoutRect size');
  c.eq(nveLayoutRectSize, layoutRectFloats * 4, 'layout rect floats');
  c.eq(nveLayoutRectSize, 16, 'NveLayoutRect is 16 bytes');
  c.eq(nveNodeRowSize, 32, 'NveNodeRow is 32 bytes');
  c.eq(scrollOffsetWords * 4, 12, 'NveScrollOffset is 12 bytes');
  c.eq(ffi.sizeOf<NveScrollOffset>(), scrollOffsetWords * 4, 'NveScrollOffset size');
  c.eq(ffi.sizeOf<NveTableView>(), 32 + 5 * _ptr, 'NveTableView size (72 on 64-bit, 52 on 32-bit)');
  c.eq(ffi.sizeOf<RawEventBytes>(), 8, 'RawEventBytes size');
  c.eq(ffi.sizeOf<NveEvent>(), nveEventSize, 'NveEvent size (64 bytes)');
  c.eq(ffi.sizeOf<NveEventBatch>(), 2 * _ptr + 8, 'NveEventBatch size (24 on 64-bit, 16 on 32-bit)');
  int evOffset(void Function(NveEvent e) write, [int width = 4]) =>
      _offsetOf<NveEvent>(nveEventSize, (p) => write(p.ref), width);
  c.eq(evOffset((e) => e.timestamp = 1, 8), NveEventLayout.timestamp, 'NveEvent.timestamp @0');
  c.eq(evOffset((e) => e.table = 1, 8), NveEventLayout.table, 'NveEvent.table @8');
  c.eq(evOffset((e) => e.surface = 1), NveEventLayout.surface, 'NveEvent.surface @16');
  c.eq(evOffset((e) => e.node = 1), NveEventLayout.node, 'NveEvent.node @20');
  c.eq(evOffset((e) => e.event = 1), NveEventLayout.event, 'NveEvent.event @24');
  c.eq(evOffset((e) => e.flags = 1), NveEventLayout.flags, 'NveEvent.flags @28');
  c.eq(evOffset((e) => e.payload[0] = 1), NveEventLayout.payload, 'NveEvent.payload @32');
  c.eq(evOffset((e) => e.bytes.offset = 1), NveEventLayout.bytes, 'NveEvent.bytes @56');
  final batchSize = ffi.sizeOf<NveEventBatch>();
  int batchOffset(void Function(NveEventBatch b) write, [int width = 4]) =>
      _offsetOf<NveEventBatch>(batchSize, (p) => write(p.ref), width);
  c.eq(batchOffset((b) => b.events = ffi.Pointer.fromAddress(0x10), _ptr), 0, 'NveEventBatch.events @0');
  c.eq(batchOffset((b) => b.bytes = ffi.Pointer.fromAddress(0x10), _ptr), _ptr, 'NveEventBatch.bytes');
  c.eq(batchOffset((b) => b.eventCount = 1), 2 * _ptr, 'NveEventBatch.eventCount');
  c.eq(batchOffset((b) => b.bytesLen = 1), 2 * _ptr + 4, 'NveEventBatch.bytesLen');
  final viewSize = ffi.sizeOf<NveSceneView>();
  // 56 + 10 pointers is a multiple of 8 on both data models: no tail padding after `moved_indices`.
  c.eq(viewSize, 56 + 10 * _ptr, 'NveSceneView size (136 on 64-bit, 96 on 32-bit)');
  // The measurer's style: fixed scalars, then the family slice at 24 on every data model.
  c.eq(ffi.sizeOf<NveRuntimeTextStyle>(), 24 + ffi.sizeOf<NveStrSlice>(), 'NveRuntimeTextStyle size');
  c.eq(
      _offsetOf<NveRuntimeTextStyle>(
          ffi.sizeOf<NveRuntimeTextStyle>(), (s) => s.ref.fontFamily.ptr = ffi.Pointer.fromAddress(0x10), _ptr),
      24,
      'NveRuntimeTextStyle.font_family @24');
  int viewOffset(void Function(NveSceneView v) write, [int width = 4]) =>
      _offsetOf<NveSceneView>(viewSize, (p) => write(p.ref), width);
  c.eq(viewOffset((v) => v.schemaHash = 1, 8), 0, 'schema_hash @0');
  c.eq(viewOffset((v) => v.frameId = 1, 8), 8, 'frame_id @8');
  c.eq(viewOffset((v) => v.nodeCapacity = 1), 16, 'node_capacity @16');
  c.eq(viewOffset((v) => v.tableCount = 1), 36, 'table_count @36');
  c.eq(viewOffset((v) => v.scrollCount = 1), 40, 'scroll_count @40');
  c.eq(viewOffset((v) => v.sceneEpoch = 1), 44, 'scene_epoch @44');
  c.eq(viewOffset((v) => v.dirtyResourceCount = 1), 48, 'dirty_resource_count @48');
  c.eq(viewOffset((v) => v.movedCount = 1), 52, 'moved_count @52');
  final p = ffi.Pointer<ffi.Uint32>.fromAddress(0x10);
  c.eq(viewOffset((v) => v.rows = p.cast(), _ptr), 56, 'rows @56');
  c.eq(viewOffset((v) => v.tables = p.cast(), _ptr), 56 + 6 * _ptr, 'tables');
  c.eq(viewOffset((v) => v.scrollOffsets = p.cast(), _ptr), 56 + 7 * _ptr, 'scroll_offsets');
  c.eq(viewOffset((v) => v.dirtyResources = p.cast(), _ptr), 56 + 8 * _ptr, 'dirty_resources');
  c.eq(viewOffset((v) => v.movedIndices = p.cast(), _ptr), 56 + 9 * _ptr, 'moved_indices is last');
  c.eq(_offsetOf<NveScrollOffset>(12, (s) => s.ref.y = 1, 4), 8, 'NveScrollOffset.y @8');
  int tableOffset(void Function(NveTableView t) write, [int width = 4]) =>
      _offsetOf<NveTableView>(ffi.sizeOf<NveTableView>(), (t) => write(t.ref), width);
  c.eq(tableOffset((t) => t.rowIndexDirLen = 1), 24, 'NveTableView.row_index_dir_len @24');
  c.eq(tableOffset((t) => t.rowIndexPageCount = 1), 28, 'NveTableView.row_index_page_count @28');
  c.eq(tableOffset((t) => t.rows = p, _ptr), 32, 'NveTableView.rows @32');
  c.eq(tableOffset((t) => t.rowIndexDir = p, _ptr), 32 + 3 * _ptr, 'NveTableView.row_index_dir');
  c.eq(tableOffset((t) => t.rowIndexPages = p, _ptr), 32 + 4 * _ptr, 'NveTableView.row_index_pages is last');

  // NveRuntimeConfig / NveRuntimeMetrics (Rust `layout_asserts.rs`) and the NveRuntimeApi vtable.
  c.eq(ffi.sizeOf<NveRuntimeConfig>(), 24, 'NveRuntimeConfig size');
  int configOffset(void Function(NveRuntimeConfig c) write) => _offsetOf<NveRuntimeConfig>(24, (p) => write(p.ref), 4);
  c.eq(configOffset((v) => v.surfaceId = 1), 0, 'config.surface_id @0');
  c.eq(configOffset((v) => v.width = 1), 4, 'config.width @4');
  c.eq(configOffset((v) => v.height = 1), 8, 'config.height @8');
  c.eq(configOffset((v) => v.scaleFactor = 1), 12, 'config.scale_factor @12');
  c.eq(configOffset((v) => v.flags = 1), 16, 'config.flags @16');
  c.eq(ffi.sizeOf<NveRuntimeMetrics>(), 64, 'NveRuntimeMetrics size');
  int metricsOffset(void Function(NveRuntimeMetrics m) write, [int width = 8]) =>
      _offsetOf<NveRuntimeMetrics>(64, (p) => write(p.ref), width);
  c.eq(metricsOffset((m) => m.frameId = 1), 0, 'metrics.frame_id @0');
  c.eq(metricsOffset((m) => m.reconcileUs = 1), 8, 'metrics.reconcile_us @8');
  c.eq(metricsOffset((m) => m.layoutUs = 1), 16, 'metrics.layout_us @16');
  c.eq(metricsOffset((m) => m.publishUs = 1), 24, 'metrics.publish_us @24');
  c.eq(metricsOffset((m) => m.wireBytesRx = 1), 32, 'metrics.wire_bytes_rx @32');
  c.eq(metricsOffset((m) => m.wireBytesTx = 1), 40, 'metrics.wire_bytes_tx @40');
  c.eq(metricsOffset((m) => m.liveNodes = 1, 4), 48, 'metrics.live_nodes @48');
  c.eq(metricsOffset((m) => m.dirtyNodes = 1, 4), 52, 'metrics.dirty_nodes @52');
  c.eq(metricsOffset((m) => m.pairConnected = 1, 4), 56, 'metrics.pair_connected @56');
  final apiSize = ffi.sizeOf<NveRuntimeApi>();
  c.eq(apiSize, 16 + nveRuntimeApiSlotCount * _ptr, 'NveRuntimeApi size: header + $nveRuntimeApiSlotCount slots');
  int apiOffset(void Function(NveRuntimeApi a) write, [int? width]) =>
      _offsetOf<NveRuntimeApi>(apiSize, (p) => write(p.ref), width ?? _ptr);
  c.eq(apiOffset((a) => a.abiVersion = 1, 4), 0, 'api.abi_version @0');
  c.eq(apiOffset((a) => a.structSize = 1, 4), 4, 'api.struct_size @4');
  c.eq(apiOffset((a) => a.schemaHash = 1, 8), 8, 'api.schema_hash @8');
  c.eq(apiOffset((a) => a.create = p.cast()), 16, 'api.create is slot 0');
  c.eq(apiOffset((a) => a.setTextMeasurer = p.cast()), 16 + 4 * _ptr, 'api.set_text_measurer is slot 4');
  c.eq(apiOffset((a) => a.lastError = p.cast()), 16 + (nveRuntimeApiSlotCount - 1) * _ptr, 'api.last_error is last');
  final api = NativeHeap.zeroed<NveRuntimeApi>(apiSize);
  try {
    c.eq(runtimeApiSlotsComplete(ffi.nullptr), false, 'NULL vtable is incomplete');
    c.eq(runtimeApiSlotsComplete(api), false, 'zeroed vtable is incomplete');
    api.cast<ffi.Uint8>().asTypedList(apiSize).fillRange(16, apiSize, 0x10);
    c.eq(runtimeApiSlotsComplete(api), true, 'every slot set');
    (api.cast<ffi.Uint8>() + 16 + 9 * _ptr).asTypedList(_ptr).fillRange(0, _ptr, 0);
    c.eq(runtimeApiSlotsComplete(api), false, 'one NULL slot is incomplete');
  } finally {
    NativeHeap.free(api);
  }

  // ---- untrusted ingress ----------------------------------------------------------------------
  final view = NativeHeap.zeroed<NveSceneView>(viewSize);
  final table = NativeHeap.zeroed<NveTableView>(ffi.sizeOf<NveTableView>());
  final words = NativeHeap.zeroed<ffi.Uint32>(64 * 4);
  try {
    void reset() {
      view.cast<ffi.Uint8>().asTypedList(viewSize).fillRange(0, viewSize, 0);
      table.cast<ffi.Uint8>().asTypedList(ffi.sizeOf<NveTableView>()).fillRange(0, ffi.sizeOf<NveTableView>(), 0);
      view.ref.schemaHash = nveRuntimeSchemaHash.toBits();
    }

    void rejects(SceneErrorKind kind, String what, void Function(NveSceneView v) corrupt) {
      reset();
      corrupt(view.ref);
      final e = c.throwsA<SceneException>(() => readScene(view.ref), what);
      if (e != null) {
        c.eq(e.kind, kind, '$what kind');
      }
    }

    reset();
    c.eq(readScene(view.ref).nodeCapacity, 0, 'all-zero view (before the first frame) reads empty');
    rejects(SceneErrorKind.schemaMismatch, 'foreign schema', (v) => v.schemaHash = 7);
    rejects(SceneErrorKind.missingSlab, 'NULL rows with capacity', (v) => v.nodeCapacity = 2);
    rejects(SceneErrorKind.missingSlab, 'NULL text slab with length', (v) => v.textSlabLen = 3);
    rejects(SceneErrorKind.missingSlab, 'NULL children with length', (v) => v.childrenSlabLen = 1);
    rejects(SceneErrorKind.missingSlab, 'NULL dirty indices with count', (v) => v.dirtyCount = 1);
    rejects(SceneErrorKind.missingSlab, 'NULL tables with count', (v) => v.tableCount = 1);
    rejects(SceneErrorKind.missingSlab, 'NULL scroll offsets with count', (v) => v.scrollCount = 1);
    rejects(SceneErrorKind.missingSlab, 'NULL dirty resources with count', (v) => v.dirtyResourceCount = 1);
    rejects(SceneErrorKind.missingSlab, 'NULL moved indices with count', (v) => v.movedCount = 1);
    rejects(SceneErrorKind.missingSlab, 'misaligned children slab', (v) {
      v
        ..childrenSlabLen = 1
        ..childrenSlab = (words.cast<ffi.Uint8>() + 2).cast();
    });
    rejects(SceneErrorKind.missingSlab, 'misaligned table views', (v) {
      v
        ..tableCount = 1
        ..tables = (table.cast<ffi.Uint8>() + (_ptr ~/ 2)).cast();
    });
    rejects(SceneErrorKind.missingSlab, 'rows byte size past 32 bits', (v) {
      v
        ..nodeCapacity = 0xFFFFFFFF
        ..rows = words.cast()
        ..rects = words.cast();
    });
    reset();
    view.ref
      ..sceneEpoch = 9
      ..dirtyResourceCount = 2
      ..dirtyResources = words;
    words[0] = 3;
    words[1] = 7;
    final withResources = readScene(view.ref);
    c.eq(withResources.sceneEpoch, 9, 'scene epoch');
    c.eq(withResources.dirtyResources.toList().join(','), '3,7', 'dirty resources');
    words[0] = 0;
    words[1] = 0;
    rejects(SceneErrorKind.missingSlab, 'scroll slab with id 0', (v) {
      v
        ..scrollCount = 1
        ..scrollOffsets = words.cast();
    });
    void withTable(NveSceneView v, void Function(NveTableView t) set) {
      v
        ..tableCount = 1
        ..tables = table;
      table.ref
        ..keyLo = 1
        ..words = 1
        ..rows = words;
      set(table.ref);
    }

    rejects(SceneErrorKind.invalidTable, 'table words 0', (v) => withTable(v, (t) => t.words = 0));
    rejects(SceneErrorKind.invalidTable, 'table words > 32', (v) => withTable(v, (t) => t.words = maxTableWords + 1));
    rejects(SceneErrorKind.missingSlab, 'NULL table rows with rows', (v) => withTable(v, (t) {
          t
            ..rowCount = 2
            ..rows = ffi.nullptr;
        }));
    rejects(SceneErrorKind.missingSlab, 'NULL dirty rows with count', (v) => withTable(v, (t) => t.dirtyCount = 1));
    rejects(SceneErrorKind.invalidTable, 'table slot out of range', (v) => withTable(v, (t) => t.slot = 1000));
    rejects(SceneErrorKind.missingSlab, 'NULL row index dir with length', (v) => withTable(v, (t) => t.rowIndexDirLen = 1));
    rejects(SceneErrorKind.missingSlab, 'NULL row index pages with count', (v) => withTable(v, (t) => t.rowIndexPageCount = 1));
    rejects(SceneErrorKind.invalidTable, 'sparse rows without a row index', (v) => withTable(v, (t) {
          t
            ..rowCount = 1
            ..owners = words;
        }));
    words.asTypedList(4).setAll(0, [1, 0, 0, 0]);
    reset();
    withTable(view.ref, (t) => t.rowCount = 1);
    c.eq(readScene(view.ref).tables.length, 1, 'well-formed table view reads');

    // Moved indices are validated like dirty ones and read zero-copy.
    words.asTypedList(2).setAll(0, [1, 0]);
    reset();
    view.ref
      ..nodeCapacity = 2
      ..rows = NativeHeap.zeroed<NveNodeRow>(2 * ffi.sizeOf<NveNodeRow>())
      ..rects = NativeHeap.zeroed<NveLayoutRect>(2 * ffi.sizeOf<NveLayoutRect>())
      ..movedCount = 1
      ..movedIndices = words;
    try {
      c.eq(readScene(view.ref).movedIndices.join(','), '1', 'moved indices');
      words[0] = 2;
      final e = c.throwsA<SceneException>(() => readScene(view.ref), 'moved index >= capacity');
      c.eq(e?.kind, SceneErrorKind.missingSlab, 'moved index >= capacity kind');
    } finally {
      NativeHeap.free(view.ref.rows);
      NativeHeap.free(view.ref.rects);
    }

    // A sparse table resolves rows through the published row index in O(1): node index 300 lives
    // in directory slot 1 -> page 0, entry 300 & 255 = 44 holds `row + 1`.
    final index = NativeHeap.zeroed<ffi.Uint32>(4 * (2 + nveRowIndexPageLen));
    try {
      final ix = index.asTypedList(2 + nveRowIndexPageLen);
      ix[0] = nveRowIndexNoPage;
      ix[1] = 0;
      ix[2 + 44] = 2;
      words.asTypedList(4).setAll(0, [11, 22, 0xAB000005, 300 | (3 << 24)]);
      reset();
      withTable(view.ref, (t) {
        t
          ..rowCount = 2
          ..owners = words + 2
          ..rowIndexDirLen = 2
          ..rowIndexPageCount = 1
          ..rowIndexDir = index
          ..rowIndexPages = index + 2;
      });
      final sparse = readScene(view.ref).tables.single;
      c.eq(sparse.rowOfNode(300), 1, 'sparse row of node 300');
      c.eq(sparse.rowOf(300 | (3 << 24)), 1, 'sparse row of its live id');
      c.eq(sparse.rowOf(300 | (2 << 24)), null, 'stale generation misses');
      c.eq(sparse.rowOfNode(5), null, 'NO_PAGE directory slot misses');
      c.eq(sparse.rowOfNode(512), null, 'directory slot past the end misses');
      // 2^32 + 300 would wrap onto node 300 under dart2js's 32-bit shifts.
      c.eq(sparse.rowOfNode(0x100000000 + 300), null, 'index past 24 bits misses');
    } finally {
      NativeHeap.free(index);
    }
  } finally {
    NativeHeap.free(view);
    NativeHeap.free(table);
    NativeHeap.free(words);
  }
}

extension on Key64 {
  int toBits() => (hi << 32) | lo;
}
