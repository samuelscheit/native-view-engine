import 'dart:convert';
import 'dart:typed_data';
import 'dart:ui' show Color, Rect;

/// Wire magic bytes (`"NVE1"` in little-endian: `0x3145564E`).
const int nveAbiMagic = 0x3145564E;

/// Supported binary `CommandBuffer` ABI version (`crates/nve-abi/src/codec.rs`).
const int nveAbiVersion = 1;

/// Fixed 24-byte `CommandBufferHeader` size.
const int nveHeaderSize = 24;

/// Reserved root node ID for every NVE surface (`ROOT_NODE_ID` in `crates/nve-abi/src/lib.rs`).
const int nveRootNodeId = 1;

/// Safety cap for incoming binary command buffers (64 MiB).
const int nveMaxBufferBytes = 64 * 1024 * 1024;

/// Safety cap for operations per frame.
const int nveMaxOpCount = 1000000;

/// Maximum finite layout/style scalar magnitude (matches `kMaxLayoutScalar` in C++).
const double nveMaxLayoutScalar = 1e7;

/// Event subscription bitflags matching `EventMask` in `crates/nve-abi/src/events.rs`.
abstract final class NveEventMask {
  static const int none = 0;
  static const int press = 1 << 0;
  static const int longPress = 1 << 1;
  static const int scroll = 1 << 2;
  static const int textChange = 1 << 3;
  static const int focus = 1 << 4;
  static const int blur = 1 << 5;
  static const int layout = 1 << 6;
  static const int custom = 1 << 7;
}

/// Wire opcodes matching `Opcode` in `crates/nve-abi/src/codec.rs`.
enum NveOpcode {
  createNode(1),
  removeNode(2),
  setChildren(3),
  setText(4),
  setLayout(5),
  setStyle(6),
  setRawProp(7),
  setEventMask(8),
  dispatchCommand(9),
  commit(10);

  const NveOpcode(this.code);
  final int code;

  static NveOpcode? fromCode(int code) {
    return switch (code) {
      1 => NveOpcode.createNode,
      2 => NveOpcode.removeNode,
      3 => NveOpcode.setChildren,
      4 => NveOpcode.setText,
      5 => NveOpcode.setLayout,
      6 => NveOpcode.setStyle,
      7 => NveOpcode.setRawProp,
      8 => NveOpcode.setEventMask,
      9 => NveOpcode.dispatchCommand,
      10 => NveOpcode.commit,
      _ => null,
    };
  }
}

/// Component kinds matching `ComponentKind` in `crates/nve-abi/src/codec.rs`.
enum NveComponentKind {
  root(0),
  view(1),
  text(2),
  scrollView(3),
  image(4),
  textInput(5),
  pressable(6),
  skiaCanvas(7),
  custom(0xFFFF);

  const NveComponentKind(this.code);
  final int code;

  static NveComponentKind? fromCode(int code) {
    return switch (code) {
      0 => NveComponentKind.root,
      1 => NveComponentKind.view,
      2 => NveComponentKind.text,
      3 => NveComponentKind.scrollView,
      4 => NveComponentKind.image,
      5 => NveComponentKind.textInput,
      6 => NveComponentKind.pressable,
      7 => NveComponentKind.skiaCanvas,
      0xFFFF => NveComponentKind.custom,
      _ => null,
    };
  }
}

/// Dimension unit tag matching `Dimension` in `crates/nve-abi/src/codec.rs`.
enum NveDimensionUnit {
  auto(0),
  points(1),
  percent(2);

  const NveDimensionUnit(this.code);
  final int code;

  static NveDimensionUnit? fromCode(int code) {
    return switch (code) {
      0 => NveDimensionUnit.auto,
      1 => NveDimensionUnit.points,
      2 => NveDimensionUnit.percent,
      _ => null,
    };
  }
}

/// CSS/Yoga dimension value (`Auto`, `Points(v)`, `Percent(v)`).
class NveDimension {
  const NveDimension(this.unit, this.value);

  const NveDimension.auto()
      : unit = NveDimensionUnit.auto,
        value = 0.0;

  const NveDimension.points(this.value) : unit = NveDimensionUnit.points;

