#include "NveQtWidgetInternal.hpp"

#include <QFont>
#include <QLabel>
#include <QPalette>
#include <QScrollArea>
#include <QScrollBar>

#include <algorithm>
#include <cmath>
#include <span>
#include <unordered_map>
#include <utility>

namespace nve::qt {

void NveQtWidget::Impl::refreshRawPropsFromView(const NveSceneView& view) {
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

void NveQtWidget::Impl::syncViews(bool check_raw_props) {
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

void NveQtWidget::Impl::clearViews() {
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

const NodeSemantics* NveQtWidget::Impl::findSemantics(uint32_t node_id) const noexcept {
  const auto it = node_semantics_.find(node_id);
  return (it != node_semantics_.end()) ? &it->second : nullptr;
}

ImageResizeMode NveQtWidget::Impl::findResizeMode(uint32_t node_id) const noexcept {
  const auto it = node_resize_modes_.find(node_id);
  return (it != node_resize_modes_.end()) ? it->second : ImageResizeMode::Cover;
}

void NveQtWidget::Impl::QtViewAdapter::update_root(
    const NveSceneView& /*view*/,
    const NveRenderNode& node) {
  const QColor next_bg = qcolorFromRgba(node.bg_rgba);
  if (self->root_bg_color_ != next_bg) {
    self->root_bg_color_ = next_bg;
    self->owner_->update();
  }
  if (const NodeSemantics* sem = self->findSemantics(node.node_id)) {
    applyWidgetAccessibility(self->owner_, *sem);
  }
}

QWidget* NveQtWidget::Impl::QtViewAdapter::create_view(
    const NveSceneView& /*view*/,
    const NveRenderNode& node) {
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

void NveQtWidget::Impl::QtViewAdapter::update_view(
    const NveSceneView& view,
    QWidget*& widget,
    const NveRenderNode& node) {
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
    applyWidgetOpacity(label, opacity);
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
    applyWidgetOpacity(edit, opacity);
  } else if (auto* scroll = qobject_cast<QScrollArea*>(widget)) {
    if (auto* inner = dynamic_cast<NveQtBoxWidget*>(scroll->widget())) {
      inner->applyStyle(node, interactive);
      if (inner->width() < w || inner->height() < h) {
        inner->resize(std::max(inner->width(), w), std::max(inner->height(), h));
      }
    }
    applyWidgetOpacity(scroll, opacity);
  }

  applyWidgetAccessibility(widget, sem);
}

void NveQtWidget::Impl::QtViewAdapter::destroy_view(uint32_t node_id, QWidget*& view) {
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

void NveQtWidget::Impl::syncChildrenImpl(
    uint32_t parent_id,
    QWidget** parent_view,
    const std::vector<QWidget*>& resolved_children) {
  QWidget* container = (parent_id == NVE_ROOT_NODE_ID)
                           ? static_cast<QWidget*>(owner_)
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

  for (QWidget* child : resolved_children) {
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

} // namespace nve::qt
