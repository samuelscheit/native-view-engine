#pragma once

#include "NveQtWidget.hpp"

#include <QObject>
#include <QRect>
#include <QString>

#include <cstdint>
#include <functional>
#include <map>
#include <vector>

namespace nve::qt {

/// Descriptor for opening a top-level Qt 6 window managed by `NveQtWindowManager`.
struct QtWindowSpec {
  uint32_t surface_id{1};
  QString title{QStringLiteral("NVE Qt 6 Host Shell")};
  int x{0};
  int y{0};
  int width{640};
  int height{420};
  uint32_t parent_surface_id{0};
  bool modal{false};
  bool always_on_top{false};
  bool cascade_close{false};
};

/// Multi-window Qt 6 lifecycle manager mapping `surface_id -> NveQtWidget*`.
///
/// Enforces parent-before-child modal focus blocking, cancelable close-request vetoes,
/// bottom-up cascading close of owned child windows, and `allWindowsClosed` emission when
/// the final managed window closes.
class NveQtWindowManager : public QObject {
  Q_OBJECT

 public:
  using CloseVetoCallback = std::function<bool(uint32_t)>;
  using WindowClosedCallback = std::function<void(uint32_t)>;
  using AllWindowsClosedCallback = std::function<void()>;

  explicit NveQtWindowManager(QObject* parent = nullptr);
  ~NveQtWindowManager() override;

  NveQtWindowManager(const NveQtWindowManager&) = delete;
  NveQtWindowManager& operator=(const NveQtWindowManager&) = delete;

  NveQtWidget* openWindow(const QtWindowSpec& spec, bool showWindow = false);
  NveQtWidget* widgetForSurface(uint32_t surfaceId) const noexcept;

  uint32_t activeModalBlocker(uint32_t surfaceId) const noexcept;
  bool focusWindow(uint32_t surfaceId);
  bool setTitle(uint32_t surfaceId, const QString& title);
  bool setBounds(uint32_t surfaceId, const QRect& bounds);

  bool requestClose(uint32_t surfaceId);
  bool closeWindow(uint32_t surfaceId);
  void closeAllWindows();

  size_t windowCount() const noexcept { return entries_.size(); }
  uint32_t focusedSurfaceId() const noexcept { return focused_surface_id_; }
  std::vector<uint32_t> activeSurfaceIds() const;

  void setShouldCloseCallback(CloseVetoCallback cb) { should_close_ = std::move(cb); }
  void setOnWindowClosedCallback(WindowClosedCallback cb) { on_window_closed_ = std::move(cb); }
  void setOnAllWindowsClosedCallback(AllWindowsClosedCallback cb) {
    on_all_closed_ = std::move(cb);
  }

 signals:
  void windowOpened(uint32_t surfaceId);
  void windowClosed(uint32_t surfaceId);
  void allWindowsClosed();

 protected:
  bool eventFilter(QObject* watched, QEvent* event) override;

 private:
  struct Entry {
    QtWindowSpec spec{};
    NveQtWidget* widget{nullptr};
    std::vector<uint32_t> children;
  };

  uint32_t surfaceIdForObject(const QObject* obj) const noexcept;
  void collectCascadePostOrder(uint32_t surfaceId, std::vector<uint32_t>& out) const;

  std::map<uint32_t, Entry> entries_;
  uint32_t focused_surface_id_{0};
  bool closing_internal_{false};
  CloseVetoCallback should_close_;
  WindowClosedCallback on_window_closed_;
  AllWindowsClosedCallback on_all_closed_;
};

} // namespace nve::qt
