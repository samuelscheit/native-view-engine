/// Validated zero-copy reader over one published `NveSceneView` frame.
///
/// Node records are read by byte offset: the generated ABI supplies the node stride, field
/// offsets, and (for linear memory) the [SceneLayout], so this static runtime never hard-codes a
/// layout. Construction validates the schema hash, every slab against its count, and the text
/// slab's UTF-8 once; later reads are O(1) bounds checks (out of range throws [RangeError]).
library;

import 'dart:typed_data';

import 'key64.dart';
import 'table_reader.dart';
import 'utf8.dart';

/// Byte offsets of `NveTableView` fields inside wasm32 linear memory (pointers are u32).
final class TableViewLayout {
  const TableViewLayout({
    required this.stride,
    required this.keyLo,
    required this.keyHi,
    required this.slot,
    required this.words,
    required this.rowCount,
    required this.dirtyCount,
    required this.rows,
    required this.owners,
    required this.dirtyRows,
  });

  final int stride, keyLo, keyHi, slot, words, rowCount, dirtyCount, rows, owners, dirtyRows;
}

/// Byte offsets of `NveSceneView` fields inside wasm32 linear memory (pointers are u32), plus the
/// node record stride. Emitted by codegen from the Rust `offset_of!`-locked ABI.
final class SceneLayout {
  const SceneLayout({
    required this.schemaHash,
    required this.frameId,
    required this.nodeCount,
    required this.dirtyCount,
    required this.removedCount,
    required this.textSlabLen,
    required this.childrenSlabLen,
    required this.tableCount,
    required this.nodes,
    required this.textSlab,
    required this.childrenSlab,
    required this.dirtyIndices,
    required this.removedIds,
    required this.tables,
    required this.nodeStride,
    required this.tableView,
  });

  final int schemaHash, frameId, nodeCount, dirtyCount, removedCount, textSlabLen, childrenSlabLen, tableCount;
  final int nodes, textSlab, childrenSlab, dirtyIndices, removedIds, tables;
  final int nodeStride;
  final TableViewLayout tableView;
}

enum SceneErrorKind { schemaMismatch, missingSlab, invalidUtf8, invalidTable }

final class SceneException implements Exception {
  const SceneException(this.kind, this.message);

  final SceneErrorKind kind;
  final String message;

  @override
  String toString() => 'SceneException(${kind.name}): $message';
}

final class SceneReader {
  /// Wraps per-slab views (e.g. `Pointer.asTypedList` on native, or linear-memory views).
  ///
  /// Throws [SceneException] when the schema, a slab, the text UTF-8, or the node stride is invalid.
  SceneReader({
    required Key64 expectedSchema,
    required Key64 schemaHash,
    required this.frameId,
    required ByteData nodes,
    required this.nodeCount,
    required this.nodeStride,
    required Uint8List textSlab,
    required ByteData childrenSlab,
    required this.childrenSlabLen,
    required ByteData dirtyIndices,
    required this.dirtyCount,
    required ByteData removedIds,
    required this.removedCount,
    this.tables = const <TableReader>[],
  })  : _nodes = nodes,
        _text = textSlab,
        _children = childrenSlab,
        _dirty = dirtyIndices,
        _removed = removedIds {
    if (schemaHash != expectedSchema) {
      throw SceneException(SceneErrorKind.schemaMismatch, 'expected $expectedSchema, got $schemaHash');
    }
    if (nodeStride <= 0) {
      throw SceneException(SceneErrorKind.missingSlab, 'nodeStride $nodeStride');
    }
    _requireRecords(nodes, nodeCount, nodeStride, 'nodes');
    _requireRecords(childrenSlab, childrenSlabLen, 4, 'childrenSlab');
    _requireRecords(dirtyIndices, dirtyCount, 4, 'dirtyIndices');
    _requireRecords(removedIds, removedCount, 4, 'removedIds');
    if (!isValidUtf8(textSlab)) {
      throw const SceneException(SceneErrorKind.invalidUtf8, 'textSlab');
    }
  }

