import 'dart:convert' show utf8;
import 'dart:typed_data';
import 'dart:ui' show Rect;

import 'command_types.dart';

export 'command_builder.dart';
export 'command_types.dart';

sealed class _DecodedCmd {
  const _DecodedCmd();
}

final class _CreateNodeCmd extends _DecodedCmd {
  const _CreateNodeCmd(this.nodeId, this.kind, this.customName);
  final int nodeId;
  final NveComponentKind kind;
  final String? customName;
}

final class _RemoveNodeCmd extends _DecodedCmd {
  const _RemoveNodeCmd(this.nodeId);
  final int nodeId;
}

final class _SetChildrenCmd extends _DecodedCmd {
  const _SetChildrenCmd(this.parentId, this.children);
  final int parentId;
  final List<int> children;
}

final class _SetTextCmd extends _DecodedCmd {
  const _SetTextCmd(this.nodeId, this.text);
  final int nodeId;
  final String text;
}

final class _SetLayoutCmd extends _DecodedCmd {
  const _SetLayoutCmd(this.nodeId, this.layout);
  final int nodeId;
  final NveLayoutProps layout;
}

final class _SetStyleCmd extends _DecodedCmd {
  const _SetStyleCmd(this.nodeId, this.style);
  final int nodeId;
  final NveStyleProps style;
}

final class _SetRawPropCmd extends _DecodedCmd {
  const _SetRawPropCmd(this.nodeId, this.key, this.value);
  final int nodeId;
  final String key;
  final NveRawPropValue value;
}

final class _SetEventMaskCmd extends _DecodedCmd {
  const _SetEventMaskCmd(this.nodeId, this.mask);
  final int nodeId;
  final int mask;
}

final class _DispatchCommandCmd extends _DecodedCmd {
  const _DispatchCommandCmd(this.nodeId, this.commandName, this.payload);
  final int nodeId;
  final String commandName;
  final Uint8List payload;
}

final class _CommitCmd extends _DecodedCmd {
  const _CommitCmd(this.syncMount);
  final bool syncMount;
}

bool _isValidScalar(double v) => v.isFinite && v.abs() <= nveMaxLayoutScalar;

bool _isValidDimension(NveDimension d) =>
    d.isAuto || ((d.isPoints || d.isPercent) && _isValidScalar(d.value));

bool _isValidInsets(NveEdgeInsets e) =>
    _isValidScalar(e.top) &&
    _isValidScalar(e.right) &&
    _isValidScalar(e.bottom) &&
    _isValidScalar(e.left);

bool _isValidLayout(NveLayoutProps l) =>
    _isValidDimension(l.width) &&
    _isValidDimension(l.height) &&
    _isValidDimension(l.minWidth) &&
    _isValidDimension(l.minHeight) &&
    _isValidDimension(l.maxWidth) &&
    _isValidDimension(l.maxHeight) &&
    _isValidScalar(l.flexGrow) &&
    l.flexGrow >= 0.0 &&
    _isValidScalar(l.flexShrink) &&
    l.flexShrink >= 0.0 &&
    _isValidScalar(l.gap) &&
    l.gap >= 0.0 &&
    _isValidInsets(l.padding) &&
    _isValidInsets(l.margin);

bool _isValidStyle(NveStyleProps s) =>
    _isValidScalar(s.borderWidth) &&
    _isValidScalar(s.borderRadius) &&
    _isValidScalar(s.opacity) &&
    _isValidScalar(s.fontSize);

/// Zero-copy bounds-checked binary reader over `ByteData.sublistView(Uint8List)`.
class _ZeroCopyReader {
  _ZeroCopyReader(this.bytes) : bd = ByteData.sublistView(bytes);

  final Uint8List bytes;
  final ByteData bd;
  int pos = 0;

  int get remaining => bytes.length - pos;

  bool has(int count) => count >= 0 && count <= bytes.length - pos;

  int readU8() {
    final v = bytes[pos];
    pos += 1;
    return v;
  }

