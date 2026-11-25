import 'dart:convert' show utf8;
import 'dart:typed_data';

import 'command_types.dart';

/// Binary `CommandBuffer` encoder mirroring `CommandBufferBuilder` in `packages/core/abi/src/codec.rs`.
///
/// Useful for tests, synthetic surfaces, and Dart-side golden fixtures.
class NveCommandBufferBuilder {
  NveCommandBufferBuilder({this.surfaceId = 1, this.frameId = 1, this.flags = 0});

  final int surfaceId;
  final int frameId;
  final int flags;
  final BytesBuilder _body = BytesBuilder(copy: false);
  int _opCount = 0;

  int get opCount => _opCount;

  void createNode(int nodeId, NveComponentKind kind, {String? customName}) {
    final nameBytes = customName != null ? utf8.encode(customName) : const <int>[];
    final hdr = ByteData(9)
      ..setUint8(0, NveOpcode.createNode.code)
      ..setUint32(1, nodeId, Endian.little)
      ..setUint16(5, kind.code, Endian.little)
      ..setUint16(7, nameBytes.length, Endian.little);
    _body.add(hdr.buffer.asUint8List());
    if (nameBytes.isNotEmpty) {
      _body.add(nameBytes);
    }
    _opCount += 1;
  }

  void createCustomNode(int nodeId, String customName) {
    createNode(nodeId, NveComponentKind.custom, customName: customName);
  }

  void removeNode(int nodeId) {
    final b = ByteData(5)
      ..setUint8(0, NveOpcode.removeNode.code)
      ..setUint32(1, nodeId, Endian.little);
    _body.add(b.buffer.asUint8List());
    _opCount += 1;
  }

  void setChildren(int parentId, List<int> children) {
    final b = ByteData(9 + children.length * 4)
      ..setUint8(0, NveOpcode.setChildren.code)
      ..setUint32(1, parentId, Endian.little)
      ..setUint32(5, children.length, Endian.little);
    for (var i = 0; i < children.length; i++) {
      b.setUint32(9 + i * 4, children[i], Endian.little);
    }
    _body.add(b.buffer.asUint8List());
    _opCount += 1;
  }

  void setText(int nodeId, String text) {
    final encoded = utf8.encode(text);
    final b = ByteData(9)
      ..setUint8(0, NveOpcode.setText.code)
      ..setUint32(1, nodeId, Endian.little)
      ..setUint32(5, encoded.length, Endian.little);
    _body.add(b.buffer.asUint8List());
    _body.add(encoded);
    _opCount += 1;
  }

  void setLayout(int nodeId, NveLayoutProps layout) {
    final b = ByteData(82);
    var pos = 0;
    b.setUint8(pos++, NveOpcode.setLayout.code);
    b.setUint32(pos, nodeId, Endian.little);
    pos += 4;
    void writeDim(NveDimension d) {
      b.setUint8(pos++, d.unit.code);
      b.setFloat32(pos, d.value, Endian.little);
      pos += 4;
    }

    writeDim(layout.width);
    writeDim(layout.height);
    writeDim(layout.minWidth);
    writeDim(layout.minHeight);
    writeDim(layout.maxWidth);
    writeDim(layout.maxHeight);
    b.setUint8(pos++, layout.flexDirection.code);
    b.setUint8(pos++, layout.justifyContent.code);
    b.setUint8(pos++, layout.alignItems.code);
    b.setFloat32(pos, layout.flexGrow, Endian.little);
    pos += 4;
    b.setFloat32(pos, layout.flexShrink, Endian.little);
    pos += 4;
    b.setFloat32(pos, layout.gap, Endian.little);
    pos += 4;
    void writeInsets(NveEdgeInsets e) {
      b.setFloat32(pos, e.top, Endian.little);
      b.setFloat32(pos + 4, e.right, Endian.little);
      b.setFloat32(pos + 8, e.bottom, Endian.little);
      b.setFloat32(pos + 12, e.left, Endian.little);
      pos += 16;
    }

    writeInsets(layout.padding);
    writeInsets(layout.margin);
    _body.add(b.buffer.asUint8List());
    _opCount += 1;
  }