  /// Reads the `NveSceneView` at [viewPtr] inside wasm32 linear [memory] (zero copy).
  factory SceneReader.fromLinearMemory(ByteData memory, int viewPtr, SceneLayout layout, Key64 expectedSchema) {
    int u32(int at) {
      if (at < 0 || at > memory.lengthInBytes - 4) {
        throw SceneException(SceneErrorKind.missingSlab, 'read at $at outside memory (${memory.lengthInBytes})');
      }
      return memory.getUint32(at, Endian.little);
    }

    ByteData slab(int ptr, int bytes, String what) {
      if (bytes == 0) {
        return ByteData(0);
      }
      if (ptr == 0 || ptr + bytes > memory.lengthInBytes) {
        throw SceneException(SceneErrorKind.missingSlab, '$what [$ptr, +$bytes) outside memory');
      }
      return ByteData.sublistView(memory, ptr, ptr + bytes);
    }

    final tl = layout.tableView;
    if (layout.nodeStride < 1 || tl.stride < 1) {
      throw SceneException(SceneErrorKind.missingSlab, 'layout strides must be >= 1 (node ${layout.nodeStride}, table ${tl.stride})');
    }
    final h = viewPtr;
    final schema = Key64(u32(h + layout.schemaHash), u32(h + layout.schemaHash + 4));
    if (schema != expectedSchema) {
      throw SceneException(SceneErrorKind.schemaMismatch, 'expected $expectedSchema, got $schema');
    }
    final nodeCount = u32(h + layout.nodeCount);
    final textLen = u32(h + layout.textSlabLen);
    final childrenLen = u32(h + layout.childrenSlabLen);
    final dirtyCount = u32(h + layout.dirtyCount);
    final removedCount = u32(h + layout.removedCount);
    final tableCount = u32(h + layout.tableCount);
    final tablesBase = u32(h + layout.tables);
    if (tableCount > 0 && (tablesBase == 0 || tablesBase > memory.lengthInBytes || (memory.lengthInBytes - tablesBase) ~/ tl.stride < tableCount)) {
      throw SceneException(SceneErrorKind.invalidTable, 'tables [$tablesBase, x$tableCount) outside memory');
    }
    final tables = List<TableReader>.generate(tableCount, (i) {
      final t = tablesBase + i * tl.stride;
      final words = u32(t + tl.words);
      if (words < 1 || words > maxTableWords) {
        throw SceneException(SceneErrorKind.invalidTable, 'table $i: words $words outside 1..=$maxTableWords');
      }
      final rowCount = u32(t + tl.rowCount);
      final tableDirty = u32(t + tl.dirtyCount);
      final ownersPtr = u32(t + tl.owners);
      try {
        return TableReader(
          key: Key64(u32(t + tl.keyLo), u32(t + tl.keyHi)),
          slot: u32(t + tl.slot),
          words: words,
          rowCount: rowCount,
          rows: slab(u32(t + tl.rows), rowCount * words * 4, 'table $i rows'),
          owners: ownersPtr == 0 ? null : slab(ownersPtr, rowCount * 4, 'table $i owners'),
          dirtyRows: slab(u32(t + tl.dirtyRows), tableDirty * 4, 'table $i dirtyRows'),
          dirtyCount: tableDirty,
        );
      } on ArgumentError catch (e) {
        throw SceneException(SceneErrorKind.invalidTable, 'table $i: ${e.message}');
      } on SceneException catch (e) {
        throw SceneException(SceneErrorKind.invalidTable, e.message);
      }
    }, growable: false);
    final text = slab(u32(h + layout.textSlab), textLen, 'textSlab');
    return SceneReader(
      expectedSchema: expectedSchema,
      schemaHash: schema,
      frameId: Key64(u32(h + layout.frameId), u32(h + layout.frameId + 4)),
      nodes: slab(u32(h + layout.nodes), nodeCount * layout.nodeStride, 'nodes'),
      nodeCount: nodeCount,
      nodeStride: layout.nodeStride,
      textSlab: Uint8List.sublistView(text),
      childrenSlab: slab(u32(h + layout.childrenSlab), childrenLen * 4, 'childrenSlab'),
      childrenSlabLen: childrenLen,
      dirtyIndices: slab(u32(h + layout.dirtyIndices), dirtyCount * 4, 'dirtyIndices'),
      dirtyCount: dirtyCount,
      removedIds: slab(u32(h + layout.removedIds), removedCount * 4, 'removedIds'),
      removedCount: removedCount,
      tables: tables,
    );
  }

