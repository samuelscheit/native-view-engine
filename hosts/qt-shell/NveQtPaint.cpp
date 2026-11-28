#include "NveQtPaint.hpp"

#include <QPaintDevice>

#include <algorithm>
#include <cmath>
#include <numbers>

namespace nve::qt {

namespace {

/// Bounds of a painted box shadow: a blur's raster grows with its radius and every pixel is
/// blurred, so an authored `blur(1e6)` must not allocate a gigantic image (Chromium caps too).
constexpr qreal kMaxShadowBlur = 256.0;
constexpr qreal kMaxShadowReach = 4096.0;  // spread / offset

/// One shadow's geometry, finite and bounded.
struct ShadowShape {
  qreal blur, spread, dx, dy;
};

ShadowShape shadow_shape(const host::BoxShadow& s) noexcept {
  const auto reach = [](float v) {
    return std::isfinite(v) ? std::clamp(static_cast<qreal>(v), -kMaxShadowReach, kMaxShadowReach) : 0.0;
  };
  const qreal blur = std::isfinite(s.blur_radius) ? std::clamp(static_cast<qreal>(s.blur_radius), 0.0, kMaxShadowBlur)
                                                  : 0.0;
  return {blur, reach(s.spread_radius), reach(s.offset_x), reach(s.offset_y)};
}

/// One box-blur pass of half-width `r` along rows (`horizontal`) or columns of a premultiplied
/// ARGB32 image; pixels outside the image count as transparent.
void box_blur_pass(QImage& img, int r, bool horizontal) {
  const int len = horizontal ? img.width() : img.height();
  const int lines = horizontal ? img.height() : img.width();
  const int div = 2 * r + 1;
  std::vector<uint32_t> line(static_cast<size_t>(len));
  auto* const bits = reinterpret_cast<uint32_t*>(img.bits());
  const qsizetype stride = img.bytesPerLine() / 4;
  for (int l = 0; l < lines; ++l) {
    const auto at = [&](int i) -> uint32_t& {
      return horizontal ? bits[l * stride + i] : bits[static_cast<qsizetype>(i) * stride + l];
    };
    for (int i = 0; i < len; ++i) line[static_cast<size_t>(i)] = at(i);
    int sum[4] = {0, 0, 0, 0};
    const auto add = [&](uint32_t v, int sign) {
      for (int c = 0; c < 4; ++c) sum[c] += sign * static_cast<int>((v >> (8 * c)) & 0xFFu);
    };
    for (int i = 0; i <= r && i < len; ++i) add(line[static_cast<size_t>(i)], 1);
    for (int i = 0; i < len; ++i) {
      uint32_t out = 0u;
      for (int c = 0; c < 4; ++c) out |= static_cast<uint32_t>(sum[c] / div) << (8 * c);
      at(i) = out;
      if (i - r >= 0) add(line[static_cast<size_t>(i - r)], -1);
      if (i + r + 1 < len) add(line[static_cast<size_t>(i + r + 1)], 1);
    }
  }
}

/// Gaussian blur of standard deviation `sigma` (device pixels) as three box blurs (SVG
/// `feGaussianBlur` approximation).
void gaussian_blur(QImage& img, qreal sigma) {
  const int d = static_cast<int>(std::floor(sigma * 3.0 * std::sqrt(2.0 * std::numbers::pi) / 4.0 + 0.5));
  if (d < 2) return;
  const int r = d / 2;
  for (int pass = 0; pass < 3; ++pass) {
    box_blur_pass(img, r, true);
    box_blur_pass(img, r, false);
  }
}

qreal device_ratio(const QPainter& p) noexcept {
  const qreal dpr = p.device() != nullptr ? p.device()->devicePixelRatioF() : 1.0;
  return std::isfinite(dpr) && dpr > 0.0 ? dpr : 1.0;
}

/// Blurred image of `area` (logical) after `draw` paints into it.
template <class Draw>
QImage blurred(const QRectF& area, qreal blur, qreal dpr, Draw&& draw) {
  QImage img(std::max(1, static_cast<int>(std::ceil(area.width() * dpr))),
             std::max(1, static_cast<int>(std::ceil(area.height() * dpr))), QImage::Format_ARGB32_Premultiplied);
  img.fill(Qt::transparent);
  {
    QPainter ip(&img);
    ip.setRenderHint(QPainter::Antialiasing, true);
    ip.scale(dpr, dpr);
    ip.translate(-area.topLeft());
    draw(ip);
  }
  gaussian_blur(img, blur * 0.5 * dpr);
  img.setDevicePixelRatio(dpr);
  return img;
}

void paint_inset_shadows(QPainter& p, const QRectF& box, qreal radius, std::span<const host::BoxShadow> shadows) {
  const QPainterPath clip = rounded_path(box, radius);
  for (auto it = shadows.rbegin(); it != shadows.rend(); ++it) {  // the first shadow paints on top
    const host::BoxShadow& s = *it;
    const QColor color = qcolor(s.color);
    if (!s.inset || color.alpha() == 0) continue;
    const auto [blur, spread, dx, dy] = shadow_shape(s);
    const QRectF hole = box.translated(dx, dy).adjusted(spread, spread, -spread, -spread);
    const qreal hole_radius = radius > 0.0 ? std::max(0.0, radius - spread) : 0.0;
    const qreal pad = blur + std::abs(dx) + std::abs(dy) + 1.0;
    const QRectF area = box.adjusted(-pad, -pad, pad, pad);
    const QImage img = blurred(area, blur, device_ratio(p), [&](QPainter& ip) {
      ip.fillRect(area, color);
      if (hole.width() > 0.0 && hole.height() > 0.0) {
        ip.setCompositionMode(QPainter::CompositionMode_Clear);
        ip.fillPath(rounded_path(hole, hole_radius), Qt::black);
      }
    });
    p.save();
    p.setClipPath(clip, Qt::IntersectClip);
    p.drawImage(area.topLeft(), img);
    p.restore();
  }
}

}  // namespace

QColor qcolor(uint32_t rgba) noexcept {
  return QColor(static_cast<int>(rgba & 0xFFu), static_cast<int>((rgba >> 8u) & 0xFFu),
                static_cast<int>((rgba >> 16u) & 0xFFu), static_cast<int>((rgba >> 24u) & 0xFFu));
}

bool BoxVisual::has_outer_shadow() const noexcept {
  return std::any_of(shadows.begin(), shadows.end(), [](const host::BoxShadow& s) { return !s.inset; });
}

bool BoxVisual::has_inset_shadow() const noexcept {
  return std::any_of(shadows.begin(), shadows.end(), [](const host::BoxShadow& s) { return s.inset; });
}

BoxVisual box_visual(const host::NodeData& node, std::vector<host::BoxShadow> shadows) {
  const host::VisualStyle& v = node.visual;
  BoxVisual out;
  out.background = qcolor(v.background);
  out.border_color = qcolor(v.border_color);
  out.border_width = v.border_width;  // sealed by `read_node`
  out.radius = v.border_radius;
  out.shadows = std::move(shadows);
  return out;
}

QPainterPath rounded_path(const QRectF& r, qreal radius) {
  QPainterPath path;
  const qreal clamped = std::clamp(radius, 0.0, std::min(r.width(), r.height()) * 0.5);
  if (clamped > 0.0) {
    path.addRoundedRect(r, clamped, clamped);
  } else {
    path.addRect(r);
  }
  return path;
}

void paint_box_background(QPainter& p, const QRectF& box, const BoxVisual& v, ShadowCache* inset_cache) {
  if (box.width() <= 0.0 || box.height() <= 0.0) return;
  if (v.background.alpha() > 0) {
    p.save();
    p.setRenderHint(QPainter::Antialiasing, true);
    p.fillPath(rounded_path(box, v.radius), v.background);
    p.restore();
  }
  if (!v.has_inset_shadow()) return;
  // CSS: inset shadows are cast inside the padding box (the border box minus the border).
  const qreal bw = v.has_border() ? std::min(v.border_width, std::min(box.width(), box.height()) * 0.5) : 0.0;
  const QRectF padding_box = box.adjusted(bw, bw, -bw, -bw);
  if (padding_box.width() > 0.0 && padding_box.height() > 0.0) {
    const qreal radius = std::max(0.0, v.radius - bw);
    if (inset_cache != nullptr) {
      inset_cache->paint(p, padding_box, radius, v.shadows, true);
    } else {
      paint_inset_shadows(p, padding_box, radius, v.shadows);
    }
  }
}

void ShadowCache::paint(QPainter& p, const QRectF& box, qreal radius, std::span<const host::BoxShadow> shadows,
                        bool inset) {
  if (box.width() <= 0.0 || box.height() <= 0.0) return;
  const qreal dpr = device_ratio(p);
  if (image_.isNull() || size_ != box.size() || radius_ != radius || dpr_ != dpr || inset_ != inset ||
      !std::ranges::equal(shadows_, shadows)) {
    size_ = box.size();
    radius_ = radius;
    dpr_ = dpr;
    inset_ = inset;
    shadows_.assign(shadows.begin(), shadows.end());
    const QRectF local(QPointF(), size_);
    const QRectF area = inset ? local : local.marginsAdded(outer_shadow_margins(shadows));
    image_ = QImage(std::max(1, static_cast<int>(std::ceil(area.width() * dpr))),
                    std::max(1, static_cast<int>(std::ceil(area.height() * dpr))), QImage::Format_ARGB32_Premultiplied);
    image_.setDevicePixelRatio(dpr);
    image_.fill(Qt::transparent);
    QPainter ip(&image_);
    ip.translate(-area.topLeft());
    if (inset) {
      paint_inset_shadows(ip, local, radius, shadows);
    } else {
      paint_outer_shadows(ip, local, radius, shadows);
    }
    offset_ = area.topLeft();
  }
  p.drawImage(box.topLeft() + offset_, image_);
}

void paint_box_border(QPainter& p, const QRectF& box, const BoxVisual& v) {
  if (!v.has_border()) return;
  const qreal bw = std::min(v.border_width, std::min(box.width(), box.height()) * 0.5);
  if (bw <= 0.0) return;
  // Fill the ring between the outer and the inner rounded rect (CSS border geometry).
  const QRectF inner = box.adjusted(bw, bw, -bw, -bw);
  QPainterPath ring = rounded_path(box, v.radius);
  if (inner.width() > 0.0 && inner.height() > 0.0) ring = ring.subtracted(rounded_path(inner, std::max(0.0, v.radius - bw)));
  p.save();
  p.setRenderHint(QPainter::Antialiasing, true);
  p.fillPath(ring, v.border_color);
  p.restore();
}

QMarginsF outer_shadow_margins(std::span<const host::BoxShadow> shadows) noexcept {
  QMarginsF m;
  for (const host::BoxShadow& s : shadows) {
    if (s.inset) continue;
    const auto [blur, spread, ox, oy] = shadow_shape(s);
    const qreal reach = blur + spread;
    m.setLeft(std::max(m.left(), reach - ox));
    m.setRight(std::max(m.right(), reach + ox));
    m.setTop(std::max(m.top(), reach - oy));
    m.setBottom(std::max(m.bottom(), reach + oy));
  }
  return QMarginsF(std::ceil(m.left()), std::ceil(m.top()), std::ceil(m.right()), std::ceil(m.bottom()));
}

void paint_outer_shadows(QPainter& p, const QRectF& box, qreal radius, std::span<const host::BoxShadow> shadows) {
  const QPainterPath body = rounded_path(box, radius);
  for (auto it = shadows.rbegin(); it != shadows.rend(); ++it) {  // the first shadow paints on top
    const host::BoxShadow& s = *it;
    const QColor color = qcolor(s.color);
    if (s.inset || color.alpha() == 0) continue;
    const auto [blur, spread, dx, dy] = shadow_shape(s);
    const QRectF shape = box.translated(dx, dy)
                             .adjusted(-spread, -spread, spread, spread);
    if (shape.width() <= 0.0 || shape.height() <= 0.0) continue;
    const qreal shape_radius = radius > 0.0 ? std::max(0.0, radius + spread) : 0.0;
    const QRectF area = shape.adjusted(-blur - 1.0, -blur - 1.0, blur + 1.0, blur + 1.0);
    QPainterPath outside;
    outside.addRect(area.united(box));
    outside = outside.subtracted(body);
    p.save();
    p.setRenderHint(QPainter::Antialiasing, true);
    p.setClipPath(outside, Qt::IntersectClip);
    if (blur <= 0.0) {
      p.fillPath(rounded_path(shape, shape_radius), color);
    } else {
      p.drawImage(area.topLeft(), blurred(area, blur, device_ratio(p), [&](QPainter& ip) {
                    ip.fillPath(rounded_path(shape, shape_radius), color);
                  }));
    }
    p.restore();
  }
}

bool image_fit_rects(uint8_t fit, const QRectF& box, const QSizeF& image, QRectF& out_dst, QRectF& out_src) noexcept {
  const qreal dw = box.width(), dh = box.height(), iw = image.width(), ih = image.height();
  if (dw <= 0.0 || dh <= 0.0 || iw <= 0.0 || ih <= 0.0) return false;
  const auto centered = [&](qreal w, qreal h) {
    return QRectF(box.x() + (dw - w) * 0.5, box.y() + (dh - h) * 0.5, w, h);
  };
  const auto natural = [&] {  // unscaled, centered, cropped to the box
    const qreal w = std::min(dw, iw), h = std::min(dh, ih);
    out_dst = centered(w, h);
    out_src = QRectF((iw - w) * 0.5, (ih - h) * 0.5, w, h);
  };
  const auto contain = [&] {
    const qreal s = std::min(dw / iw, dh / ih);
    out_dst = centered(iw * s, ih * s);
    out_src = QRectF(0.0, 0.0, iw, ih);
  };
  switch (fit) {
    case NVE_IMAGE_FIT_FILL:
      out_dst = box;
      out_src = QRectF(0.0, 0.0, iw, ih);
      return true;
    case NVE_IMAGE_FIT_CONTAIN: contain(); return true;
    case NVE_IMAGE_FIT_NONE: natural(); return true;
    case NVE_IMAGE_FIT_SCALE_DOWN:
      if (iw > dw || ih > dh) {
        contain();
      } else {
        natural();
      }
      return true;
    default: {  // cover
      const qreal s = std::max(dw / iw, dh / ih);
      const qreal cw = std::min(iw, dw / s), ch = std::min(ih, dh / s);
      out_dst = box;
      out_src = QRectF((iw - cw) * 0.5, (ih - ch) * 0.5, cw, ch);
      return true;
    }
  }
}

QImage tinted_image(const QImage& image, const QColor& tint) {
  QImage out = image.convertToFormat(QImage::Format_ARGB32_Premultiplied);
  QPainter p(&out);
  p.setCompositionMode(QPainter::CompositionMode_SourceIn);
  p.fillRect(out.rect(), tint);
  return out;
}

void paint_image(QPainter& p, const QRectF& box, qreal radius, const QImage& image, uint8_t fit) {
  QRectF dst, src;
  if (image.isNull() || !image_fit_rects(fit, box, QSizeF(image.size()) / image.devicePixelRatio(), dst, src)) return;
  src = QRectF(src.topLeft() * image.devicePixelRatio(), src.size() * image.devicePixelRatio());
  p.save();
  p.setRenderHint(QPainter::Antialiasing, true);
  p.setRenderHint(QPainter::SmoothPixmapTransform, true);
  p.setClipPath(rounded_path(box, radius), Qt::IntersectClip);
  p.drawImage(dst, image, src);
  p.restore();
}

void paint_node(QPainter& p, const QRectF& box, const NodePaint& n) {
  p.setRenderHint(QPainter::Antialiasing, true);
  paint_box_background(p, box, n.visual, &n.inset_shadows);
  const QRectF content = n.content_box.translated(box.topLeft());
  switch (n.content) {
    case NodeContent::Text:
      p.save();
      p.setClipPath(rounded_path(box, n.visual.radius), Qt::IntersectClip);
      (void)n.text_layout.paint(n.text, n.style, content.width(), &p, content.topLeft());
      p.restore();
      break;
    case NodeContent::Image: paint_image(p, content, n.visual.radius, n.image, n.fit); break;
    case NodeContent::Canvas:
      p.save();
      p.setClipPath(rounded_path(box, n.visual.radius), Qt::IntersectClip);
      (void)paint_draw_list(p, content, n.draw_list);
      p.restore();
      break;
    case NodeContent::None: break;
  }
  paint_box_border(p, box, n.visual);
}

}  // namespace nve::qt
