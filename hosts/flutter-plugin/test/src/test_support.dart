/// Shared test support: a synthetic [RuntimeSceneReader] builder (node rows, rects, dense/sparse
/// tables seeded from generated defaults, string fields, scroll offsets) and raster helpers.
library;

import 'dart:convert';
import 'dart:io' show ZLibCodec;
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/painting.dart';
import 'package:nve_flutter/nve_flutter.dart';

/// Shape of one generated table (`Nve*Table` constants).
final class TableShape {
  const TableShape(this.key, this.words, this.sparse, this.defaults);

  final Key64 key;
  final int words;
  final bool sparse;
  final List<int> defaults;
}

final visualShape = TableShape(VisualTable.key, VisualTable.words, !VisualTable.dense, VisualTable.defaults);
final layoutShape = TableShape(LayoutTable.key, LayoutTable.words, !LayoutTable.dense, LayoutTable.defaults);
final scrollShape = TableShape(ScrollTable.key, ScrollTable.words, !ScrollTable.dense, ScrollTable.defaults);
final textShape = TableShape(TextTable.key, TextTable.words, !TextTable.dense, TextTable.defaults);
final typographyShape =
    TableShape(TypographyTable.key, TypographyTable.words, !TypographyTable.dense, TypographyTable.defaults);
final interactionShape =
    TableShape(InteractionTable.key, InteractionTable.words, !InteractionTable.dense, InteractionTable.defaults);
final a11yShape = TableShape(
    AccessibilityTable.key, AccessibilityTable.words, !AccessibilityTable.dense, AccessibilityTable.defaults);
final transformShape =
    TableShape(TransformTable.key, TransformTable.words, !TransformTable.dense, TransformTable.defaults);
final textInputShape =
    TableShape(TextInputTable.key, TextInputTable.words, !TextInputTable.dense, TextInputTable.defaults);

final class _Table {
  _Table(this.shape, this.slot);

  final TableShape shape;
  final int slot;

  /// Node index → row words (dense tables are materialized for every node at build time).
  final Map<int, Uint32List> rows = <int, Uint32List>{};
}

/// Builds one synthetic published frame. Node indices start at 1 (root); generations are 0.
final class SceneBuilder {
  static const Key64 _schema = Key64(0x1234, 0x5678);

  final List<Rect> _rects = <Rect>[Rect.zero];
  final List<int> _parents = <int>[0];
  final List<List<int>> _children = <List<int>>[<int>[]];
  final Map<Key64, _Table> _tables = <Key64, _Table>{};
  final BytesBuilder _text = BytesBuilder();
  final List<(int, double, double)> _scroll = <(int, double, double)>[];

  /// Adds a node under [parent] (`0` = root) and returns its index (= raw `NodeId`).
  int node(Rect rect, {int parent = 0}) {
    final index = _rects.length;
    _rects.add(rect);
    _parents.add(parent);
    _children.add(<int>[]);
    if (parent != 0) {
      _children[parent].add(index);
    }
    return index;
  }

  Uint32List _row(TableShape shape, int index) {
    final t = _tables.putIfAbsent(shape.key, () => _Table(shape, _tables.length));
    return t.rows.putIfAbsent(index, () => Uint32List.fromList(shape.defaults));
  }

  /// Writes raw field bits (attaching the table to [index]).
  void bits(TableShape shape, int index, FieldSpec f, int value) {
    final row = _row(shape, index);
    final mask = f.bits >= 32 ? 0xFFFFFFFF : ((1 << f.bits) - 1) << f.shift;
    row[f.word] = (row[f.word] & ~mask & 0xFFFFFFFF) | ((value << f.shift) & mask);
  }

  void f32(TableShape shape, int index, FieldSpec f, double value) =>
      bits(shape, index, f, (ByteData(4)..setFloat32(0, value, Endian.little)).getUint32(0, Endian.little));

  /// `Length` (f32 bits with the unit tag in the low two bits: `0` points, `1` percent).
  void length(TableShape shape, int index, FieldSpec f, double value, {bool percent = false}) => bits(shape, index, f,
      ((ByteData(4)..setFloat32(0, value, Endian.little)).getUint32(0, Endian.little) & ~3) | (percent ? 1 : 0));

  void str(TableShape shape, int index, FieldSpec f, String value) {
    final bytes = utf8.encode(value);
    final row = _row(shape, index);
    row[f.word] = _text.length;
    row[f.word + 1] = bytes.length;
    _text.add(bytes);
  }

  /// Attaches [shape] to [index] with its defaults only.
  void attach(TableShape shape, int index) => _row(shape, index);

  void scrollOffset(int index, double x, double y) => _scroll.add((index, x, y));

  /// Corrupts the frame: [child] also appears in [parent]'s children slab (its own parent row is
  /// untouched), so a walk can loop.
  void extraChild(int parent, int child) => _children[parent].add(child);

