/// Paints a published `libnve_runtime` scene onto a Flutter [Canvas] straight from its tables:
/// `Visual` (fill, border, radius, opacity, visibility, overflow clip, z-order), `Transform`,
/// inherited `Typography`, `TextContent`, `TextInput`, `Image`, scroll shifts, and module tables
/// through [NveTablePainter]s (the canvas draw list ships built in).
library;

import 'dart:math' as math;
import 'dart:typed_data';

import 'package:flutter/painting.dart';
import 'package:nve_host/nve_host.dart';

import 'draw_list_painter.dart';
import 'scene_image_cache.dart';
import 'scene_resources.dart';
import 'scene_style.dart';
import 'scene_tree.dart';

/// Everything a [NveTablePainter] may use while painting one node.
final class NvePaintContext {
  NvePaintContext._(this.canvas, this.tree, this.resources);

  final Canvas canvas;
  final NveSceneTree tree;
  final NveSceneResources? resources;
}

/// Paints the leaf content of nodes carrying a module table (keyed by its `Key64`).
abstract interface class NveTablePainter {
  Key64 get tableKey;
  void paint(NvePaintContext ctx, TableReader table, int row, int index, Rect rect, double radius);
}

/// `nve.canvas` (`packages/modules/canvas`): `draw_list: Res<DrawList>` (`NVE_DRAW1` payload).
final class NveCanvasTablePainter implements NveTablePainter {
  NveCanvasTablePainter([NveDrawListPainter? painter]) : _painter = painter ?? NveDrawListPainter();

  static final Key64 key = constId('nve.canvas');
  static final FieldSpec drawList = FieldSpec(0, FieldKind.resource);

  final NveDrawListPainter _painter;

  @override
  Key64 get tableKey => key;

  @override
  void paint(NvePaintContext ctx, TableReader table, int row, int index, Rect rect, double radius) {
    final bytes = ctx.resources?.payload(table.resource(row, drawList));
    if (bytes != null && bytes.isNotEmpty) {
      _painter.paint(ctx.canvas, rect, bytes, borderRadius: radius);
    }
  }
}

/// Retained-free scene painter; keeps only a per-paint text layout cache and the image cache.
final class NveScenePainter {
  NveScenePainter({NveSceneImageCache? imageCache, List<NveTablePainter>? tablePainters})
      : imageCache = imageCache ?? NveSceneImageCache(),
        tablePainters = tablePainters ?? <NveTablePainter>[NveCanvasTablePainter()];

  static const int _placeholderColor = 0xB4B8A394; // rgba(148, 163, 184, 180)

  final NveSceneImageCache imageCache;
  final List<NveTablePainter> tablePainters;
  final Map<_TextKey, TextPainter> _text = <_TextKey, TextPainter>{};
  final Set<_TextKey> _usedText = <_TextKey>{};

  late Canvas _canvas;
  late NveSceneTree _tree;
  late Offset _origin;
  late NvePaintContext _ctx;
  NveSceneResources? _resources;
  VoidCallback? _onImage;
  TableReader? _typo, _textTable, _input, _image, _transform;
  final List<(NveTablePainter, TableReader)> _modules = <(NveTablePainter, TableReader)>[];

  /// Paints [tree] with the surface origin at [origin].
  void paint(Canvas canvas, NveSceneTree tree, Offset origin,
      {NveSceneResources? resources, String? defaultFontFamily, VoidCallback? onImageLoaded}) {
    final root = tree.root;
    if (root < 0) {
      return;
    }
    final s = tree.s;
    _canvas = canvas;
    _tree = tree;
    _origin = origin;
    _resources = resources;
    _onImage = onImageLoaded;
    _ctx = NvePaintContext._(canvas, tree, resources);
    _typo = s.findTable(NveTypographyTable.key);
    _textTable = s.findTable(NveTextTable.key);
    _input = s.findTable(NveTextInputTable.key);
    _image = s.findTable(NveImageTable.key);
    _transform = s.findTable(NveTransformTable.key);
    _modules
      ..clear()
      ..addAll([
        for (final p in tablePainters)
          if (s.findTable(p.tableKey) case final t?) (p, t)
      ]);
    _usedText.clear();
    _node(root, Offset.zero, false, NveTextStyle(fontFamily: defaultFontFamily));
    _text.removeWhere((k, p) {
      final stale = !_usedText.contains(k);
      if (stale) {
        p.dispose();
      }
      return stale;
    });
    _resources = null;
    _onImage = null;
  }

