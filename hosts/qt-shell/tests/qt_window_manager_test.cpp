#include "NveQtWindowManager.hpp"

#include <QRect>
#include <QString>

#include <cstdlib>
#include <iostream>
#include <vector>

#define NVE_QT_WM_CHECK(expr)                                                                     \
  do {                                                                                            \
    if (!(expr)) {                                                                                \
      std::cerr << "NVE_QT_WM_CHECK failed at " << __FILE__ << ":" << __LINE__ << ": " #expr "\n"; \
      std::abort();                                                                               \
    }                                                                                             \
  } while (0)

void nve_test_qt_window_manager() {
  nve::qt::NveQtWindowManager mgr;
  bool allowPrimary = false;
  std::vector<uint32_t> closedOrder;
  int allClosedCalls = 0;

  mgr.setShouldCloseCallback([&](uint32_t sid) { return sid == 1u ? allowPrimary : true; });
  mgr.setOnWindowClosedCallback([&](uint32_t sid) { closedOrder.push_back(sid); });
  mgr.setOnAllWindowsClosedCallback([&]() { ++allClosedCalls; });

  auto* w1 = mgr.openWindow({
      .surface_id = 1u,
      .title = QStringLiteral("Qt Primary"),
      .width = 640,
      .height = 420,
  });
  auto* w2 = mgr.openWindow({
      .surface_id = 2u,
      .title = QStringLiteral("Qt Inspector"),
      .width = 360,
      .height = 300,
  });
  NVE_QT_WM_CHECK(w1 != nullptr && w2 != nullptr && w1 != w2);
  NVE_QT_WM_CHECK(mgr.windowCount() == 2u);
  NVE_QT_WM_CHECK(mgr.focusedSurfaceId() == 2u);
  NVE_QT_WM_CHECK(mgr.openWindow({.surface_id = 1u, .title = QStringLiteral("Dup")}) == nullptr);

  NVE_QT_WM_CHECK(mgr.setTitle(1u, QStringLiteral("Qt Primary Updated")));
  NVE_QT_WM_CHECK(w1->windowTitle() == QStringLiteral("Qt Primary Updated"));
  NVE_QT_WM_CHECK(mgr.setBounds(1u, QRect(40, 50, 800, 520)));
  NVE_QT_WM_CHECK(w1->width() == 800 && w1->height() == 520);

  auto* w3 = mgr.openWindow({
      .surface_id = 3u,
      .title = QStringLiteral("Qt Modal"),
      .width = 320,
      .height = 180,
      .parent_surface_id = 1u,
      .modal = true,
      .cascade_close = true,
  });
  auto* w4 = mgr.openWindow({
      .surface_id = 4u,
      .title = QStringLiteral("Qt Popover"),
      .width = 200,
      .height = 120,
      .parent_surface_id = 3u,
      .cascade_close = true,
  });
  NVE_QT_WM_CHECK(w3 != nullptr && w4 != nullptr);
  NVE_QT_WM_CHECK(mgr.activeModalBlocker(1u) == 3u);
  NVE_QT_WM_CHECK(!mgr.focusWindow(1u));
  NVE_QT_WM_CHECK(mgr.focusWindow(2u));

  // Vetoed close on primary preserves all 4 windows.
  NVE_QT_WM_CHECK(!mgr.requestClose(1u));
  NVE_QT_WM_CHECK(mgr.windowCount() == 4u && allClosedCalls == 0);

  // Allowed close on primary cascades 4 -> 3 -> 1, leaving independent window 2 open.
  allowPrimary = true;
  NVE_QT_WM_CHECK(mgr.requestClose(1u));
  NVE_QT_WM_CHECK(mgr.windowCount() == 1u && allClosedCalls == 0);
  NVE_QT_WM_CHECK((closedOrder == std::vector<uint32_t>{4u, 3u, 1u}));

  // Closing final window triggers all-closed callback.
  NVE_QT_WM_CHECK(mgr.requestClose(2u));
  NVE_QT_WM_CHECK(mgr.windowCount() == 0u && allClosedCalls == 1);
}
