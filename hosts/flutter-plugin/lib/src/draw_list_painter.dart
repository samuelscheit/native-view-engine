import 'dart:math' as math;
import 'dart:typed_data';

import 'package:flutter/rendering.dart';
import 'package:nve_host/nve_draw_list.generated.dart';

/// Fixed byte length of the flat `nve.res.draw_list` header (word 0: little-endian `u32 opCount`).
const int nveDrawHeaderSize = NveDrawList.headerSize;

/// Strict upper bound on operations per draw list (`MAX_DRAW_OPS`).
const int nveDrawMaxOps = NveDrawList.maxDrawOps;

/// Strict upper bound on vertices inside a single `StrokePolyline` (`MAX_POLYLINE_POINTS`).
const int nveDrawMaxPolylinePoints = NveDrawList.maxPolylinePoints;

/// Maximum nested `ClipRect` stack depth guarded during painting.
const int nveDrawMaxClipDepth = NveDrawList.maxClipDepth;

/// Little-endian `u32` opcodes encoded inside a flat `nve.res.draw_list` word stream
/// matching `packages/modules/canvas/src/flat.rs`.
abstract final class NveDrawOpcode {
  static const int clear = NveDrawList.opClear;
  static const int fillRect = NveDrawList.opFillRect;
  static const int fillRoundedRect = NveDrawList.opFillRoundedRect;
  static const int fillCircle = NveDrawList.opFillCircle;
  static const int strokeLine = NveDrawList.opStrokeLine;
  static const int strokePolyline = NveDrawList.opStrokePolyline;
  static const int clipRect = NveDrawList.opClipRect;
  static const int restoreClip = NveDrawList.opRestoreClip;
}

/// Error categories returned when validating or painting a flat `nve.res.draw_list` buffer.
enum NveDrawListErrorCode {
  truncated,
  opCountExceedsLimit,
  polylinePointCountExceedsLimit,
  unknownOpcode,
  nonFiniteFloat,
  negativeFloat,
  trailingBytes,
}

/// Structured, non-throwing validation or paint result for a flat `nve.res.draw_list` buffer.
class NveDrawListResult {
  const NveDrawListResult.ok({
    required this.opCount,
  })  : errorCode = null,
        errorMessage = null;

  const NveDrawListResult.err(this.errorCode, this.errorMessage) : opCount = 0;

  final int opCount;
  final NveDrawListErrorCode? errorCode;
  final String? errorMessage;

  bool get isOk => errorCode == null;

  @override
  String toString() => isOk
      ? 'NveDrawListResult.ok(ops=$opCount)'
      : 'NveDrawListResult.err($errorCode: $errorMessage)';
}

/// Zero-copy flat `nve.res.draw_list` (`u32`-word) Flutter [Canvas] painter driven by the
/// generated [NveDrawList] validator and balanced streaming decoder.
class NveDrawListPainter {
  NveDrawListPainter();

  final _FlutterDrawListSink _sink = _FlutterDrawListSink();

  /// Strictly validates a flat `nve.res.draw_list` (`u32`-word) buffer zero-copy via the
  /// generated [NveDrawList.validateView] decoder without allocating per-op objects.
  static NveDrawListResult validate(Uint8List bytes) {
    if (bytes.lengthInBytes < nveDrawHeaderSize) {
      return _mapError(NveDrawListError.truncated);
    }
    final ByteData view = ByteData.sublistView(bytes);
    final NveDrawListError? err = NveDrawList.validateView(view);
    if (err != null) {
      return _mapError(err);
    }
    return NveDrawListResult.ok(opCount: view.getUint32(0, Endian.little));
  }

  /// Validates and paints raw flat `nve.res.draw_list` word bytes onto [canvas] within [bounds]
  /// (with local origin translated to `bounds.topLeft` and clipped).
  NveDrawListResult paint(
    Canvas canvas,
    Rect bounds,
    Uint8List rawBytes, {
    double borderRadius = 0.0,
  }) {
    if (rawBytes.lengthInBytes < nveDrawHeaderSize) {
      return _mapError(NveDrawListError.truncated);
    }
    final ByteData view = ByteData.sublistView(rawBytes);
    final NveDrawListError? err = NveDrawList.validateView(view);
    if (err != null) {
      return _mapError(err);
    }
    final NveDrawListResult check = NveDrawListResult.ok(
      opCount: view.getUint32(0, Endian.little),
    );
    if (bounds.width <= 0.0 || bounds.height <= 0.0) {
      return check;
    }

    canvas.save();
    try {
      if (borderRadius > 0.0) {
        canvas.clipRRect(RRect.fromRectAndRadius(bounds, Radius.circular(borderRadius)));
      } else {
        canvas.clipRect(bounds);
      }
      canvas.translate(bounds.left, bounds.top);
      _sink.bind(canvas, Rect.fromLTWH(0.0, 0.0, bounds.width, bounds.height));
      try {
        NveDrawList.streamValidatedView(view, _sink);
      } finally {
        _sink.unbind();
      }
    } finally {
      canvas.restore();
    }

    return check;
  }