  void _node(int index, Offset shift, bool parentHidden, NveTextStyle inherited) {
    final t = _tree, s = t.s;
    if (t.isDisplayNone(index)) {
      return;
    }
    final v = t.visual;
    final vr = s.rowOf(v, index);
    final hidden = vr < 0
        ? parentHidden
        : switch (v!.enumValue(vr, NveVisualTable.visibility)) {
            NveVisibility.visible => false,
            NveVisibility.hidden => true,
            _ => parentHidden,
          };
    final opacity = vr < 0 ? 1.0 : v!.f32(vr, NveVisualTable.opacity).clamp(0.0, 1.0);
    if (opacity <= 0.001) {
      return;
    }
    final rect = t.rectOf(index).shift(_origin - shift);
    var saves = 0;
    if (opacity < 0.999) {
      _canvas.saveLayer(null, Paint()..color = Color.fromRGBO(0, 0, 0, opacity));
      saves++;
    }
    final tr = s.rowOf(_transform, index);
    if (tr >= 0) {
      _canvas
        ..save()
        ..transform(_matrix(_transform!, tr, rect));
      saves++;
    }
    final radius = vr < 0 ? 0.0 : math.max(0.0, v!.f32(vr, NveVisualTable.borderRadius));
    final rrect = RRect.fromRectAndRadius(rect, Radius.circular(radius));
    final style = inherited.resolve(s, _typo, index);
    if (!hidden && rect.width > 0 && rect.height > 0) {
      if (vr >= 0) {
        _box(v!, vr, rect, rrect, radius);
      }
      _leaf(index, rect, rrect, radius, style);
    }
    final kids = t.paintOrder(index);
    if (kids.isNotEmpty) {
      final clip = t.clipsChildren(index);
      if (clip) {
        _canvas
          ..save()
          ..clipRRect(rrect);
      }
      final childShift = shift + t.scrollShift(index);
      for (final kid in kids) {
        _node(kid, childShift, hidden, style);
      }
      if (clip) {
        _canvas.restore();
      }
    }
    for (; saves > 0; saves--) {
      _canvas.restore();
    }
  }

  Float64List _matrix(TableReader tf, int row, Rect rect) {
    final ox = rect.left + rect.width * tf.f32(row, NveTransformTable.originX);
    final oy = rect.top + rect.height * tf.f32(row, NveTransformTable.originY);
    final rad = tf.f32(row, NveTransformTable.rotateDeg) * math.pi / 180;
    final sx = tf.f32(row, NveTransformTable.scaleX), sy = tf.f32(row, NveTransformTable.scaleY);
    final c = math.cos(rad), n = math.sin(rad);
    // translate(t + origin) · rotate · scale · translate(-origin), column-major 4×4.
    final tx = tf.f32(row, NveTransformTable.translateX) + ox, ty = tf.f32(row, NveTransformTable.translateY) + oy;
    final a = c * sx, b = n * sx, cc = -n * sy, d = c * sy;
    return Float64List.fromList(<double>[
      a, b, 0, 0, //
      cc, d, 0, 0, //
      0, 0, 1, 0, //
      tx - a * ox - cc * oy, ty - b * ox - d * oy, 0, 1,
    ]);
  }

  void _box(TableReader v, int row, Rect rect, RRect rrect, double radius) {
    final bg = v.color(row, NveVisualTable.background).packed;
    if ((bg >>> 24) != 0) {
      _canvas.drawRRect(rrect, Paint()..color = nveColor(bg));
    }
    final bw = v.f32(row, NveVisualTable.borderWidth);
    final bc = v.color(row, NveVisualTable.borderColor).packed;
    if (bw > 0 && (bc >>> 24) != 0) {
      final inset = rect.width > bw && rect.height > bw ? bw / 2 : 0.0;
      _canvas.drawRRect(
        RRect.fromRectAndRadius(rect.deflate(inset), Radius.circular(math.max(0.0, radius - inset))),
        Paint()
          ..style = PaintingStyle.stroke
          ..strokeWidth = bw
          ..color = nveColor(bc),
      );
    }
  }

  void _leaf(int index, Rect rect, RRect rrect, double radius, NveTextStyle style) {
    final s = _tree.s;
    final ir = s.rowOf(_image, index);
    if (ir >= 0) {
      _paintImage(_image!, ir, rect, rrect);
    }
    final inputRow = s.rowOf(_input, index);
    if (inputRow >= 0) {
      _paintInput(_input!, inputRow, index, rect, style);
    } else if (s.rowOf(_textTable, index) case final tr when tr >= 0) {
      final text = s.str(_textTable!, tr, NveTextTable.text);
      if (text.isNotEmpty && style.fontSize > 0 && (style.color >>> 24) != 0) {
        _layout(text, style, rect.width, style.maxLines).paint(_canvas, rect.topLeft);
      }
    }
    for (final (painter, table) in _modules) {
      final row = s.rowOf(table, index);
      if (row >= 0) {
        painter.paint(_ctx, table, row, index, rect, radius);
      }
    }
  }