  const NveDimension.percent(this.value) : unit = NveDimensionUnit.percent;

  final NveDimensionUnit unit;
  final double value;

  bool get isAuto => unit == NveDimensionUnit.auto;
  bool get isPoints => unit == NveDimensionUnit.points;
  bool get isPercent => unit == NveDimensionUnit.percent;

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other is NveDimension && other.unit == unit && other.value == value);
  }

  @override
  int get hashCode => Object.hash(unit, value);

  @override
  String toString() => switch (unit) {
        NveDimensionUnit.auto => 'auto',
        NveDimensionUnit.points => '${value}pt',
        NveDimensionUnit.percent => '$value%',
      };
}

/// Flexbox main-axis direction matching `FlexDirection` in `crates/nve-abi/src/codec.rs`.
enum NveFlexDirection {
  column(0),
  row(1),
  columnReverse(2),
  rowReverse(3);

  const NveFlexDirection(this.code);
  final int code;

  bool get isRow => this == row || this == rowReverse;
  bool get isReverse => this == columnReverse || this == rowReverse;

  static NveFlexDirection? fromCode(int code) {
    return switch (code) {
      0 => NveFlexDirection.column,
      1 => NveFlexDirection.row,
      2 => NveFlexDirection.columnReverse,
      3 => NveFlexDirection.rowReverse,
      _ => null,
    };
  }
}

/// Flexbox main-axis justification matching `FlexJustify` in `crates/nve-abi/src/codec.rs`.
enum NveFlexJustify {
  flexStart(0),
  center(1),
  flexEnd(2),
  spaceBetween(3),
  spaceAround(4),
  spaceEvenly(5);

  const NveFlexJustify(this.code);
  final int code;

  static NveFlexJustify? fromCode(int code) {
    return switch (code) {
      0 => NveFlexJustify.flexStart,
      1 => NveFlexJustify.center,
      2 => NveFlexJustify.flexEnd,
      3 => NveFlexJustify.spaceBetween,
      4 => NveFlexJustify.spaceAround,
      5 => NveFlexJustify.spaceEvenly,
      _ => null,
    };
  }
}

/// Flexbox cross-axis alignment matching `FlexAlign` in `crates/nve-abi/src/codec.rs`.
enum NveFlexAlign {
  stretch(0),
  flexStart(1),
  center(2),
  flexEnd(3),
  baseline(4);

  const NveFlexAlign(this.code);
  final int code;

  static NveFlexAlign? fromCode(int code) {
    return switch (code) {
      0 => NveFlexAlign.stretch,
      1 => NveFlexAlign.flexStart,
      2 => NveFlexAlign.center,
      3 => NveFlexAlign.flexEnd,
      4 => NveFlexAlign.baseline,
      _ => null,
    };
  }
}

/// Edge insets matching `EdgeInsets` in `crates/nve-abi/src/codec.rs`.
class NveEdgeInsets {
  const NveEdgeInsets({
    this.top = 0.0,
    this.right = 0.0,
    this.bottom = 0.0,
    this.left = 0.0,
  });

  const NveEdgeInsets.all(double value)
      : top = value,
        right = value,
        bottom = value,
        left = value;

  const NveEdgeInsets.symmetric({double vertical = 0.0, double horizontal = 0.0})
      : top = vertical,
        right = horizontal,
        bottom = vertical,
        left = horizontal;

  static const NveEdgeInsets zero = NveEdgeInsets();

  final double top;
  final double right;
  final double bottom;
  final double left;

  double get horizontal => left + right;
  double get vertical => top + bottom;

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other is NveEdgeInsets &&
            other.top == top &&
            other.right == right &&
            other.bottom == bottom &&
            other.left == left);
  }

  @override
  int get hashCode => Object.hash(top, right, bottom, left);
}

/// Flexbox layout properties matching `LayoutProps` in `crates/nve-abi/src/codec.rs`.
class NveLayoutProps {
  const NveLayoutProps({
    this.width = const NveDimension.auto(),
    this.height = const NveDimension.auto(),
    this.minWidth = const NveDimension.auto(),
    this.minHeight = const NveDimension.auto(),
    this.maxWidth = const NveDimension.auto(),
    this.maxHeight = const NveDimension.auto(),
    this.flexDirection = NveFlexDirection.column,
    this.justifyContent = NveFlexJustify.flexStart,
    this.alignItems = NveFlexAlign.stretch,
    this.flexGrow = 0.0,
    this.flexShrink = 1.0,
    this.gap = 0.0,
    this.padding = NveEdgeInsets.zero,
    this.margin = NveEdgeInsets.zero,
  });