  /// Corrupts the frame: [index]'s parent row points at [parent] (children slab untouched).
  void reparent(int index, int parent) => _parents[index] = parent;

  RuntimeSceneReader build({Key64 frameId = const Key64(1, 0), int sceneEpoch = 0, List<int> dirtyResources = const []}) {
    final cap = _rects.length;
    final rows = Uint32List(cap * nodeRowWords);
    final rects = Float32List(cap * layoutRectFloats);
    final kids = <int>[];
    for (var i = 1; i < cap; i++) {
      final b = i * nodeRowWords;
      rows[b + 4] = _parents[i];
      rows[b + 5] = kids.length;
      rows[b + 6] = _children[i].length;
      rows[b + 7] = (0xFF << 8) | (1 << 24);
      kids.addAll(_children[i]);
      final r = _rects[i];
      rects.setAll(i * layoutRectFloats, [r.left, r.top, r.width, r.height]);
    }
    final tables = <TableReader>[];
    for (final t in _tables.values) {
      final shape = t.shape;
      for (final i in t.rows.keys) {
        rows[i * nodeRowWords + (t.slot >> 5)] |= 1 << (t.slot & 31);
      }
      final owners = shape.sparse ? (t.rows.keys.toList()..sort()) : null;
      final count = owners?.length ?? cap;
      final words = Uint32List(count * shape.words);
      for (var r = 0; r < count; r++) {
        final index = owners?[r] ?? r;
        words.setAll(r * shape.words, t.rows[index] ?? shape.defaults);
      }
      tables.add(owners == null
          ? TableReader(key: shape.key, slot: t.slot, words: shape.words, rowCount: count, rows: ByteData.sublistView(words))
          : TableReader.sparse(
              key: shape.key, slot: t.slot, words: shape.words, rows: ByteData.sublistView(words), owners: owners));
    }
    final scroll = Uint32List(_scroll.length * 3);
    final scrollF = Float32List.sublistView(scroll);
    final sorted = [..._scroll]..sort((a, b) => a.$1.compareTo(b.$1));
    for (var i = 0; i < sorted.length; i++) {
      scroll[i * 3] = sorted[i].$1;
      scrollF[i * 3 + 1] = sorted[i].$2;
      scrollF[i * 3 + 2] = sorted[i].$3;
    }
    return RuntimeSceneReader(
      expectedSchema: _schema,
      schemaHash: _schema,
      frameId: frameId,
      sceneEpoch: sceneEpoch,
      dirtyResources: Uint32List.fromList(dirtyResources),
      nodeCapacity: cap,
      rows: rows,
      rects: rects,
      textSlab: _text.toBytes(),
      childrenSlab: Uint32List.fromList(kids),
      dirtyIndices: Uint32List(0),
      removedIds: Uint32List(0),
      tables: tables,
      scrollOffsets: scroll,
    );
  }

  NveSceneTree tree({Key64 frameId = const Key64(1, 0), int sceneEpoch = 0, List<int> dirtyResources = const []}) =>
      NveSceneTree(build(frameId: frameId, sceneEpoch: sceneEpoch, dirtyResources: dirtyResources));
}

/// Packed `r | g << 8 | b << 16 | a << 24`.
int packRgba(int r, int g, int b, [int a = 255]) => r | (g << 8) | (b << 16) | (a << 24);

/// Rasterizes [draw] into a `width × height` RGBA buffer.
Future<Uint8List> rasterize(int width, int height, void Function(Canvas canvas) draw) async {
  final recorder = ui.PictureRecorder();
  draw(Canvas(recorder, Rect.fromLTWH(0, 0, width.toDouble(), height.toDouble())));
  final picture = recorder.endRecording();
  final img = await picture.toImage(width, height);
  final bytes = await img.toByteData(format: ui.ImageByteFormat.rawRgba);
  img.dispose();
  picture.dispose();
  return bytes!.buffer.asUint8List();
}

(int, int, int, int) pixelAt(Uint8List rgba, int width, int x, int y) {
  final off = (y * width + x) * 4;
  return (rgba[off], rgba[off + 1], rgba[off + 2], rgba[off + 3]);
}

/// Minimal binary `NVE_DRAW1` encoder matching `packages/core/components/src/canvas_ir.rs`.
class DrawListEncoder {
  DrawListEncoder() {
    _bytes.add(nveDrawMagic);
    _writeU16(nveDrawVersion);
    _writeU32(0); // opCount placeholder
  }

  final BytesBuilder _bytes = BytesBuilder(copy: false);
  int _opCount = 0;

  void _writeU8(int v) => _bytes.addByte(v & 0xFF);

  void _writeU16(int v) {
    final bd = ByteData(2)..setUint16(0, v, Endian.little);
    _bytes.add(bd.buffer.asUint8List());
  }