  int readU16() {
    final v = bd.getUint16(pos, Endian.little);
    pos += 2;
    return v;
  }

  int readU32() {
    final v = bd.getUint32(pos, Endian.little);
    pos += 4;
    return v;
  }

  int readI32() {
    final v = bd.getInt32(pos, Endian.little);
    pos += 4;
    return v;
  }

  int readU64() {
    final v = bd.getUint64(pos, Endian.little);
    pos += 8;
    return v;
  }

  int readI64() {
    final v = bd.getInt64(pos, Endian.little);
    pos += 8;
    return v;
  }

  double readF32() {
    final v = bd.getFloat32(pos, Endian.little);
    pos += 4;
    return v;
  }

  double readF64() {
    final v = bd.getFloat64(pos, Endian.little);
    pos += 8;
    return v;
  }

  Uint8List readBytesCopy(int len) {
    final out = Uint8List.fromList(Uint8List.sublistView(bytes, pos, pos + len));
    pos += len;
    return out;
  }

  String? readUtf8(int len) {
    final slice = Uint8List.sublistView(bytes, pos, pos + len);
    pos += len;
    try {
      return utf8.decode(slice, allowMalformed: false);
    } on FormatException {
      return null;
    }
  }

  NveDimension? readDimension() {
    if (!has(5)) return null;
    final tag = readU8();
    final value = readF32();
    final unit = NveDimensionUnit.fromCode(tag);
    if (unit == null) return null;
    return NveDimension(unit, value);
  }

  NveEdgeInsets readInsets() {
    final top = readF32();
    final right = readF32();
    final bottom = readF32();
    final left = readF32();
    return NveEdgeInsets(top: top, right: right, bottom: bottom, left: left);
  }
}

/// Retained NVE node tree populated directly from binary `CommandBuffer` frames.
class NveRetainedTree {
  NveRetainedTree() {
    reset();
  }

  final Map<int, NveNode> _nodes = <int, NveNode>{};

  Map<int, NveNode> get nodes => Map<int, NveNode>.unmodifiable(_nodes);

  int get nodeCount => _nodes.length;

  NveNode? node(int nodeId) => _nodes[nodeId];

  NveNode get root => _nodes[nveRootNodeId]!;

  Rect? layoutOf(int nodeId) {
    final n = _nodes[nodeId];
    if (n == null || !n.hasComputedLayout) return null;
    return n.computedRect;
  }

  void reset() {
    _nodes
      ..clear()
      ..[nveRootNodeId] = NveNode(id: nveRootNodeId, kind: NveComponentKind.root);
  }

