// Accessibility of `NveQtQuickItem`: the item exposes the ROOT node's semantics; every scene node
// is a registered, object-less `QAccessibleInterface` (role / name / description / value / state,
// press action, value + attributes) whose tree mirrors the scene. Interfaces are created lazily,
// deleted with their node, and announce semantics and focus changes.
#include "NveQtQuickInternal.hpp"

#include <QPointer>
#include <QQuickWindow>

#include <mutex>

namespace nve::qt {

namespace {

using Impl = NveQtQuickItem::Impl;

/// Base of the item and node interfaces: everything derived from one record.
class RecordAccessible : public QAccessibleInterface,
                         public QAccessibleActionInterface,
                         public SemanticsInterfaces {
 public:
  explicit RecordAccessible(NveQtQuickItem* item) : item_(item) {}

  [[nodiscard]] Impl* impl() const { return item_ != nullptr ? item_->impl() : nullptr; }
  [[nodiscard]] virtual const QuickNodeView* record() const = 0;
  const NodeSemantics& accessible_semantics() const override {
    static const NodeSemantics none;
    const QuickNodeView* v = record();
    return v != nullptr ? v->sem : none;
  }

  bool isValid() const override { return record() != nullptr; }
  QWindow* window() const override { return item_ != nullptr ? item_->window() : nullptr; }

  int childCount() const override {
    const QuickNodeView* v = record();
    return v != nullptr ? static_cast<int>(v->children.size()) : 0;
  }
  QAccessibleInterface* child(int index) const override {
    const QuickNodeView* v = record();
    if (v == nullptr || index < 0 || index >= static_cast<int>(v->children.size())) return nullptr;
    return impl()->a11y_node(v->children[static_cast<size_t>(index)]->id);
  }
  int indexOfChild(const QAccessibleInterface* child) const override {
    const QuickNodeView* v = record();
    if (v == nullptr || child == nullptr) return -1;
    for (size_t i = 0; i < v->children.size(); ++i) {
      if (impl()->a11y_node(v->children[i]->id) == child) return static_cast<int>(i);
    }
    return -1;
  }
  QAccessibleInterface* childAt(int x, int y) const override {
    for (int i = 0; i < childCount(); ++i) {
      QAccessibleInterface* c = child(i);
      if (c != nullptr && c->rect().contains(x, y)) return c;
    }
    return nullptr;
  }
  QAccessibleInterface* focusChild() const override {
    Impl* host = impl();
    const QuickNodeView* f = host != nullptr ? host->find(host->focused_) : nullptr;
    if (f == nullptr || host->focused_ == 0u) return nullptr;
    for (const QuickNodeView* p = f->parent; p != nullptr; p = p->parent) {  // descendants only
      if (p == record()) return host->a11y_node(f->id);
    }
    return nullptr;
  }

  QString text(QAccessible::Text t) const override {
    const NodeSemantics& s = accessible_semantics();
    switch (t) {
      case QAccessible::Name: return s.label;
      case QAccessible::Description: return s.hint;
      case QAccessible::Value: {
        const QuickNodeView* v = record();
        if (v != nullptr && v->is_input) return s.secure ? QString(v->input.edit.size(), QChar(0x2022)) : v->input.edit;
        return accessible_value(s);
      }
      default: return {};
    }
  }
  void setText(QAccessible::Text, const QString&) override {}
  QAccessible::Role role() const override { return accessible_semantics().role; }
  QAccessible::State state() const override {
    QAccessible::State st = accessible_state(accessible_semantics());
    const Impl* host = impl();
    const QuickNodeView* v = record();
    if (host != nullptr && v != nullptr && v->id != host::kRootIndex && host->focused_ == v->id) st.focused = true;
    if (v != nullptr && !v->visible) st.invisible = true;
    return st;
  }

  void* interface_cast(QAccessible::InterfaceType t) override {
    if (t == QAccessible::ActionInterface) return static_cast<QAccessibleActionInterface*>(this);
    return semantics_interface(t);
  }

  QStringList actionNames() const override { return press_action_names(accessible_semantics()); }
  QStringList keyBindingsForAction(const QString&) const override { return {}; }
  void doAction(const QString& name) override {
    const QuickNodeView* v = record();
    if (v == nullptr || !is_press_action(v->sem, name)) return;
    const std::string href = v->href;
    (void)impl()->activate(v->id, v->wants_press, href, static_cast<float>(v->local.width() * 0.5),
                           static_cast<float>(v->local.height() * 0.5));
  }

 protected:
  QPointer<NveQtQuickItem> item_;
};

/// The item itself: the ROOT node.
class ItemAccessible final : public RecordAccessible {
 public:
  explicit ItemAccessible(NveQtQuickItem* item) : RecordAccessible(item) {}
  const QuickNodeView* record() const override {
    const Impl* host = impl();
    return host != nullptr ? &host->root_ : nullptr;
  }
  QObject* object() const override { return item_; }
  QAccessibleInterface* parent() const override {
    if (item_ == nullptr) return nullptr;
    if (QQuickItem* p = item_->parentItem(); p != nullptr && p != item_->window()->contentItem()) {
      return QAccessible::queryAccessibleInterface(p);
    }
    return item_->window() != nullptr ? QAccessible::queryAccessibleInterface(item_->window()) : nullptr;
  }
  QRect rect() const override {
    if (item_ == nullptr || item_->window() == nullptr) return {};
    return QRectF(item_->mapToGlobal(QPointF()), item_->size()).toAlignedRect();
  }
  QAccessible::Role role() const override {
    const NodeSemantics& s = accessible_semantics();
    return s.role == QAccessible::Grouping ? QAccessible::Pane : s.role;  // an app surface
  }
};

/// One scene node.
class NodeAccessible final : public RecordAccessible {
 public:
  NodeAccessible(NveQtQuickItem* item, uint32_t id) : RecordAccessible(item), id_(id) {}
  const QuickNodeView* record() const override {
    const Impl* host = impl();
    const QuickNodeView* v = host != nullptr ? host->find(id_) : nullptr;
    return v != nullptr && v->id == id_ ? v : nullptr;  // the slot may hold a newer generation
  }
  QObject* object() const override { return nullptr; }
  QAccessibleInterface* parent() const override {
    const QuickNodeView* v = record();
    if (v == nullptr || v->parent == nullptr) return nullptr;
    return impl()->a11y_node(v->parent->id);
  }
  QRect rect() const override {
    const QuickNodeView* v = record();
    if (v == nullptr || item_ == nullptr || item_->window() == nullptr) return {};
    return QRectF(item_->mapToGlobal(impl()->origin_of(*v)), v->local.size()).toAlignedRect();
  }