  void _writeU32(int v) {
    final bd = ByteData(4)..setUint32(0, v, Endian.little);
    _bytes.add(bd.buffer.asUint8List());
  }

  void _writeF32(double v) {
    final bd = ByteData(4)..setFloat32(0, v, Endian.little);
    _bytes.add(bd.buffer.asUint8List());
  }

  void clear(int rgba) {
    _writeU8(NveDrawOpcode.clear);
    _writeU32(rgba);
    _opCount++;
  }

  void fillRect(double x, double y, double w, double h, int rgba) {
    _writeU8(NveDrawOpcode.fillRect);
    _writeF32(x);
    _writeF32(y);
    _writeF32(w);
    _writeF32(h);
    _writeU32(rgba);
    _opCount++;
  }

  void fillRoundedRect(double x, double y, double w, double h, double radius, int rgba) {
    _writeU8(NveDrawOpcode.fillRoundedRect);
    _writeF32(x);
    _writeF32(y);
    _writeF32(w);
    _writeF32(h);
    _writeF32(radius);
    _writeU32(rgba);
    _opCount++;
  }

  void fillCircle(double cx, double cy, double radius, int rgba) {
    _writeU8(NveDrawOpcode.fillCircle);
    _writeF32(cx);
    _writeF32(cy);
    _writeF32(radius);
    _writeU32(rgba);
    _opCount++;
  }

  void strokeLine(double x0, double y0, double x1, double y1, double width, int rgba) {
    _writeU8(NveDrawOpcode.strokeLine);
    _writeF32(x0);
    _writeF32(y0);
    _writeF32(x1);
    _writeF32(y1);
    _writeF32(width);
    _writeU32(rgba);
    _opCount++;
  }

  void strokePolyline(List<(double, double)> points, double width, int rgba) {
    _writeU8(NveDrawOpcode.strokePolyline);
    _writeF32(width);
    _writeU32(rgba);
    _writeU32(points.length);
    for (final (px, py) in points) {
      _writeF32(px);
      _writeF32(py);
    }
    _opCount++;
  }

  void clipRect(double x, double y, double w, double h) {
    _writeU8(NveDrawOpcode.clipRect);
    _writeF32(x);
    _writeF32(y);
    _writeF32(w);
    _writeF32(h);
    _opCount++;
  }

  void restoreClip() {
    _writeU8(NveDrawOpcode.restoreClip);
    _opCount++;
  }

  Uint8List finish() {
    final out = _bytes.toBytes();
    ByteData.sublistView(out).setUint32(11, _opCount, Endian.little);
    return out;
  }
}

/// Solid-color uncompressed PNG (`zlib`-wrapped stored IDAT) of `width × height`.
Uint8List makeSolidPngBytes(int r, int g, int b, {int width = 2, int height = 2}) {
  final raw = BytesBuilder(copy: false);
  for (var y = 0; y < height; y++) {
    raw.addByte(0); // filter type None
    for (var x = 0; x < width; x++) {
      raw.addByte(r & 0xFF);
      raw.addByte(g & 0xFF);
      raw.addByte(b & 0xFF);
      raw.addByte(255);
    }
  }
  final idatPayload = Uint8List.fromList(ZLibCodec(level: 0).encode(raw.toBytes()));

  int crc32(List<int> tag, List<int> data) {
    var crc = 0xFFFFFFFF;
    for (final byte in <int>[...tag, ...data]) {
      crc ^= byte & 0xFF;
      for (var k = 0; k < 8; k++) {
        crc = (crc & 1) != 0 ? (0xEDB88320 ^ (crc >>> 1)) : (crc >>> 1);
      }
    }
    return (~crc) & 0xFFFFFFFF;
  }

  final out = BytesBuilder(copy: false);
  out.add(<int>[137, 80, 78, 71, 13, 10, 26, 10]);

  void writeChunk(String name, Uint8List payload) {
    final tag = ascii.encode(name);
    final lenBd = ByteData(4)..setUint32(0, payload.length, Endian.big);
    out.add(lenBd.buffer.asUint8List());
    out.add(tag);
    out.add(payload);
    final crcBd = ByteData(4)..setUint32(0, crc32(tag, payload), Endian.big);
    out.add(crcBd.buffer.asUint8List());
  }

  final ihdr = ByteData(13)
    ..setUint32(0, width, Endian.big)
    ..setUint32(4, height, Endian.big)
    ..setUint8(8, 8) // 8-bit depth
    ..setUint8(9, 6) // RGBA
    ..setUint8(10, 0)
    ..setUint8(11, 0)
    ..setUint8(12, 0);
  writeChunk('IHDR', ihdr.buffer.asUint8List());
  writeChunk('IDAT', idatPayload);
  writeChunk('IEND', Uint8List(0));
  return out.toBytes();
}
