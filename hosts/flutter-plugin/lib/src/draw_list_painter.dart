import 'dart:convert';
import 'dart:math' as math;
import 'dart:typed_data';

import 'package:flutter/rendering.dart';

/// 9-byte ASCII magic header (`b"NVE_DRAW1"`) matching `crates/nve-components/src/canvas_ir.rs`.
const List<int> nveDrawMagic = <int>[78, 86, 69, 95, 68, 82, 65, 87, 49];

/// Supported binary format version (`NVE_DRAW_VERSION == 1`).
const int nveDrawVersion = 1;

/// Fixed byte length of the `NVE_DRAW1` header (`9 + 2 + 4`).
const int nveDrawHeaderSize = 15;

/// Strict upper bound on operations per draw list (`MAX_DRAW_OPS`).
const int nveDrawMaxOps = 65536;

/// Strict upper bound on vertices inside a single `StrokePolyline` (`MAX_POLYLINE_POINTS`).
const int nveDrawMaxPolylinePoints = 16384;

/// Maximum nested `ClipRect` stack depth guarded during painting.
const int nveDrawMaxClipDepth = 64;

/// Compact prefix used when transporting `NVE_DRAW1` bytes through UTF-8 attributes.
const String nveDrawListAttrPrefix = 'nve-draw1;b64,';

/// Opcodes encoded inside an `NVE_DRAW1` binary vector stream.
abstract final class NveDrawOpcode {
  static const int clear = 1;
  static const int fillRect = 2;
  static const int fillRoundedRect = 3;
  static const int fillCircle = 4;
  static const int strokeLine = 5;
  static const int strokePolyline = 6;
  static const int clipRect = 7;
  static const int restoreClip = 8;
}

/// Error categories returned when validating or painting an `NVE_DRAW1` buffer.
enum NveDrawListErrorCode {
  truncated,
  badMagic,
  unsupportedVersion,
  opCountExceedsLimit,
  polylinePointCountExceedsLimit,
  unknownOpcode,
  nonFiniteFloat,
  trailingBytes,
  invalidBase64,
}

/// Structured, non-throwing validation or paint result for an `NVE_DRAW1` draw list.
class NveDrawListResult {
  const NveDrawListResult.ok({
    required this.version,
    required this.opCount,
  })  : errorCode = null,
        errorMessage = null;

  const NveDrawListResult.err(this.errorCode, this.errorMessage)
      : version = 0,
        opCount = 0;

  final int version;
  final int opCount;
  final NveDrawListErrorCode? errorCode;
  final String? errorMessage;

  bool get isOk => errorCode == null;

  @override
  String toString() => isOk
      ? 'NveDrawListResult.ok(v$version, ops=$opCount)'
      : 'NveDrawListResult.err($errorCode: $errorMessage)';
}

/// Zero-copy `NVE_DRAW1` binary draw-list decoder and Flutter [Canvas] painter with
/// reusable [Paint] and [Path] objects and strict bounds/NaN/vertex guards.
class NveDrawListPainter {
  NveDrawListPainter();

  final Paint _fillPaint = Paint()
    ..style = PaintingStyle.fill
    ..isAntiAlias = true;

  final Paint _strokePaint = Paint()
    ..style = PaintingStyle.stroke
    ..strokeCap = StrokeCap.round
    ..strokeJoin = StrokeJoin.round
    ..isAntiAlias = true;

  final Path _polylinePath = Path();

  static final List<int> _attrPrefixBytes = ascii.encode(nveDrawListAttrPrefix);

