// Shared Qt typography for the Widgets and Quick hosts: inherited `nve.typography` as a `QFont`
// plus paragraph style, one `QTextLayout`-based layout used for both painting and the runtime's
// text measurer (so measured and painted text always wrap identically).
#pragma once

#include <nve/host/scene_nodes.hpp>

#include <QColor>
#include <QFont>
#include <QPainter>
#include <QPointF>
#include <QSizeF>
#include <QString>
#include <QTextLayout>

#include <cstdint>
#include <optional>
#include <string_view>

namespace nve::qt {

/// Owned, resolved paragraph style of one text-bearing node.
struct QtTextStyle {
  QFont font;
  QColor color{Qt::black};
  Qt::Alignment align = Qt::AlignLeft;
  bool justify = false;
  host::TextWrap wrap = host::TextWrap::Word;
  int max_lines = 0;         // 0 = unlimited; overflow is elided with "…"
  qreal line_height = 0.0;   // line box height in px, 0 = the font's natural line height

  friend bool operator==(const QtTextStyle&, const QtTextStyle&) = default;
};

/// `QFont` of `size` px, CSS `weight` (1..1000), `NVE_FONT_STYLE_*`, optional CSS family list
/// (`host::for_each_font_family`; CSS generics resolve through Qt style hints), letter spacing (px) and
/// `NVE_TEXT_DECORATION_*` bits, on top of the application font.
[[nodiscard]] QFont make_font(float size, uint16_t weight, uint8_t font_style = NVE_FONT_STYLE_NORMAL,
                              std::string_view family = {}, float letter_spacing = 0.0f, uint8_t decoration = 0u);

/// Paragraph style of a resolved (inherited) `TextStyle`.
[[nodiscard]] QtTextStyle text_style(const host::TextStyle& s);

/// Paragraph style the runtime measurer is asked about: the same font and line metrics as the
/// painted `text_style(host::TextStyle)` (colour, alignment and decoration do not change sizes).
[[nodiscard]] QtTextStyle text_style(const NveRuntimeTextStyle& s);

/// UTF-8 scene text for a Qt setter: C0 / C1 control characters dropped, tabs as spaces, `\r\n` as
/// `\n`, newlines kept (multi-line inputs) or turned into spaces (names, single-line fields).
[[nodiscard]] QString scene_string(std::string_view utf8, bool keep_newlines = false);

/// UTF-8 scene text as layout text (`\n` / `\r\n` become hard line breaks, as the measurer sees them).
[[nodiscard]] QString display_text(std::string_view utf8);

/// Lays `text` out at `width` (non-finite: unbounded, no wrapping; `0` wraps at every opportunity,
/// the min-content probe) and returns its natural size (widest
/// line, total line-box height). With `painter`, also paints it with its top-left at `origin`,
/// aligned within `width`, in `style.color`.
QSizeF layout_text(const QString& text, const QtTextStyle& style, qreal width, QPainter* painter = nullptr,
                   QPointF origin = {});

/// `layout_text` keeping the shaped lines: repaints of the same text, style and width (a node's
/// widget repainting) reuse them. Copies start empty.
class TextLayoutCache {
 public:
  TextLayoutCache() = default;
  TextLayoutCache(const TextLayoutCache&) noexcept {}
  TextLayoutCache& operator=(const TextLayoutCache&) noexcept {
    layout_.reset();
    return *this;
  }

  QSizeF paint(const QString& text, const QtTextStyle& style, qreal width, QPainter* painter = nullptr,
               QPointF origin = {});

 private:
  QString text_;
  QtTextStyle style_;
  qreal width_ = 0.0;
  std::optional<QTextLayout> layout_;
  QSizeF size_;
  int visible_ = 0;
  bool truncated_ = false;
};

/// Runtime text measurer (`NveRuntimeMeasureTextFn` semantics): `text` laid out exactly as it is
/// painted with the node's resolved `style`, wrapped at `max_width` (`INFINITY` = unconstrained);
/// sizes are ceiled to whole px.
[[nodiscard]] NveIntrinsicSize measure_text(std::string_view text, const NveRuntimeTextStyle& style,
                                            float max_width);

}  // namespace nve::qt