  /// Division, never `count * size`: hostile counts cannot wrap the 64-bit VM `int`.
  static void _requireRecords(ByteData view, int count, int size, String what) {
    if (count < 0 || view.lengthInBytes ~/ size < count) {
      throw SceneException(SceneErrorKind.missingSlab, '$what holds ${view.lengthInBytes} bytes, need $count x $size');
    }
  }

  /// Exact 64-bit frame counter.
  final Key64 frameId;
  final int nodeCount;
  final int nodeStride;
  final int childrenSlabLen;
  final int dirtyCount;
  final int removedCount;
  final List<TableReader> tables;
  final ByteData _nodes;
  final Uint8List _text;
  final ByteData _children;
  final ByteData _dirty;
  final ByteData _removed;

  Uint8List get textSlab => _text;

  // ---- node records (offsets from the generated ABI) -------------------------------------

  int nodeU8(int index, int offset) => _nodes.getUint8(_nodeAt(index, offset, 1));
  int nodeU16(int index, int offset) => _nodes.getUint16(_nodeAt(index, offset, 2), Endian.little);
  int nodeU32(int index, int offset) => _nodes.getUint32(_nodeAt(index, offset, 4), Endian.little);
  int nodeI32(int index, int offset) => _nodes.getInt32(_nodeAt(index, offset, 4), Endian.little);
  double nodeF32(int index, int offset) => _nodes.getFloat32(_nodeAt(index, offset, 4), Endian.little);

  int _nodeAt(int index, int offset, int size) {
    RangeError.checkValidIndex(index, this, 'node', nodeCount);
    RangeError.checkValueInInterval(offset, 0, nodeStride - size, 'offset');
    return index * nodeStride + offset;
  }

  // ---- slabs ---------------------------------------------------------------------------------

  /// View of `textSlab[offset ..< offset + len]`; throws [RangeError] when out of range or not on
  /// UTF-8 boundaries.
  Uint8List textBytes(int offset, int len) {
    if (offset < 0 || len < 0 || len > _text.length - offset || !_isBoundary(offset) || !_isBoundary(offset + len)) {
      throw RangeError('text [$offset, +$len) outside textSlab (${_text.length}) or splits a code point');
    }
    return Uint8List.sublistView(_text, offset, offset + len);
  }

  /// Text at `[offset, offset + len)`; a leading U+FEFF is kept (never stripped).
  String text(int offset, int len) => decodeUtf8PreserveBom(textBytes(offset, len));

  bool _isBoundary(int at) => at == _text.length || (_text[at] & 0xC0) != 0x80;

  int childId(int slabIndex) {
    RangeError.checkValidIndex(slabIndex, this, 'child', childrenSlabLen);
    return _children.getUint32(slabIndex * 4, Endian.little);
  }

  int dirtyIndex(int i) {
    RangeError.checkValidIndex(i, this, 'dirty', dirtyCount);
    return _dirty.getUint32(i * 4, Endian.little);
  }

  int removedId(int i) {
    RangeError.checkValidIndex(i, this, 'removed', removedCount);
    return _removed.getUint32(i * 4, Endian.little);
  }

  /// Visits every in-range dirty node index in O(dirty).
  void forEachDirty(void Function(int index) visit) {
    for (var i = 0; i < dirtyCount; i++) {
      final index = dirtyIndex(i);
      if (index < nodeCount) {
        visit(index);
      }
    }
  }

  /// Published table with [key], or `null` (O(tables)).
  TableReader? findTable(Key64 key) {
    for (final t in tables) {
      if (t.key == key) {
        return t;
      }
    }
    return null;
  }
}
