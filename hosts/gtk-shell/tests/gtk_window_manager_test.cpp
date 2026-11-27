#include "NveGtkWindowManager.hpp"

#include <cstdlib>
#include <iostream>
#include <string>
#include <vector>

#define NVE_GTK_WM_CHECK(expr)                                                                     \
  do {                                                                                             \
    if (!(expr)) {                                                                                 \
      std::cerr << "NVE_GTK_WM_CHECK failed at " << __FILE__ << ":" << __LINE__ << ": " #expr "\n"; \
      std::abort();                                                                                \
    }                                                                                              \
  } while (0)

void nve_test_gtk_window_manager() {
  nve::gtk::NveGtkWindowManager mgr;
  bool allow_primary = false;
  std::vector<uint32_t> closed_order;
  int all_closed_calls = 0;

  mgr.set_should_close_callback([&](uint32_t sid) { return sid == 1u ? allow_primary : true; });
  mgr.set_on_window_closed_callback([&](uint32_t sid) { closed_order.push_back(sid); });
  mgr.set_on_all_windows_closed_callback([&]() { ++all_closed_calls; });

  auto* h1 = mgr.open_window({.surface_id = 1u, .title = "GTK Primary", .width = 640, .height = 420});
  auto* h2 = mgr.open_window({.surface_id = 2u, .title = "GTK Inspector", .width = 360, .height = 300});
  NVE_GTK_WM_CHECK(h1 != nullptr && h2 != nullptr && h1 != h2);
  NVE_GTK_WM_CHECK(h1->surface_id() == 1u && h2->surface_id() == 2u);  // one runtime surface per window
  NVE_GTK_WM_CHECK(h1->session().surface_id() == 1u && h2->session().surface_id() == 2u);
  NVE_GTK_WM_CHECK(mgr.window_count() == 2u);
  NVE_GTK_WM_CHECK(mgr.focused_surface_id() == 2u);
  NVE_GTK_WM_CHECK(mgr.open_window({.surface_id = 1u, .title = "Dup"}) == nullptr);

  NVE_GTK_WM_CHECK(mgr.set_title(1u, "GTK Primary Updated"));
  NVE_GTK_WM_CHECK(std::string(gtk_window_get_title(mgr.window_for_surface(1u))) == "GTK Primary Updated");
  NVE_GTK_WM_CHECK(mgr.set_bounds(1u, 800, 520));
  int w = 0, h = 0;
  gtk_window_get_default_size(mgr.window_for_surface(1u), &w, &h);
  NVE_GTK_WM_CHECK(w == 800 && h == 520);

  auto* h3 = mgr.open_window({
      .surface_id = 3u,
      .title = "GTK Modal",
      .width = 320,
      .height = 180,
      .parent_surface_id = 1u,
      .modal = true,
      .cascade_close = true,
  });
  auto* h4 = mgr.open_window({
      .surface_id = 4u,
      .title = "GTK Popover",
      .width = 200,
      .height = 120,
      .parent_surface_id = 3u,
      .cascade_close = true,
  });
  NVE_GTK_WM_CHECK(h3 != nullptr && h4 != nullptr);
  NVE_GTK_WM_CHECK(mgr.active_modal_blocker(1u) == 3u);
  NVE_GTK_WM_CHECK(!mgr.focus_window(1u));
  NVE_GTK_WM_CHECK(mgr.focus_window(2u));

  // Vetoed close on primary preserves all 4 windows.
  NVE_GTK_WM_CHECK(!mgr.request_close(1u));
  NVE_GTK_WM_CHECK(mgr.window_count() == 4u && all_closed_calls == 0);

  // Allowed close on primary cascades 4 -> 3 -> 1, leaving independent window 2 open.
  allow_primary = true;
  NVE_GTK_WM_CHECK(mgr.request_close(1u));
  NVE_GTK_WM_CHECK(mgr.window_count() == 1u && all_closed_calls == 0);
  NVE_GTK_WM_CHECK((closed_order == std::vector<uint32_t>{4u, 3u, 1u}));

  // Closing last window triggers all-closed callback.
  NVE_GTK_WM_CHECK(mgr.request_close(2u));
  NVE_GTK_WM_CHECK(mgr.window_count() == 0u && all_closed_calls == 1);
}
