#include "NveQtWidget.hpp"
#include "NveQtCommon.hpp"

#include <QAccessible>
#include <QAccessibleWidget>
#include <QApplication>
#include <QColor>
#include <QFocusEvent>
#include <QFont>
#include <QGraphicsOpacityEffect>
#include <QInputMethodEvent>
#include <QLabel>
#include <QLineEdit>
#include <QMouseEvent>
#include <QPainter>
#include <QPainterPath>
#include <QPalette>
#include <QPen>
#include <QPixmap>
#include <QResizeEvent>
#include <QScrollArea>
#include <QScrollBar>

#include <algorithm>
#include <chrono>
#include <cmath>
#include <functional>
#include <mutex>
#include <span>
#include <string>
#include <unordered_map>
#include <utility>
#include <vector>

namespace nve::qt {

namespace {

constexpr const char* kNodeIdProperty = "nveNodeId";
constexpr const char* kA11yRoleProperty = "nveAccessibleRole";
constexpr const char* kA11yValueProperty = "nveAccessibleValue";
constexpr const char* kImageSrcProperty = "nveImageSrc";
constexpr const char* kImageDecodedProperty = "nveImageDecoded";
constexpr const char* kImageSizeProperty = "nveImageSize";
constexpr const char* kImageResizeModeProperty = "nveImageResizeMode";

class NveQtAccessibleWidget final : public QAccessibleWidget {
 public:
  explicit NveQtAccessibleWidget(QWidget* w)
      : QAccessibleWidget(
            w,
            static_cast<QAccessible::Role>(
                w->property(kA11yRoleProperty).toInt())) {}

  QAccessible::Role role() const override {
    if (QWidget* w = widget()) {
      const QVariant v = w->property(kA11yRoleProperty);
      if (v.isValid()) {
        return static_cast<QAccessible::Role>(v.toInt());
      }
    }
    return QAccessibleWidget::role();
  }

  QString text(QAccessible::Text t) const override {
    if (QWidget* w = widget()) {
      if (t == QAccessible::Name && !w->accessibleName().isEmpty()) {
        return w->accessibleName();
      }
      if (t == QAccessible::Description && !w->accessibleDescription().isEmpty()) {
        return w->accessibleDescription();
      }
      if (t == QAccessible::Value) {
        const QVariant v = w->property(kA11yValueProperty);
        if (v.isValid()) {
          return v.toString();
        }
        if (auto* edit = qobject_cast<QLineEdit*>(w)) {
          return edit->text();
        }
      }
    }
    return QAccessibleWidget::text(t);
  }

  QAccessible::State state() const override {
    QAccessible::State st = QAccessibleWidget::state();
    if (QWidget* w = widget()) {
      st.disabled = !w->isEnabled();
    }
    return st;
  }
};

QAccessibleInterface* nveQtAccessibleFactory(const QString& /*classname*/, QObject* object) {
  auto* widget = qobject_cast<QWidget*>(object);
  if (widget != nullptr && widget->property(kA11yRoleProperty).isValid()) {
    return new NveQtAccessibleWidget(widget);
  }
  return nullptr;
}

void ensureAccessibleFactoryInstalled() {
  static std::once_flag flag;
  std::call_once(flag, []() { QAccessible::installFactory(&nveQtAccessibleFactory); });
}

void applyWidgetAccessibility(QWidget* widget, const NodeSemantics& sem) {
  if (widget == nullptr) {
    return;
  }
  const QString prev_name = widget->accessibleName();
  const QString prev_desc = widget->accessibleDescription();

  widget->setAccessibleName(sem.label);
  widget->setAccessibleDescription(sem.hint);
  widget->setProperty(kA11yRoleProperty, static_cast<int>(sem.accessibleRole));
  if (sem.hasExplicitValue) {
    widget->setProperty(kA11yValueProperty, sem.value);
  } else {
    widget->setProperty(kA11yValueProperty, QVariant());
  }
  if (!sem.testId.isEmpty() && widget->objectName() != sem.testId) {
    widget->setObjectName(sem.testId);
  }
  if (widget->isEnabled() == sem.disabled) {
    widget->setEnabled(!sem.disabled);
  }

  if (prev_name != sem.label) {
    QAccessibleEvent ev(widget, QAccessible::NameChanged);
    QAccessible::updateAccessibility(&ev);
  }
  if (prev_desc != sem.hint) {
    QAccessibleEvent ev(widget, QAccessible::DescriptionChanged);
    QAccessible::updateAccessibility(&ev);
  }
}

/// IME-safe `QLineEdit` subclass supporting `Qt::WA_InputMethodEnabled`, preedit vs commit
/// disambiguation without dropping multi-byte CJK/accented characters, `placeholder`,
/// `secureTextEntry`, focus/blur, and Return-key submit.
class NveQtLineEdit final : public QLineEdit {
 public:
  using TextChangeHandler = std::function<void(uint32_t, std::string_view)>;
  using SubmitHandler = std::function<void(uint32_t, std::string_view)>;
  using FocusHandler = std::function<void(uint32_t, bool)>;

  NveQtLineEdit(
      uint32_t node_id,
      TextChangeHandler on_text_change,
      SubmitHandler on_submit,
      FocusHandler on_focus,
      QWidget* parent = nullptr)
      : QLineEdit(parent),
        node_id_(node_id),
        on_text_change_(std::move(on_text_change)),
        on_submit_(std::move(on_submit)),
        on_focus_(std::move(on_focus)) {
    setAttribute(Qt::WA_InputMethodEnabled, true);
    setProperty(kNodeIdProperty, QVariant::fromValue<qulonglong>(node_id));

    connect(this, &QLineEdit::textEdited, this, [this](const QString&) {
      if (!updating_programmatically_ && !composing_ime_) {
        dispatchCurrentTextIfChanged();
      }
    });
    connect(this, &QLineEdit::returnPressed, this, [this]() {
      if (!composing_ime_ && on_submit_) {
        const QByteArray utf8 = text().toUtf8();
        auto cb = on_submit_;
        const uint32_t id = node_id_;
        cb(id, std::string_view(utf8.constData(), static_cast<size_t>(utf8.size())));
      }
    });
  }

