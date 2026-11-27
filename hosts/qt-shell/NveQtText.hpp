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

#include <cstdint>
#include <string_view>

namespace nve::qt {

/// Owned, resolved paragraph style of one text-bearing node.
struct QtTextStyle {
  QFont font;
  QColor color{Qt::black};
  Qt::Alignment align = Qt::AlignLeft;
  bool justify = false;
  bool wrap = true;
  int max_lines = 0;         // 0 = unlimited; overflow is elided with "…"
  qreal line_height = 0.0;   // line box height in px, 0 = the font's natural line height

  friend bool operator==(const QtTextStyle&, const QtTextStyle&) = default;
};

/// `QFont` of `size` px, CSS `weight` (1..1000), `NVE_FONT_STYLE_*`, optional family, letter
/// spacing (px) and `NVE_TEXT_DECORATION_*` bits, on top of the application font.
[[nodiscard]] QFont make_font(float size, uint16_t weight, uint8_t font_style = NVE_FONT_STYLE_NORMAL,
                              std::string_view family = {}, float letter_spacing = 0.0f, uint8_t decoration = 0u);

/// Paragraph style of a resolved (inherited) `TextStyle`.
[[nodiscard]] QtTextStyle text_style(const host::TextStyle& s);

/// UTF-8 scene text for a Qt setter: C0 / C1 control characters dropped, tabs as spaces, `\r\n` as
/// `\n`, newlines kept (multi-line inputs) or turned into spaces (names, single-line fields).
[[nodiscard]] QString scene_string(std::string_view utf8, bool keep_newlines = false);

/// UTF-8 scene text as layout text (`\n` / `\r\n` become hard line breaks, as the measurer sees them).
[[nodiscard]] QString display_text(std::string_view utf8);

/// Lays `text` out at `width` (<= 0: unbounded, no wrapping) and returns its natural size (widest
/// line, total line-box height). With `painter`, also paints it with its top-left at `origin`,
/// aligned within `width`, in `style.color`.
QSizeF layout_text(const QString& text, const QtTextStyle& style, qreal width, QPainter* painter = nullptr,
                   QPointF origin = {});

/// Runtime text measurer (`NveRuntimeMeasureTextFn` semantics): the default family at `font_size`
/// / `font_weight`, wrapped at `max_width` when finite and positive; sizes are ceiled to whole px.
[[nodiscard]] NveIntrinsicSize measure_text(std::string_view text, float font_size, uint16_t font_weight,
                                            float max_width);

/// `NveRuntimeMeasureTextFn` trampoline over `measure_text` (`ctx` unused).
void measure_text_callback(void* ctx, NveStrSlice text, float font_size, uint32_t font_weight, float max_width,
                           float* out_width, float* out_height);

}  // namespace nve::qt
