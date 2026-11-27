/// Validated zero-copy reader over one borrowed `NveRuntimeSceneView` frame.
///
/// Mirrors the C++ `nve::host::RuntimeSceneReader`: construction validates the schema hash, every
/// slab against its count, the text slab's UTF-8, every live node's child range, every dirty
/// index, and every published table once; later reads are plain typed-list loads. Native hosts
/// wrap the borrowed pointers with `Pointer.asTypedList` (`package:nve_host/nve_host_ffi.dart`);
/// tests and web hosts pass ordinary typed lists.
library;

import 'dart:typed_data';

import 'key64.dart';
import 'scene_reader.dart' show SceneErrorKind, SceneException;
import 'table_reader.dart';
import 'utf8.dart';

/// `u32` words per `NveNodeRow` (32 bytes).
const int nodeRowWords = 8;

/// `f32` values per `NveLayoutRect` (16 bytes).
const int layoutRectFloats = 4;

/// `u32` words per `NveScrollOffset` (`node_id`, `x` bits, `y` bits; 12 bytes).
const int scrollOffsetWords = 3;

/// Low bits of a raw `NodeId` holding its slot index (`index: 24 | generation: 8`).
const int nodeIndexMask = 0xFFFFFF;

/// Raw `NodeId` of every surface root (index 1, generation 0).
const int rootNodeId = 1;

/// `NveNodeRow.primary_slot` of a plain container.
const int primarySlotNone = 0xFF;

/// Highest table slot a 128-bit `table_set` can address.
const int maxTableSlots = 128;

const int _parentWord = 4;
const int _childOffWord = 5;
const int _childLenWord = 6;
const int _packedWord = 7; // generation | primary_slot << 8 | effects << 16 | flags << 24
const int _flagLive = 1;

final class RuntimeSceneReader {
  /// Throws [SceneException] when any slab, child range, dirty index, or table is invalid.
  RuntimeSceneReader({
    required Key64 expectedSchema,
    required Key64 schemaHash,
    required this.frameId,
    required this.nodeCapacity,
    required Uint32List rows,
    required Float32List rects,
    required Uint8List textSlab,
    required Uint32List childrenSlab,
    required Uint32List dirtyIndices,
    required Uint32List removedIds,
    List<TableReader> tables = const <TableReader>[],
    Uint32List? scrollOffsets,
    this.sceneEpoch = 0,
    Uint32List? dirtyResources,
  })  : _rows = rows,
        dirtyResources = dirtyResources ?? Uint32List(0),
        _rects = rects,
        _scroll = scrollOffsets ?? Uint32List(0),
        _text = textSlab,
        _children = childrenSlab,
        dirtyIndices = dirtyIndices,
        removedIds = removedIds,
        tables = List<TableReader>.unmodifiable(tables),
        _bySlot = List<TableReader?>.filled(maxTableSlots, null) {
    if (schemaHash != expectedSchema) {
      throw SceneException(SceneErrorKind.schemaMismatch, 'expected $expectedSchema, got $schemaHash');
    }
    // Division, never `nodeCapacity * words`: a hostile count cannot wrap the 64-bit VM `int`.
    if (nodeCapacity < 0 ||
        rows.length ~/ nodeRowWords < nodeCapacity ||
        rects.length ~/ layoutRectFloats < nodeCapacity) {
      throw SceneException(SceneErrorKind.missingSlab,
          'rows (${rows.length} words) / rects (${rects.length} floats) cannot hold $nodeCapacity nodes');
    }
    if (!isValidUtf8(textSlab)) {
      throw const SceneException(SceneErrorKind.invalidUtf8, 'textSlab');
    }
    final childLen = childrenSlab.length;
    for (var i = 0; i < nodeCapacity; i++) {
      if (!isLive(i)) {
        continue;
      }
      final off = rows[i * nodeRowWords + _childOffWord];
      final len = rows[i * nodeRowWords + _childLenWord];
      if (off > childLen || len > childLen - off) {
        throw SceneException(SceneErrorKind.missingSlab, 'node $i children [$off, +$len) outside slab ($childLen)');
      }
    }
    for (final index in dirtyIndices) {
      if (index >= nodeCapacity) {
        throw SceneException(SceneErrorKind.missingSlab, 'dirty index $index >= capacity $nodeCapacity');
      }
    }
    for (final t in this.tables) {
      if (t.slot < 0 || t.slot >= maxTableSlots || _bySlot[t.slot] != null) {
        throw SceneException(SceneErrorKind.invalidTable, 'table ${t.key} slot ${t.slot} invalid or duplicated');
      }
      _bySlot[t.slot] = t;
    }
    if (_scroll.length % scrollOffsetWords != 0) {
      throw SceneException(SceneErrorKind.missingSlab, 'scrollOffsets has ${_scroll.length} words, not a multiple of 3');
    }
    _scrollF = Float32List.sublistView(_scroll);
    var previous = 0;
    for (var i = 0; i < _scroll.length; i += scrollOffsetWords) {
      final id = _scroll[i];
      if (id <= previous) {
        throw SceneException(SceneErrorKind.missingSlab, 'scroll offset node ids not strictly ascending at $id');
      }
      previous = id;
    }
  }

