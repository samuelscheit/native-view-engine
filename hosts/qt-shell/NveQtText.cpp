#include "NveQtText.hpp"

#include "NveQtPaint.hpp"

#include <nve/host/font_family.hpp>
#include <nve/host/runtime_session.hpp>

#include <QFontInfo>
#include <QFontMetricsF>
#include <QGuiApplication>
#include <QStringList>
#include <QTextOption>

#include <algorithm>
#include <cmath>
#include <iterator>
#include <limits>
#include <utility>

namespace nve::qt {

namespace {

constexpr qreal kUnbounded = 1.0e7;

[[nodiscard]] float font_px(float size) noexcept { return std::isfinite(size) && size > 0.0f ? size : 16.0f; }

/// The installed family a CSS generic (`monospace`, `serif`, ...) resolves to through Qt's style
/// hints, or empty for a named family. `setFamilies` does not resolve generics (nor the alias
/// `QFontDatabase::systemFont` may report); resolved once per generic on the GUI thread.
[[nodiscard]] QString generic_family(host::GenericFamily generic) {
  struct Generic {
    std::string_view css;
    QFont::StyleHint hint;
  };
  // Indexed by `host::GenericFamily`.
  static constexpr Generic kGenerics[] = {
      {"", QFont::AnyStyle},          {"serif", QFont::Serif},     {"sans-serif", QFont::SansSerif},
      {"monospace", QFont::Monospace}, {"cursive", QFont::Cursive}, {"fantasy", QFont::Fantasy},
      {"system-ui", QFont::System},
  };
  static_assert(std::size(kGenerics) == static_cast<size_t>(host::GenericFamily::SystemUi) + 1u);
  static QString resolved[std::size(kGenerics)];
  const auto i = static_cast<size_t>(generic);
  if (generic == host::GenericFamily::None || i >= std::size(kGenerics)) return {};
  if (resolved[i].isEmpty()) {
    QFont probe(QString::fromLatin1(kGenerics[i].css.data(), static_cast<qsizetype>(kGenerics[i].css.size())));
    probe.setStyleHint(kGenerics[i].hint);
    resolved[i] = QFontInfo(probe).family();
  }
  return resolved[i];
}

/// Wrapping, line clamp and absolute line box shared by the painted and measured styles.
[[nodiscard]] QtTextStyle paragraph(float font_size, host::TextWrap wrap, uint32_t max_lines,
                                    uint32_t line_height_unit, float line_height) {
  QtTextStyle out;
  out.wrap = wrap;
  out.max_lines = static_cast<int>(std::min<uint32_t>(max_lines, static_cast<uint32_t>(std::numeric_limits<int>::max())));
  out.line_height = static_cast<qreal>(host::line_height_points(line_height_unit, line_height, font_px(font_size)));
  return out;
}

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
  font.setPixelSize(std::max(1, static_cast<int>(std::lround(std::min(font_px(size), 1.0e6f)))));
  font.setWeight(static_cast<QFont::Weight>(std::clamp<int>(weight, 1, 1000)));
  font.setStyle(font_style == NVE_FONT_STYLE_ITALIC    ? QFont::StyleItalic
                : font_style == NVE_FONT_STYLE_OBLIQUE ? QFont::StyleOblique
                                                       : QFont::StyleNormal);
  QStringList families;
  host::for_each_font_family(family, [&](std::string_view name, bool quoted) {
    // A quoted "serif" is a family name; unquoted generics match ASCII case-insensitively.
    QString family = quoted ? QString() : generic_family(host::generic_font_family(name));
    families.push_back(!family.isEmpty() ? std::move(family)
                                         : QString::fromUtf8(name.data(), static_cast<qsizetype>(name.size())));
    return true;
  });
  if (!families.isEmpty()) font.setFamilies(families);
  if (std::isfinite(letter_spacing) && letter_spacing != 0.0f) {
    font.setLetterSpacing(QFont::AbsoluteSpacing, static_cast<qreal>(letter_spacing));
  }
  font.setUnderline((decoration & NVE_TEXT_DECORATION_UNDERLINE) != 0u);
  font.setOverline((decoration & NVE_TEXT_DECORATION_OVERLINE) != 0u);
  font.setStrikeOut((decoration & NVE_TEXT_DECORATION_LINE_THROUGH) != 0u);
  return font;
}

QtTextStyle text_style(const host::TextStyle& s) {
  QtTextStyle out = paragraph(s.font_size, s.wrap(), s.max_lines, static_cast<uint32_t>(s.line_height.unit),
                              s.line_height.value);
  out.font = make_font(s.font_size, s.font_weight, s.font_style, s.font_family, s.letter_spacing, s.text_decoration);
  out.color = qcolor(s.color);
  out.align = alignment(s.text_align);
  out.justify = s.text_align == NVE_TEXT_ALIGN_JUSTIFY;
  return out;
}

QtTextStyle text_style(const NveRuntimeTextStyle& s) {
  QtTextStyle out = paragraph(s.font_size, host::text_wrap(s.white_space, s.word_break, s.overflow_wrap), s.max_lines,
                              s.line_height_unit, s.line_height);
  out.font = make_font(s.font_size, s.font_weight, s.font_style, host::str_view(s.font_family), s.letter_spacing);
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

namespace {

[[nodiscard]] bool bounded_width(qreal width) noexcept { return std::isfinite(width) && width >= 0.0; }

/// `QTextOption` wrap mode of a resolved `TextWrap` (`Word`: an over-long word overflows, so
/// min-content is the longest word).
[[nodiscard]] QTextOption::WrapMode qt_wrap(host::TextWrap wrap) noexcept {
  switch (wrap) {
    case host::TextWrap::None: return QTextOption::NoWrap;
    case host::TextWrap::Word: return QTextOption::WordWrap;
    case host::TextWrap::Anywhere: return QTextOption::WrapAnywhere;
    case host::TextWrap::WordOrAnywhere: return QTextOption::WrapAtWordBoundaryOrAnywhere;
  }
  return QTextOption::WordWrap;
}

struct LaidOut {
  QSizeF size;
  int visible = 0;
  bool truncated = false;
};

/// Lays `layout` (holding the text) out at `width` with `style`.
LaidOut lay_out(QTextLayout& layout, const QtTextStyle& style, qreal width) {
  const bool bounded = bounded_width(width);
  layout.setFont(style.font);
  QTextOption option;
  option.setWrapMode(bounded ? qt_wrap(style.wrap) : QTextOption::NoWrap);
  option.setAlignment(style.justify ? Qt::AlignJustify : style.align);
  layout.setTextOption(option);

  LaidOut out;
  qreal y = 0.0;
  qreal widest = 0.0;
  layout.beginLayout();
  for (QTextLine line = layout.createLine(); line.isValid(); line = layout.createLine()) {
    line.setLineWidth(bounded ? width : kUnbounded);
    if (style.max_lines > 0 && out.visible == style.max_lines) {
      out.truncated = true;
      break;
    }
    const qreal natural = line.height();
    const qreal box = style.line_height > 0.0 ? style.line_height : natural;
    line.setPosition(QPointF(0.0, y + (box - natural) * 0.5));  // CSS half-leading
    widest = std::max(widest, line.naturalTextWidth());
    y += box;
    ++out.visible;
  }
  layout.endLayout();
  out.size = QSizeF(widest, y);
  return out;
}

/// Paints laid-out `layout` (of `text`) with its top-left at `origin`.
void draw_laid_out(const QTextLayout& layout, const LaidOut& laid, const QString& text, const QtTextStyle& style,
                   qreal width, QPainter& p, QPointF origin) {
  p.save();
  p.setFont(style.font);
  p.setPen(style.color);
  const int plain = laid.truncated ? laid.visible - 1 : laid.visible;
  for (int i = 0; i < plain; ++i) layout.lineAt(i).draw(&p, origin);
  if (laid.truncated && laid.visible > 0) {
    // The last visible line carries the ellipsis for everything after it.
    const QTextLine last = layout.lineAt(laid.visible - 1);
    QString rest = text.mid(last.textStart());
    rest.replace(QChar::LineSeparator, QChar(' '));
    const QFontMetricsF fm(style.font);
    const qreal avail = bounded_width(width) ? width : last.naturalTextWidth();
    const QString elided = fm.elidedText(rest, Qt::ElideRight, avail);
    const qreal advance = fm.horizontalAdvance(elided);
    const qreal dx = (style.align & Qt::AlignHCenter) ? (avail - advance) * 0.5
                     : (style.align & Qt::AlignRight) ? avail - advance
                                                      : 0.0;
    p.drawText(QPointF(origin.x() + dx, origin.y() + last.y() + last.ascent()), elided);
  }
  p.restore();
}

}  // namespace

QSizeF layout_text(const QString& text, const QtTextStyle& style, qreal width, QPainter* painter, QPointF origin) {
  if (text.isEmpty()) return QSizeF(0.0, 0.0);
  QTextLayout layout(text);
  const LaidOut laid = lay_out(layout, style, width);
  if (painter != nullptr) draw_laid_out(layout, laid, text, style, width, *painter, origin);
  return laid.size;
}

QSizeF TextLayoutCache::paint(const QString& text, const QtTextStyle& style, qreal width, QPainter* painter,
                              QPointF origin) {
  if (text.isEmpty()) return QSizeF(0.0, 0.0);
  if (!layout_ || text != text_ || !(style == style_) || width != width_) {
    text_ = text;
    style_ = style;
    width_ = width;
    layout_.emplace(text);
    const LaidOut laid = lay_out(*layout_, style, width);
    size_ = laid.size;
    visible_ = laid.visible;
    truncated_ = laid.truncated;
  }
  if (painter != nullptr) {
    draw_laid_out(*layout_, LaidOut{size_, visible_, truncated_}, text_, style_, width_, *painter, origin);
  }
  return size_;
}

NveIntrinsicSize measure_text(std::string_view text, const NveRuntimeTextStyle& style, float max_width) {
  if (text.empty()) return NveIntrinsicSize{0.0f, 0.0f};
  const QSizeF size = layout_text(display_text(text), text_style(style), max_width);
  return NveIntrinsicSize{static_cast<float>(std::ceil(size.width())), static_cast<float>(std::ceil(size.height()))};
}

}  // namespace nve::qt