  static NveDrawListResult _mapError(NveDrawListError err) {
    switch (err) {
      case NveDrawListError.truncated:
        return const NveDrawListResult.err(
          NveDrawListErrorCode.truncated,
          'Truncated flat draw-list payload',
        );
      case NveDrawListError.opCountExceedsLimit:
        return const NveDrawListResult.err(
          NveDrawListErrorCode.opCountExceedsLimit,
          'Draw-list op count exceeds limit $nveDrawMaxOps',
        );
      case NveDrawListError.sliceLenExceedsLimit:
        return const NveDrawListResult.err(
          NveDrawListErrorCode.polylinePointCountExceedsLimit,
          'StrokePolyline point count exceeds limit $nveDrawMaxPolylinePoints',
        );
      case NveDrawListError.unknownOpcode:
        return const NveDrawListResult.err(
          NveDrawListErrorCode.unknownOpcode,
          'Unknown draw-list opcode',
        );
      case NveDrawListError.nonFiniteFloat:
        return const NveDrawListResult.err(
          NveDrawListErrorCode.nonFiniteFloat,
          'Non-finite float in draw-list stream',
        );
      case NveDrawListError.negativeFloat:
        return const NveDrawListResult.err(
          NveDrawListErrorCode.negativeFloat,
          'Negative radius or stroke width in draw-list stream',
        );
      case NveDrawListError.trailingBytes:
        return const NveDrawListResult.err(
          NveDrawListErrorCode.trailingBytes,
          'Unexpected trailing bytes after draw-list op stream',
        );
    }
  }
}

final class _FlutterDrawListSink implements NveDrawListSink {
  final Paint _fillPaint = Paint()
    ..style = PaintingStyle.fill
    ..isAntiAlias = true;

  final Paint _strokePaint = Paint()
    ..style = PaintingStyle.stroke
    ..strokeCap = StrokeCap.round
    ..strokeJoin = StrokeJoin.round
    ..isAntiAlias = true;

  final Path _polylinePath = Path();

  Canvas? _canvas;
  Rect _localBounds = Rect.zero;
  bool _polyDrawn = false;

  void bind(Canvas canvas, Rect localBounds) {
    _canvas = canvas;
    _localBounds = localBounds;
  }

  void unbind() {
    _canvas = null;
    _polyDrawn = false;
  }

  @override
  void onClear(int color) {
    if (((color >> 24) & 0xFF) > 0) {
      _fillPaint.color = _colorFromWireRgba(color);
      _canvas!.drawRect(_localBounds, _fillPaint);
    }
  }

  @override
  void onFillRect(double x, double y, double w, double h, int color) {
    if (w > 0.0 && h > 0.0 && ((color >> 24) & 0xFF) > 0) {
      _fillPaint.color = _colorFromWireRgba(color);
      _canvas!.drawRect(Rect.fromLTWH(x, y, w, h), _fillPaint);
    }
  }

  @override
  void onFillRoundedRect(double x, double y, double w, double h, double radius, int color) {
    if (w > 0.0 && h > 0.0 && ((color >> 24) & 0xFF) > 0) {
      _fillPaint.color = _colorFromWireRgba(color);
      final Rect rect = Rect.fromLTWH(x, y, w, h);
      final double r = math.max(0.0, radius);
      if (r > 0.0) {
        _canvas!.drawRRect(RRect.fromRectAndRadius(rect, Radius.circular(r)), _fillPaint);
      } else {
        _canvas!.drawRect(rect, _fillPaint);
      }
    }
  }

  @override
  void onFillCircle(double cx, double cy, double radius, int color) {
    if (radius > 0.0 && ((color >> 24) & 0xFF) > 0) {
      _fillPaint.color = _colorFromWireRgba(color);
      _canvas!.drawCircle(Offset(cx, cy), radius, _fillPaint);
    }
  }

  @override
  void onStrokeLine(double x0, double y0, double x1, double y1, double width, int color) {
    if (width > 0.0 && ((color >> 24) & 0xFF) > 0) {
      _strokePaint
        ..color = _colorFromWireRgba(color)
        ..strokeWidth = width;
      _canvas!.drawLine(Offset(x0, y0), Offset(x1, y1), _strokePaint);
    }
  }

  @override
  void onStrokePolylineStart(int count, double width, int color) {
    _polyDrawn = count >= 2 && width > 0.0 && ((color >> 24) & 0xFF) > 0;
    if (!_polyDrawn) {
      return;
    }
    _polylinePath.reset();
    _strokePaint
      ..color = _colorFromWireRgba(color)
      ..strokeWidth = width;
  }

  @override
  void onStrokePolylinePoint(int index, double x, double y) {
    if (!_polyDrawn) {
      return;
    }
    if (index == 0) {
      _polylinePath.moveTo(x, y);
    } else {
      _polylinePath.lineTo(x, y);
    }
  }

  @override
  void onStrokePolylineEnd() {
    if (_polyDrawn) {
      _canvas!.drawPath(_polylinePath, _strokePaint);
    }
  }

  @override
  void onClipRect(double x, double y, double w, double h) {
    final Canvas c = _canvas!;
    c.save();
    c.clipRect(Rect.fromLTWH(x, y, math.max(0.0, w), math.max(0.0, h)));
  }

  @override
  void onRestoreClip() {
    _canvas!.restore();
  }

  static Color _colorFromWireRgba(int rgba) {
    return Color.fromARGB(
      (rgba >> 24) & 0xFF,
      rgba & 0xFF,
      (rgba >> 8) & 0xFF,
      (rgba >> 16) & 0xFF,
    );
  }
}