 private:
  uint32_t id_;
};

QAccessibleInterface* quick_accessible_factory(const QString&, QObject* object) {
  auto* item = qobject_cast<NveQtQuickItem*>(object);
  return item != nullptr ? new ItemAccessible(item) : nullptr;
}

}  // namespace

QAccessibleInterface* NveQtQuickItem::Impl::a11y_node(uint32_t id) {
  if (host::node_index(id) == host::kRootIndex) return QAccessible::queryAccessibleInterface(owner_);
  const QuickNodeView* v = find(id);
  if (v == nullptr || v->id != id) return nullptr;
  if (const auto it = a11y_ids_.find(id); it != a11y_ids_.end()) return QAccessible::accessibleInterface(it->second);
  auto* iface = new NodeAccessible(owner_, id);
  a11y_ids_.emplace(id, QAccessible::registerAccessibleInterface(iface));
  return iface;
}

void NveQtQuickItem::Impl::a11y_node_destroyed(uint32_t id) {
  const auto it = a11y_ids_.find(id);
  if (it == a11y_ids_.end()) return;
  const QAccessible::Id unique = it->second;
  a11y_ids_.erase(it);
  QAccessible::deleteAccessibleInterface(unique);  // posts ObjectDestroyed itself
}

void NveQtQuickItem::Impl::a11y_semantics_changed(QuickNodeView& v, const NodeSemantics& before) {
  if (!QAccessible::isActive() || tearing_down_) return;
  notify_semantics_change(a11y_node(v.id), before, v.sem);
}

void NveQtQuickItem::Impl::a11y_focused(uint32_t id) {
  if (!QAccessible::isActive() || tearing_down_) return;
  if (QAccessibleInterface* iface = a11y_node(id)) {
    QAccessibleEvent ev(iface, QAccessible::Focus);
    QAccessible::updateAccessibility(&ev);
  }
}

void install_quick_accessible_factory() {
  static std::once_flag once;
  std::call_once(once, [] { QAccessible::installFactory(&quick_accessible_factory); });
}

}  // namespace nve::qt
