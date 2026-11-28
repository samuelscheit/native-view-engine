#pragma once

#include "NveQtQuickItem.hpp"
#include "qt_test_helpers.hpp"

#include <QAccessible>
#include <QKeyEvent>
#include <QQuickWindow>
#include <QWheelEvent>

#include <string>
#include <utility>
#include <vector>

namespace nve::qt::test {

/// `NveQtQuickItem` keeping the root of the scene graph it returned (render thread = GUI thread
/// with the software backend), so tests can inspect the retained `QSGNode` tree.
class ProbeItem final : public NveQtQuickItem {
 public:
  using NveQtQuickItem::NveQtQuickItem;
  const QSGNode* sg_root = nullptr;

 protected:
  QSGNode* updatePaintNode(QSGNode* old, UpdatePaintNodeData* data) override {
    QSGNode* root = NveQtQuickItem::updatePaintNode(old, data);
    sg_root = root;
    return root;
  }
};

/// A shown `QQuickWindow` whose content is one `ProbeItem` filling it (owned by the content item:
/// `setParentItem` alone is only the visual parent and would leak the item).
struct QuickHarness {
  QQuickWindow window;
  ProbeItem* item = new ProbeItem(window.contentItem());

  explicit QuickHarness(QSize size = QSize(320, 240)) {
    window.resize(size);
    item->setSize(QSizeF(size));
    window.show();
    spin();
  }
  ~QuickHarness() { window.hide(); }

  QtHost& host() { return item->host(); }
  /// Syncs the scene graph (`updatePaintNode`) and renders: the window's pixels.
  QImage grab() { return window.grabWindow(); }
  QColor at(int x, int y) { return grab().pixelColor(x, y); }
  QPointF center(uint32_t id) const { return item->nodeRect(id).center(); }
  const QSGNode* sg(uint32_t id) const { return NveQtQuickItem::retainedSGNodeForNodeId(item->sg_root, id); }

  void mouse(QEvent::Type type, QPointF pos, Qt::MouseButton button, Qt::MouseButtons buttons) {
    QMouseEvent ev(type, pos, pos, window.mapToGlobal(pos), button, buttons, Qt::NoModifier);
    QCoreApplication::sendEvent(&window, &ev);
  }
  void press(QPointF p) { mouse(QEvent::MouseButtonPress, p, Qt::LeftButton, Qt::LeftButton); }
  void release(QPointF p) { mouse(QEvent::MouseButtonRelease, p, Qt::LeftButton, Qt::NoButton); }
  void click(QPointF p) {
    press(p);
    release(p);
  }
  void move(QPointF p) { mouse(QEvent::MouseMove, p, Qt::NoButton, Qt::NoButton); }
  void wheel(QPointF p, QPoint pixel_delta, QPoint angle_delta = {}) {
    QWheelEvent ev(p, window.mapToGlobal(p), pixel_delta, angle_delta, Qt::NoButton, Qt::NoModifier,
                   Qt::NoScrollPhase, false);
    QCoreApplication::sendEvent(&window, &ev);
  }
  void key(int key, const QString& text = {}, Qt::KeyboardModifiers mods = Qt::NoModifier) {
    QKeyEvent ev(QEvent::KeyPress, key, mods, text);
    QCoreApplication::sendEvent(&window, &ev);
  }
  void type(const QString& text) {
    for (const QChar c : text) key(Qt::Key_unknown, QString(c));
  }
};

/// Accessibility events recorded while a `A11yRecorder` lives (accessibility forced active).
struct A11yRecorder {
  struct Entry {
    QAccessible::Event type;
    QString name;
    QAccessible::State changed;
  };
  static inline std::vector<Entry> events;

  A11yRecorder() {
    events.clear();
    QAccessible::installUpdateHandler(&record);
    QAccessible::installRootObjectHandler(&ignore_root);
    QAccessible::setActive(true);
  }
  ~A11yRecorder() {
    QAccessible::setActive(false);
    QAccessible::installUpdateHandler(nullptr);
    QAccessible::installRootObjectHandler(nullptr);
  }
  static size_t count(QAccessible::Event type, const QString& name) {
    size_t n = 0u;
    for (const Entry& e : events) n += e.type == type && e.name == name ? 1u : 0u;
    return n;
  }

 private:
  static void ignore_root(QObject*) {}
  static void record(QAccessibleEvent* e) {
    QAccessibleInterface* iface = e->accessibleInterface();
    QAccessible::State changed;
    if (e->type() == QAccessible::StateChanged) changed = static_cast<QAccessibleStateChangeEvent*>(e)->changedStates();
    events.push_back({e->type(), iface != nullptr ? iface->text(QAccessible::Name) : QString(), changed});
  }
};

/// Depth-first search of the accessible tree under `root` for an interface named `name`.
inline QAccessibleInterface* find_accessible(QAccessibleInterface* root, const QString& name) {
  if (root == nullptr) return nullptr;
  if (root->text(QAccessible::Name) == name) return root;
  for (int i = 0; i < root->childCount(); ++i) {
    if (QAccessibleInterface* hit = find_accessible(root->child(i), name)) return hit;
  }
  return nullptr;
}

}  // namespace nve::qt::test