  /// Resolves either raw `NVE_DRAW1` bytes (returned zero-copy without copying) or an
  /// ASCII `nve-draw1;b64,...` base64 attribute payload into raw `NVE_DRAW1` bytes.
  /// Returns `null` if the payload is neither raw `NVE_DRAW1` nor valid base64.
  static Uint8List? resolveDrawListBytes(Uint8List rawBytes) {
    if (rawBytes.length >= nveDrawMagic.length && _hasMagic(rawBytes)) {
      return rawBytes;
    }
    if (rawBytes.length > _attrPrefixBytes.length && _startsWithAttrPrefix(rawBytes)) {
      try {
        final b64 = ascii.decode(
          Uint8List.sublistView(rawBytes, _attrPrefixBytes.length),
          allowInvalid: false,
        );
        return _decodeBase64Clean(b64);
      } catch (_) {
        return null;
      }
    }
    return rawBytes;
  }

  /// Decodes a UTF-8 `nve-draw1;b64,...` or bare base64 attribute string into raw bytes.
  static Uint8List? decodeAttrString(String raw) {
    final trimmed = raw.trim();
    if (trimmed.isEmpty) return null;
    final b64 = trimmed.startsWith(nveDrawListAttrPrefix)
        ? trimmed.substring(nveDrawListAttrPrefix.length)
        : trimmed;
    return _decodeBase64Clean(b64);
  }