  void applySemantics(const NodeSemantics& sem) {
    if (placeholderText() != sem.placeholder) {
      setPlaceholderText(sem.placeholder);
    }
    const EchoMode target_mode = sem.secureTextEntry ? QLineEdit::Password : QLineEdit::Normal;
    if (echoMode() != target_mode) {
      setEchoMode(target_mode);
    }
  }

  void applySceneText(const QString& next_text) {
    // Never clobber live IME composition mid-preedit.
    if (composing_ime_) {
      return;
    }
    if (!has_synced_scene_ || (next_text != last_scene_text_ && next_text != text())) {
      updating_programmatically_ = true;
      setText(next_text);
      last_dispatched_text_ = next_text;
      updating_programmatically_ = false;
    }
    last_scene_text_ = next_text;
    has_synced_scene_ = true;
  }

 protected:
  void inputMethodEvent(QInputMethodEvent* event) override {
    if (event == nullptr) {
      QLineEdit::inputMethodEvent(event);
      return;
    }
    const QString preedit = event->preeditString();
    const QString commit = event->commitString();
    const bool has_commit = !commit.isEmpty() || event->replacementLength() > 0;
    composing_ime_ = !preedit.isEmpty();

    QLineEdit::inputMethodEvent(event);

    if (has_commit && !composing_ime_) {
      dispatchCurrentTextIfChanged();
    }
  }

  void focusInEvent(QFocusEvent* event) override {
    QLineEdit::focusInEvent(event);
    if (on_focus_) {
      auto cb = on_focus_;
      const uint32_t id = node_id_;
      cb(id, true);
    }
  }

  void focusOutEvent(QFocusEvent* event) override {
    QLineEdit::focusOutEvent(event);
    if (on_focus_) {
      auto cb = on_focus_;
      const uint32_t id = node_id_;
      cb(id, false);
    }
  }

 private:
  void dispatchCurrentTextIfChanged() {
    const QString current = text();
    if (current == last_dispatched_text_) {
      return;
    }
    last_dispatched_text_ = current;
    if (on_text_change_) {
      const QByteArray utf8 = current.toUtf8();
      auto cb = on_text_change_;
      const uint32_t id = node_id_;
      cb(id, std::string_view(utf8.constData(), static_cast<size_t>(utf8.size())));
    }
  }

