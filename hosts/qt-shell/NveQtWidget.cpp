#include "NveQtWidget.hpp"
#include "NveQtCommon.hpp"

#include <QApplication>
#include <QColor>
#include <QFont>
#include <QGraphicsOpacityEffect>
#include <QLabel>
#include <QLineEdit>
#include <QMouseEvent>
#include <QPainter>
#include <QPainterPath>
#include <QPalette>
#include <QPen>
#include <QResizeEvent>
#include <QScrollArea>

#include <algorithm>
#include <chrono>
#include <cmath>
#include <functional>
#include <string>
#include <utility>
#include <vector>

namespace nve::qt {

namespace {

constexpr const char* kNodeIdProperty = "nveNodeId";

inline QColor qcolor_from_rgba(uint32_t rgba) noexcept {
  return qcolorFromRgba(rgba);
}

inline QFont make_nve_font(float font_size) {
  return makeNveFont(font_size);
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

/// Lightweight custom-painted container widget for `NVE_COMP_VIEW`, `NVE_COMP_PRESSABLE`, and
/// wrapper `NVE_COMP_TEXT` nodes. Avoids QSS cascade issues and renders antialiased rounded rects
/// and crisp borders via `QPainter`.
class NveQtBoxWidget final : public QWidget {
 public:
  using PressHandler = std::function<void(uint32_t, float, float)>;

  explicit NveQtBoxWidget(uint32_t node_id, PressHandler on_press, QWidget* parent = nullptr)
      : QWidget(parent), node_id_(node_id), on_press_(std::move(on_press)) {
    setAutoFillBackground(false);
    setAttribute(Qt::WA_NoSystemBackground, true);
    setProperty(kNodeIdProperty, QVariant::fromValue<qulonglong>(node_id));
  }

  void applyStyle(const nve::RenderedNodeSnapshot& snap, bool interactive) {
    const QColor next_bg = qcolor_from_rgba(snap.background_rgba);
    const QColor next_border = qcolor_from_rgba(snap.border_color_rgba);
    const float next_bw = std::max(0.0f, snap.border_width);
    const float next_br = std::max(0.0f, snap.border_radius);
    const float next_opacity = std::clamp(snap.opacity, 0.0f, 1.0f);

    const bool visual_changed = (bg_color_ != next_bg) || (border_color_ != next_border) ||
                                (border_width_ != next_bw) || (border_radius_ != next_br) ||
                                (opacity_ != next_opacity);

    bg_color_ = next_bg;
    border_color_ = next_border;
    border_width_ = next_bw;
    border_radius_ = next_br;
    opacity_ = next_opacity;

    if (interactive_ != interactive) {
      interactive_ = interactive;
      if (interactive_) {
        setCursor(Qt::PointingHandCursor);
      } else {
        unsetCursor();
      }
    }

    if (visual_changed) {
      update();
    }
  }

  uint32_t nodeId() const noexcept { return node_id_; }
  bool isInteractive() const noexcept { return interactive_; }

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
        // Copy callback and ID before invoking because the press may trigger view teardown.
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

} // namespace

class NveQtWidget::Impl {
 public:
  explicit Impl(NveQtWidget* owner) : owner_(owner), adapter_(this) {
    session_.set_text_measurer(NveQtWidget::makeQtTextMeasurer());
  }

  ~Impl() {
    destroying_ = true;
    reconciler_.clear(adapter_);
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

  void syncViews() {
    if (session_.active_fabric_host() == nullptr) {
      return;
    }
    const auto snapshots = session_.snapshot_nodes();
    reconciler_.sync(snapshots, adapter_);
    owner_->update();
  }

  void clearViews() {
    reconciler_.clear(adapter_);
    root_bg_color_ = QColor(0, 0, 0, 0);
    owner_->update();
  }

  void handlePressFromWidget(uint32_t node_id, float x, float y) {
    (void)dispatchPress(node_id, x, y);
  }

  uint32_t dispatchPress(uint32_t node_id, float x, float y) {
    if (!session_.is_valid()) {
      return 0;
    }
    const uint32_t dirty = session_.dispatch_press(node_id, x, y, /*auto_render=*/true);
    if (dirty != 0) {
      syncViews();
    }
    (void)session_.drain_events();
    return dirty;
  }

  struct QtViewAdapter {
    Impl* self{nullptr};

    void update_root(const nve::RenderedNodeSnapshot& snap) {
      const QColor next_bg = qcolor_from_rgba(snap.background_rgba);
      if (self->root_bg_color_ != next_bg) {
        self->root_bg_color_ = next_bg;
        self->owner_->update();
      }
    }

    QWidget* create_view(const nve::RenderedNodeSnapshot& snap) {
      const bool is_wrapper_text = (snap.kind == NVE_COMP_TEXT && !snap.children.empty());
      QWidget* widget = nullptr;

      if (is_wrapper_text) {
        widget = new NveQtBoxWidget(
            snap.node_id,
            [owner_impl = self](uint32_t nid, float x, float y) {
              owner_impl->handlePressFromWidget(nid, x, y);
            },
            self->owner_);
      } else {
        switch (snap.kind) {
          case NVE_COMP_TEXT: {
            auto* label = new QLabel(self->owner_);
            label->setWordWrap(true);
            label->setAlignment(Qt::AlignLeft | Qt::AlignVCenter);
            label->setContentsMargins(0, 0, 0, 0);
            label->setMargin(0);
            label->setIndent(0);
            label->setTextFormat(Qt::PlainText);
            label->setAutoFillBackground(false);
            label->setAttribute(Qt::WA_TransparentForMouseEvents, true);
            label->setProperty(kNodeIdProperty, QVariant::fromValue<qulonglong>(snap.node_id));
            widget = label;
            break;
          }
          case NVE_COMP_SCROLL_VIEW: {
            auto* scroll = new QScrollArea(self->owner_);
            scroll->setFrameShape(QFrame::NoFrame);
            scroll->setWidgetResizable(false);
            scroll->setAutoFillBackground(false);
            if (scroll->viewport() != nullptr) {
              scroll->viewport()->setAutoFillBackground(false);
            }
            auto* inner = new NveQtBoxWidget(
                snap.node_id,
                [owner_impl = self](uint32_t nid, float x, float y) {
                  owner_impl->handlePressFromWidget(nid, x, y);
                },
                scroll);
            scroll->setWidget(inner);
            scroll->setProperty(kNodeIdProperty, QVariant::fromValue<qulonglong>(snap.node_id));
            widget = scroll;
            break;
          }
          case NVE_COMP_TEXT_INPUT: {
            auto* edit = new QLineEdit(self->owner_);
            edit->setProperty(kNodeIdProperty, QVariant::fromValue<qulonglong>(snap.node_id));
            widget = edit;
            break;
          }
          default: {
            widget = new NveQtBoxWidget(
                snap.node_id,
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

    void update_view(QWidget*& view, const nve::RenderedNodeSnapshot& snap) {
      if (view == nullptr) {
        return;
      }

      const int x = static_cast<int>(std::lround(snap.layout.x));
      const int y = static_cast<int>(std::lround(snap.layout.y));
      const int w = std::max(0, static_cast<int>(std::lround(snap.layout.width)));
      const int h = std::max(0, static_cast<int>(std::lround(snap.layout.height)));
      view->setGeometry(x, y, w, h);

      const bool interactive =
          (snap.kind == NVE_COMP_PRESSABLE || (snap.event_mask & nve::kEventMaskPress) != 0);

      if (auto* box = dynamic_cast<NveQtBoxWidget*>(view)) {
        box->applyStyle(snap, interactive);
      } else if (auto* label = qobject_cast<QLabel*>(view)) {
        const QString qtext =
            QString::fromUtf8(snap.text.data(), static_cast<qsizetype>(snap.text.size()));
        if (label->text() != qtext) {
          label->setText(qtext);
        }
        const QFont font = make_nve_font(snap.font_size);
        if (label->font().pixelSize() != font.pixelSize() || label->font().family() != font.family()) {
          label->setFont(font);
        }

        QPalette pal = label->palette();
        const QColor fg = qcolor_from_rgba(snap.text_color_rgba);
        pal.setColor(QPalette::WindowText, fg);
        pal.setColor(QPalette::Text, fg);
        if (snap.background_rgba != 0) {
          pal.setColor(QPalette::Window, qcolor_from_rgba(snap.background_rgba));
          label->setAutoFillBackground(true);
        } else {
          pal.setColor(QPalette::Window, Qt::transparent);
          label->setAutoFillBackground(false);
        }
        label->setPalette(pal);
        apply_widget_opacity(label, snap.opacity);
      } else if (auto* edit = qobject_cast<QLineEdit*>(view)) {
        const QString qtext =
            QString::fromUtf8(snap.text.data(), static_cast<qsizetype>(snap.text.size()));
        if (edit->text() != qtext) {
          edit->setText(qtext);
        }
        edit->setFont(make_nve_font(snap.font_size));
        QPalette pal = edit->palette();
        if (snap.text_color_rgba != 0) {
          pal.setColor(QPalette::Text, qcolor_from_rgba(snap.text_color_rgba));
        }
        if (snap.background_rgba != 0) {
          pal.setColor(QPalette::Base, qcolor_from_rgba(snap.background_rgba));
        }
        edit->setPalette(pal);
        apply_widget_opacity(edit, snap.opacity);
      } else if (auto* scroll = qobject_cast<QScrollArea*>(view)) {
        if (auto* inner = dynamic_cast<NveQtBoxWidget*>(scroll->widget())) {
          inner->applyStyle(snap, interactive);
          if (inner->width() < w || inner->height() < h) {
            inner->resize(std::max(inner->width(), w), std::max(inner->height(), h));
          }
        }
        apply_widget_opacity(scroll, snap.opacity);
      }
    }

    void destroy_view(uint32_t /*node_id*/, QWidget*& view) {
      if (view == nullptr) {
        return;
      }
      // Detach any NVE-managed child widgets first so destroying a parent QWidget never
      // implicitly frees children that are either still live (reparented in Pass 3) or
      // scheduled for destruction later in the same Pass 1 / clear() loop.
      QWidget* child_host = view;
      if (auto* scroll = qobject_cast<QScrollArea*>(view); scroll != nullptr && scroll->widget() != nullptr) {
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
        const std::vector<uint32_t>& child_ids,
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
  nve::RetainedViewReconciler<QWidget*, QtViewAdapter> reconciler_;
  QColor root_bg_color_{0, 0, 0, 0};
  uint32_t surface_id_{1};
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
  impl_->syncViews();
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
  impl_->syncViews();
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
  impl_->syncViews();
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
  impl_->syncViews();
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
    impl_->syncViews();
  }
  (void)impl_->session_.drain_events();
  return dirty;
}

bool NveQtWidget::applyHotPatchJson(std::string_view patchJson) {
  if (!impl_->session_.is_valid() || impl_->session_.bridge() == nullptr) {
    return false;
  }
  const bool dirty = impl_->session_.bridge()->apply_hot_patch_json(patchJson, /*auto_render=*/true);
  if (dirty) {
    impl_->syncViews();
  }
  (void)impl_->session_.drain_events();
  return dirty;
}

int32_t NveQtWidget::renderFrame() {
  const int32_t rc = impl_->session_.render(nullptr);
  if (rc == 0) {
    impl_->syncViews();
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
    impl_->syncViews();
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
      impl_->syncViews();
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
