/// Typography resolution and color conversion over a published runtime scene: one inherited
/// [NveTextStyle] per node, following the native backends' rule (a `Typography` field applies when
/// its `set` bit is on or it differs from the default; otherwise the parent's value flows down).
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

  /// `TypographyTable.decoration*` bits.
  final int decoration;

  /// This style overridden by node [index]'s `Typography` row in [table] (if any).
  NveTextStyle resolve(RuntimeSceneReader s, TableReader? table, int index) {
    final row = s.rowOf(table, index);
    if (row < 0) {
      return this;
    }
    final t = table!;
    final set = t.u32(row, TypographyTable.set);
    bool has(int bit) => set & bit != 0;
    final rawColor = t.color(row, TypographyTable.color).packed;
    final rawSize = t.f32(row, TypographyTable.fontSize);
    final rawWeight = t.u32(row, TypographyTable.fontWeight);
    final size = has(NveTypographySet.fontSize) || (rawSize - 16).abs() > 1e-4 ? rawSize : fontSize;
    final lh = t.length(row, TypographyTable.lineHeight);
    final family = has(NveTypographySet.fontFamily) ? s.str(t, row, TypographyTable.fontFamily) : fontFamily;
    return NveTextStyle(
      color: has(NveTypographySet.color) || rawColor != nveDefaultTextColor ? rawColor : color,
      fontSize: size,
      fontWeight: has(NveTypographySet.fontWeight) || rawWeight != 400 ? rawWeight : fontWeight,
      italic: has(NveTypographySet.fontStyle)
          ? t.enumValue(row, TypographyTable.fontStyle) != NveFontStyle.normal
          : italic,
      textAlign: has(NveTypographySet.textAlign) || t.enumValue(row, TypographyTable.textAlign) != NveTextAlign.start
          ? t.enumValue(row, TypographyTable.textAlign)
          : textAlign,
      fontFamily: family == null || family.isEmpty ? null : family,
      letterSpacing: has(NveTypographySet.letterSpacing) ? t.f32(row, TypographyTable.letterSpacing) : letterSpacing,
      lineHeight: has(NveTypographySet.lineHeight)
          ? switch (lh.unit) {
              LengthUnit.points => lh.value,
              LengthUnit.percent => size * lh.value / 100,
              LengthUnit.auto => null,
            }
          : lineHeight,
      maxLines: has(NveTypographySet.maxLines) ? t.u32(row, TypographyTable.maxLines) : maxLines,
      decoration: has(NveTypographySet.textDecoration) ? t.u32(row, TypographyTable.textDecoration) : decoration,
    );
  }

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