  uint32_t node_id_{0};
  TextChangeHandler on_text_change_;
  SubmitHandler on_submit_;
  FocusHandler on_focus_;
  QString last_dispatched_text_;
  QString last_scene_text_;
  bool has_synced_scene_{false};
  bool composing_ime_{false};
  bool updating_programmatically_{false};
};

inline float opacityFromU8(uint8_t opacity_u8) noexcept {
  return static_cast<float>(opacity_u8) / 255.0f;
}

void apply_widget_opacity(QWidget* widget, float opacity) {
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

/// Lightweight custom-painted container widget for `NVE_NODE_KIND_VIEW`,
/// `NVE_NODE_KIND_PRESSABLE`, and wrapper `NVE_NODE_KIND_TEXT` nodes.
class NveQtBoxWidget final : public QWidget {
 public:
  using PressHandler = std::function<void(uint32_t, float, float)>;

  explicit NveQtBoxWidget(uint32_t node_id, PressHandler on_press, QWidget* parent = nullptr)
      : QWidget(parent), node_id_(node_id), on_press_(std::move(on_press)) {
    setAutoFillBackground(false);
    setAttribute(Qt::WA_NoSystemBackground, true);
    setProperty(kNodeIdProperty, QVariant::fromValue<qulonglong>(node_id));
  }

  void applyStyle(const NveRenderNode& node, bool interactive) {
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

  void setInteractive(bool interactive) {
    if (interactive_ != interactive) {
      interactive_ = interactive;
      if (interactive_) {
        setCursor(Qt::PointingHandCursor);
      } else {
        unsetCursor();
      }
    }
  }

 protected:
  void paintEvent(QPaintEvent* /*event*/) override {
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

  void mousePressEvent(QMouseEvent* event) override {
    if (interactive_ && event->button() == Qt::LeftButton) {
      pressed_ = true;
      event->accept();
      return;
    }
    event->ignore();
  }

  void mouseReleaseEvent(QMouseEvent* event) override {
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

 private:
  uint32_t node_id_{0};
  PressHandler on_press_;
  QColor bg_color_{0, 0, 0, 0};
  QColor border_color_{0, 0, 0, 0};
  float border_width_{0.0f};
  float border_radius_{0.0f};
  float opacity_{1.0f};
  bool interactive_{false};
  bool pressed_{false};
};

/// Native `QWidget` for `NVE_NODE_KIND_IMAGE` rendering cached `QPixmap`s from `NveQtImageCache`
/// with all 4 React Native `resizeMode`s (`cover`, `contain`, `stretch`, `center`) and rounded clip.
class NveQtImageWidget final : public QWidget {
 public:
  using PressHandler = std::function<void(uint32_t, float, float)>;

  explicit NveQtImageWidget(uint32_t node_id, PressHandler on_press, QWidget* parent = nullptr)
      : QWidget(parent), node_id_(node_id), on_press_(std::move(on_press)) {
    setAutoFillBackground(false);
    setAttribute(Qt::WA_NoSystemBackground, true);
    setProperty(kNodeIdProperty, QVariant::fromValue<qulonglong>(node_id));
    setProperty(kImageDecodedProperty, false);
    setProperty(kImageResizeModeProperty, static_cast<int>(ImageResizeMode::Cover));
  }

  void applyNode(
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

  void setResizeMode(ImageResizeMode resize_mode) {
    if (resize_mode_ != resize_mode) {
      resize_mode_ = resize_mode;
      setProperty(kImageResizeModeProperty, static_cast<int>(resize_mode_));
      update();
    }
  }

  void setInteractive(bool interactive) {
    if (interactive_ != interactive) {
      interactive_ = interactive;
      if (interactive_) {
        setCursor(Qt::PointingHandCursor);
      } else {
        unsetCursor();
      }
    }
  }

 protected:
  void paintEvent(QPaintEvent* /*event*/) override {
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

  void mousePressEvent(QMouseEvent* event) override {
    if (interactive_ && event->button() == Qt::LeftButton) {
      pressed_ = true;
      event->accept();
      return;
    }
    event->ignore();
  }

  void mouseReleaseEvent(QMouseEvent* event) override {
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

 private:
  uint32_t node_id_{0};
  PressHandler on_press_;
  QColor bg_color_{0, 0, 0, 0};
  QColor border_color_{0, 0, 0, 0};
  float border_width_{0.0f};
  float border_radius_{0.0f};
  float opacity_{1.0f};
  bool interactive_{false};
  bool pressed_{false};
  std::string image_src_;
  ImageResizeMode resize_mode_{ImageResizeMode::Cover};
  QPixmap pixmap_;
};

} // namespace

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

class NveQtWidget::Impl {
 public:
  explicit Impl(NveQtWidget* owner) : owner_(owner), adapter_(this) {
    ensureAccessibleFactoryInstalled();
    session_.set_text_measurer(NveQtWidget::makeQtTextMeasurer());
  }

  ~Impl() {
    destroying_ = true;
    reconciler_.clear(adapter_);
    node_semantics_.clear();
    node_resize_modes_.clear();
    node_interactive_.clear();
    session_.reset();
  }

  NveSurfaceConfig currentSurfaceConfig(uint32_t surface_id) const {
    NveSurfaceConfig cfg{};
    cfg.surface_id = surface_id;
    cfg.width = static_cast<float>(owner_->width() > 0 ? owner_->width() : 480);
    cfg.height = static_cast<float>(owner_->height() > 0 ? owner_->height() : 320);
    const qreal dpr = owner_->devicePixelRatioF();
    cfg.scale_factor = static_cast<float>(dpr > 0.0 ? dpr : 1.0);
    return cfg;
  }

  void refreshRawPropsFromView(const NveSceneView& view) {
    const auto snapshots = session_.snapshot_nodes();
    const auto raw_props = extractRawPropsByNode(snapshots);

    std::unordered_map<uint32_t, NodeSemantics> next_semantics;
    next_semantics.reserve(view.node_count);
    std::unordered_map<uint32_t, ImageResizeMode> next_resize_modes;

    for (uint32_t i = 0u; i < view.node_count; ++i) {
      const NveRenderNode& node = view.nodes[i];
      const auto rp_it = raw_props.find(node.node_id);
      const nve::RawPropMap* rp = (rp_it != raw_props.end()) ? &rp_it->second : nullptr;
      const std::string_view text = nve::scene_node_text(view, node);
      NodeSemantics sem = extractNodeSemantics(node, text, rp);

      const auto prev_sem_it = node_semantics_.find(node.node_id);
      const bool sem_changed = (prev_sem_it == node_semantics_.end() || prev_sem_it->second != sem);
      next_semantics.emplace(node.node_id, sem);

      ImageResizeMode mode = ImageResizeMode::Cover;
      bool mode_changed = false;
      if (node.kind == NVE_NODE_KIND_IMAGE) {
        mode = extractImageResizeMode(rp);
        const auto prev_mode_it = node_resize_modes_.find(node.node_id);
        mode_changed = (prev_mode_it == node_resize_modes_.end() || prev_mode_it->second != mode);
        next_resize_modes.emplace(node.node_id, mode);
      }

      // Apply raw-prop-only deltas directly to already-mounted widgets (e.g., disabled,
      // secureTextEntry, placeholder, resizeMode) even if NveSceneView did not mark layout/style dirty.
      if (sem_changed || mode_changed) {
        if (node.node_id == NVE_ROOT_NODE_ID) {
          applyWidgetAccessibility(owner_, sem);
        } else if (QWidget** slot = reconciler_.find(node.node_id); slot != nullptr && *slot != nullptr) {
          QWidget* w = *slot;
          if (sem_changed) {
            applyWidgetAccessibility(w, sem);
            const bool base_interactive =
                (node.kind == NVE_NODE_KIND_PRESSABLE) || ((node.flags & NVE_NODE_INTERACTIVE) != 0u);
            const bool eff_interactive = !sem.disabled && base_interactive;
            if (auto* box = dynamic_cast<NveQtBoxWidget*>(w)) {
              box->setInteractive(eff_interactive);
            } else if (auto* img = dynamic_cast<NveQtImageWidget*>(w)) {
              img->setInteractive(eff_interactive);
            } else if (auto* canvas = qobject_cast<NveQtCanvasWidget*>(w)) {
              canvas->setInteractive(eff_interactive);
            } else if (auto* edit = dynamic_cast<NveQtLineEdit*>(w)) {
              edit->applySemantics(sem);
            } else if (auto* scroll = qobject_cast<QScrollArea*>(w)) {
              if (auto* inner = dynamic_cast<NveQtBoxWidget*>(scroll->widget())) {
                inner->setInteractive(eff_interactive);
              }
            }
          }
          if (mode_changed) {
            if (auto* img = dynamic_cast<NveQtImageWidget*>(w)) {
              img->setResizeMode(mode);
            }
          }
        }
      }
    }

    node_semantics_ = std::move(next_semantics);
    node_resize_modes_ = std::move(next_resize_modes);
  }

  void syncViews(bool check_raw_props = false) {
    NveFabricHostHandle host = session_.active_fabric_host();
    if (host == nullptr) {
      return;
    }

    const NveSceneView view = session_.scene_view();
    if (view.schema_hash != NVE_SCENE_SCHEMA_HASH) {
      return;
    }

    // Prune removed metadata first.
    for (uint32_t i = 0u; i < view.removed_count; ++i) {
      const uint32_t rid = view.removed_ids[i];
      node_semantics_.erase(rid);
      node_resize_modes_.erase(rid);
      node_interactive_.erase(rid);
    }

    bool need_raw_props = check_raw_props || !raw_props_initialized_;
    if (!need_raw_props && view.nodes != nullptr) {
      for (uint32_t i = 0u; i < view.dirty_count; ++i) {
        const uint32_t idx = view.dirty_indices[i];
        if (idx < view.node_count) {
          const uint8_t k = view.nodes[idx].kind;
          if (k == NVE_NODE_KIND_IMAGE || k == NVE_NODE_KIND_TEXT_INPUT) {
            need_raw_props = true;
            break;
          }
        }
      }
    }

    in_sync_views_ = true;
    if (need_raw_props) {
      refreshRawPropsFromView(view);
      raw_props_initialized_ = true;
    }
    (void)reconciler_.sync(view, adapter_);
    in_sync_views_ = false;
    owner_->update();
  }

  void clearViews() {
    in_sync_views_ = true;
    reconciler_.clear(adapter_);
    node_semantics_.clear();
    node_resize_modes_.clear();
    node_interactive_.clear();
    raw_props_initialized_ = false;
    root_bg_color_ = QColor(0, 0, 0, 0);
    in_sync_views_ = false;
    owner_->update();
  }

  const NodeSemantics* findSemantics(uint32_t node_id) const noexcept {
    const auto it = node_semantics_.find(node_id);
    return (it != node_semantics_.end()) ? &it->second : nullptr;
  }

  ImageResizeMode findResizeMode(uint32_t node_id) const noexcept {
    const auto it = node_resize_modes_.find(node_id);
    return (it != node_resize_modes_.end()) ? it->second : ImageResizeMode::Cover;
  }

  void handlePressFromWidget(uint32_t node_id, float x, float y) {
    (void)dispatchPress(node_id, x, y);
  }

  void handleScrollFromWidget(uint32_t node_id, QScrollArea* scroll) {
    if (in_sync_views_ || destroying_ || scroll == nullptr) {
      return;
    }
    const QScrollBar* hbar = scroll->horizontalScrollBar();
    const QScrollBar* vbar = scroll->verticalScrollBar();
    const float ox = (hbar != nullptr) ? static_cast<float>(hbar->value()) : 0.0f;
    const float oy = (vbar != nullptr) ? static_cast<float>(vbar->value()) : 0.0f;
    const QWidget* inner = scroll->widget();
    const float cw = static_cast<float>(
        (inner != nullptr) ? std::max(inner->width(), scroll->width()) : scroll->width());
    const float ch = static_cast<float>(
        (inner != nullptr) ? std::max(inner->height(), scroll->height()) : scroll->height());
    (void)dispatchScroll(node_id, ox, oy, cw, ch);
  }

  NveDirectEvent makeBaseEvent(uint32_t node_id, nve::EventKind kind, uint8_t payload_tag) const {
    NveDirectEvent ev{};
    ev.surface_id = surface_id_;
    ev.node_id = node_id;
    ev.timestamp_us = nve::current_timestamp_us();
    ev.kind = static_cast<uint8_t>(kind);
    ev.payload_tag = payload_tag;
    ev.value = nve::make_null_value();
    return ev;
  }

  uint32_t dispatchSingleDirectEvent(const NveDirectEvent& ev) {
    if (!session_.is_valid()) {
      return 0;
    }
    const uint32_t dirty =
        session_.dispatch_direct_events(NveEventBatch{&ev, 1u}, /*auto_render=*/true);
    if (dirty != 0) {
      syncViews(/*check_raw_props=*/false);
    }
    (void)session_.drain_events();
    return dirty;
  }

  uint32_t dispatchPress(uint32_t node_id, float x, float y) {
    if (!session_.is_valid()) {
      return 0;
    }
    const uint32_t dirty = session_.dispatch_press(node_id, x, y, /*auto_render=*/true);
    if (dirty != 0) {
      syncViews(/*check_raw_props=*/false);
    }
    (void)session_.drain_events();
    return dirty;
  }

  uint32_t dispatchScroll(
      uint32_t node_id,
      float offset_x,
      float offset_y,
      float content_width,
      float content_height) {
    NveDirectEvent ev = makeBaseEvent(node_id, nve::EventKind::Scroll, 2u);
    ev.x = offset_x;
    ev.y = offset_y;
    ev.width = content_width;
    ev.height = content_height;
    return dispatchSingleDirectEvent(ev);
  }

  uint32_t dispatchTextChange(uint32_t node_id, std::string_view text) {
    NveDirectEvent ev = makeBaseEvent(node_id, nve::EventKind::TextChange, 3u);
    ev.text = nve::make_str_slice(text);
    return dispatchSingleDirectEvent(ev);
  }

  uint32_t dispatchSubmit(uint32_t node_id, std::string_view text) {
    NveDirectEvent ev = makeBaseEvent(node_id, nve::EventKind::Custom, 5u);
    ev.name = nve::make_str_slice("onSubmit");
    ev.text = nve::make_str_slice(text);
    ev.value = nve::make_str_value(text);
    return dispatchSingleDirectEvent(ev);
  }

  uint32_t dispatchFocus(uint32_t node_id, bool focused) {
    NveDirectEvent ev =
        makeBaseEvent(node_id, focused ? nve::EventKind::Focus : nve::EventKind::Blur, 0u);
    return dispatchSingleDirectEvent(ev);
  }

  struct QtViewAdapter {
    Impl* self{nullptr};

    void update_root(const NveSceneView& /*view*/, const NveRenderNode& node) {
      const QColor next_bg = qcolorFromRgba(node.bg_rgba);
      if (self->root_bg_color_ != next_bg) {
        self->root_bg_color_ = next_bg;
        self->owner_->update();
      }
      if (const NodeSemantics* sem = self->findSemantics(node.node_id)) {
        applyWidgetAccessibility(self->owner_, *sem);
      }
    }

    QWidget* create_view(const NveSceneView& /*view*/, const NveRenderNode& node) {
      const bool is_leaf_text = (node.flags & NVE_NODE_LEAF_TEXT) != 0u;
      const bool is_wrapper_text = (node.kind == NVE_NODE_KIND_TEXT && !is_leaf_text);
      QWidget* widget = nullptr;

      if (is_wrapper_text) {
        widget = new NveQtBoxWidget(
            node.node_id,
            [owner_impl = self](uint32_t nid, float x, float y) {
              owner_impl->handlePressFromWidget(nid, x, y);
            },
            self->owner_);
      } else {
        switch (node.kind) {
          case NVE_NODE_KIND_TEXT: {
            auto* label = new QLabel(self->owner_);
            label->setWordWrap(true);
            label->setAlignment(Qt::AlignLeft | Qt::AlignVCenter);
            label->setContentsMargins(0, 0, 0, 0);
            label->setMargin(0);
            label->setIndent(0);
            label->setTextFormat(Qt::PlainText);
            label->setAutoFillBackground(false);
            label->setAttribute(Qt::WA_TransparentForMouseEvents, true);
            label->setProperty(kNodeIdProperty, QVariant::fromValue<qulonglong>(node.node_id));
            widget = label;
            break;
          }
          case NVE_NODE_KIND_IMAGE: {
            widget = new NveQtImageWidget(
                node.node_id,
                [owner_impl = self](uint32_t nid, float x, float y) {
                  owner_impl->handlePressFromWidget(nid, x, y);
                },
                self->owner_);
            break;
          }
          case NVE_NODE_KIND_SKIA_CANVAS: {
            widget = new NveQtCanvasWidget(
                node.node_id,
                [owner_impl = self](uint32_t nid, float x, float y) {
                  owner_impl->handlePressFromWidget(nid, x, y);
                },
                self->owner_);
            break;
          }
          case NVE_NODE_KIND_SCROLL_VIEW: {
            auto* scroll = new QScrollArea(self->owner_);
            scroll->setFrameShape(QFrame::NoFrame);
            scroll->setWidgetResizable(false);
            scroll->setAutoFillBackground(false);
            if (scroll->viewport() != nullptr) {
              scroll->viewport()->setAutoFillBackground(false);
            }
            auto* inner = new NveQtBoxWidget(
                node.node_id,
                [owner_impl = self](uint32_t nid, float x, float y) {
                  owner_impl->handlePressFromWidget(nid, x, y);
                },
                scroll);
            scroll->setWidget(inner);
            scroll->setProperty(kNodeIdProperty, QVariant::fromValue<qulonglong>(node.node_id));

            const uint32_t scroll_node_id = node.node_id;
            if (QScrollBar* vbar = scroll->verticalScrollBar()) {
              QObject::connect(vbar, &QScrollBar::valueChanged, scroll, [owner_impl = self, scroll_node_id, scroll](int) {
                owner_impl->handleScrollFromWidget(scroll_node_id, scroll);
              });
            }
            if (QScrollBar* hbar = scroll->horizontalScrollBar()) {
              QObject::connect(hbar, &QScrollBar::valueChanged, scroll, [owner_impl = self, scroll_node_id, scroll](int) {
                owner_impl->handleScrollFromWidget(scroll_node_id, scroll);
              });
            }
            widget = scroll;
            break;
          }
          case NVE_NODE_KIND_TEXT_INPUT: {
            auto* edit = new NveQtLineEdit(
                node.node_id,
                [owner_impl = self](uint32_t nid, std::string_view text) {
                  (void)owner_impl->dispatchTextChange(nid, text);
                },
                [owner_impl = self](uint32_t nid, std::string_view text) {
                  (void)owner_impl->dispatchSubmit(nid, text);
                },
                [owner_impl = self](uint32_t nid, bool focused) {
                  (void)owner_impl->dispatchFocus(nid, focused);
                },
                self->owner_);
            widget = edit;
            break;
          }
          default: {
            widget = new NveQtBoxWidget(
                node.node_id,
                [owner_impl = self](uint32_t nid, float x, float y) {
                  owner_impl->handlePressFromWidget(nid, x, y);
                },
                self->owner_);
            break;
          }
        }
      }

      return widget;
    }

    void update_view(const NveSceneView& view, QWidget*& widget, const NveRenderNode& node) {
      if (widget == nullptr) {
        return;
      }
      ++self->reconcile_update_count_;

      const std::string_view text = nve::scene_node_text(view, node);
      const NodeSemantics default_sem = extractNodeSemantics(node, text, nullptr);
      const NodeSemantics& sem =
          self->findSemantics(node.node_id) ? *self->findSemantics(node.node_id) : default_sem;

      const int x = static_cast<int>(std::lround(node.x));
      const int y = static_cast<int>(std::lround(node.y));
      const int w = std::max(0, static_cast<int>(std::lround(node.width)));
      const int h = std::max(0, static_cast<int>(std::lround(node.height)));
      widget->setGeometry(x, y, w, h);

      const bool interactive =
          !sem.disabled &&
          (node.kind == NVE_NODE_KIND_PRESSABLE || (node.flags & NVE_NODE_INTERACTIVE) != 0u);
      self->node_interactive_[node.node_id] = interactive;

      const float opacity = opacityFromU8(node.opacity_u8);

      if (auto* box = dynamic_cast<NveQtBoxWidget*>(widget)) {
        box->applyStyle(node, interactive);
      } else if (auto* img = dynamic_cast<NveQtImageWidget*>(widget)) {
        img->applyNode(node, text, self->findResizeMode(node.node_id), interactive);
      } else if (auto* canvas = qobject_cast<NveQtCanvasWidget*>(widget)) {
        const std::span<const uint8_t> raw_bytes(
            reinterpret_cast<const uint8_t*>(text.data()), text.size());
        canvas->applyNode(node, raw_bytes, interactive);
      } else if (auto* label = qobject_cast<QLabel*>(widget)) {
        const QString qtext = detail::qstringFromUtf8(text);
        if (label->text() != qtext) {
          label->setText(qtext);
        }
        const QFont font = makeNveFont(node.font_size);
        if (label->font().pixelSize() != font.pixelSize() ||
            label->font().family() != font.family()) {
          label->setFont(font);
        }

        QPalette pal = label->palette();
        const QColor fg = qcolorFromRgba(node.text_rgba);
        pal.setColor(QPalette::WindowText, fg);
        pal.setColor(QPalette::Text, fg);
        if (node.bg_rgba != 0u) {
          pal.setColor(QPalette::Window, qcolorFromRgba(node.bg_rgba));
          label->setAutoFillBackground(true);
        } else {
          pal.setColor(QPalette::Window, Qt::transparent);
          label->setAutoFillBackground(false);
        }
        label->setPalette(pal);
        apply_widget_opacity(label, opacity);
      } else if (auto* edit = dynamic_cast<NveQtLineEdit*>(widget)) {
        const QString qtext = detail::qstringFromUtf8(text);
        edit->applySemantics(sem);
        edit->applySceneText(qtext);
        edit->setFont(makeNveFont(node.font_size));
        QPalette pal = edit->palette();
        if (node.text_rgba != 0u) {
          pal.setColor(QPalette::Text, qcolorFromRgba(node.text_rgba));
        }
        if (node.bg_rgba != 0u) {
          pal.setColor(QPalette::Base, qcolorFromRgba(node.bg_rgba));
        }
        edit->setPalette(pal);
        apply_widget_opacity(edit, opacity);
      } else if (auto* scroll = qobject_cast<QScrollArea*>(widget)) {
        if (auto* inner = dynamic_cast<NveQtBoxWidget*>(scroll->widget())) {
          inner->applyStyle(node, interactive);
          if (inner->width() < w || inner->height() < h) {
            inner->resize(std::max(inner->width(), w), std::max(inner->height(), h));
          }
        }
        apply_widget_opacity(scroll, opacity);
      }

      applyWidgetAccessibility(widget, sem);
    }

    void destroy_view(uint32_t node_id, QWidget*& view) {
      self->node_semantics_.erase(node_id);
      self->node_resize_modes_.erase(node_id);
      self->node_interactive_.erase(node_id);
      if (view == nullptr) {
        return;
      }
      QWidget* child_host = view;
      if (auto* scroll = qobject_cast<QScrollArea*>(view);
          scroll != nullptr && scroll->widget() != nullptr) {
        child_host = scroll->widget();
      }
      const QObjectList direct_children = child_host->children();
      for (QObject* obj : direct_children) {
        if (auto* child_widget = qobject_cast<QWidget*>(obj);
            child_widget != nullptr && child_widget->property(kNodeIdProperty).isValid()) {
          child_widget->hide();
          child_widget->setParent(nullptr);
        }
      }

      view->hide();
      view->setParent(nullptr);
      if (self->destroying_) {
        delete view;
      } else {
        view->deleteLater();
      }
      view = nullptr;
    }

    template <typename LookupFn>
    void sync_children(
        uint32_t parent_id,
        QWidget** parent_view,
        std::span<const uint32_t> child_ids,
        LookupFn&& lookup_fn) {
      QWidget* container = (parent_id == NVE_ROOT_NODE_ID)
                               ? static_cast<QWidget*>(self->owner_)
                               : ((parent_view != nullptr) ? *parent_view : nullptr);
      if (container == nullptr) {
        return;
      }

      QScrollArea* scroll_parent = qobject_cast<QScrollArea*>(container);
      if (scroll_parent != nullptr && scroll_parent->widget() != nullptr) {
        container = scroll_parent->widget();
      }

      int max_right = container->width();
      int max_bottom = container->height();
      if (scroll_parent != nullptr) {
        max_right = scroll_parent->width();
        max_bottom = scroll_parent->height();
      }

      for (uint32_t child_id : child_ids) {
        QWidget** child_slot = lookup_fn(child_id);
        if (child_slot == nullptr || *child_slot == nullptr) {
          continue;
        }
        QWidget* child = *child_slot;
        const QRect saved_geom = child->geometry();
        if (child->parentWidget() != container) {
          child->setParent(container);
          child->setGeometry(saved_geom);
        }
        child->raise();
        if (child->isHidden()) {
          child->show();
        }
        if (scroll_parent != nullptr) {
          max_right = std::max(max_right, saved_geom.right() + 1);
          max_bottom = std::max(max_bottom, saved_geom.bottom() + 1);
        }
      }

      if (scroll_parent != nullptr) {
        container->resize(std::max(0, max_right), std::max(0, max_bottom));
      }
    }
  };

  NveQtWidget* owner_{nullptr};
  nve::NativeHostSession session_;
  QtViewAdapter adapter_;
  nve::IncrementalSceneReconciler<QWidget*, QtViewAdapter> reconciler_;
  std::unordered_map<uint32_t, NodeSemantics> node_semantics_;
  std::unordered_map<uint32_t, ImageResizeMode> node_resize_modes_;
  std::unordered_map<uint32_t, bool> node_interactive_;
  QColor root_bg_color_{0, 0, 0, 0};
  uint32_t surface_id_{1};
  uint64_t reconcile_update_count_{0};
  bool raw_props_initialized_{false};
  bool in_sync_views_{false};
  bool destroying_{false};
};

NveQtWidget::NveQtWidget(QWidget* parent) : QWidget(parent), impl_(std::make_unique<Impl>(this)) {
  setAutoFillBackground(false);
}

NveQtWidget::~NveQtWidget() = default;

nve::TextMeasurerCallback NveQtWidget::makeQtTextMeasurer() {
  return makeSharedQtTextMeasurer();
}

bool NveQtWidget::attachInterface(NveGetInterfaceFn getInterface, uint32_t surfaceId) {
  const NveSurfaceConfig cfg = impl_->currentSurfaceConfig(surfaceId);
  if (!impl_->session_.attach_interface(getInterface, cfg)) {
    return false;
  }
  impl_->surface_id_ = surfaceId;
  impl_->clearViews();
  impl_->syncViews(/*check_raw_props=*/true);
  (void)impl_->session_.drain_events();
  return true;
}

bool NveQtWidget::loadDylib(
    const QString& path,
    uint32_t surfaceId,
    int* outErrorCode,
    QString* outErrorMessage) {
  const QByteArray utf8 = path.toUtf8();
  const NveSurfaceConfig cfg = impl_->currentSurfaceConfig(surfaceId);
  int err_code = 0;
  std::string err_msg;
  const bool ok = impl_->session_.load_dylib(
      std::string_view(utf8.constData(), static_cast<size_t>(utf8.size())),
      cfg,
      &err_code,
      &err_msg);
  if (outErrorCode != nullptr) {
    *outErrorCode = err_code;
  }
  if (outErrorMessage != nullptr) {
    *outErrorMessage = QString::fromStdString(err_msg);
  }
  if (!ok) {
    return false;
  }
  impl_->surface_id_ = surfaceId;
  impl_->clearViews();
  impl_->syncViews(/*check_raw_props=*/true);
  (void)impl_->session_.drain_events();
  return true;
}

bool NveQtWidget::reloadDylib(
    const QString& path,
    int* outErrorCode,
    QString* outErrorMessage) {
  const QByteArray utf8 = path.toUtf8();
  if (!impl_->session_.is_valid()) {
    const NveSurfaceConfig cfg = impl_->currentSurfaceConfig(impl_->surface_id_);
    impl_->session_.set_surface_size(cfg.width, cfg.height, cfg.scale_factor);
  }
  int err_code = 0;
  std::string err_msg;
  const bool ok = impl_->session_.reload_dylib(
      std::string_view(utf8.constData(), static_cast<size_t>(utf8.size())),
      &err_code,
      &err_msg);
  if (outErrorCode != nullptr) {
    *outErrorCode = err_code;
  }
  if (outErrorMessage != nullptr) {
    *outErrorMessage = QString::fromStdString(err_msg);
  }
  if (!ok) {
    return false;
  }
  impl_->clearViews();
  impl_->syncViews(/*check_raw_props=*/true);
  (void)impl_->session_.drain_events();
  return true;
}

bool NveQtWidget::replayFrame0Snapshot(
    const QString& snapshotPath,
    uint32_t expectedSchemaVersion,
    uint32_t surfaceId,
    uint64_t* outRestoreUs) {
  const QByteArray utf8 = snapshotPath.toUtf8();
  const NveSurfaceConfig cfg = impl_->currentSurfaceConfig(surfaceId);
  impl_->session_.set_surface_config(cfg);

  using Clock = std::chrono::steady_clock;
  const auto t0 = Clock::now();
  const bool ok = impl_->session_.replay_frame0_snapshot(
      std::string_view(utf8.constData(), static_cast<size_t>(utf8.size())),
      expectedSchemaVersion);
  if (!ok) {
    return false;
  }
  impl_->surface_id_ = surfaceId;
  impl_->clearViews();
  impl_->syncViews(/*check_raw_props=*/true);
  const auto t1 = Clock::now();
  if (outRestoreUs != nullptr) {
    *outRestoreUs = static_cast<uint64_t>(
        std::chrono::duration_cast<std::chrono::microseconds>(t1 - t0).count());
  }
  return true;
}

bool NveQtWidget::saveSnapshot(const QString& snapshotPath, uint32_t schemaVersion) const {
  const QByteArray utf8 = snapshotPath.toUtf8();
  return impl_->session_.save_snapshot(
      std::string_view(utf8.constData(), static_cast<size_t>(utf8.size())), schemaVersion);
}

bool NveQtWidget::applyPropsJson(std::string_view propsJson) {
  if (!impl_->session_.is_valid()) {
    return false;
  }
  const bool dirty = impl_->session_.update_props_json(propsJson, /*auto_render=*/true);
  if (dirty) {
    impl_->syncViews(/*check_raw_props=*/true);
  }
  (void)impl_->session_.drain_events();
  return dirty;
}

bool NveQtWidget::applyHotPatchJson(std::string_view patchJson) {
  if (!impl_->session_.is_valid() || impl_->session_.bridge() == nullptr) {
    return false;
  }
  const bool dirty =
      impl_->session_.bridge()->apply_hot_patch_json(patchJson, /*auto_render=*/true);
  if (dirty) {
    impl_->syncViews(/*check_raw_props=*/true);
  }
  (void)impl_->session_.drain_events();
  return dirty;
}

int32_t NveQtWidget::renderFrame() {
  const int32_t rc = impl_->session_.render(nullptr);
  if (rc == 0) {
    impl_->syncViews(/*check_raw_props=*/false);
  }
  (void)impl_->session_.drain_events();
  return rc;
}

uint32_t NveQtWidget::simulatePressOnNodeId(uint32_t nodeId, float x, float y) {
  return impl_->dispatchPress(nodeId, x, y);
}

uint32_t NveQtWidget::simulatePressOnNodeId(uint32_t nodeId, const QPointF& point) {
  return impl_->dispatchPress(
      nodeId, static_cast<float>(point.x()), static_cast<float>(point.y()));
}

uint32_t NveQtWidget::dispatchTextChangeOnNodeId(uint32_t nodeId, std::string_view text) {
  return impl_->dispatchTextChange(nodeId, text);
}

uint32_t NveQtWidget::dispatchSubmitOnNodeId(uint32_t nodeId, std::string_view text) {
  return impl_->dispatchSubmit(nodeId, text);
}

uint32_t NveQtWidget::dispatchFocusOnNodeId(uint32_t nodeId, bool focused) {
  return impl_->dispatchFocus(nodeId, focused);
}

uint32_t NveQtWidget::dispatchScrollOnNodeId(
    uint32_t nodeId,
    float offsetX,
    float offsetY,
    float contentWidth,
    float contentHeight) {
  return impl_->dispatchScroll(nodeId, offsetX, offsetY, contentWidth, contentHeight);
}

uint64_t NveQtWidget::reconcileUpdateCount() const noexcept {
  return impl_->reconcile_update_count_;
}

std::optional<NodeSemantics> NveQtWidget::accessibilityInfoForNodeId(uint32_t nodeId) const {
  if (const NodeSemantics* sem = impl_->findSemantics(nodeId)) {
    return *sem;
  }
  return std::nullopt;
}

QAccessible::Role NveQtWidget::accessibleRoleForNodeId(uint32_t nodeId) const {
  if (const NodeSemantics* sem = impl_->findSemantics(nodeId)) {
    return sem->accessibleRole;
  }
  return QAccessible::NoRole;
}

QWidget* NveQtWidget::nativeWidgetForNodeId(uint32_t nodeId) const {
  if (nodeId == NVE_ROOT_NODE_ID) {
    return const_cast<NveQtWidget*>(this);
  }
  QWidget* const* slot = impl_->reconciler_.find(nodeId);
  return (slot != nullptr) ? *slot : nullptr;
}

uint32_t NveQtWidget::nodeIdForNativeWidget(const QWidget* widget) const {
  if (widget == nullptr) {
    return 0;
  }
  if (widget == this) {
    return NVE_ROOT_NODE_ID;
  }
  const QVariant v = widget->property(kNodeIdProperty);
  if (v.isValid()) {
    return static_cast<uint32_t>(v.toULongLong());
  }
  return 0;
}

uint32_t NveQtWidget::nodeCount() const {
  return impl_->session_.node_count();
}

uint32_t NveQtWidget::surfaceId() const noexcept {
  return impl_->surface_id_;
}

bool NveQtWidget::isAppLoaded() const noexcept {
  return impl_->session_.is_valid();
}

void NveQtWidget::setOnNveEvent(nve::EventCallback callback) {
  impl_->session_.set_event_callback(std::move(callback));
}

void NveQtWidget::setTextMeasurer(nve::TextMeasurerCallback measurer) {
  impl_->session_.set_text_measurer(std::move(measurer));
  if (impl_->session_.active_fabric_host() != nullptr) {
    const NveSurfaceConfig cfg = impl_->currentSurfaceConfig(impl_->surface_id_);
    impl_->session_.set_surface_size(cfg.width, cfg.height, cfg.scale_factor);
    impl_->syncViews(/*check_raw_props=*/false);
  }
}

nve::NativeHostSession& NveQtWidget::session() noexcept {
  return impl_->session_;
}

const nve::NativeHostSession& NveQtWidget::session() const noexcept {
  return impl_->session_;
}

QSize NveQtWidget::sizeHint() const {
  if (NveFabricHostHandle host = impl_->session_.active_fabric_host()) {
    nve::LayoutRect root_rect{};
    if (nve::fabric_host_get_node_layout(host, NVE_ROOT_NODE_ID, &root_rect) &&
        root_rect.width > 0.0f && root_rect.height > 0.0f) {
      return QSize(
          static_cast<int>(std::lround(root_rect.width)),
          static_cast<int>(std::lround(root_rect.height)));
    }
  }
  return QSize(640, 420);
}

void NveQtWidget::resizeEvent(QResizeEvent* event) {
  QWidget::resizeEvent(event);
  const QSize sz = event->size();
  if (sz.width() > 0 && sz.height() > 0) {
    const qreal dpr = devicePixelRatioF();
    impl_->session_.set_surface_size(
        static_cast<float>(sz.width()),
        static_cast<float>(sz.height()),
        static_cast<float>(dpr > 0.0 ? dpr : 1.0));
    if (impl_->session_.active_fabric_host() != nullptr) {
      impl_->syncViews(/*check_raw_props=*/false);
    }
  }
}

void NveQtWidget::paintEvent(QPaintEvent* /*event*/) {
  if (impl_->root_bg_color_.alpha() > 0) {
    QPainter painter(this);
    painter.fillRect(rect(), impl_->root_bg_color_);
  }
}

} // namespace nve::qt
