/// Typography resolution and color conversion over a published runtime scene: one inherited
/// [NveTextStyle] per node, following the host rule (`nve::host::inherit_text_style`): a
/// `Typography` field applies exactly when its `__explicit` override bit is on; otherwise the
/// parent's value flows down, whatever the row's default bits hold.
library;

import 'package:flutter/painting.dart';
import 'package:nve_host/nve_host.dart';

/// Opaque black: the default text color of every backend.
const int nveDefaultTextColor = 0xFF000000;

/// Flutter [Color] of a packed `r | g << 8 | b << 16 | a << 24` word.
Color nveColor(int packed) =>
    Color.fromARGB((packed >>> 24) & 0xFF, packed & 0xFF, (packed >>> 8) & 0xFF, (packed >>> 16) & 0xFF);

/// Inherited text attributes of one node.
final class NveTextStyle {
  const NveTextStyle({
    this.color = nveDefaultTextColor,
    this.fontSize = 16,
    this.fontWeight = 400,
    this.italic = false,
    this.textAlign = NveTextAlign.start,
    this.fontFamily,
    this.letterSpacing,
    this.lineHeight,
    this.maxLines,
    this.decoration = 0,
  });

  /// Packed RGBA.
  final int color;
  final double fontSize;
  final int fontWeight;
  final bool italic;

  /// `NveTextAlign` word value.
  final int textAlign;
  final String? fontFamily;
  final double? letterSpacing;

  /// Line height in points (percent lengths resolve against [fontSize]).
  final double? lineHeight;
  final int? maxLines;

  /// `NveTextDecoration` bits.
  final int decoration;

  /// This style overridden by node [index]'s `Typography` row in [table] (if any).
  NveTextStyle resolve(SceneReader s, TableReader? table, int index) {
    final row = s.rowOf(table, index);
    if (row < 0) {
      return this;
    }
    final t = table!;
    final mask = t.explicitMask(row);
    if (mask == 0) {
      return this;
    }
    bool has(FieldSpec f) => f.isExplicitIn(mask);
    final rawSize = has(TypographyTable.fontSize) ? t.f32(row, TypographyTable.fontSize) : fontSize;
    final size = rawSize.isFinite && rawSize > 0 ? rawSize : fontSize;
    final family = has(TypographyTable.fontFamily) ? s.str(t, row, TypographyTable.fontFamily) : fontFamily;
    return NveTextStyle(
      color: has(TypographyTable.color) ? t.color(row, TypographyTable.color).packed : color,
      fontSize: size,
      fontWeight: has(TypographyTable.fontWeight) ? t.u32(row, TypographyTable.fontWeight) : fontWeight,
      italic: has(TypographyTable.fontStyle)
          ? t.enumValue(row, TypographyTable.fontStyle) != NveFontStyle.normal
          : italic,
      textAlign: has(TypographyTable.textAlign) ? t.enumValue(row, TypographyTable.textAlign) : textAlign,
      fontFamily: family == null || family.isEmpty ? null : family,
      letterSpacing: has(TypographyTable.letterSpacing) ? _finite(t.f32(row, TypographyTable.letterSpacing)) : letterSpacing,
      // The `LengthValue` is decoded only when the row sets a line height.
      lineHeight: has(TypographyTable.lineHeight) ? _lineHeight(t.length(row, TypographyTable.lineHeight), size) : lineHeight,
      maxLines: has(TypographyTable.maxLines) ? t.optU32(row, TypographyTable.maxLines) : maxLines,
      decoration: has(TypographyTable.textDecoration) ? t.u32(row, TypographyTable.textDecoration) : decoration,
    );
  }

  /// `null` (inherit Flutter's default) for a NaN / infinite wire value.
  static double? _finite(double v) => v.isFinite ? v : null;

  static double? _lineHeight(LengthValue lh, double fontSize) => switch (lh.unit) {
        LengthUnit.points => lh.value.isFinite && lh.value > 0 ? lh.value : null,
        LengthUnit.percent => lh.value.isFinite && lh.value > 0 ? fontSize * lh.value / 100 : null,
        LengthUnit.auto => null,
      };

  /// Flutter text style (optionally with another [colorOverride]).
  TextStyle toTextStyle({int? colorOverride}) => TextStyle(
        color: nveColor(colorOverride ?? color),
        fontSize: fontSize,
        fontWeight: FontWeight.values[((fontWeight.clamp(100, 900) + 50) ~/ 100) - 1],
        fontStyle: italic ? FontStyle.italic : FontStyle.normal,
        fontFamily: fontFamily,
        letterSpacing: letterSpacing,
        height: lineHeight != null && fontSize > 0 ? lineHeight! / fontSize : null,
        decoration: TextDecoration.combine([
          if (decoration & NveTextDecoration.underline != 0) TextDecoration.underline,
          if (decoration & NveTextDecoration.overline != 0) TextDecoration.overline,
          if (decoration & NveTextDecoration.lineThrough != 0) TextDecoration.lineThrough,
        ]),
      );

  @override
  bool operator ==(Object other) =>
      other is NveTextStyle &&
      other.color == color &&
      other.fontSize == fontSize &&
      other.fontWeight == fontWeight &&
      other.italic == italic &&
      other.textAlign == textAlign &&
      other.fontFamily == fontFamily &&
      other.letterSpacing == letterSpacing &&
      other.lineHeight == lineHeight &&
      other.maxLines == maxLines &&
      other.decoration == decoration;

  @override
  int get hashCode => Object.hash(
      color, fontSize, fontWeight, italic, textAlign, fontFamily, letterSpacing, lineHeight, maxLines, decoration);

  TextAlign get flutterAlign => switch (textAlign) {
        NveTextAlign.end => TextAlign.end,
        NveTextAlign.left => TextAlign.left,
        NveTextAlign.right => TextAlign.right,
        NveTextAlign.center => TextAlign.center,
        NveTextAlign.justify => TextAlign.justify,
        _ => TextAlign.start,
      };
}
