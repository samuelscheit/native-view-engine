import 'dart:typed_data';
import 'dart:ui' show Color, Rect;

import 'nve_core_abi.generated.dart';

export 'nve_core_abi.generated.dart'
    show NveComponentKind, NveOpcode, nveAbiMagic, nveAbiVersion, nveRootNodeId;

/// Fixed 24-byte `CommandBufferHeader` size.
const int nveHeaderSize = 24;

/// Safety cap for incoming binary command buffers (64 MiB).
const int nveMaxBufferBytes = 64 * 1024 * 1024;

/// Safety cap for operations per frame.
const int nveMaxOpCount = 1000000;

/// Maximum finite layout/style scalar magnitude (matches `kMaxLayoutScalar` in C++).
const double nveMaxLayoutScalar = 1e7;

/// Legacy NVE1 command-buffer event subscription bitflags.
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

/// Dimension unit tag matching `Dimension` in `packages/core/abi/src/codec.rs`.
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

/// Flexbox main-axis direction matching `FlexDirection` in `packages/core/abi/src/codec.rs`.
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

/// Flexbox main-axis justification matching `FlexJustify` in `packages/core/abi/src/codec.rs`.
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

/// Flexbox cross-axis alignment matching `FlexAlign` in `packages/core/abi/src/codec.rs`.
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

/// Edge insets matching `EdgeInsets` in `packages/core/abi/src/codec.rs`.
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

/// Flexbox layout properties matching `LayoutProps` in `packages/core/abi/src/codec.rs`.
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

/// Wire RGBA color matching `ColorRgba` in `packages/core/abi/src/codec.rs`.
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

/// Visual style properties matching `StyleProps` in `packages/core/abi/src/codec.rs`.
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

/// Discriminator for `RawPropValue` in `packages/core/abi/src/codec.rs`.
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