  /// Decodes and atomically applies a binary `CommandBuffer` frame without throwing.
  NveFrameApplyResult applyCommandBuffer(Uint8List bytes) {
    if (bytes.length > nveMaxBufferBytes) {
      return NveFrameApplyResult.err(
        NveDecodeError(
          NveDecodeErrorCode.bufferTooLarge,
          'Buffer size ${bytes.length} exceeds limit $nveMaxBufferBytes',
        ),
      );
    }
    if (bytes.length < nveHeaderSize) {
      return NveFrameApplyResult.err(
        NveDecodeError(
          NveDecodeErrorCode.truncated,
          'Buffer length ${bytes.length} is shorter than $nveHeaderSize-byte header',
        ),
      );
    }

    final reader = _ZeroCopyReader(bytes);
    final magic = reader.readU32();
    if (magic != nveAbiMagic) {
      return NveFrameApplyResult.err(
        NveDecodeError(
          NveDecodeErrorCode.invalidMagic,
          'Invalid magic 0x${magic.toRadixString(16)} (expected 0x${nveAbiMagic.toRadixString(16)})',
        ),
      );
    }
    final version = reader.readU16();
    if (version != nveAbiVersion) {
      return NveFrameApplyResult.err(
        NveDecodeError(
          NveDecodeErrorCode.unsupportedVersion,
          'Unsupported ABI version $version (expected $nveAbiVersion)',
          offset: 4,
        ),
      );
    }
    final flags = reader.readU16();
    final surfaceId = reader.readU32();
    final frameId = reader.readU64();
    final opCount = reader.readU32();

    final header = NveCommandBufferHeader(
      magic: magic,
      version: version,
      flags: flags,
      surfaceId: surfaceId,
      frameId: frameId,
      opCount: opCount,
    );

    if (opCount > nveMaxOpCount || opCount > reader.remaining) {
      return NveFrameApplyResult.err(
        NveDecodeError(
          opCount > nveMaxOpCount
              ? NveDecodeErrorCode.bufferTooLarge
              : NveDecodeErrorCode.truncated,
          'Invalid opCount $opCount for ${reader.remaining} remaining bytes',
          offset: 20,
        ),
        header: header,
      );
    }

    final decoded = <_DecodedCmd>[];
    final decodeErr = _decodeStream(reader, opCount, decoded);
    if (decodeErr != null) {
      return NveFrameApplyResult.err(decodeErr, header: header);
    }

    if (reader.remaining != 0) {
      return NveFrameApplyResult.err(
        NveDecodeError(
          NveDecodeErrorCode.trailingBytes,
          '${reader.remaining} unexpected trailing bytes',
          offset: reader.pos,
        ),
        header: header,
      );
    }

    return _applyCommands(header, decoded);
  }

