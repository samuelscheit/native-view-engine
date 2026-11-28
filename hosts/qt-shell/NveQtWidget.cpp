#include "NveQtWidgetInternal.hpp"

#include <QPainter>
#include <QResizeEvent>
#include <QScrollArea>
#include <QScrollBar>

#include <algorithm>
#include <cmath>

namespace nve::qt {

// --- Impl --------------------------------------------------------------------------------------

NveQtWidget::Impl::Impl(NveQtWidget* owner, QtHostConfig config) : QtHost(owner, config), owner_(owner) {
  install_accessible_factory();
}

NveQtWidget::Impl::~Impl() {
  tearing_down_ = true;
  shutdown();
}

QtNodeView* NveQtWidget::Impl::find(uint32_t id) noexcept {
  QtNodeView** v = tree_.find(id);
  return v != nullptr ? *v : nullptr;
}

QtNodeView* NveQtWidget::Impl::find_by_dom_id(std::string_view dom_id) noexcept {
  if (dom_id.empty()) return nullptr;
  QtNodeView* found = nullptr;
  tree_.for_each_view([&](QtNodeView*& v) {
    if (found == nullptr && v != nullptr && v->dom_id == dom_id) found = v;
  });
  return found;
}

QList<QPair<QAccessibleInterface*, QAccessible::Relation>> NveQtWidget::Impl::accessible_relations_for(
    const QtNodeView& view, QAccessible::Relation match) const {
  QList<QPair<QAccessibleInterface*, QAccessible::Relation>> out;
  auto* self = const_cast<Impl*>(this);
  const auto add_targets = [&](std::string_view idrefs, QAccessible::RelationFlag rel) {
    if ((match & rel) == 0 || idrefs.empty()) return;
    for (const std::string_view token : split_aria_idrefs(idrefs)) {
      if (QtNodeView* target = self->find_by_dom_id(token); target != nullptr && target->widget != nullptr) {
        if (QAccessibleInterface* iface = QAccessible::queryAccessibleInterface(target->widget)) {
          out.emplace_back(iface, rel);
        }
      }
    }
  };
  add_targets(view.sem.labelled_by, QAccessible::Labelled);
  add_targets(view.sem.described_by, QAccessible::Described);
  add_targets(view.sem.controls, QAccessible::Controller);
  return out;
}

QAccessibleInterface* NveQtWidget::Impl::active_descendant_for(const QtNodeView& view) const {
  if (view.sem.active_descendant.empty()) return nullptr;
  auto* self = const_cast<Impl*>(this);
  for (const std::string_view token : split_aria_idrefs(view.sem.active_descendant)) {
    if (QtNodeView* target = self->find_by_dom_id(token); target != nullptr && target->widget != nullptr) {
      return QAccessible::queryAccessibleInterface(target->widget);
    }
  }
  return nullptr;
}

void NveQtWidget::Impl::clear_views() {
  tree_.clear(adapter_);
  restack_.clear();
  scroll_dirty_.clear();
  root_synced_ = false;
}

void NveQtWidget::Impl::on_frame0_booted() {
  if (QWidget* fw = owner_->focusWidget(); fw != nullptr && owner_->isAncestorOf(fw)) {
    fw->clearFocus();
  }
  root_synced_ = false;
}

size_t NveQtWidget::Impl::sync_views(const host::RuntimeSceneGuard& scene) {
  canvas_.bind(scene.reader());
  return tree_.sync(scene.reader(), tables_, adapter_);
}

void NveQtWidget::Impl::after_sync() {
  for (QWidget* container : restack_) restack(container);
  restack_.clear();
  // Only scroll views something touched: extents, then programmatic scroll targets (once each,
  // now that the extents are known). The runtime offset follows (hit testing, clamping); this is
  // not a user scroll, so no Scroll event.
  const std::vector<QtNodeView*> dirty(scroll_dirty_.begin(), scroll_dirty_.end());
  scroll_dirty_.clear();
  for (QtNodeView* v : dirty) update_scroll_extents(*v);
  std::vector<std::pair<uint32_t, QPointF>> moved;
  for (QtNodeView* v : dirty) {
    if (!v->request_pending) continue;
    v->request_pending = false;
    QScrollBar* h = v->scroll->horizontalScrollBar();
    QScrollBar* vert = v->scroll->verticalScrollBar();
    // NaN = no request; clamp in float first (a huge target would overflow `int`).
    const auto apply = [](QScrollBar* bar, float target) {
      if (!std::isnan(target)) bar->setValue(static_cast<int>(std::lround(std::clamp(target, 0.0f, float(bar->maximum())))));
    };
    v->scrolling_programmatically = true;
    apply(h, v->requested_x);
    apply(vert, v->requested_y);
    v->scrolling_programmatically = false;
    moved.emplace_back(v->id, QPointF(h->value(), vert->value()));
  }
  for (const auto& [id, offset] : moved) {
    (void)set_scroll_offset(id, static_cast<float>(offset.x()), static_cast<float>(offset.y()));
  }
}

// --- NveQtWidget -------------------------------------------------------------------------------

NveQtWidget::NveQtWidget(QtHostConfig config, QWidget* parent)
    : QWidget(parent), impl_(std::make_unique<Impl>(this, config)) {
  setAttribute(Qt::WA_OpaquePaintEvent, false);
  setFocusPolicy(Qt::NoFocus);
  resize(static_cast<int>(std::lround(impl_->config().width)), static_cast<int>(std::lround(impl_->config().height)));
}

NveQtWidget::~NveQtWidget() = default;

QtHost& NveQtWidget::host() noexcept { return *impl_; }
const QtHost& NveQtWidget::host() const noexcept { return *impl_; }

QWidget* NveQtWidget::nativeWidgetForNodeId(uint32_t nodeId) const {
  if (host::node_index(nodeId) == host::kRootIndex) return const_cast<NveQtWidget*>(this);
  const QtNodeView* v = impl_->find(nodeId);
  return v != nullptr ? v->widget : nullptr;
}

uint32_t NveQtWidget::nodeIdForNativeWidget(const QWidget* widget) const {
  if (widget == nullptr) return 0u;
  const QVariant id = widget->property(kNodeIdProperty);
  if (!id.isValid()) return 0u;
  const auto node = static_cast<uint32_t>(id.toULongLong());
  return impl_->find(node) != nullptr ? node : 0u;
}

std::optional<NodeSemantics> NveQtWidget::accessibilityInfoForNodeId(uint32_t nodeId) const {
  if (host::node_index(nodeId) == host::kRootIndex) return impl_->root_sem_;
  const QtNodeView* v = impl_->find(nodeId);
  return v != nullptr ? std::optional<NodeSemantics>(v->sem) : std::nullopt;
}

std::span<const uint8_t> NveQtWidget::canvasDrawListForNodeId(uint32_t nodeId) const {
  const QtNodeView* v = impl_->find(nodeId);
  return v != nullptr && v->frame != nullptr ? v->frame->draw_list() : std::span<const uint8_t>{};
}

uint64_t NveQtWidget::canvasRedrawCountForNodeId(uint32_t nodeId) const {
  const QtNodeView* v = impl_->find(nodeId);
  return v != nullptr ? v->redraw_count : 0u;
}

size_t NveQtWidget::viewCount() const { return impl_->tree_.size(); }

uint64_t NveQtWidget::reconcileUpdateCount() const noexcept { return impl_->reconcile_update_count_; }
uint64_t NveQtWidget::reconcileMoveCount() const noexcept { return impl_->reconcile_move_count_; }

const NodeSemantics& NveQtWidget::semantics() const noexcept { return impl_->root_sem_; }

QSize NveQtWidget::sizeHint() const {
  return QSize(static_cast<int>(std::lround(impl_->config().width)),
               static_cast<int>(std::lround(impl_->config().height)));
}

void NveQtWidget::resizeEvent(QResizeEvent* event) {
  QWidget::resizeEvent(event);
  if (event->size().width() > 0 && event->size().height() > 0) {
    impl_->set_surface_size(static_cast<float>(event->size().width()), static_cast<float>(event->size().height()),
                            static_cast<float>(devicePixelRatioF()));
  }
}

void NveQtWidget::paintEvent(QPaintEvent*) {
  if (impl_->root_background_.alpha() == 0) return;
  QPainter p(this);
  p.fillRect(rect(), impl_->root_background_);
}

}  // namespace nve::qt
