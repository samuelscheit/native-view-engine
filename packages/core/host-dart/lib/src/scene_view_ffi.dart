/// `dart:ffi` scene-view reader and native scratch helpers backing [RuntimeSession].
library;

import 'dart:ffi' as ffi;
import 'dart:typed_data';

import 'generated/nve_abi_const.generated.dart';
import 'generated/nve_abi_ffi.generated.dart';
import 'generated/nve_runtime_abi.generated.dart';
import 'key64.dart';
import 'native_heap.dart';
import 'runtime_api.dart' show key64FromBits;
import 'scene_reader.dart';
import 'table_reader.dart';
import 'utf8.dart' show encodeUtf8CodePointAt, utf16UnitsAt;

/// Growable zero-filled native block reused across FFI calls (never shrinks).
final class NativeBlock {
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

/// Encodes [s] into `out[o ..]` in place (lone surrogates throw); returns the end offset.
int encodeUtf8Into(String s, Uint8List out, int o) {
  for (var i = 0; i < s.length; i += utf16UnitsAt(s, i)) {
    o = encodeUtf8CodePointAt(s, i, out, o);
  }
  return o;
}

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

  // `NveTableView` holds 32 bytes of `u32` fields plus 5 pointers, so its ABI alignment is
  // `alignof(void*)` (4 on 32-bit targets, 8 on 64-bit targets).
  final tableAlign = ffi.sizeOf<ffi.Pointer<ffi.Void>>();
  final tablesPtr = v.tableCount == 0
      ? ffi.nullptr.cast<NveTableView>()
      : require(v.tables, v.tableCount, ffi.sizeOf<NveTableView>(), 'tables', align: tableAlign);
  final tables = List<TableReader>.generate(v.tableCount, (i) {
    final t = (tablesPtr + i).ref;
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
    textSlab: v.textSlabLen == 0
        ? Uint8List(0)
        : require(v.textSlab, v.textSlabLen, 1, 'textSlab', align: 1).asTypedList(v.textSlabLen),
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