  NveDecodeError? _decodeStream(
    _ZeroCopyReader reader,
    int opCount,
    List<_DecodedCmd> decoded,
  ) {
    for (var i = 0; i < opCount; i++) {
      final opOffset = reader.pos;
      if (!reader.has(1)) {
        return NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated before opcode #$i', offset: opOffset);
      }
      final opByte = reader.readU8();
      final opcode = NveOpcode.fromCode(opByte);
      if (opcode == null) {
        return NveDecodeError(NveDecodeErrorCode.unknownOpcode, 'Unknown opcode $opByte', offset: opOffset);
      }

      switch (opcode) {
        case NveOpcode.createNode:
          if (!reader.has(8)) {
            return NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated CreateNode', offset: opOffset);
          }
          final nodeId = reader.readU32();
          final kindCode = reader.readU16();
          final kind = NveComponentKind.fromCode(kindCode);
          if (kind == null) {
            return NveDecodeError(NveDecodeErrorCode.invalidEnum, 'Invalid ComponentKind $kindCode', offset: opOffset);
          }
          final nameLen = reader.readU16();
          if (!reader.has(nameLen)) {
            return NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated custom name', offset: reader.pos);
          }
          String? customName;
          if (nameLen > 0) {
            customName = reader.readUtf8(nameLen);
            if (customName == null) {
              return NveDecodeError(NveDecodeErrorCode.invalidUtf8, 'Invalid UTF-8 in custom name', offset: opOffset);
            }
          }
          decoded.add(_CreateNodeCmd(nodeId, kind, customName));

        case NveOpcode.removeNode:
          if (!reader.has(4)) {
            return NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated RemoveNode', offset: opOffset);
          }
          decoded.add(_RemoveNodeCmd(reader.readU32()));

        case NveOpcode.setChildren:
          if (!reader.has(8)) {
            return NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated SetChildren', offset: opOffset);
          }
          final parentId = reader.readU32();
          final count = reader.readU32();
          if (count > reader.remaining ~/ 4) {
            return NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated SetChildren list', offset: reader.pos);
          }
          decoded.add(_SetChildrenCmd(parentId, List<int>.generate(count, (_) => reader.readU32(), growable: false)));

        case NveOpcode.setText:
          if (!reader.has(8)) {
            return NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated SetText', offset: opOffset);
          }
          final nodeId = reader.readU32();
          final len = reader.readU32();
          if (!reader.has(len)) {
            return NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated SetText payload', offset: reader.pos);
          }
          final text = reader.readUtf8(len);
          if (text == null) {
            return NveDecodeError(NveDecodeErrorCode.invalidUtf8, 'Invalid UTF-8 in SetText', offset: opOffset);
          }
          decoded.add(_SetTextCmd(nodeId, text));

        case NveOpcode.setLayout:
          final err = _decodeSetLayout(reader, opOffset, decoded);
          if (err != null) return err;

        case NveOpcode.setStyle:
          if (!reader.has(36)) {
            return NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated SetStyle', offset: opOffset);
          }
          final nodeId = reader.readU32();
          final style = NveStyleProps(
            background: NveColorRgba.fromWireUint32(reader.readU32()),
            textColor: NveColorRgba.fromWireUint32(reader.readU32()),
            borderColor: NveColorRgba.fromWireUint32(reader.readU32()),
            borderWidth: reader.readF32(),
            borderRadius: reader.readF32(),
            opacity: reader.readF32(),
            fontSize: reader.readF32(),
            zIndex: reader.readI32(),
          );
          if (!_isValidStyle(style)) {
            return NveDecodeError(NveDecodeErrorCode.invalidScalar, 'Invalid scalar in SetStyle', offset: opOffset);
          }
          decoded.add(_SetStyleCmd(nodeId, style));

        case NveOpcode.setRawProp:
          final err = _decodeSetRawProp(reader, opOffset, decoded);
          if (err != null) return err;

        case NveOpcode.setEventMask:
          if (!reader.has(12)) {
            return NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated SetEventMask', offset: opOffset);
          }
          decoded.add(_SetEventMaskCmd(reader.readU32(), reader.readU64()));

        case NveOpcode.dispatchCommand:
          if (!reader.has(6)) {
            return NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated DispatchCommand', offset: opOffset);
          }
          final nodeId = reader.readU32();
          final nameLen = reader.readU16();
          if (!reader.has(nameLen + 4)) {
            return NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated DispatchCommand name/len', offset: reader.pos);
          }
          final name = reader.readUtf8(nameLen);
          if (name == null) {
            return NveDecodeError(NveDecodeErrorCode.invalidUtf8, 'Invalid UTF-8 in command name', offset: opOffset);
          }
          final payloadLen = reader.readU32();
          if (!reader.has(payloadLen)) {
            return NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated DispatchCommand payload', offset: reader.pos);
          }
          decoded.add(_DispatchCommandCmd(nodeId, name, reader.readBytesCopy(payloadLen)));

        case NveOpcode.commit:
          if (!reader.has(1)) {
            return NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated Commit', offset: opOffset);
          }
          decoded.add(_CommitCmd(reader.readU8() != 0));
      }
    }
    return null;
  }

  NveDecodeError? _decodeSetLayout(
    _ZeroCopyReader reader,
    int opOffset,
    List<_DecodedCmd> decoded,
  ) {
    if (!reader.has(81)) {
      return NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated SetLayout', offset: opOffset);
    }
    final nodeId = reader.readU32();
    final width = reader.readDimension();
    final height = reader.readDimension();
    final minWidth = reader.readDimension();
    final minHeight = reader.readDimension();
    final maxWidth = reader.readDimension();
    final maxHeight = reader.readDimension();
    if (width == null ||
        height == null ||
        minWidth == null ||
        minHeight == null ||
        maxWidth == null ||
        maxHeight == null) {
      return NveDecodeError(NveDecodeErrorCode.invalidEnum, 'Invalid Dimension unit in SetLayout', offset: opOffset);
    }
    final flexDir = NveFlexDirection.fromCode(reader.readU8());
    final justify = NveFlexJustify.fromCode(reader.readU8());
    final align = NveFlexAlign.fromCode(reader.readU8());
    if (flexDir == null || justify == null || align == null) {
      return NveDecodeError(NveDecodeErrorCode.invalidEnum, 'Invalid Flexbox enum in SetLayout', offset: opOffset);
    }
    final layout = NveLayoutProps(
      width: width,
      height: height,
      minWidth: minWidth,
      minHeight: minHeight,
      maxWidth: maxWidth,
      maxHeight: maxHeight,
      flexDirection: flexDir,
      justifyContent: justify,
      alignItems: align,
      flexGrow: reader.readF32(),
      flexShrink: reader.readF32(),
      gap: reader.readF32(),
      padding: reader.readInsets(),
      margin: reader.readInsets(),
    );
    if (!_isValidLayout(layout)) {
      return NveDecodeError(NveDecodeErrorCode.invalidScalar, 'Invalid scalar in SetLayout', offset: opOffset);
    }
    decoded.add(_SetLayoutCmd(nodeId, layout));
    return null;
  }