  /// Strictly validates an `NVE_DRAW1` buffer zero-copy via [ByteData.sublistView]
  /// without allocating per-op objects.
  static NveDrawListResult validate(Uint8List bytes) {
    if (bytes.length < nveDrawHeaderSize) {
      return const NveDrawListResult.err(
        NveDrawListErrorCode.truncated,
        'NVE_DRAW1 buffer shorter than 15-byte header',
      );
    }
    if (!_hasMagic(bytes)) {
      return const NveDrawListResult.err(
        NveDrawListErrorCode.badMagic,
        'NVE_DRAW1 magic header mismatch',
      );
    }

    final view = ByteData.sublistView(bytes);
    final version = view.getUint16(9, Endian.little);
    if (version != nveDrawVersion) {
      return NveDrawListResult.err(
        NveDrawListErrorCode.unsupportedVersion,
        'Unsupported NVE_DRAW1 version $version',
      );
    }

    final opCount = view.getUint32(11, Endian.little);
    if (opCount > nveDrawMaxOps) {
      return NveDrawListResult.err(
        NveDrawListErrorCode.opCountExceedsLimit,
        'NVE_DRAW1 op count $opCount exceeds limit $nveDrawMaxOps',
      );
    }

    final len = bytes.length;
    var pos = nveDrawHeaderSize;
    if (len - pos < opCount) {
      return const NveDrawListResult.err(
        NveDrawListErrorCode.truncated,
        'NVE_DRAW1 op count exceeds remaining byte length',
      );
    }

    for (var i = 0; i < opCount; i++) {
      if (pos >= len) {
        return const NveDrawListResult.err(
          NveDrawListErrorCode.truncated,
          'Unexpected EOF reading opcode',
        );
      }
      final opcode = bytes[pos++];
      switch (opcode) {
        case NveDrawOpcode.clear:
          if (len - pos < 4) {
            return const NveDrawListResult.err(
              NveDrawListErrorCode.truncated,
              'Truncated Clear payload',
            );
          }
          pos += 4;
          break;

        case NveDrawOpcode.fillRect:
          if (len - pos < 20) {
            return const NveDrawListResult.err(
              NveDrawListErrorCode.truncated,
              'Truncated FillRect payload',
            );
          }
          if (!_areFinite4(view, pos)) {
            return const NveDrawListResult.err(
              NveDrawListErrorCode.nonFiniteFloat,
              'Non-finite float in FillRect',
            );
          }
          pos += 20;
          break;

        case NveDrawOpcode.fillRoundedRect:
          if (len - pos < 24) {
            return const NveDrawListResult.err(
              NveDrawListErrorCode.truncated,
              'Truncated FillRoundedRect payload',
            );
          }
          if (!_areFinite4(view, pos) || !view.getFloat32(pos + 16, Endian.little).isFinite) {
            return const NveDrawListResult.err(
              NveDrawListErrorCode.nonFiniteFloat,
              'Non-finite float in FillRoundedRect',
            );
          }
          pos += 24;
          break;

        case NveDrawOpcode.fillCircle:
          if (len - pos < 16) {
            return const NveDrawListResult.err(
              NveDrawListErrorCode.truncated,
              'Truncated FillCircle payload',
            );
          }
          if (!view.getFloat32(pos, Endian.little).isFinite ||
              !view.getFloat32(pos + 4, Endian.little).isFinite ||
              !view.getFloat32(pos + 8, Endian.little).isFinite) {
            return const NveDrawListResult.err(
              NveDrawListErrorCode.nonFiniteFloat,
              'Non-finite float in FillCircle',
            );
          }
          pos += 16;
          break;

        case NveDrawOpcode.strokeLine:
          if (len - pos < 24) {
            return const NveDrawListResult.err(
              NveDrawListErrorCode.truncated,
              'Truncated StrokeLine payload',
            );
          }
          if (!_areFinite4(view, pos) || !view.getFloat32(pos + 16, Endian.little).isFinite) {
            return const NveDrawListResult.err(
              NveDrawListErrorCode.nonFiniteFloat,
              'Non-finite float in StrokeLine',
            );
          }
          pos += 24;
          break;

        case NveDrawOpcode.strokePolyline:
          if (len - pos < 12) {
            return const NveDrawListResult.err(
              NveDrawListErrorCode.truncated,
              'Truncated StrokePolyline header',
            );
          }
          if (!view.getFloat32(pos, Endian.little).isFinite) {
            return const NveDrawListResult.err(
              NveDrawListErrorCode.nonFiniteFloat,
              'Non-finite width in StrokePolyline',
            );
          }
          final ptCount = view.getUint32(pos + 8, Endian.little);
          pos += 12;
          if (ptCount > nveDrawMaxPolylinePoints) {
            return NveDrawListResult.err(
              NveDrawListErrorCode.polylinePointCountExceedsLimit,
              'StrokePolyline point count $ptCount exceeds limit $nveDrawMaxPolylinePoints',
            );
          }
          final needed = ptCount * 8;
          if (len - pos < needed) {
            return const NveDrawListResult.err(
              NveDrawListErrorCode.truncated,
              'Truncated StrokePolyline vertices',
            );
          }
          for (var p = 0; p < ptCount; p++) {
            final px = view.getFloat32(pos, Endian.little);
            final py = view.getFloat32(pos + 4, Endian.little);
            if (!px.isFinite || !py.isFinite) {
              return const NveDrawListResult.err(
                NveDrawListErrorCode.nonFiniteFloat,
                'Non-finite vertex in StrokePolyline',
              );
            }
            pos += 8;
          }
          break;

        case NveDrawOpcode.clipRect:
          if (len - pos < 16) {
            return const NveDrawListResult.err(
              NveDrawListErrorCode.truncated,
              'Truncated ClipRect payload',
            );
          }
          if (!_areFinite4(view, pos)) {
            return const NveDrawListResult.err(
              NveDrawListErrorCode.nonFiniteFloat,
              'Non-finite float in ClipRect',
            );
          }
          pos += 16;
          break;

        case NveDrawOpcode.restoreClip:
          break;

        default:
          return NveDrawListResult.err(
            NveDrawListErrorCode.unknownOpcode,
            'Unknown NVE_DRAW1 opcode $opcode',
          );
      }
    }

    if (pos != len) {
      return NveDrawListResult.err(
        NveDrawListErrorCode.trailingBytes,
        '${len - pos} trailing bytes after NVE_DRAW1 op stream',
      );
    }

    return NveDrawListResult.ok(version: version, opCount: opCount);
  }

