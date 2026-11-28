// Flat `nve.res.draw_list` canvas draw lists painted with `QPainter` (Widgets and Quick hosts).
#include "NveQtPaint.hpp"

#include <QPaintDevice>
#include <QPen>

#include <algorithm>
#include <cmath>

namespace nve::qt {

namespace {

using namespace nve::canvas;

QColor color_of(DrawColorRgba c) noexcept { return QColor(c.r, c.g, c.b, c.a); }

QPen stroke_pen(DrawColorRgba color, float width) {
  return QPen(color_of(color), static_cast<qreal>(width), Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin);
}

struct PainterVisitor {
  QPainter& p;
  QRectF bounds;      // local (0, 0, w, h)
  bool replace_clear;  // image devices: `Clear` replaces pixels (transparent clears work)

  void on_clear(const DrawClearOp& op) {
    if (replace_clear) {
      const auto mode = p.compositionMode();
      p.setCompositionMode(QPainter::CompositionMode_Source);
      p.fillRect(bounds, color_of(op.color));
      p.setCompositionMode(mode);
    } else if (op.color.a > 0u) {
      p.fillRect(bounds, color_of(op.color));
    }
  }

  void on_fill_rect(const DrawFillRectOp& op) {
    if (op.w <= 0.0f || op.h <= 0.0f || op.color.a == 0u) return;
    p.fillRect(QRectF(op.x, op.y, op.w, op.h), color_of(op.color));
  }

  void on_fill_rounded_rect(const DrawFillRoundedRectOp& op) {
    if (op.w <= 0.0f || op.h <= 0.0f || op.color.a == 0u) return;
    p.fillPath(rounded_path(QRectF(op.x, op.y, op.w, op.h), static_cast<qreal>(op.radius)), color_of(op.color));
  }

  void on_fill_circle(const DrawFillCircleOp& op) {
    if (op.radius <= 0.0f || op.color.a == 0u) return;
    p.setPen(Qt::NoPen);
    p.setBrush(color_of(op.color));
    p.drawEllipse(QPointF(op.cx, op.cy), static_cast<qreal>(op.radius), static_cast<qreal>(op.radius));
  }

  void on_stroke_line(const DrawStrokeLineOp& op) {
    if (op.width <= 0.0f || op.color.a == 0u) return;
    p.setPen(stroke_pen(op.color, op.width));
    p.setBrush(Qt::NoBrush);
    p.drawLine(QPointF(op.x0, op.y0), QPointF(op.x1, op.y1));
  }

  void on_stroke_polyline(const DrawStrokePolylineOp& op) {
    if (op.points.size() < 2u || op.width <= 0.0f || op.color.a == 0u) return;
    p.setPen(stroke_pen(op.color, op.width));
    p.setBrush(Qt::NoBrush);
    QPainterPath path;
    path.moveTo(op.points[0u].x, op.points[0u].y);
    for (size_t i = 1u; i < op.points.size(); ++i) path.lineTo(op.points[i].x, op.points[i].y);
    p.drawPath(path);
  }

  void on_clip_rect(const DrawClipRectOp& op) {
    p.save();
    p.setClipRect(QRectF(op.x, op.y, std::max(0.0f, op.w), std::max(0.0f, op.h)), Qt::IntersectClip);
  }

  void on_restore_clip(const DrawRestoreClipOp&) {
    p.restore();
  }
};

struct PainterSaveGuard {
  QPainter& painter;
  explicit PainterSaveGuard(QPainter& target) : painter(target) { painter.save(); }
  ~PainterSaveGuard() { painter.restore(); }
  PainterSaveGuard(const PainterSaveGuard&) = delete;
  PainterSaveGuard& operator=(const PainterSaveGuard&) = delete;
};

}  // namespace

DrawListDecodeResult paint_draw_list(QPainter& p, const QRectF& bounds, std::span<const uint8_t> bytes) {
  if (bytes.empty()) return DrawListDecodeResult{DrawListError::Ok, 0u, 0u};
  const DrawListDecodeResult check = validate_draw_list(bytes);
  if (!check.ok()) return check;
  const qreal w = std::max<qreal>(0.0, bounds.width());
  const qreal h = std::max<qreal>(0.0, bounds.height());
  if (w <= 0.0 || h <= 0.0) return DrawListDecodeResult{DrawListError::Ok, 0u, bytes.size()};

  PainterSaveGuard save_guard(p);
  p.setRenderHint(QPainter::Antialiasing, true);
  p.translate(bounds.topLeft());
  const QRectF local(0.0, 0.0, w, h);
  p.setClipRect(local, Qt::IntersectClip);
  const bool image_device = p.device() != nullptr && p.device()->devType() == QInternal::Image;
  PainterVisitor visitor{p, local, image_device};
  (void)stream_validated_draw_list(bytes, visitor);
  return check;
}

QImage rasterize_draw_list(std::span<const uint8_t> bytes, QSizeF size, qreal dpr) {
  if (bytes.empty() || !(size.width() > 0.0) || !(size.height() > 0.0) || !validate_draw_list(bytes).ok()) return {};
  const qreal ratio = std::isfinite(dpr) && dpr > 0.0 ? dpr : 1.0;
  const QSizeF logical(std::ceil(size.width()), std::ceil(size.height()));
  QImage img(std::max(1, static_cast<int>(std::ceil(logical.width() * ratio))),
             std::max(1, static_cast<int>(std::ceil(logical.height() * ratio))), QImage::Format_RGBA8888_Premultiplied);
  img.setDevicePixelRatio(ratio);
  img.fill(Qt::transparent);
  QPainter p(&img);
  (void)paint_draw_list(p, QRectF(QPointF(0.0, 0.0), logical), bytes);
  return img;
}

}  // namespace nve::qt