  NveDecodeError? _decodeSetRawProp(
    _ZeroCopyReader reader,
    int opOffset,
    List<_DecodedCmd> decoded,
  ) {
    if (!reader.has(6)) {
      return NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated SetRawProp', offset: opOffset);
    }
    final nodeId = reader.readU32();
    final keyLen = reader.readU16();
    if (!reader.has(keyLen + 1)) {
      return NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated SetRawProp key/tag', offset: reader.pos);
    }
    final key = reader.readUtf8(keyLen);
    if (key == null) {
      return NveDecodeError(NveDecodeErrorCode.invalidUtf8, 'Invalid UTF-8 in SetRawProp key', offset: opOffset);
    }
    final tag = reader.readU8();
    final valKind = NveRawPropKind.fromCode(tag);
    if (valKind == null) {
      return NveDecodeError(NveDecodeErrorCode.invalidEnum, 'Invalid RawPropValue tag $tag', offset: reader.pos - 1);
    }
    final NveRawPropValue propValue;
    switch (valKind) {
      case NveRawPropKind.nullValue:
        propValue = const NveRawPropValue.nullValue();
      case NveRawPropKind.boolValue:
        if (!reader.has(1)) {
          return NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated Bool prop', offset: reader.pos);
        }
        propValue = NveRawPropValue.boolValue(reader.readU8() != 0);
      case NveRawPropKind.i64Value:
        if (!reader.has(8)) {
          return NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated I64 prop', offset: reader.pos);
        }
        propValue = NveRawPropValue.i64Value(reader.readI64());
      case NveRawPropKind.f64Value:
        if (!reader.has(8)) {
          return NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated F64 prop', offset: reader.pos);
        }
        propValue = NveRawPropValue.f64Value(reader.readF64());
      case NveRawPropKind.stringValue:
        if (!reader.has(4)) {
          return NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated String prop length', offset: reader.pos);
        }
        final vlen = reader.readU32();
        if (!reader.has(vlen)) {
          return NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated String prop bytes', offset: reader.pos);
        }
        final str = reader.readUtf8(vlen);
        if (str == null) {
          return NveDecodeError(NveDecodeErrorCode.invalidUtf8, 'Invalid UTF-8 in String prop', offset: opOffset);
        }
        propValue = NveRawPropValue.stringValue(str);
      case NveRawPropKind.bytesValue:
        if (!reader.has(4)) {
          return NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated Bytes prop length', offset: reader.pos);
        }
        final vlen = reader.readU32();
        if (!reader.has(vlen)) {
          return NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated Bytes prop payload', offset: reader.pos);
        }
        propValue = NveRawPropValue.bytesValue(reader.readBytesCopy(vlen));
    }
    decoded.add(_SetRawPropCmd(nodeId, key, propValue));
    return null;
  }