  /// Validates and paints raw `NVE_DRAW1` bytes (or `nve-draw1;b64,...` bytes) onto
  /// [canvas] within [bounds] (with local origin translated to `bounds.topLeft` and clipped).
  NveDrawListResult paint(
    Canvas canvas,
    Rect bounds,
    Uint8List rawBytes, {
    double borderRadius = 0.0,
  }) {
    final resolved = resolveDrawListBytes(rawBytes);
    if (resolved == null) {
      return const NveDrawListResult.err(
        NveDrawListErrorCode.invalidBase64,
        'Invalid base64 NVE_DRAW1 payload',
      );
    }
    final check = validate(resolved);
    if (!check.isOk) {
      return check;
    }
    if (bounds.width <= 0.0 || bounds.height <= 0.0) {
      return check;
    }

    final view = ByteData.sublistView(resolved);
    final opCount = check.opCount;
    var pos = nveDrawHeaderSize;
    var clipDepth = 0;

    canvas.save();
    try {
      if (borderRadius > 0.0) {
        canvas.clipRRect(RRect.fromRectAndRadius(bounds, Radius.circular(borderRadius)));
      } else {
        canvas.clipRect(bounds);
      }
      canvas.translate(bounds.left, bounds.top);
      final localBounds = Rect.fromLTWH(0.0, 0.0, bounds.width, bounds.height);

      for (var i = 0; i < opCount; i++) {
        final opcode = resolved[pos++];
        switch (opcode) {
          case NveDrawOpcode.clear:
            final rgba = view.getUint32(pos, Endian.little);
            pos += 4;
            final alpha = (rgba >> 24) & 0xFF;
            if (alpha > 0) {
              _fillPaint.color = _colorFromWireRgba(rgba);
              canvas.drawRect(localBounds, _fillPaint);
            }
            break;

          case NveDrawOpcode.fillRect:
            final x = view.getFloat32(pos, Endian.little);
            final y = view.getFloat32(pos + 4, Endian.little);
            final w = view.getFloat32(pos + 8, Endian.little);
            final h = view.getFloat32(pos + 12, Endian.little);
            final rgba = view.getUint32(pos + 16, Endian.little);
            pos += 20;
            if (w > 0.0 && h > 0.0 && ((rgba >> 24) & 0xFF) > 0) {
              _fillPaint.color = _colorFromWireRgba(rgba);
              canvas.drawRect(Rect.fromLTWH(x, y, w, h), _fillPaint);
            }
            break;

          case NveDrawOpcode.fillRoundedRect:
            final x = view.getFloat32(pos, Endian.little);
            final y = view.getFloat32(pos + 4, Endian.little);
            final w = view.getFloat32(pos + 8, Endian.little);
            final h = view.getFloat32(pos + 12, Endian.little);
            final radius = math.max(0.0, view.getFloat32(pos + 16, Endian.little));
            final rgba = view.getUint32(pos + 20, Endian.little);
            pos += 24;
            if (w > 0.0 && h > 0.0 && ((rgba >> 24) & 0xFF) > 0) {
              _fillPaint.color = _colorFromWireRgba(rgba);
              final rect = Rect.fromLTWH(x, y, w, h);
              if (radius > 0.0) {
                canvas.drawRRect(
                  RRect.fromRectAndRadius(rect, Radius.circular(radius)),
                  _fillPaint,
                );
              } else {
                canvas.drawRect(rect, _fillPaint);
              }
            }
            break;

          case NveDrawOpcode.fillCircle:
            final cx = view.getFloat32(pos, Endian.little);
            final cy = view.getFloat32(pos + 4, Endian.little);
            final radius = view.getFloat32(pos + 8, Endian.little);
            final rgba = view.getUint32(pos + 12, Endian.little);
            pos += 16;
            if (radius > 0.0 && ((rgba >> 24) & 0xFF) > 0) {
              _fillPaint.color = _colorFromWireRgba(rgba);
              canvas.drawCircle(Offset(cx, cy), radius, _fillPaint);
            }
            break;

          case NveDrawOpcode.strokeLine:
            final x0 = view.getFloat32(pos, Endian.little);
            final y0 = view.getFloat32(pos + 4, Endian.little);
            final x1 = view.getFloat32(pos + 8, Endian.little);
            final y1 = view.getFloat32(pos + 12, Endian.little);
            final width = view.getFloat32(pos + 16, Endian.little);
            final rgba = view.getUint32(pos + 20, Endian.little);
            pos += 24;
            if (width > 0.0 && ((rgba >> 24) & 0xFF) > 0) {
              _strokePaint
                ..color = _colorFromWireRgba(rgba)
                ..strokeWidth = width;
              canvas.drawLine(Offset(x0, y0), Offset(x1, y1), _strokePaint);
            }
            break;

          case NveDrawOpcode.strokePolyline:
            final width = view.getFloat32(pos, Endian.little);
            final rgba = view.getUint32(pos + 4, Endian.little);
            final ptCount = view.getUint32(pos + 8, Endian.little);
            pos += 12;
            if (ptCount >= 2 && width > 0.0 && ((rgba >> 24) & 0xFF) > 0) {
              _polylinePath.reset();
              final firstX = view.getFloat32(pos, Endian.little);
              final firstY = view.getFloat32(pos + 4, Endian.little);
              _polylinePath.moveTo(firstX, firstY);
              for (var p = 1; p < ptCount; p++) {
                final px = view.getFloat32(pos + p * 8, Endian.little);
                final py = view.getFloat32(pos + p * 8 + 4, Endian.little);
                _polylinePath.lineTo(px, py);
              }
              _strokePaint
                ..color = _colorFromWireRgba(rgba)
                ..strokeWidth = width;
              canvas.drawPath(_polylinePath, _strokePaint);
            }
            pos += ptCount * 8;
            break;

          case NveDrawOpcode.clipRect:
            final x = view.getFloat32(pos, Endian.little);
            final y = view.getFloat32(pos + 4, Endian.little);
            final w = math.max(0.0, view.getFloat32(pos + 8, Endian.little));
            final h = math.max(0.0, view.getFloat32(pos + 12, Endian.little));
            pos += 16;
            if (clipDepth < nveDrawMaxClipDepth) {
              canvas.save();
              canvas.clipRect(Rect.fromLTWH(x, y, w, h));
              clipDepth += 1;
            }
            break;

          case NveDrawOpcode.restoreClip:
            if (clipDepth > 0) {
              canvas.restore();
              clipDepth -= 1;
            }
            break;
        }
      }
    } finally {
      while (clipDepth > 0) {
        canvas.restore();
        clipDepth -= 1;
      }
      canvas.restore();
    }

    return check;
  }