  static const NveLayoutProps defaults = NveLayoutProps();

  final NveDimension width;
  final NveDimension height;
  final NveDimension minWidth;
  final NveDimension minHeight;
  final NveDimension maxWidth;
  final NveDimension maxHeight;
  final NveFlexDirection flexDirection;
  final NveFlexJustify justifyContent;
  final NveFlexAlign alignItems;
  final double flexGrow;
  final double flexShrink;
  final double gap;
  final NveEdgeInsets padding;
  final NveEdgeInsets margin;
}

/// Wire RGBA color matching `ColorRgba` in `crates/nve-abi/src/codec.rs`.
///
/// On the wire (`ColorRgba::to_u32().to_le_bytes()`), bytes are laid out as
/// `[r, g, b, a]` (so little-endian `uint32` is `0xAABBGGRR`).
class NveColorRgba {
  const NveColorRgba(this.r, this.g, this.b, this.a);

  const NveColorRgba.rgb(this.r, this.g, this.b) : a = 255;

  static const NveColorRgba transparent = NveColorRgba(0, 0, 0, 0);
  static const NveColorRgba black = NveColorRgba(0, 0, 0, 255);
  static const NveColorRgba white = NveColorRgba(255, 255, 255, 255);

  /// Decodes little-endian wire `uint32` (`u32::from_le_bytes([r, g, b, a])`).
  factory NveColorRgba.fromWireUint32(int value) {
    return NveColorRgba(
      value & 0xFF,
      (value >> 8) & 0xFF,
      (value >> 16) & 0xFF,
      (value >> 24) & 0xFF,
    );
  }

  final int r;
  final int g;
  final int b;
  final int a;

  /// Encodes to little-endian wire `uint32` (`[r, g, b, a]`).
  int toWireUint32() => (r & 0xFF) | ((g & 0xFF) << 8) | ((b & 0xFF) << 16) | ((a & 0xFF) << 24);

  /// Converts to Flutter `Color` (`ARGB` order).
  Color toFlutterColor() => Color.fromARGB(a & 0xFF, r & 0xFF, g & 0xFF, b & 0xFF);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other is NveColorRgba && other.r == r && other.g == g && other.b == b && other.a == a);
  }

  @override
  int get hashCode => Object.hash(r, g, b, a);

  @override
  String toString() => 'NveColorRgba($r, $g, $b, $a)';
}

/// Visual style properties matching `StyleProps` in `crates/nve-abi/src/codec.rs`.
class NveStyleProps {
  const NveStyleProps({
    this.background = NveColorRgba.transparent,
    this.textColor = NveColorRgba.black,
    this.borderColor = NveColorRgba.transparent,
    this.borderWidth = 0.0,
    this.borderRadius = 0.0,
    this.opacity = 1.0,
    this.fontSize = 16.0,
    this.zIndex = 0,
  });

  static const NveStyleProps defaults = NveStyleProps();

  final NveColorRgba background;
  final NveColorRgba textColor;
  final NveColorRgba borderColor;
  final double borderWidth;
  final double borderRadius;
  final double opacity;
  final double fontSize;
  final int zIndex;

  NveStyleProps copyWith({
    NveColorRgba? background,
    NveColorRgba? textColor,
    NveColorRgba? borderColor,
    double? borderWidth,
    double? borderRadius,
    double? opacity,
    double? fontSize,
    int? zIndex,
  }) {
    return NveStyleProps(
      background: background ?? this.background,
      textColor: textColor ?? this.textColor,
      borderColor: borderColor ?? this.borderColor,
      borderWidth: borderWidth ?? this.borderWidth,
      borderRadius: borderRadius ?? this.borderRadius,
      opacity: opacity ?? this.opacity,
      fontSize: fontSize ?? this.fontSize,
      zIndex: zIndex ?? this.zIndex,
    );
  }
}