  void setStyle(int nodeId, NveStyleProps style) {
    final b = ByteData(37)
      ..setUint8(0, NveOpcode.setStyle.code)
      ..setUint32(1, nodeId, Endian.little)
      ..setUint32(5, style.background.toWireUint32(), Endian.little)
      ..setUint32(9, style.textColor.toWireUint32(), Endian.little)
      ..setUint32(13, style.borderColor.toWireUint32(), Endian.little)
      ..setFloat32(17, style.borderWidth, Endian.little)
      ..setFloat32(21, style.borderRadius, Endian.little)
      ..setFloat32(25, style.opacity, Endian.little)
      ..setFloat32(29, style.fontSize, Endian.little)
      ..setInt32(33, style.zIndex, Endian.little);
    _body.add(b.buffer.asUint8List());
    _opCount += 1;
  }

  void setRawProp(int nodeId, String key, NveRawPropValue value) {
    final keyBytes = utf8.encode(key);
    final hdr = ByteData(7)
      ..setUint8(0, NveOpcode.setRawProp.code)
      ..setUint32(1, nodeId, Endian.little)
      ..setUint16(5, keyBytes.length, Endian.little);
    _body.add(hdr.buffer.asUint8List());
    _body.add(keyBytes);
    _body.addByte(value.kind.code);

    switch (value.kind) {
      case NveRawPropKind.nullValue:
        break;
      case NveRawPropKind.boolValue:
        _body.addByte((value.asBool ?? false) ? 1 : 0);
      case NveRawPropKind.i64Value:
        final b = ByteData(8)..setInt64(0, value.asInt ?? 0, Endian.little);
        _body.add(b.buffer.asUint8List());
      case NveRawPropKind.f64Value:
        final b = ByteData(8)..setFloat64(0, value.asDouble ?? 0.0, Endian.little);
        _body.add(b.buffer.asUint8List());
      case NveRawPropKind.stringValue:
        final bytes = utf8.encode(value.asString ?? '');
        final len = ByteData(4)..setUint32(0, bytes.length, Endian.little);
        _body.add(len.buffer.asUint8List());
        _body.add(bytes);
      case NveRawPropKind.bytesValue:
        final bytes = value.asBytes ?? Uint8List(0);
        final len = ByteData(4)..setUint32(0, bytes.length, Endian.little);
        _body.add(len.buffer.asUint8List());
        _body.add(bytes);
    }
    _opCount += 1;
  }

  void setEventMask(int nodeId, int mask) {
    final b = ByteData(13)
      ..setUint8(0, NveOpcode.setEventMask.code)
      ..setUint32(1, nodeId, Endian.little)
      ..setUint64(5, mask, Endian.little);
    _body.add(b.buffer.asUint8List());
    _opCount += 1;
  }

  void dispatchCommand(int nodeId, String commandName, Uint8List payload) {
    final nameBytes = utf8.encode(commandName);
    final hdr = ByteData(7)
      ..setUint8(0, NveOpcode.dispatchCommand.code)
      ..setUint32(1, nodeId, Endian.little)
      ..setUint16(5, nameBytes.length, Endian.little);
    _body.add(hdr.buffer.asUint8List());
    _body.add(nameBytes);
    final len = ByteData(4)..setUint32(0, payload.length, Endian.little);
    _body.add(len.buffer.asUint8List());
    _body.add(payload);
    _opCount += 1;
  }

  void commit({bool syncMount = false}) {
    _body.add(<int>[NveOpcode.commit.code, syncMount ? 1 : 0]);
    _opCount += 1;
  }

  /// Finalizes the 24-byte header + opcode stream into a contiguous `Uint8List`.
  Uint8List toBytes() {
    final bodyBytes = _body.toBytes();
    final out = Uint8List(nveHeaderSize + bodyBytes.length);
    final bd = ByteData.sublistView(out);
    bd
      ..setUint32(0, nveAbiMagic, Endian.little)
      ..setUint16(4, nveAbiVersion, Endian.little)
      ..setUint16(6, flags, Endian.little)
      ..setUint32(8, surfaceId, Endian.little)
      ..setUint64(12, frameId, Endian.little)
      ..setUint32(20, _opCount, Endian.little);
    out.setRange(nveHeaderSize, out.length, bodyBytes);
    return out;
  }
}