  NveFrameApplyResult _applyCommands(NveCommandBufferHeader header, List<_DecodedCmd> decoded) {
    var mutations = 0;
    var committed = false;
    var syncMount = false;
    final dispatched = <NveDispatchedCommand>[];

    for (final cmd in decoded) {
      switch (cmd) {
        case _CreateNodeCmd(:final nodeId, :final kind, :final customName):
          _destroyNodeIfExists(nodeId);
          _nodes[nodeId] = NveNode(id: nodeId, kind: kind, customName: customName);
          mutations += 1;

        case _RemoveNodeCmd(:final nodeId):
          if (nodeId == nveRootNodeId) {
            return NveFrameApplyResult.err(
              const NveDecodeError(
                NveDecodeErrorCode.cannotRemoveRoot,
                'Cannot remove reserved root node (id=1)',
              ),
              header: header,
            );
          }
          _destroyNodeIfExists(nodeId);
          mutations += 1;

        case _SetChildrenCmd(:final parentId, :final children):
          final err = _applySetChildren(parentId, children);
          if (err != null) {
            return NveFrameApplyResult.err(err, header: header);
          }
          mutations += 1;

        case _SetTextCmd(:final nodeId, :final text):
          final target = _nodes[nodeId];
          if (target == null) {
            return NveFrameApplyResult.err(
              NveDecodeError(NveDecodeErrorCode.unknownNode, 'SetText on unknown node $nodeId'),
              header: header,
            );
          }
          target.text = text;
          mutations += 1;

        case _SetLayoutCmd(:final nodeId, :final layout):
          final target = _nodes[nodeId];
          if (target == null) {
            return NveFrameApplyResult.err(
              NveDecodeError(NveDecodeErrorCode.unknownNode, 'SetLayout on unknown node $nodeId'),
              header: header,
            );
          }
          target.layout = layout;
          mutations += 1;

        case _SetStyleCmd(:final nodeId, :final style):
          final target = _nodes[nodeId];
          if (target == null) {
            return NveFrameApplyResult.err(
              NveDecodeError(NveDecodeErrorCode.unknownNode, 'SetStyle on unknown node $nodeId'),
              header: header,
            );
          }
          target.style = style;
          target.hasExplicitStyle = true;
          mutations += 1;

        case _SetRawPropCmd(:final nodeId, :final key, :final value):
          final target = _nodes[nodeId];
          if (target == null) {
            return NveFrameApplyResult.err(
              NveDecodeError(NveDecodeErrorCode.unknownNode, 'SetRawProp on unknown node $nodeId'),
              header: header,
            );
          }
          target.rawProps[key] = value;
          mutations += 1;

        case _SetEventMaskCmd(:final nodeId, :final mask):
          final target = _nodes[nodeId];
          if (target == null) {
            return NveFrameApplyResult.err(
              NveDecodeError(NveDecodeErrorCode.unknownNode, 'SetEventMask on unknown node $nodeId'),
              header: header,
            );
          }
          target.eventMask = mask;
          mutations += 1;

        case _DispatchCommandCmd(:final nodeId, :final commandName, :final payload):
          if (!_nodes.containsKey(nodeId)) {
            return NveFrameApplyResult.err(
              NveDecodeError(
                NveDecodeErrorCode.unknownNode,
                'DispatchCommand on unknown node $nodeId',
              ),
              header: header,
            );
          }
          dispatched.add(
            NveDispatchedCommand(nodeId: nodeId, commandName: commandName, payload: payload),
          );

        case _CommitCmd(syncMount: final syncFlag):
          committed = true;
          syncMount = syncFlag;
      }
    }

    if (committed) {
      propagateTextStyles();
    }

    return NveFrameApplyResult.ok(
      header: header,
      mutationCount: mutations,
      committed: committed,
      syncMount: syncMount,
      dispatchedCommands: dispatched,
    );
  }