  /// Exact 64-bit frame counter.
  final Key64 frameId;

  /// Slot capacity of rows / rects (slot 0 is the reserved null node).
  final int nodeCapacity;

  /// Node indices modified on [frameId] (all `< nodeCapacity`).
  final Uint32List dirtyIndices;

  /// Raw `NodeId`s removed on [frameId].
  final Uint32List removedIds;

  /// Bumped on every runtime scene reset: node / resource ids and caches of an earlier epoch are stale.
  final int sceneEpoch;

  /// Live resource ids (re)defined on [frameId] (created, recycled, rewritten in place): caches
  /// keyed by resource id drop exactly these.
  final Uint32List dirtyResources;

  final List<TableReader> tables;

  final Uint32List _rows;
  final Float32List _rects;
  final Uint8List _text;
  final Uint32List _children;
  final Uint32List _scroll;
  late final Float32List _scrollF;
  final List<TableReader?> _bySlot;
  final Map<int, Int32List> _sparseIndex = <int, Int32List>{};

  Uint8List get textSlab => _text;

  // ---- node rows -----------------------------------------------------------------------------

  int _packed(int index) => _rows[index * nodeRowWords + _packedWord];

  bool isLive(int index) => index > 0 && index < nodeCapacity && (_packed(index) >>> 24) & _flagLive != 0;

  int generation(int index) => _packed(index) & 0xFF;
  int primarySlot(int index) => (_packed(index) >>> 8) & 0xFF;
  int effects(int index) => (_packed(index) >>> 16) & 0xFF;
  int flags(int index) => _packed(index) >>> 24;

  /// Raw parent `NodeId` (`0` = root / detached).
  int parentId(int index) => _rows[index * nodeRowWords + _parentWord];

  /// `true` when table [slot] is attached to node [index].
  bool hasTable(int index, int slot) {
    if (slot < 0 || slot >= maxTableSlots || index <= 0 || index >= nodeCapacity) {
      return false;
    }
    return (_rows[index * nodeRowWords + (slot >>> 5)] >>> (slot & 31)) & 1 != 0;
  }

  /// Raw `NodeId` of live slot [index].
  int nodeId(int index) => (generation(index) << 24) | index;

  /// Slot index of raw [nodeId], or `-1` when it is null, stale, or dead.
  int indexOf(int nodeId) {
    final index = nodeId & nodeIndexMask;
    if (!isLive(index) || generation(index) != (nodeId >>> 24) & 0xFF) {
      return -1;
    }
    return index;
  }

  // ---- layout rects --------------------------------------------------------------------------

  double x(int index) => _rects[index * layoutRectFloats];
  double y(int index) => _rects[index * layoutRectFloats + 1];
  double width(int index) => _rects[index * layoutRectFloats + 2];
  double height(int index) => _rects[index * layoutRectFloats + 3];

  // ---- scroll offsets ------------------------------------------------------------------------

  /// Number of published scroll containers (a scrolled-back container may publish a zero shift).
  int get scrollCount => _scroll.length ~/ scrollOffsetWords;

  /// Visual child shift `(x, y)` of live node [index] (children paint and hit-test at
  /// `rect - shift`); `(0, 0)` when it is not scrolled. O(log scrollCount).
  (double, double) scrollShift(int index) {
    if (!isLive(index)) {
      return (0.0, 0.0);
    }
    final id = nodeId(index);
    var lo = 0;
    var hi = scrollCount;
    while (lo < hi) {
      final mid = (lo + hi) >>> 1;
      final at = _scroll[mid * scrollOffsetWords];
      if (at == id) {
        return (_scrollF[mid * scrollOffsetWords + 1], _scrollF[mid * scrollOffsetWords + 2]);
      }
      if (at < id) {
        lo = mid + 1;
      } else {
        hi = mid;
      }
    }
    return (0.0, 0.0);
  }

  // ---- children ------------------------------------------------------------------------------

