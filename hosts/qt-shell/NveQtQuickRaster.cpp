// GUI-thread rasterization of the Qt Quick records (`NveQtQuickItem::updatePolish`). Text
// shaping (`QTextLayout`, `QFontMetricsF`), font resolution, image decoding, and shadow blurs run
// here, never in `updatePaintNode`: with the threaded render loop that runs on the render thread,
// where Qt's font machinery is not safe to use. The render thread only uploads the images
// (NveQtSGBuilder.cpp).
#include "NveQtQuickInternal.hpp"

#include <QFontMetricsF>
#include <QPainter>

#include <algorithm>
#include <cmath>

namespace nve::qt {

namespace {

/// A premultiplied HiDPI canvas of logical `size`, cleared, with a painter set up by `paint`.
template <class Paint>
QImage raster(QSizeF size, qreal dpr, Paint&& paint) {
  const QSize px(std::max(1, static_cast<int>(std::ceil(size.width() * dpr))),
                 std::max(1, static_cast<int>(std::ceil(size.height() * dpr))));
  QImage image(px, QImage::Format_ARGB32_Premultiplied);
  if (image.isNull()) return image;  // unallocatable: nothing painted (never a null-device painter)
  image.setDevicePixelRatio(dpr);
  image.fill(Qt::transparent);
  QPainter p(&image);
  p.setRenderHint(QPainter::Antialiasing, true);
  paint(p);
  return image;
}

}  // namespace

void NveQtQuickItem::Impl::rasterize_dirty(qreal dpr) {
  for (QuickNodeView* v : sg_dirty_) {
    if (v != nullptr && v != &root_) rasterize(*v, dpr);
  }
}

void NveQtQuickItem::Impl::rasterize(QuickNodeView& v, qreal dpr) {
  QuickRaster& r = v.raster;
  if (r.generation == v.paint_generation && r.dpr == dpr) return;
  r.generation = v.paint_generation;
  r.dpr = dpr;
  const QSizeF size = v.local.size();
  const bool empty = size.width() <= 0.0 || size.height() <= 0.0;
  const BoxVisual& visual = v.paint.visual;
  r.shadow = QImage();
  if (!empty && visual.has_outer_shadow()) {
    const QMarginsF m = outer_shadow_margins(visual.shadows);
    r.shadow_rect = QRectF(QPointF(), size).marginsAdded(m);
    ShadowCanvas c = shadow_canvas(r.shadow_rect.size(), dpr);  // bounded; null when unallocatable
    if (!c.image.isNull()) {
      QPainter p(&c.image);
      p.setRenderHint(QPainter::Antialiasing, true);
      p.scale(c.scale, c.scale);
      paint_outer_shadows(p, QRectF(QPointF(m.left(), m.top()), size), visual.radius, visual.shadows);
    }
    r.shadow = std::move(c.image);  // uploaded into `shadow_rect`
  }
  if (empty || (!v.is_input && v.paint.paints_only_background())) {
    r.painted = QImage();
    r.solid = !empty && visual.background.alpha() > 0;
    return;
  }
  r.solid = false;
  const bool focused = focused_ == v.id;
  r.painted = raster(size, dpr, [&](QPainter& p) {
    paint_node(p, QRectF(QPointF(), size), v.paint, nullptr);  // the raster is the cache
    if (v.is_input) paint_input(p, QRectF(QPointF(), size), v, focused);
  });
  switch (v.paint.content) {
    case NodeContent::Text: ++stats_.textRasterizations; break;
    case NodeContent::Image: ++stats_.imageRasterizations; break;
    case NodeContent::Canvas: ++stats_.canvasRasterizations; break;
    case NodeContent::None: v.is_input ? ++stats_.textRasterizations : ++stats_.boxRasterizations; break;
  }
}

void paint_input(QPainter& p, const QRectF& box, const QuickNodeView& v, bool focused) {
  const QuickInput& in = v.input;
  const QRectF content = v.paint.content_box.translated(box.topLeft());
  QtTextStyle style = v.paint.style;
  // Text areas wrap like a browser's (`overflow-wrap: break-word`); single-line fields never.
  style.wrap = in.multiline ? host::TextWrap::BreakWord : host::TextWrap::None;
  style.max_lines = in.multiline ? 0 : 1;
  const QString shown = in.secure ? QString(in.edit.size(), QChar(0x2022)) : in.edit;
  const int caret = std::clamp(in.caret, 0, static_cast<int>(shown.size()));
  QString text = shown;
  text.insert(caret, in.preedit);
  p.save();
  p.setClipRect(content);
  if (text.isEmpty() && !in.placeholder.isEmpty()) {
    QtTextStyle dim = style;
    dim.color.setAlphaF(style.color.alphaF() * 0.5f);
    (void)layout_text(in.placeholder, dim, content.width(), &p, content.topLeft());
  } else {
    (void)layout_text(text, style, content.width(), &p, content.topLeft());
  }
  const QFontMetricsF fm(style.font);
  const qreal line = fm.height();
  if (!in.preedit.isEmpty()) {  // composition underline
    const qreal x0 = content.left() + fm.horizontalAdvance(shown.left(caret));
    p.fillRect(QRectF(x0, content.top() + line - 1.0, fm.horizontalAdvance(in.preedit), 1.0), style.color);
  }
  if (focused && !in.read_only) {
    const qreal x = content.left() + fm.horizontalAdvance(text.left(caret + static_cast<int>(in.preedit.size())));
    p.fillRect(QRectF(std::min(x, content.right() - 1.0), content.top(), 1.0, line), style.color);
  }
  p.restore();
}

}  // namespace nve::qt