/// Discriminator for `RawPropValue` in `crates/nve-abi/src/codec.rs`.
enum NveRawPropKind {
  nullValue(0),
  boolValue(1),
  i64Value(2),
  f64Value(3),
  stringValue(4),
  bytesValue(5);

  const NveRawPropKind(this.code);
  final int code;

  static NveRawPropKind? fromCode(int code) {
    return switch (code) {
      0 => NveRawPropKind.nullValue,
      1 => NveRawPropKind.boolValue,
      2 => NveRawPropKind.i64Value,
      3 => NveRawPropKind.f64Value,
      4 => NveRawPropKind.stringValue,
      5 => NveRawPropKind.bytesValue,
      _ => null,
    };
  }
}

/// Decoded raw prop value (`Command::SetRawProp`).
class NveRawPropValue {
  const NveRawPropValue._(this.kind, this.value);

  const NveRawPropValue.nullValue()
      : kind = NveRawPropKind.nullValue,
        value = null;

  const NveRawPropValue.boolValue(bool v) : this._(NveRawPropKind.boolValue, v);
  const NveRawPropValue.i64Value(int v) : this._(NveRawPropKind.i64Value, v);
  const NveRawPropValue.f64Value(double v) : this._(NveRawPropKind.f64Value, v);
  const NveRawPropValue.stringValue(String v) : this._(NveRawPropKind.stringValue, v);
  NveRawPropValue.bytesValue(Uint8List v) : this._(NveRawPropKind.bytesValue, v);

  final NveRawPropKind kind;
  final Object? value;

  bool? get asBool => kind == NveRawPropKind.boolValue ? value as bool : null;
  int? get asInt => kind == NveRawPropKind.i64Value ? value as int : null;
  double? get asDouble => kind == NveRawPropKind.f64Value ? value as double : null;
  String? get asString => kind == NveRawPropKind.stringValue ? value as String : null;
  Uint8List? get asBytes => kind == NveRawPropKind.bytesValue ? value as Uint8List : null;
}

/// Imperative command emitted by `Opcode::DispatchCommand`.
class NveDispatchedCommand {
  const NveDispatchedCommand({
    required this.nodeId,
    required this.commandName,
    required this.payload,
  });

  final int nodeId;
  final String commandName;
  final Uint8List payload;
}

/// Decoded `CommandBufferHeader` (24 bytes).
class NveCommandBufferHeader {
  const NveCommandBufferHeader({
    required this.magic,
    required this.version,
    required this.flags,
    required this.surfaceId,
    required this.frameId,
    required this.opCount,
  });

  final int magic;
  final int version;
  final int flags;
  final int surfaceId;
  final int frameId;
  final int opCount;
}

/// Error categories returned when decoding or applying a `CommandBuffer`.
enum NveDecodeErrorCode {
  truncated,
  invalidMagic,
  unsupportedVersion,
  unknownOpcode,
  invalidEnum,
  invalidUtf8,
  invalidScalar,
  trailingBytes,
  bufferTooLarge,
  unknownNode,
  cannotRemoveRoot,
  cycleOrDuplicateChild,
}

/// Structured, non-throwing decode/apply error.
class NveDecodeError {
  const NveDecodeError(this.code, this.message, {this.offset = 0});

  final NveDecodeErrorCode code;
  final String message;
  final int offset;

  @override
  String toString() => 'NveDecodeError($code at $offset: $message)';
}

/// Result of decoding and applying a binary `CommandBuffer` to an [NveRetainedTree].
class NveFrameApplyResult {
  const NveFrameApplyResult.ok({
    required this.header,
    required this.mutationCount,
    required this.committed,
    required this.syncMount,
    required this.dispatchedCommands,
  }) : error = null;

  const NveFrameApplyResult.err(this.error, {this.header})
      : mutationCount = 0,
        committed = false,
        syncMount = false,
        dispatchedCommands = const <NveDispatchedCommand>[];

  final NveCommandBufferHeader? header;
  final NveDecodeError? error;
  final int mutationCount;
  final bool committed;
  final bool syncMount;
  final List<NveDispatchedCommand> dispatchedCommands;

