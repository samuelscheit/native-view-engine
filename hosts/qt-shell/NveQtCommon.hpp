#pragma once

#include "nve_host_runtime.hpp"

#include <QColor>
#include <QFont>
#include <QFontMetricsF>
#include <QGuiApplication>
#include <QRectF>
#include <QString>

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <string_view>

namespace nve::qt {

inline QColor qcolorFromRgba(uint32_t rgba) noexcept {
  const auto c = nve::unpack_rgba(rgba);
  return QColor(c.r8, c.g8, c.b8, c.a8);
}

inline QFont makeNveFont(float font_size) {
  QFont font;
  if (QGuiApplication::instance() != nullptr) {
    font = QGuiApplication::font();
  }
  const int px =
      std::max(1, static_cast<int>(std::lround(font_size > 0.0f ? font_size : 16.0f)));
  font.setPixelSize(px);
  return font;
}

inline nve::TextMeasurerCallback makeSharedQtTextMeasurer() {
  return [](std::string_view text, float font_size, float max_width) -> NveIntrinsicSize {
    if (text.empty()) {
      return NveIntrinsicSize{0.0f, 0.0f};
    }
    const QString qtext = QString::fromUtf8(text.data(), static_cast<qsizetype>(text.size()));
    const QFont font = makeNveFont(font_size);
    const QFontMetricsF fm(font);
    const bool constrained = (max_width > 0.0f && std::isfinite(max_width));
    const qreal constraint_w = constrained ? static_cast<qreal>(max_width) : 1.0e6;
    const QRectF bounds = fm.boundingRect(
        QRectF(0.0, 0.0, constraint_w, 1.0e6),
        Qt::AlignLeft | Qt::AlignTop | Qt::TextWordWrap,
        qtext);

    qreal w = bounds.width();
    qreal h = std::max(bounds.height(), fm.height());

    if (!qtext.contains(QChar('\n'))) {
      const qreal adv = fm.horizontalAdvance(qtext);
      if (!constrained || adv <= constraint_w) {
        w = std::max(w, adv);
      }
    }

    // Add 2px horizontal slack so word-wrapped labels and QTextLayout never wrap the final glyph.
    qreal final_w = std::ceil(w) + 2.0;
    if (constrained) {
      final_w = std::min(final_w, std::ceil(constraint_w));
    }
    return NveIntrinsicSize{
        static_cast<float>(final_w),
        static_cast<float>(std::ceil(h)),
    };
  };
}

} // namespace nve::qt