  void _paintInput(TableReader t, int row, int index, Rect rect, NveTextStyle style) {
    final s = _tree.s;
    final value = s.str(t, row, NveTextInputTable.value);
    final secure = t.boolean(row, NveTextInputTable.secure);
    final multiline = t.boolean(row, NveTextInputTable.multiline);
    final shown = value.isEmpty ? s.str(t, row, NveTextInputTable.placeholder) : (secure ? '•' * value.runes.length : value);
    if (shown.isEmpty) {
      return;
    }
    final inner = _inputInnerRect(index, rect, style.fontSize);
    final p = _layout(shown, style, inner.width, multiline ? null : 1,
        colorOverride: value.isEmpty ? _placeholderColor : null);
    _canvas
      ..save()
      ..clipRect(rect)
      ..translate(inner.left, inner.top);
    p.paint(_canvas, Offset.zero);
    _canvas.restore();
  }

  /// Content box of a text input: `Layout` point paddings, else vertically centered (matches the
  /// native backends' `compute_text_input_inner_rect`).
  Rect _inputInnerRect(int index, Rect rect, double fontSize) {
    final layout = _tree.layout;
    final row = _tree.s.rowOf(layout, index);
    double pad(FieldSpec f) {
      if (row < 0) {
        return 0;
      }
      final l = layout!.length(row, f);
      return l.unit == LengthUnit.points ? math.max(0.0, l.value) : 0.0;
    }

    final left = pad(NveLayoutTable.paddingLeft), right = pad(NveLayoutTable.paddingRight);
    var top = pad(NveLayoutTable.paddingTop);
    final bottom = pad(NveLayoutTable.paddingBottom);
    if (top == 0 && bottom == 0 && rect.height > fontSize * 1.4) {
      top = math.max(0.0, (rect.height - fontSize * 1.35) / 2);
    }
    return Rect.fromLTWH(rect.left + left, rect.top + top, math.max(1.0, rect.width - left - right),
        math.max(1.0, rect.height - top - bottom));
  }

  void _paintImage(TableReader t, int row, Rect rect, RRect rrect) {
    final payload = _resources?.payload(t.resource(row, NveImageTable.src));
    final uri = payload == null ? null : decodeImageSourceUri(payload);
    final img = uri == null ? null : imageCache.resolveOrRequest(uri, onDecoded: _onImage);
    if (img == null) {
      return;
    }
    final w = img.width.toDouble(), h = img.height.toDouble();
    final mode = switch (t.enumValue(row, NveImageTable.fit)) {
      NveImageFit.contain => NveImageResizeMode.contain,
      NveImageFit.fill => NveImageResizeMode.stretch,
      NveImageFit.none => NveImageResizeMode.center,
      NveImageFit.scaleDown => w > rect.width || h > rect.height ? NveImageResizeMode.contain : NveImageResizeMode.center,
      _ => NveImageResizeMode.cover,
    };
    final placement = NveImagePlacement.compute(rect, w, h, mode);
    if (placement == null) {
      return;
    }
    final paint = Paint()
      ..filterQuality = FilterQuality.medium
      ..isAntiAlias = true;
    if (t.boolean(row, NveImageTable.tintEnabled)) {
      paint.colorFilter = ColorFilter.mode(nveColor(t.color(row, NveImageTable.tint).packed), BlendMode.srcIn);
    }
    _canvas
      ..save()
      ..clipRRect(rrect)
      ..drawImageRect(img, placement.srcRect, placement.dstRect, paint)
      ..restore();
  }

  TextPainter _layout(String text, NveTextStyle style, double width, int? maxLines, {int? colorOverride}) {
    // Runtime text measurement and Flutter shaping can differ by sub-pixel amounts: half a point of
    // slack keeps a line the runtime measured as fitting from wrapping here.
    final key = _TextKey(text, style, width, maxLines, colorOverride);
    _usedText.add(key);
    return _text[key] ??= TextPainter(
      text: TextSpan(text: text, style: style.toTextStyle(colorOverride: colorOverride)),
      textAlign: style.flutterAlign,
      textDirection: TextDirection.ltr,
      maxLines: maxLines == null || maxLines == 0 ? null : maxLines,
      ellipsis: maxLines == null || maxLines == 0 ? null : '…',
    )..layout(maxWidth: math.max(0.0, width + 0.5));
  }

  /// Releases cached text layouts and decoded images.
  void dispose() {
    for (final p in _text.values) {
      p.dispose();
    }
    _text.clear();
    imageCache.dispose();
  }
}

final class _TextKey {
  _TextKey(this.text, this.style, this.width, this.maxLines, this.color);

  final String text;
  final NveTextStyle style;
  final double width;
  final int? maxLines;
  final int? color;

  @override
  bool operator ==(Object other) =>
      other is _TextKey &&
      other.text == text &&
      other.width == width &&
      other.maxLines == maxLines &&
      other.color == color &&
      other.style == style;

  @override
  int get hashCode => Object.hash(text, width, maxLines, color, style);
}