  bool get isOk => error == null;
}

/// Retained node inside [NveRetainedTree].
class NveNode {
  NveNode({
    required this.id,
    required this.kind,
    this.customName,
    this.text,
    this.layout = NveLayoutProps.defaults,
    this.style = NveStyleProps.defaults,
    this.hasExplicitStyle = false,
    this.eventMask = 0,
    List<int>? children,
    Map<String, NveRawPropValue>? rawProps,
    this.parentId,
    this.computedRect = Rect.zero,
    this.hasComputedLayout = false,
  })  : children = children ?? <int>[],
        rawProps = rawProps ?? <String, NveRawPropValue>{};

  final int id;
  NveComponentKind kind;
  String? customName;
  String? text;
  NveLayoutProps layout;
  NveStyleProps style;
  bool hasExplicitStyle;
  int eventMask;
  List<int> children;
  final Map<String, NveRawPropValue> rawProps;
  int? parentId;

  /// Surface-absolute bounding rectangle computed by [NveFlexLayoutSolver].
  Rect computedRect;
  bool hasComputedLayout;

  /// True for leaf `Text` nodes that actually paint glyphs (skips wrapper `Text` nodes with children).
  bool get isTextLeaf => kind == NveComponentKind.text && children.isEmpty;

  /// True for nodes that accept pointer hit-testing (`Pressable` or non-zero `eventMask`).
  bool get isInteractive => kind == NveComponentKind.pressable || eventMask != 0;
}

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

bool _isValidDimension(NveDimension d) {
  return d.isAuto || ((d.isPoints || d.isPercent) && _isValidScalar(d.value));
}

bool _isValidInsets(NveEdgeInsets e) {
  return _isValidScalar(e.top) &&
      _isValidScalar(e.right) &&
      _isValidScalar(e.bottom) &&
      _isValidScalar(e.left);
}

bool _isValidLayout(NveLayoutProps l) {
  return _isValidDimension(l.width) &&
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
}