  static bool _hasMagic(Uint8List bytes) {
    for (var i = 0; i < nveDrawMagic.length; i++) {
      if (bytes[i] != nveDrawMagic[i]) return false;
    }
    return true;
  }

  static bool _startsWithAttrPrefix(Uint8List bytes) {
    for (var i = 0; i < _attrPrefixBytes.length; i++) {
      if (bytes[i] != _attrPrefixBytes[i]) return false;
    }
    return true;
  }

  static bool _areFinite4(ByteData view, int pos) {
    return view.getFloat32(pos, Endian.little).isFinite &&
        view.getFloat32(pos + 4, Endian.little).isFinite &&
        view.getFloat32(pos + 8, Endian.little).isFinite &&
        view.getFloat32(pos + 12, Endian.little).isFinite;
  }

  static Color _colorFromWireRgba(int rgba) {
    return Color.fromARGB(
      (rgba >> 24) & 0xFF,
      rgba & 0xFF,
      (rgba >> 8) & 0xFF,
      (rgba >> 16) & 0xFF,
    );
  }

  static Uint8List? _decodeBase64Clean(String b64) {
    try {
      final cleaned = b64.replaceAll(RegExp(r'\s+'), '');
      if (cleaned.isEmpty) return null;
      final normalized = base64.normalize(cleaned);
      return base64.decode(normalized);
    } catch (_) {
      return null;
    }
  }
}
