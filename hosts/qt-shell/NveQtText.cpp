#include "NveQtText.hpp"

#include "NveQtPaint.hpp"

#include <QFontMetricsF>
#include <QGuiApplication>
#include <QTextLayout>
#include <QTextOption>

#include <algorithm>
#include <cmath>
#include <limits>

namespace nve::qt {

namespace {

constexpr qreal kUnbounded = 1.0e7;

Qt::Alignment alignment(uint8_t text_align) noexcept {
  switch (text_align) {
    case NVE_TEXT_ALIGN_CENTER: return Qt::AlignHCenter;
    case NVE_TEXT_ALIGN_END:
    case NVE_TEXT_ALIGN_RIGHT: return Qt::AlignRight;
    default: return Qt::AlignLeft;
  }
}

}  // namespace

QFont make_font(float size, uint16_t weight, uint8_t font_style, std::string_view family, float letter_spacing,
                uint8_t decoration) {
  QFont font = QGuiApplication::instance() != nullptr ? QGuiApplication::font() : QFont();
  const float px = std::isfinite(size) && size > 0.0f ? size : 16.0f;
  font.setPixelSize(std::max(1, static_cast<int>(std::lround(px))));
  font.setWeight(static_cast<QFont::Weight>(std::clamp<int>(weight, 1, 1000)));
  font.setStyle(font_style == NVE_FONT_STYLE_ITALIC    ? QFont::StyleItalic
                : font_style == NVE_FONT_STYLE_OBLIQUE ? QFont::StyleOblique
                                                       : QFont::StyleNormal);
  if (!family.empty()) {
    font.setFamilies({QString::fromUtf8(family.data(), static_cast<qsizetype>(family.size()))});
  }
  if (std::isfinite(letter_spacing) && letter_spacing != 0.0f) {
    font.setLetterSpacing(QFont::AbsoluteSpacing, static_cast<qreal>(letter_spacing));
  }
  font.setUnderline((decoration & NVE_TEXT_DECORATION_UNDERLINE) != 0u);
  font.setOverline((decoration & NVE_TEXT_DECORATION_OVERLINE) != 0u);
  font.setStrikeOut((decoration & NVE_TEXT_DECORATION_LINE_THROUGH) != 0u);
  return font;
}

QtTextStyle text_style(const host::TextStyle& s) {
  QtTextStyle out;
  out.font = make_font(s.font_size, s.font_weight, s.font_style, s.font_family, s.letter_spacing, s.text_decoration);
  out.color = qcolor(s.color);
  out.align = alignment(s.text_align);
  out.justify = s.text_align == NVE_TEXT_ALIGN_JUSTIFY;
  out.wrap = s.white_space != NVE_WHITE_SPACE_NO_WRAP && s.white_space != NVE_WHITE_SPACE_PRE;
  out.max_lines = static_cast<int>(std::min<uint32_t>(s.max_lines, static_cast<uint32_t>(std::numeric_limits<int>::max())));
  const float font_px = std::isfinite(s.font_size) && s.font_size > 0.0f ? s.font_size : 16.0f;
  if (s.line_height.unit == NVE_LENGTH_POINTS && s.line_height.value > 0.0f) {
    out.line_height = static_cast<qreal>(s.line_height.value);
  } else if (s.line_height.unit == NVE_LENGTH_PERCENT && s.line_height.value > 0.0f) {
    out.line_height = static_cast<qreal>(font_px * s.line_height.value / 100.0f);
  }
  return out;
}

QString scene_string(std::string_view utf8, bool keep_newlines) {
  QString text = QString::fromUtf8(utf8.data(), static_cast<qsizetype>(utf8.size()));
  text.replace(QStringLiteral("\r\n"), QStringLiteral("\n"));
  qsizetype out = 0;
  for (const QChar c : std::as_const(text)) {
    const char16_t u = c.unicode();
    if (u == u'\n') {
      if (keep_newlines) text[out++] = c;
      else text[out++] = QChar(u' ');
    } else if (u == u'\t') {
      text[out++] = QChar(u' ');
    } else if (u >= 0x20u && !(u >= 0x7Fu && u <= 0x9Fu)) {
      text[out++] = c;
    }
  }
  text.truncate(out);
  return text;
}

QString display_text(std::string_view utf8) {
  QString text = scene_string(utf8, true);
  text.replace(QChar(u'\n'), QChar::LineSeparator);
  return text;
}

QSizeF layout_text(const QString& text, const QtTextStyle& style, qreal width, QPainter* painter, QPointF origin) {
  if (text.isEmpty()) return QSizeF(0.0, 0.0);
  const bool bounded = std::isfinite(width) && width > 0.0;
  QTextLayout layout(text, style.font);
  QTextOption option;
  option.setWrapMode(style.wrap && bounded ? QTextOption::WrapAtWordBoundaryOrAnywhere : QTextOption::NoWrap);
  option.setAlignment(style.justify ? Qt::AlignJustify : style.align);
  layout.setTextOption(option);

  qreal y = 0.0;
  qreal widest = 0.0;
  int visible = 0;
  bool truncated = false;
  layout.beginLayout();
  for (QTextLine line = layout.createLine(); line.isValid(); line = layout.createLine()) {
    line.setLineWidth(bounded ? width : kUnbounded);
    if (style.max_lines > 0 && visible == style.max_lines) {
      truncated = true;
      break;
    }
    const qreal natural = line.height();
    const qreal box = style.line_height > 0.0 ? style.line_height : natural;
    line.setPosition(QPointF(0.0, y + (box - natural) * 0.5));  // CSS half-leading
    widest = std::max(widest, line.naturalTextWidth());
    y += box;
    ++visible;
  }
  layout.endLayout();

  if (painter != nullptr) {
    painter->save();
    painter->setFont(style.font);
    painter->setPen(style.color);
    const int plain = truncated ? visible - 1 : visible;
    for (int i = 0; i < plain; ++i) layout.lineAt(i).draw(painter, origin);
    if (truncated && visible > 0) {
      // The last visible line carries the ellipsis for everything after it.
      const QTextLine last = layout.lineAt(visible - 1);
      QString rest = text.mid(last.textStart());
      rest.replace(QChar::LineSeparator, QChar(' '));
      const QFontMetricsF fm(style.font);
      const qreal avail = bounded ? width : last.naturalTextWidth();
      const QString elided = fm.elidedText(rest, Qt::ElideRight, avail);
      const qreal advance = fm.horizontalAdvance(elided);
      const qreal dx = (style.align & Qt::AlignHCenter) ? (avail - advance) * 0.5
                       : (style.align & Qt::AlignRight) ? avail - advance
                                                        : 0.0;
      painter->drawText(QPointF(origin.x() + dx, origin.y() + last.y() + last.ascent()), elided);
    }
    painter->restore();
  }
  return QSizeF(widest, y);
}

NveIntrinsicSize measure_text(std::string_view text, float font_size, uint16_t font_weight, float max_width) {
  if (text.empty()) return NveIntrinsicSize{0.0f, 0.0f};
  QtTextStyle style;
  style.font = make_font(font_size, font_weight);
  const QSizeF size = layout_text(display_text(text), style, std::isfinite(max_width) ? max_width : 0.0f);
  return NveIntrinsicSize{static_cast<float>(std::ceil(size.width())), static_cast<float>(std::ceil(size.height()))};
}

void measure_text_callback(void*, NveStrSlice text, float font_size, uint32_t font_weight, float max_width,
                           float* out_width, float* out_height) {
  const std::string_view sv(reinterpret_cast<const char*>(text.ptr), text.ptr != nullptr ? text.len : 0u);
  const NveIntrinsicSize size =
      measure_text(sv, font_size, static_cast<uint16_t>(std::min<uint32_t>(font_weight, 1000u)), max_width);
  if (out_width != nullptr) *out_width = size.width;
  if (out_height != nullptr) *out_height = size.height;
}

}  // namespace nve::qt