  /// Propagates `textColor` and `fontSize` from parent `Text` nodes to child
  /// `Text` nodes that lack an explicit `SetStyle` (matching `fabric_adapter.cpp`).
  void propagateTextStyles() {
    if (!_nodes.containsKey(nveRootNodeId)) return;

    final visited = <int>{};
    final stack = <_TextInheritFrame>[
      const _TextInheritFrame(
        nodeId: nveRootNodeId,
        color: NveColorRgba.black,
        fontSize: 16.0,
        hasInherited: false,
      ),
    ];

    while (stack.isNotEmpty) {
      final frame = stack.removeLast();
      if (!visited.add(frame.nodeId)) continue;
      final entry = _nodes[frame.nodeId];
      if (entry == null) continue;

      var nextColor = frame.color;
      var nextFontSize = frame.fontSize;
      var nextHasInherited = frame.hasInherited;

      if (entry.kind == NveComponentKind.text) {
        if (!entry.hasExplicitStyle) {
          final targetColor = frame.hasInherited ? frame.color : NveColorRgba.black;
          final targetFontSize = frame.hasInherited ? frame.fontSize : 16.0;
          if (entry.style.textColor != targetColor ||
              (entry.style.fontSize - targetFontSize).abs() > 1e-3) {
            entry.style = entry.style.copyWith(
              textColor: targetColor,
              fontSize: targetFontSize,
            );
          }
        }
        nextColor = entry.style.textColor;
        nextFontSize = entry.style.fontSize;
        nextHasInherited = true;
      }

      for (var i = entry.children.length - 1; i >= 0; i--) {
        stack.add(
          _TextInheritFrame(
            nodeId: entry.children[i],
            color: nextColor,
            fontSize: nextFontSize,
            hasInherited: nextHasInherited,
          ),
        );
      }
    }
  }

  void _destroyNodeIfExists(int id) {
    final existing = _nodes[id];
    if (existing == null) return;
    final prevParentId = existing.parentId;
    if (prevParentId != null) {
      _nodes[prevParentId]?.children.remove(id);
    }
    for (final cid in existing.children) {
      final child = _nodes[cid];
      if (child != null && child.parentId == id) {
        child.parentId = null;
      }
    }
    _nodes.remove(id);
  }

  NveDecodeError? _applySetChildren(int parentId, List<int> children) {
    final parent = _nodes[parentId];
    if (parent == null) {
      return NveDecodeError(
        NveDecodeErrorCode.unknownNode,
        'SetChildren on unknown parent node $parentId',
      );
    }

    final ancestors = <int>{};
    int? walk = parentId;
    for (var hops = 0; walk != null && hops <= _nodes.length; hops++) {
      if (!ancestors.add(walk)) break;
      walk = _nodes[walk]?.parentId;
    }

    final seen = <int>{};
    for (final cid in children) {
      if (cid == parentId || cid == nveRootNodeId) {
        return NveDecodeError(
          NveDecodeErrorCode.cycleOrDuplicateChild,
          'Invalid child $cid under parent $parentId',
        );
      }
      if (ancestors.contains(cid)) {
        return NveDecodeError(
          NveDecodeErrorCode.cycleOrDuplicateChild,
          'Cycle detected: child $cid is an ancestor of $parentId',
        );
      }
      if (!seen.add(cid)) {
        return NveDecodeError(
          NveDecodeErrorCode.cycleOrDuplicateChild,
          'Duplicate child $cid under parent $parentId',
        );
      }
    }

    for (final oldCid in parent.children) {
      final oldChild = _nodes[oldCid];
      if (oldChild != null && oldChild.parentId == parentId) {
        oldChild.parentId = null;
      }
    }

    for (final cid in children) {
      final child = _nodes[cid];
      if (child != null) {
        final prevParent = child.parentId;
        if (prevParent != null && prevParent != parentId) {
          _nodes[prevParent]?.children.remove(cid);
        }
        child.parentId = parentId;
      }
    }

    parent.children = List<int>.of(children);
    return null;
  }
}

class _TextInheritFrame {
  const _TextInheritFrame({
    required this.nodeId,
    required this.color,
    required this.fontSize,
    required this.hasInherited,
  });

  final int nodeId;
  final NveColorRgba color;
  final double fontSize;
  final bool hasInherited;
}