bool _isValidStyle(NveStyleProps s) {
  return _isValidScalar(s.borderWidth) &&
      _isValidScalar(s.borderRadius) &&
      _isValidScalar(s.opacity) &&
      _isValidScalar(s.fontSize);
}

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

  /// Unmodifiable map of all live retained nodes keyed by `nodeId`.
  Map<int, NveNode> get nodes => Map<int, NveNode>.unmodifiable(_nodes);

  /// Live node count (always >= 1 because [nveRootNodeId] is always present).
  int get nodeCount => _nodes.length;

  /// Returns a node by ID, or `null` if absent.
  NveNode? node(int nodeId) => _nodes[nodeId];

  /// Returns the root node (`id == 1`).
  NveNode get root => _nodes[nveRootNodeId]!;

  /// Returns the computed surface-local [Rect] for `nodeId`, or `null` if not laid out.
  Rect? layoutOf(int nodeId) {
    final n = _nodes[nodeId];
    if (n == null || !n.hasComputedLayout) return null;
    return n.computedRect;
  }

  /// Resets the tree to a fresh root-only state (used on module hot-swap).
  void reset() {
    _nodes
      ..clear()
      ..[nveRootNodeId] = NveNode(id: nveRootNodeId, kind: NveComponentKind.root);
  }

  /// Decodes and atomically applies a binary `CommandBuffer` frame.
  ///
  /// Never throws `RangeError` or `FormatException` on corrupt, truncated, or
  /// malformed inputs; returns [NveFrameApplyResult.err] while leaving the
  /// existing tree unmodified when decoding fails.
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

    // Phase 1: Decode and validate the entire binary stream without mutating state.
    final decoded = <_DecodedCmd>[];
    for (var i = 0; i < opCount; i++) {
      final opOffset = reader.pos;
      if (!reader.has(1)) {
        return NveFrameApplyResult.err(
          NveDecodeError(
            NveDecodeErrorCode.truncated,
            'Truncated before opcode #$i',
            offset: opOffset,
          ),
          header: header,
        );
      }
      final opByte = reader.readU8();
      final opcode = NveOpcode.fromCode(opByte);
      if (opcode == null) {
        return NveFrameApplyResult.err(
          NveDecodeError(
            NveDecodeErrorCode.unknownOpcode,
            'Unknown opcode $opByte',
            offset: opOffset,
          ),
          header: header,
        );
      }

      switch (opcode) {
        case NveOpcode.createNode:
          if (!reader.has(8)) {
            return NveFrameApplyResult.err(
              NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated CreateNode', offset: opOffset),
              header: header,
            );
          }
          final nodeId = reader.readU32();
          final kindCode = reader.readU16();
          final kind = NveComponentKind.fromCode(kindCode);
          if (kind == null) {
            return NveFrameApplyResult.err(
              NveDecodeError(
                NveDecodeErrorCode.invalidEnum,
                'Invalid ComponentKind $kindCode',
                offset: opOffset,
              ),
              header: header,
            );
          }
          final nameLen = reader.readU16();
          if (!reader.has(nameLen)) {
            return NveFrameApplyResult.err(
              NveDecodeError(
                NveDecodeErrorCode.truncated,
                'Truncated custom component name ($nameLen bytes)',
                offset: reader.pos,
              ),
              header: header,
            );
          }
          String? customName;
          if (nameLen > 0) {
            customName = reader.readUtf8(nameLen);
            if (customName == null) {
              return NveFrameApplyResult.err(
                NveDecodeError(
                  NveDecodeErrorCode.invalidUtf8,
                  'Invalid UTF-8 in custom component name',
                  offset: opOffset,
                ),
                header: header,
              );
            }
          }
          decoded.add(_CreateNodeCmd(nodeId, kind, customName));

        case NveOpcode.removeNode:
          if (!reader.has(4)) {
            return NveFrameApplyResult.err(
              NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated RemoveNode', offset: opOffset),
              header: header,
            );
          }
          decoded.add(_RemoveNodeCmd(reader.readU32()));

        case NveOpcode.setChildren:
          if (!reader.has(8)) {
            return NveFrameApplyResult.err(
              NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated SetChildren', offset: opOffset),
              header: header,
            );
          }
          final parentId = reader.readU32();
          final count = reader.readU32();
          if (count > reader.remaining ~/ 4) {
            return NveFrameApplyResult.err(
              NveDecodeError(
                NveDecodeErrorCode.truncated,
                'Truncated SetChildren list ($count children)',
                offset: reader.pos,
              ),
              header: header,
            );
          }
          final children = List<int>.generate(count, (_) => reader.readU32(), growable: false);
          decoded.add(_SetChildrenCmd(parentId, children));

        case NveOpcode.setText:
          if (!reader.has(8)) {
            return NveFrameApplyResult.err(
              NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated SetText', offset: opOffset),
              header: header,
            );
          }
          final nodeId = reader.readU32();
          final len = reader.readU32();
          if (!reader.has(len)) {
            return NveFrameApplyResult.err(
              NveDecodeError(
                NveDecodeErrorCode.truncated,
                'Truncated SetText payload ($len bytes)',
                offset: reader.pos,
              ),
              header: header,
            );
          }
          final text = reader.readUtf8(len);
          if (text == null) {
            return NveFrameApplyResult.err(
              NveDecodeError(
                NveDecodeErrorCode.invalidUtf8,
                'Invalid UTF-8 in SetText',
                offset: opOffset,
              ),
              header: header,
            );
          }
          decoded.add(_SetTextCmd(nodeId, text));

        case NveOpcode.setLayout:
          // nodeId (4) + 6 * Dimension (30) + 3 enums (3) + 3 f32 (12) + 2 * EdgeInsets (32) = 81 bytes.
          if (!reader.has(81)) {
            return NveFrameApplyResult.err(
              NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated SetLayout', offset: opOffset),
              header: header,
            );
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
            return NveFrameApplyResult.err(
              NveDecodeError(
                NveDecodeErrorCode.invalidEnum,
                'Invalid Dimension unit in SetLayout',
                offset: opOffset,
              ),
              header: header,
            );
          }
          final flexDir = NveFlexDirection.fromCode(reader.readU8());
          final justify = NveFlexJustify.fromCode(reader.readU8());
          final align = NveFlexAlign.fromCode(reader.readU8());
          if (flexDir == null || justify == null || align == null) {
            return NveFrameApplyResult.err(
              NveDecodeError(
                NveDecodeErrorCode.invalidEnum,
                'Invalid Flexbox enum in SetLayout',
                offset: opOffset,
              ),
              header: header,
            );
          }
          final flexGrow = reader.readF32();
          final flexShrink = reader.readF32();
          final gap = reader.readF32();
          final padding = reader.readInsets();
          final margin = reader.readInsets();
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
            flexGrow: flexGrow,
            flexShrink: flexShrink,
            gap: gap,
            padding: padding,
            margin: margin,
          );
          if (!_isValidLayout(layout)) {
            return NveFrameApplyResult.err(
              NveDecodeError(
                NveDecodeErrorCode.invalidScalar,
                'Non-finite or out-of-range scalar in SetLayout',
                offset: opOffset,
              ),
              header: header,
            );
          }
          decoded.add(_SetLayoutCmd(nodeId, layout));

        case NveOpcode.setStyle:
          // nodeId (4) + 3 * u32 (12) + 4 * f32 (16) + i32 (4) = 36 bytes.
          if (!reader.has(36)) {
            return NveFrameApplyResult.err(
              NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated SetStyle', offset: opOffset),
              header: header,
            );
          }
          final nodeId = reader.readU32();
          final bg = NveColorRgba.fromWireUint32(reader.readU32());
          final textColor = NveColorRgba.fromWireUint32(reader.readU32());
          final borderColor = NveColorRgba.fromWireUint32(reader.readU32());
          final borderWidth = reader.readF32();
          final borderRadius = reader.readF32();
          final opacity = reader.readF32();
          final fontSize = reader.readF32();
          final zIndex = reader.readI32();
          final style = NveStyleProps(
            background: bg,
            textColor: textColor,
            borderColor: borderColor,
            borderWidth: borderWidth,
            borderRadius: borderRadius,
            opacity: opacity,
            fontSize: fontSize,
            zIndex: zIndex,
          );
          if (!_isValidStyle(style)) {
            return NveFrameApplyResult.err(
              NveDecodeError(
                NveDecodeErrorCode.invalidScalar,
                'Non-finite or out-of-range scalar in SetStyle',
                offset: opOffset,
              ),
              header: header,
            );
          }
          decoded.add(_SetStyleCmd(nodeId, style));

        case NveOpcode.setRawProp:
          if (!reader.has(6)) {
            return NveFrameApplyResult.err(
              NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated SetRawProp', offset: opOffset),
              header: header,
            );
          }
          final nodeId = reader.readU32();
          final keyLen = reader.readU16();
          if (!reader.has(keyLen + 1)) {
            return NveFrameApplyResult.err(
              NveDecodeError(
                NveDecodeErrorCode.truncated,
                'Truncated SetRawProp key/tag',
                offset: reader.pos,
              ),
              header: header,
            );
          }
          final key = reader.readUtf8(keyLen);
          if (key == null) {
            return NveFrameApplyResult.err(
              NveDecodeError(
                NveDecodeErrorCode.invalidUtf8,
                'Invalid UTF-8 in SetRawProp key',
                offset: opOffset,
              ),
              header: header,
            );
          }
          final tag = reader.readU8();
          final valKind = NveRawPropKind.fromCode(tag);
          if (valKind == null) {
            return NveFrameApplyResult.err(
              NveDecodeError(
                NveDecodeErrorCode.invalidEnum,
                'Invalid RawPropValue tag $tag',
                offset: reader.pos - 1,
              ),
              header: header,
            );
          }
          NveRawPropValue propValue;
          switch (valKind) {
            case NveRawPropKind.nullValue:
              propValue = const NveRawPropValue.nullValue();
            case NveRawPropKind.boolValue:
              if (!reader.has(1)) {
                return NveFrameApplyResult.err(
                  NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated Bool prop', offset: reader.pos),
                  header: header,
                );
              }
              propValue = NveRawPropValue.boolValue(reader.readU8() != 0);
            case NveRawPropKind.i64Value:
              if (!reader.has(8)) {
                return NveFrameApplyResult.err(
                  NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated I64 prop', offset: reader.pos),
                  header: header,
                );
              }
              propValue = NveRawPropValue.i64Value(reader.readI64());
            case NveRawPropKind.f64Value:
              if (!reader.has(8)) {
                return NveFrameApplyResult.err(
                  NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated F64 prop', offset: reader.pos),
                  header: header,
                );
              }
              propValue = NveRawPropValue.f64Value(reader.readF64());
            case NveRawPropKind.stringValue:
              if (!reader.has(4)) {
                return NveFrameApplyResult.err(
                  NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated String prop length', offset: reader.pos),
                  header: header,
                );
              }
              final vlen = reader.readU32();
              if (!reader.has(vlen)) {
                return NveFrameApplyResult.err(
                  NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated String prop bytes', offset: reader.pos),
                  header: header,
                );
              }
              final str = reader.readUtf8(vlen);
              if (str == null) {
                return NveFrameApplyResult.err(
                  NveDecodeError(NveDecodeErrorCode.invalidUtf8, 'Invalid UTF-8 in String prop', offset: opOffset),
                  header: header,
                );
              }
              propValue = NveRawPropValue.stringValue(str);
            case NveRawPropKind.bytesValue:
              if (!reader.has(4)) {
                return NveFrameApplyResult.err(
                  NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated Bytes prop length', offset: reader.pos),
                  header: header,
                );
              }
              final vlen = reader.readU32();
              if (!reader.has(vlen)) {
                return NveFrameApplyResult.err(
                  NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated Bytes prop payload', offset: reader.pos),
                  header: header,
                );
              }
              propValue = NveRawPropValue.bytesValue(reader.readBytesCopy(vlen));
          }
          decoded.add(_SetRawPropCmd(nodeId, key, propValue));

        case NveOpcode.setEventMask:
          if (!reader.has(12)) {
            return NveFrameApplyResult.err(
              NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated SetEventMask', offset: opOffset),
              header: header,
            );
          }
          final nodeId = reader.readU32();
          final mask = reader.readU64();
          decoded.add(_SetEventMaskCmd(nodeId, mask));

        case NveOpcode.dispatchCommand:
          if (!reader.has(6)) {
            return NveFrameApplyResult.err(
              NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated DispatchCommand', offset: opOffset),
              header: header,
            );
          }
          final nodeId = reader.readU32();
          final nameLen = reader.readU16();
          if (!reader.has(nameLen + 4)) {
            return NveFrameApplyResult.err(
              NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated DispatchCommand name/len', offset: reader.pos),
              header: header,
            );
          }
          final name = reader.readUtf8(nameLen);
          if (name == null) {
            return NveFrameApplyResult.err(
              NveDecodeError(NveDecodeErrorCode.invalidUtf8, 'Invalid UTF-8 in command name', offset: opOffset),
              header: header,
            );
          }
          final payloadLen = reader.readU32();
          if (!reader.has(payloadLen)) {
            return NveFrameApplyResult.err(
              NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated DispatchCommand payload', offset: reader.pos),
              header: header,
            );
          }
          final payload = reader.readBytesCopy(payloadLen);
          decoded.add(_DispatchCommandCmd(nodeId, name, payload));

        case NveOpcode.commit:
          if (!reader.has(1)) {
            return NveFrameApplyResult.err(
              NveDecodeError(NveDecodeErrorCode.truncated, 'Truncated Commit', offset: opOffset),
              header: header,
            );
          }
          decoded.add(_CommitCmd(reader.readU8() != 0));
      }
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

    // Phase 2: Apply validated commands to the retained tree.
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
  ///
  /// Defaults to opaque black (`0xFF000000`) and `16.0` when no `Text` ancestor
  /// defines a style.
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

    // Collect ancestors of parentId to reject any cycle.
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

    // Clear old parent back-links.
    for (final oldCid in parent.children) {
      final oldChild = _nodes[oldCid];
      if (oldChild != null && oldChild.parentId == parentId) {
        oldChild.parentId = null;
      }
    }

    // Reparent new children cleanly.
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

/// Binary `CommandBuffer` encoder mirroring `CommandBufferBuilder` in `crates/nve-abi/src/codec.rs`.
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