  int childCount(int index) => isLive(index) ? _rows[index * nodeRowWords + _childLenWord] : 0;

  /// Raw child `NodeId`s of [index] in paint order (zero-copy view; empty for dead nodes).
  Uint32List children(int index) {
    if (!isLive(index)) {
      return Uint32List(0);
    }
    final off = _rows[index * nodeRowWords + _childOffWord];
    return Uint32List.sublistView(_children, off, off + _rows[index * nodeRowWords + _childLenWord]);
  }

  // ---- text ----------------------------------------------------------------------------------

  /// View of `textSlab[offset ..< offset + len]`; throws [RangeError] when out of range or not on
  /// UTF-8 boundaries.
  Uint8List textBytes(int offset, int len) {
    if (offset < 0 || len < 0 || offset > _text.length || len > _text.length - offset ||
        !_isBoundary(offset) || !_isBoundary(offset + len)) {
      throw RangeError('text [$offset, +$len) outside textSlab (${_text.length}) or splits a code point');
    }
    return Uint8List.sublistView(_text, offset, offset + len);
  }

  /// Text at `[offset, offset + len)`; a leading U+FEFF is kept (never stripped).
  String text(int offset, int len) => len == 0 ? '' : decodeUtf8PreserveBom(textBytes(offset, len));

  bool _isBoundary(int at) => at == _text.length || (_text[at] & 0xC0) != 0x80;

  // ---- tables --------------------------------------------------------------------------------

  /// Published table with [key], or `null` (O(tables)).
  TableReader? findTable(Key64 key) {
    for (final t in tables) {
      if (t.key == key) {
        return t;
      }
    }
    return null;
  }

  /// Published table bound to [slot], or `null`.
  TableReader? tableAtSlot(int slot) => slot >= 0 && slot < maxTableSlots ? _bySlot[slot] : null;

  /// Row of node [index] in [table], or `-1` when the table is absent or not attached to it.
  ///
  /// Dense rows are node indices; sparse rows resolve through an owner index built once per
  /// frame on first use (O(rows)), then O(1).
  int rowOf(TableReader? table, int index) {
    if (table == null || !hasTable(index, table.slot)) {
      return -1;
    }
    if (!table.isSparse) {
      return index < table.rowCount ? index : -1;
    }
    final map = _sparseIndex[table.slot] ??= _buildSparseIndex(table);
    return index < map.length ? map[index] : -1;
  }

  Int32List _buildSparseIndex(TableReader table) {
    final map = Int32List(nodeCapacity)..fillRange(0, nodeCapacity, -1);
    for (var r = 0; r < table.rowCount; r++) {
      final index = table.owner(r) & nodeIndexMask;
      if (index < nodeCapacity) {
        map[index] = r;
      }
    }
    return map;
  }

  /// Text of `str` field [field] in [row] of [table].
  String str(TableReader table, int row, FieldSpec field) {
    final (offset, len) = table.str(row, field);
    return text(offset, len);
  }
}

/// Content versions of published resources across observed frames, for host caches keyed by
/// resource content (decoded images): a version stays stable while the resource's bytes do and is
/// never reused, so a stale decode (or a failed one) never answers for new bytes.
///
/// Folds `scene_epoch` + per-frame `dirty_resources` in: a new epoch, or a frame gap (the
/// skipped frames' dirty lists are gone), drops every version; otherwise exactly the dirty ids.
/// Zero hashing: invalidation costs O(dirty ids) per frame.
final class ResourceVersions {
  /// Shared by every instance, so versions of different sessions / painters never collide in one
  /// shared cache.
  static int _next = 0;

  final Map<int, int> _versions = <int, int>{};
  int _epoch = -1;
  Key64? _frame;

  /// Folds in one published frame (repeat calls for the same frame are no-ops).
  void observe(int sceneEpoch, Key64 frameId, Uint32List dirtyResources) {
    final last = _frame;
    if (last == frameId && sceneEpoch == _epoch) {
      return;
    }
    if (sceneEpoch != _epoch || last == null || !_follows(frameId, last)) {
      _versions.clear();
    } else {
      dirtyResources.forEach(_versions.remove);
    }
    _epoch = sceneEpoch;
    _frame = frameId;
  }

  /// Version of [resourceId]'s current bytes.
  int versionOf(int resourceId) => _versions[resourceId] ??= ++_next;

  static bool _follows(Key64 next, Key64 prev) {
    final lo = (prev.lo + 1) & 0xFFFFFFFF;
    return next.lo == lo && next.hi == (lo == 0 ? (prev.hi + 1) & 0xFFFFFFFF : prev.hi);
  }
}
