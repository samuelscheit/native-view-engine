#include "NveQtWidgetInternal.hpp"

#include <QGraphicsOpacityEffect>
#include <QMouseEvent>
#include <QPainter>
#include <QPainterPath>
#include <QPen>

#include <algorithm>
#include <cstring>
#include <utility>

namespace nve::qt {

void applyWidgetOpacity(QWidget* widget, float opacity) {
  const float clamped = std::clamp(opacity, 0.0f, 1.0f);
  auto* existing = qobject_cast<QGraphicsOpacityEffect*>(widget->graphicsEffect());
  if (clamped >= 0.999f) {
    if (existing != nullptr) {
      widget->setGraphicsEffect(nullptr);
    }
    return;
  }
  if (existing == nullptr) {
    existing = new QGraphicsOpacityEffect(widget);
    widget->setGraphicsEffect(existing);
  }
  existing->setOpacity(static_cast<qreal>(clamped));
}

// -----------------------------------------------------------------------------
// NveQtBoxWidget
// -----------------------------------------------------------------------------

NveQtBoxWidget::NveQtBoxWidget(uint32_t node_id, PressHandler on_press, QWidget* parent)
    : QWidget(parent), node_id_(node_id), on_press_(std::move(on_press)) {
  setAutoFillBackground(false);
  setAttribute(Qt::WA_NoSystemBackground, true);
  setProperty(kNodeIdProperty, QVariant::fromValue<qulonglong>(node_id));
}

void NveQtBoxWidget::applyStyle(const NveRenderNode& node, bool interactive) {
  const QColor next_bg = qcolorFromRgba(node.bg_rgba);
  const QColor next_border = qcolorFromRgba(node.border_rgba);
  const float next_bw = std::max(0.0f, node.border_width);
  const float next_br = std::max(0.0f, node.border_radius);
  const float next_opacity = opacityFromU8(node.opacity_u8);

  const bool visual_changed = (bg_color_ != next_bg) || (border_color_ != next_border) ||
                              (border_width_ != next_bw) || (border_radius_ != next_br) ||
                              (opacity_ != next_opacity);

  bg_color_ = next_bg;
  border_color_ = next_border;
  border_width_ = next_bw;
  border_radius_ = next_br;
  opacity_ = next_opacity;

  setInteractive(interactive);

  if (visual_changed) {
    update();
  }
}

void NveQtBoxWidget::setInteractive(bool interactive) {
  if (interactive_ != interactive) {
    interactive_ = interactive;
    if (interactive_) {
      setCursor(Qt::PointingHandCursor);
    } else {
      unsetCursor();
    }
  }
}

void NveQtBoxWidget::paintEvent(QPaintEvent* /*event*/) {
  const bool has_bg = bg_color_.alpha() > 0;
  const bool has_border = (border_width_ > 0.0f) && (border_color_.alpha() > 0);
  if (!has_bg && !has_border) {
    return;
  }

  QPainter painter(this);
  painter.setRenderHint(QPainter::Antialiasing, true);
  if (opacity_ < 0.999f) {
    painter.setOpacity(static_cast<qreal>(opacity_));
  }

  const QRectF bounds(rect());
  if (bounds.width() <= 0.0 || bounds.height() <= 0.0) {
    return;
  }

  const qreal max_r = std::min(bounds.width(), bounds.height()) * 0.5;
  const qreal radius = std::clamp(static_cast<qreal>(border_radius_), 0.0, max_r);

  if (has_bg) {
    QPainterPath fill_path;
    fill_path.addRoundedRect(bounds, radius, radius);
    painter.fillPath(fill_path, bg_color_);
  }

  if (has_border) {
    const qreal half_pen = static_cast<qreal>(border_width_) * 0.5;
    const QRectF stroke_rect = bounds.adjusted(half_pen, half_pen, -half_pen, -half_pen);
    if (stroke_rect.width() > 0.0 && stroke_rect.height() > 0.0) {
      const qreal stroke_r = std::max<qreal>(0.0, radius - half_pen);
      QPen pen(border_color_, static_cast<qreal>(border_width_));
      pen.setJoinStyle(Qt::RoundJoin);
      painter.setPen(pen);
      painter.setBrush(Qt::NoBrush);
      painter.drawRoundedRect(stroke_rect, stroke_r, stroke_r);
    }
  }
}

void NveQtBoxWidget::mousePressEvent(QMouseEvent* event) {
  if (interactive_ && event->button() == Qt::LeftButton) {
    pressed_ = true;
    event->accept();
    return;
  }
  event->ignore();
}

void NveQtBoxWidget::mouseReleaseEvent(QMouseEvent* event) {
  if (interactive_ && event->button() == Qt::LeftButton) {
    const bool was_pressed = pressed_;
    pressed_ = false;
    const QPointF pos = event->position();
    event->accept();
    if (was_pressed && rect().contains(pos.toPoint()) && on_press_) {
      auto cb = on_press_;
      const uint32_t id = node_id_;
      cb(id, static_cast<float>(pos.x()), static_cast<float>(pos.y()));
    }
    return;
  }
  event->ignore();
}

// -----------------------------------------------------------------------------
// NveQtImageWidget
// -----------------------------------------------------------------------------

NveQtImageWidget::NveQtImageWidget(uint32_t node_id, PressHandler on_press, QWidget* parent)
    : QWidget(parent), node_id_(node_id), on_press_(std::move(on_press)) {
  setAutoFillBackground(false);
  setAttribute(Qt::WA_NoSystemBackground, true);
  setProperty(kNodeIdProperty, QVariant::fromValue<qulonglong>(node_id));
  setProperty(kImageDecodedProperty, false);
  setProperty(kImageResizeModeProperty, static_cast<int>(ImageResizeMode::Cover));
}

void NveQtImageWidget::applyNode(
    const NveRenderNode& node,
    std::string_view src,
    ImageResizeMode resize_mode,
    bool interactive) {
  const QColor next_bg = qcolorFromRgba(node.bg_rgba);
  const QColor next_border = qcolorFromRgba(node.border_rgba);
  const float next_bw = std::max(0.0f, node.border_width);
  const float next_br = std::max(0.0f, node.border_radius);
  const float next_opacity = opacityFromU8(node.opacity_u8);

  bool changed = (bg_color_ != next_bg) || (border_color_ != next_border) ||
                 (border_width_ != next_bw) || (border_radius_ != next_br) ||
                 (opacity_ != next_opacity) || (resize_mode_ != resize_mode);

  bg_color_ = next_bg;
  border_color_ = next_border;
  border_width_ = next_bw;
  border_radius_ = next_br;
  opacity_ = next_opacity;
  resize_mode_ = resize_mode;
  setProperty(kImageResizeModeProperty, static_cast<int>(resize_mode_));

  if (image_src_ != src) {
    image_src_.assign(src.data(), src.size());
    pixmap_ = NveQtImageCache::shared().getPixmap(image_src_);
    setProperty(kImageSrcProperty, detail::qstringFromUtf8(image_src_));
    setProperty(kImageDecodedProperty, !pixmap_.isNull());
    setProperty(kImageSizeProperty, pixmap_.size());
    changed = true;
  }

  setInteractive(interactive);
  if (changed) {
    update();
  }
}

void NveQtImageWidget::setResizeMode(ImageResizeMode resize_mode) {
  if (resize_mode_ != resize_mode) {
    resize_mode_ = resize_mode;
    setProperty(kImageResizeModeProperty, static_cast<int>(resize_mode_));
    update();
  }
}

void NveQtImageWidget::setInteractive(bool interactive) {
  if (interactive_ != interactive) {
    interactive_ = interactive;
    if (interactive_) {
      setCursor(Qt::PointingHandCursor);
    } else {
      unsetCursor();
    }
  }
}

void NveQtImageWidget::paintEvent(QPaintEvent* /*event*/) {
  const QRectF bounds(rect());
  if (bounds.width() <= 0.0 || bounds.height() <= 0.0) {
    return;
  }

  const bool has_bg = bg_color_.alpha() > 0;
  const bool has_img = !pixmap_.isNull();
  const bool has_border = (border_width_ > 0.0f) && (border_color_.alpha() > 0);
  if (!has_bg && !has_img && !has_border) {
    return;
  }

  QPainter painter(this);
  painter.setRenderHint(QPainter::Antialiasing, true);
  painter.setRenderHint(QPainter::SmoothPixmapTransform, true);
  if (opacity_ < 0.999f) {
    painter.setOpacity(static_cast<qreal>(opacity_));
  }

  const qreal max_r = std::min(bounds.width(), bounds.height()) * 0.5;
  const qreal radius = std::clamp(static_cast<qreal>(border_radius_), 0.0, max_r);

  painter.save();
  if (radius > 0.0) {
    QPainterPath clip_path;
    clip_path.addRoundedRect(bounds, radius, radius);
    painter.setClipPath(clip_path);
  } else {
    painter.setClipRect(bounds);
  }

  if (has_bg) {
    painter.fillRect(bounds, bg_color_);
  }

  if (has_img) {
    QRectF dst_rect;
    QRectF src_rect;
    if (computeImageDrawRects(
            resize_mode_,
            bounds,
            QSizeF(pixmap_.width(), pixmap_.height()),
            dst_rect,
            src_rect)) {
      painter.drawPixmap(dst_rect, pixmap_, src_rect);
    }
  }
  painter.restore();

  if (has_border) {
    const qreal half_pen = static_cast<qreal>(border_width_) * 0.5;
    const QRectF stroke_rect = bounds.adjusted(half_pen, half_pen, -half_pen, -half_pen);
    if (stroke_rect.width() > 0.0 && stroke_rect.height() > 0.0) {
      const qreal stroke_r = std::max<qreal>(0.0, radius - half_pen);
      QPen pen(border_color_, static_cast<qreal>(border_width_));
      pen.setJoinStyle(Qt::RoundJoin);
      painter.setPen(pen);
      painter.setBrush(Qt::NoBrush);
      painter.drawRoundedRect(stroke_rect, stroke_r, stroke_r);
    }
  }
}

void NveQtImageWidget::mousePressEvent(QMouseEvent* event) {
  if (interactive_ && event->button() == Qt::LeftButton) {
    pressed_ = true;
    event->accept();
    return;
  }
  event->ignore();
}

void NveQtImageWidget::mouseReleaseEvent(QMouseEvent* event) {
  if (interactive_ && event->button() == Qt::LeftButton) {
    const bool was_pressed = pressed_;
    pressed_ = false;
    const QPointF pos = event->position();
    event->accept();
    if (was_pressed && rect().contains(pos.toPoint()) && on_press_) {
      auto cb = on_press_;
      const uint32_t id = node_id_;
      cb(id, static_cast<float>(pos.x()), static_cast<float>(pos.y()));
    }
    return;
  }
  event->ignore();
}

// -----------------------------------------------------------------------------
// NveQtCanvasWidget
// -----------------------------------------------------------------------------

NveQtCanvasWidget::NveQtCanvasWidget(
    uint32_t nodeId,
    PressHandler onPress,
    QWidget* parent)
    : QWidget(parent), node_id_(nodeId), on_press_(std::move(onPress)) {
  setAutoFillBackground(false);
  setAttribute(Qt::WA_NoSystemBackground, true);
  setProperty(kNodeIdProperty, QVariant::fromValue<qulonglong>(nodeId));
}

bool NveQtCanvasWidget::setDrawListBytes(std::span<const uint8_t> bytes) {
  if (draw_list_bytes_.size() == bytes.size() &&
      (bytes.empty() ||
       std::memcmp(draw_list_bytes_.data(), bytes.data(), bytes.size()) == 0)) {
    return false;
  }
  draw_list_bytes_.assign(bytes.begin(), bytes.end());
  ++redraw_request_count_;
  update();
  return true;
}

void NveQtCanvasWidget::applyNode(
    const NveRenderNode& node,
    std::span<const uint8_t> drawListBytes,
    bool interactive) {
  const QColor next_bg = qcolorFromRgba(node.bg_rgba);
  const QColor next_border = qcolorFromRgba(node.border_rgba);
  const float next_bw = std::max(0.0f, node.border_width);
  const float next_br = std::max(0.0f, node.border_radius);
  const float next_opacity = opacityFromU8(node.opacity_u8);

  const bool style_changed = (bg_color_ != next_bg) || (border_color_ != next_border) ||
                             (border_width_ != next_bw) || (border_radius_ != next_br) ||
                             (opacity_ != next_opacity);

  bg_color_ = next_bg;
  border_color_ = next_border;
  border_width_ = next_bw;
  border_radius_ = next_br;
  opacity_ = next_opacity;

  setInteractive(interactive);

  const bool bytes_changed = setDrawListBytes(drawListBytes);
  if (!bytes_changed && style_changed) {
    ++redraw_request_count_;
    update();
  }
}

void NveQtCanvasWidget::setInteractive(bool interactive) {
  if (interactive_ != interactive) {
    interactive_ = interactive;
    if (interactive_) {
      setCursor(Qt::PointingHandCursor);
    } else {
      unsetCursor();
    }
  }
}

void NveQtCanvasWidget::paintEvent(QPaintEvent* /*event*/) {
  const QRectF bounds(rect());
  if (bounds.width() <= 0.0 || bounds.height() <= 0.0) {
    return;
  }

  const bool has_bg = bg_color_.alpha() > 0;
  const bool has_draw_list = !draw_list_bytes_.empty();
  const bool has_border = (border_width_ > 0.0f) && (border_color_.alpha() > 0);
  if (!has_bg && !has_draw_list && !has_border) {
    return;
  }

  QPainter painter(this);
  painter.setRenderHint(QPainter::Antialiasing, true);
  if (opacity_ < 0.999f) {
    painter.setOpacity(static_cast<qreal>(opacity_));
  }

  const qreal max_r = std::min(bounds.width(), bounds.height()) * 0.5;
  const qreal radius = std::clamp(static_cast<qreal>(border_radius_), 0.0, max_r);

  painter.save();
  if (radius > 0.0) {
    QPainterPath clip_path;
    clip_path.addRoundedRect(bounds, radius, radius);
    painter.setClipPath(clip_path);
  }
  if (has_bg) {
    painter.fillRect(bounds, bg_color_);
  }
  if (has_draw_list) {
    (void)renderDrawListToPainter(painter, bounds, draw_list_bytes_);
  }
  painter.restore();

  if (has_border) {
    const qreal half_pen = static_cast<qreal>(border_width_) * 0.5;
    const QRectF stroke_rect = bounds.adjusted(half_pen, half_pen, -half_pen, -half_pen);
    if (stroke_rect.width() > 0.0 && stroke_rect.height() > 0.0) {
      const qreal stroke_r = std::max<qreal>(0.0, radius - half_pen);
      QPen pen(border_color_, static_cast<qreal>(border_width_));
      pen.setJoinStyle(Qt::RoundJoin);
      painter.setPen(pen);
      painter.setBrush(Qt::NoBrush);
      painter.drawRoundedRect(stroke_rect, stroke_r, stroke_r);
    }
  }
}

void NveQtCanvasWidget::mousePressEvent(QMouseEvent* event) {
  if (interactive_ && event->button() == Qt::LeftButton) {
    pressed_ = true;
    event->accept();
    return;
  }
  event->ignore();
}

void NveQtCanvasWidget::mouseReleaseEvent(QMouseEvent* event) {
  if (interactive_ && event->button() == Qt::LeftButton) {
    const bool was_pressed = pressed_;
    pressed_ = false;
    const QPointF pos = event->position();
    event->accept();
    if (was_pressed && rect().contains(pos.toPoint()) && on_press_) {
      auto cb = on_press_;
      const uint32_t id = node_id_;
      cb(id, static_cast<float>(pos.x()), static_cast<float>(pos.y()));
    }
    return;
  }
  event->ignore();
}

} // namespace nve::qt
