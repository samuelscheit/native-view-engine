#include "NveQtQuickItemInternal.hpp"

#include <QInputMethodEvent>
#include <QKeyEvent>
#include <QMouseEvent>
#include <QWheelEvent>

#include <algorithm>
#include <string>

namespace nve::qt {

void NveQtQuickItem::geometryChange(const QRectF& newG, const QRectF& oldG) {
  QQuickItem::geometryChange(newG, oldG);
  if (newG.size() != oldG.size() && newG.width() > 0.0 && newG.height() > 0.0) {
    impl_->session_.set_surface_size(
        static_cast<float>(newG.width()), static_cast<float>(newG.height()),
        static_cast<float>(impl_->effectiveDpr()));
    if (impl_->session_.active_fabric_host() != nullptr) impl_->syncScene(false);
  }
}

void NveQtQuickItem::mousePressEvent(QMouseEvent* event) {
  if (!event || event->button() != Qt::LeftButton) { QQuickItem::mousePressEvent(event); return; }
  const uint32_t hit = hitTestNode(event->position(), true);
  if (hit != 0u) {
    if (const NveRenderNode* n = impl_->findNode(impl_->session_.scene_view(), hit);
        n && n->kind == NVE_NODE_KIND_TEXT_INPUT) {
      forceActiveFocus(Qt::MouseFocusReason);
      impl_->setFocusedNode(hit);
    } else {
      impl_->setFocusedNode(0u);
    }
    impl_->pressed_node_id_ = hit;
    event->accept();
    return;
  }
  if (impl_->focused_node_id_ != 0u) impl_->setFocusedNode(0u);
  impl_->pressed_node_id_ = 0u;
  event->ignore();
}

void NveQtQuickItem::mouseMoveEvent(QMouseEvent* event) {
  if (!event) return;
  if (impl_->pressed_node_id_ != 0u && (event->buttons() & Qt::LeftButton)) {
    const QRectF bounds = sceneNodeGlobalBounds(impl_->pressed_node_id_);
    if (!bounds.isValid() || !bounds.contains(event->position())) impl_->pressed_node_id_ = 0u;
    event->accept();
    return;
  }
  event->ignore();
}

void NveQtQuickItem::mouseReleaseEvent(QMouseEvent* event) {
  if (!event || event->button() != Qt::LeftButton) { QQuickItem::mouseReleaseEvent(event); return; }
  const uint32_t target = impl_->pressed_node_id_;
  impl_->pressed_node_id_ = 0u;
  if (target == 0u) { event->ignore(); return; }
  const QPointF pos = event->position();
  const QRectF bounds = sceneNodeGlobalBounds(target);
  event->accept();
  if (bounds.isValid() && bounds.contains(pos)) {
    (void)dispatchPress(target, static_cast<float>(pos.x() - bounds.x()), static_cast<float>(pos.y() - bounds.y()));
  }
}

void NveQtQuickItem::wheelEvent(QWheelEvent* event) {
  if (!event) return;
  const uint32_t scroll_id = hitTestScrollableNode(event->position());
  if (scroll_id == 0u) { event->ignore(); return; }
  const NveSceneView view = impl_->session_.scene_view();
  const NveRenderNode* node = impl_->findNode(view, scroll_id);
  if (!node) { event->ignore(); return; }

  QPointF delta(event->pixelDelta());
  if (delta.isNull()) delta = QPointF(event->angleDelta()) * (45.0 / 120.0);
  const QSizeF content = computeScrollContentSize(view, impl_->id_to_index_, *node);
  const QPointF prev = impl_->scrollOffsetFor(scroll_id);
  const QPointF next(
      std::clamp(prev.x() - delta.x(), 0.0, std::max<qreal>(0.0, content.width() - node->width)),
      std::clamp(prev.y() - delta.y(), 0.0, std::max<qreal>(0.0, content.height() - node->height)));
  impl_->scroll_offsets_[scroll_id] = next;
  impl_->markLocalDirty(scroll_id);
  (void)dispatchScroll(
      scroll_id, static_cast<float>(next.x()), static_cast<float>(next.y()),
      static_cast<float>(content.width()), static_cast<float>(content.height()));
  event->accept();
}

void NveQtQuickItem::keyPressEvent(QKeyEvent* event) {
  if (!event || impl_->focused_node_id_ == 0u) { QQuickItem::keyPressEvent(event); return; }
  const uint32_t target_id = impl_->focused_node_id_;
  const NveSceneView view = impl_->session_.scene_view();
  const NveRenderNode* node = impl_->findNode(view, target_id);
  if (!node || node->kind != NVE_NODE_KIND_TEXT_INPUT) { impl_->setFocusedNode(0u); event->ignore(); return; }
  if (event->key() == Qt::Key_Escape) { impl_->setFocusedNode(0u); event->accept(); return; }

  std::string current = impl_->effectiveText(view, *node);
  const auto pre_it = impl_->ime_preedit_buffers_.find(target_id);
  const QString preedit = (pre_it != impl_->ime_preedit_buffers_.end()) ? pre_it->second : QString();
  const TextKeyAction act =
      applyTextInputKeyPress(*event, current, impl_->findSemantics(target_id), preedit);
  if (act == TextKeyAction::Submit) { (void)dispatchSubmit(target_id, current); event->accept(); return; }
  if (act == TextKeyAction::TextChanged) {
    impl_->text_input_buffers_[target_id] = current;
    impl_->markLocalDirty(target_id);
    (void)dispatchTextChange(target_id, current);
    event->accept();
    return;
  }
  event->ignore();
}

void NveQtQuickItem::inputMethodEvent(QInputMethodEvent* event) {
  if (!event || impl_->focused_node_id_ == 0u) { QQuickItem::inputMethodEvent(event); return; }
  const uint32_t target_id = impl_->focused_node_id_;
  const NveSceneView view = impl_->session_.scene_view();
  const NveRenderNode* node = impl_->findNode(view, target_id);
  if (!node || node->kind != NVE_NODE_KIND_TEXT_INPUT) { impl_->setFocusedNode(0u); event->ignore(); return; }
  std::string current = impl_->effectiveText(view, *node);
  if (applyTextInputImeEvent(*event, target_id, current, impl_->ime_preedit_buffers_)) {
    impl_->text_input_buffers_[target_id] = current;
    impl_->markLocalDirty(target_id);
    (void)dispatchTextChange(target_id, current);
  } else {
    impl_->markLocalDirty(target_id);
  }
  event->accept();
}

QVariant NveQtQuickItem::inputMethodQuery(Qt::InputMethodQuery query) const {
  const uint32_t focused = impl_->focused_node_id_;
  switch (query) {
    case Qt::ImEnabled: return focused != 0u;
    case Qt::ImCursorRectangle: return focused != 0u ? QVariant(sceneNodeGlobalBounds(focused)) : QVariant(QRectF());
    case Qt::ImSurroundingText: return focused == 0u ? QString() : detail::qstringFromUtf8(sceneNodeText(focused));
    case Qt::ImCursorPosition:
    case Qt::ImAnchorPosition: return focused == 0u ? 0 : static_cast<int>(detail::qstringFromUtf8(sceneNodeText(focused)).size());
    case Qt::ImHints: {
      const NodeSemantics* sem = impl_->findSemantics(focused);
      return static_cast<int>((sem && sem->secureTextEntry) ? (Qt::ImhHiddenText | Qt::ImhNoPredictiveText) : Qt::ImhNone);
    }
    default: return QQuickItem::inputMethodQuery(query);
  }
}

} // namespace nve::qt
