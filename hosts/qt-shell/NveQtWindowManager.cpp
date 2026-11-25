#include "NveQtWindowManager.hpp"

#include <QCloseEvent>
#include <QEvent>

#include <algorithm>

namespace nve::qt {

NveQtWindowManager::NveQtWindowManager(QObject* parent) : QObject(parent) {}

NveQtWindowManager::~NveQtWindowManager() {
  closeAllWindows();
}

NveQtWidget* NveQtWindowManager::openWindow(const QtWindowSpec& spec, bool showWindow) {
  if (spec.surface_id == 0u || spec.width <= 0 || spec.height <= 0 ||
      entries_.contains(spec.surface_id)) {
    return nullptr;
  }
  QWidget* parentWidget = nullptr;
  if (spec.parent_surface_id != 0u) {
    auto pit = entries_.find(spec.parent_surface_id);
    if (pit == entries_.end()) {
      return nullptr;
    }
    parentWidget = pit->second.widget;
  }

  auto* widget = new NveQtWidget(parentWidget);
  Qt::WindowFlags flags = Qt::Window;
  if (spec.always_on_top) {
    flags |= Qt::WindowStaysOnTopHint;
  }
  widget->setWindowFlags(flags);
  if (spec.modal) {
    widget->setWindowModality(
        parentWidget != nullptr ? Qt::WindowModal : Qt::ApplicationModal);
  }
  widget->setWindowTitle(
      spec.title.isEmpty() ? QStringLiteral("NVE Qt 6 Window") : spec.title);
  if (spec.x != 0 || spec.y != 0) {
    widget->setGeometry(spec.x, spec.y, spec.width, spec.height);
  } else {
    widget->resize(spec.width, spec.height);
  }
  widget->installEventFilter(this);

  Entry entry{};
  entry.spec = spec;
  entry.spec.cascade_close = spec.cascade_close || spec.modal;
  entry.widget = widget;
  entries_.emplace(spec.surface_id, std::move(entry));
  if (spec.parent_surface_id != 0u) {
    entries_.at(spec.parent_surface_id).children.push_back(spec.surface_id);
  }
  focused_surface_id_ = spec.surface_id;

  if (showWindow) {
    widget->show();
  }
  emit windowOpened(spec.surface_id);
  return widget;
}

NveQtWidget* NveQtWindowManager::widgetForSurface(uint32_t surfaceId) const noexcept {
  auto it = entries_.find(surfaceId);
  return it != entries_.end() ? it->second.widget : nullptr;
}

uint32_t NveQtWindowManager::activeModalBlocker(uint32_t surfaceId) const noexcept {
  auto it = entries_.find(surfaceId);
  if (it == entries_.end()) {
    return 0u;
  }
  const auto& children = it->second.children;
  for (auto rit = children.rbegin(); rit != children.rend(); ++rit) {
    const uint32_t deeper = activeModalBlocker(*rit);
    if (deeper != 0u) {
      return deeper;
    }
    auto cit = entries_.find(*rit);
    if (cit != entries_.end() && cit->second.spec.modal) {
      return *rit;
    }
  }
  return 0u;
}

bool NveQtWindowManager::focusWindow(uint32_t surfaceId) {
  auto it = entries_.find(surfaceId);
  if (it == entries_.end() || activeModalBlocker(surfaceId) != 0u) {
    return false;
  }
  focused_surface_id_ = surfaceId;
  if (it->second.widget != nullptr && it->second.widget->isVisible()) {
    it->second.widget->raise();
    it->second.widget->activateWindow();
  }
  return true;
}

bool NveQtWindowManager::setTitle(uint32_t surfaceId, const QString& title) {
  auto it = entries_.find(surfaceId);
  if (it == entries_.end() || it->second.widget == nullptr) {
    return false;
  }
  it->second.spec.title = title;
  it->second.widget->setWindowTitle(title);
  return true;
}

bool NveQtWindowManager::setBounds(uint32_t surfaceId, const QRect& bounds) {
  auto it = entries_.find(surfaceId);
  if (it == entries_.end() || it->second.widget == nullptr || !bounds.isValid() ||
      bounds.width() <= 0 || bounds.height() <= 0) {
    return false;
  }
  it->second.spec.x = bounds.x();
  it->second.spec.y = bounds.y();
  it->second.spec.width = bounds.width();
  it->second.spec.height = bounds.height();
  it->second.widget->setGeometry(bounds);
  return true;
}

void NveQtWindowManager::collectCascadePostOrder(
    uint32_t surfaceId,
    std::vector<uint32_t>& out) const {
  auto it = entries_.find(surfaceId);
  if (it == entries_.end()) {
    return;
  }
  for (uint32_t cid : it->second.children) {
    auto cit = entries_.find(cid);
    if (cit != entries_.end() && cit->second.spec.cascade_close) {
      collectCascadePostOrder(cid, out);
    }
  }
  out.push_back(surfaceId);
}

bool NveQtWindowManager::requestClose(uint32_t surfaceId) {
  if (!entries_.contains(surfaceId)) {
    return false;
  }
  if (should_close_ && !should_close_(surfaceId)) {
    return false;
  }
  return closeWindow(surfaceId);
}

bool NveQtWindowManager::closeWindow(uint32_t surfaceId) {
  auto rootIt = entries_.find(surfaceId);
  if (rootIt == entries_.end()) {
    return false;
  }
  const uint32_t fallbackParent = rootIt->second.spec.parent_surface_id;
  std::vector<uint32_t> order;
  collectCascadePostOrder(surfaceId, order);

  const bool prevClosing = closing_internal_;
  closing_internal_ = true;
  bool focusClosed = false;
  std::vector<uint32_t> closed;
  closed.reserve(order.size());

  for (uint32_t sid : order) {
    auto it = entries_.find(sid);
    if (it == entries_.end()) {
      continue;
    }
    const uint32_t pid = it->second.spec.parent_surface_id;
    if (pid != 0u) {
      if (auto pit = entries_.find(pid); pit != entries_.end()) {
        auto& kids = pit->second.children;
        kids.erase(std::remove(kids.begin(), kids.end(), sid), kids.end());
      }
    }
    for (uint32_t remainingCid : it->second.children) {
      if (auto cit = entries_.find(remainingCid); cit != entries_.end()) {
        cit->second.spec.parent_surface_id = 0u;
        if (cit->second.widget != nullptr) {
          cit->second.widget->setParent(nullptr, cit->second.widget->windowFlags());
        }
      }
    }
    NveQtWidget* widget = it->second.widget;
    entries_.erase(it);
    if (widget != nullptr) {
      widget->removeEventFilter(this);
      widget->disconnectPaired();
      widget->close();
      delete widget;
    }
    if (focused_surface_id_ == sid) {
      focused_surface_id_ = 0u;
      focusClosed = true;
    }
    closed.push_back(sid);
  }
  closing_internal_ = prevClosing;

  if (focusClosed) {
    if (fallbackParent != 0u && entries_.contains(fallbackParent) &&
        activeModalBlocker(fallbackParent) == 0u) {
      focused_surface_id_ = fallbackParent;
    } else if (!entries_.empty()) {
      focused_surface_id_ = entries_.rbegin()->first;
    }
  }

  for (uint32_t sid : closed) {
    if (on_window_closed_) {
      on_window_closed_(sid);
    }
    emit windowClosed(sid);
  }
  if (entries_.empty()) {
    if (on_all_closed_) {
      on_all_closed_();
    }
    emit allWindowsClosed();
  }
  return true;
}

void NveQtWindowManager::closeAllWindows() {
  const auto ids = activeSurfaceIds();
  for (uint32_t sid : ids) {
    if (entries_.contains(sid)) {
      (void)closeWindow(sid);
    }
  }
}

std::vector<uint32_t> NveQtWindowManager::activeSurfaceIds() const {
  std::vector<uint32_t> ids;
  ids.reserve(entries_.size());
  for (const auto& [sid, _] : entries_) {
    ids.push_back(sid);
  }
  return ids;
}

uint32_t NveQtWindowManager::surfaceIdForObject(const QObject* obj) const noexcept {
  for (const auto& [sid, entry] : entries_) {
    if (entry.widget == obj) {
      return sid;
    }
  }
  return 0u;
}

bool NveQtWindowManager::eventFilter(QObject* watched, QEvent* event) {
  if (closing_internal_ || event == nullptr) {
    return QObject::eventFilter(watched, event);
  }
  const uint32_t sid = surfaceIdForObject(watched);
  if (sid == 0u) {
    return QObject::eventFilter(watched, event);
  }
  if (event->type() == QEvent::Close) {
    auto* closeEv = static_cast<QCloseEvent*>(event);
    closeEv->ignore();
    (void)requestClose(sid);
    return true;
  }
  if (event->type() == QEvent::WindowActivate) {
    const uint32_t blocker = activeModalBlocker(sid);
    if (blocker != 0u) {
      (void)focusWindow(blocker);
    } else {
      focused_surface_id_ = sid;
    }
  }
  return QObject::eventFilter(watched, event);
}

} // namespace nve::qt
