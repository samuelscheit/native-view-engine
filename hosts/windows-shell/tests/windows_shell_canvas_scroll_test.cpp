// Windows host model (portable): `nve.canvas` draw lists (resource bytes, generation-driven
// redraws, table detach), scroll containers (wheel -> runtime `scroll_by` -> painted offset +
// Scroll event, edge chaining, programmatic `requested_scroll_*`, scroll-shifted hit testing,
// overflow views), paired thin-client lifecycle, and the headless window manager.
#include "windows_test_helpers.hpp"

#include "NveWinWindowManager.hpp"

#include <nve/canvas/draw_list_encoder.hpp>

#include <vector>

namespace {

using namespace nve::win;
using namespace nve::win::test;
using namespace nve::canvas;
namespace h = nve::host;

std::vector<uint8_t> sparkline(float y) {
  const DrawPoint pts[] = {{0.0f, y}, {10.0f, 2.0f}, {20.0f, y}};
  DrawListEncoder e;
  e.clear(DrawColorRgba::rgb(250, 250, 250)).stroke_polyline(pts, 1.5f, DrawColorRgba::rgb(37, 99, 235));
  return e.finish();
}

void test_canvas_nodes() {
  constexpr uint32_t kCanvas = 2u;
  ScriptedApp app;
  h::Nve2Writer& w = app.w;
  w.create(1u);
  w.create(kCanvas);
  sized(w, kCanvas, 120.0f, 40.0f);
  const std::vector<uint8_t> first = sparkline(10.0f);
  w.define_resource(5u, kDrawListKind, first);
  w.set(kCanvas, NVE_TABLE_CANVAS_DESC, NVE_TABLE_CANVAS_DRAW_LIST, 5u);
  w.set(kCanvas, NVE_TABLE_CANVAS_DESC, NVE_TABLE_CANVAS_GENERATION, 1u);
  const uint32_t kids[] = {kCanvas};
  w.set_children(1u, kids);

  NveWinHost host;
  NVE_CHECK(host.attach_interface(&ScriptedApp::get_interface));
  NVE_CHECK(app.renders == 1u);
  const WinNodeVisual* view = host.visual_tree().find(kCanvas);
  NVE_CHECK(view != nullptr && view->canvas && view->draw_list == first && view->redraw_count == 1u);
  NVE_CHECK(approx(view->local.width, 120.0, 0.01) && approx(view->local.height, 40.0, 0.01));
  NVE_CHECK(validate_draw_list(view->draw_list).ok());

  // Unrelated change: no redraw.
  w.set(kCanvas, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, h::rgba(1, 1, 1));
  flush(host);
  NVE_CHECK(host.visual_tree().find(kCanvas) == view && view->redraw_count == 1u);
  // Same bytes under a new generation: no redraw either.
  w.set(kCanvas, NVE_TABLE_CANVAS_DESC, NVE_TABLE_CANVAS_GENERATION, 2u);
  flush(host);
  NVE_CHECK(view->redraw_count == 1u && view->canvas_generation == 2u);
  // New draw list: one redraw with the new bytes.
  const std::vector<uint8_t> second = sparkline(30.0f);
  w.define_resource(6u, kDrawListKind, second);
  w.set(kCanvas, NVE_TABLE_CANVAS_DESC, NVE_TABLE_CANVAS_DRAW_LIST, 6u);
  w.set(kCanvas, NVE_TABLE_CANVAS_DESC, NVE_TABLE_CANVAS_GENERATION, 3u);
  w.release_resource(5u);
  flush(host);
  NVE_CHECK(view->redraw_count == 2u && view->draw_list == second);
  // Detaching the canvas row turns the node back into a plain view.
  w.detach(kCanvas, NVE_TABLE_CANVAS_DESC);
  flush(host);
  const WinNodeVisual* plain = host.visual_tree().find(kCanvas);
  NVE_CHECK(plain != nullptr && !plain->canvas && plain->draw_list.empty());
}

void test_scroll_containers() {
  constexpr uint32_t kScroll = 2u, kTall = 3u, kButton = 4u, kOverflow = 5u, kWide = 6u, kOuter = 7u;
  ScriptedApp app;
  h::Nve2Writer& w = app.w;
  w.create(1u);
  w.create(kOuter);  // an outer scroll container the inner one chains to at its edges
  w.attach(kOuter, NVE_TABLE_SCROLL_DESC);
  sized(w, kOuter, 320.0f, 200.0f);
  w.create(kScroll);
  w.attach(kScroll, NVE_TABLE_SCROLL_DESC);
  sized(w, kScroll, 320.0f, 100.0f);
  events(w, kScroll, {NVE_EVENT_SCROLL});
  w.create(kTall);
  sized(w, kTall, 320.0f, 300.0f);
  w.create(kButton);  // inside the tall content, 150pt down
  w.set(kButton, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_MARGIN_TOP, h::length_points(150.0f));
  sized(w, kButton, 100.0f, 30.0f);
  events(w, kButton, {NVE_EVENT_PRESS});
  const uint32_t tall_kids[] = {kButton};
  w.set_children(kTall, tall_kids);
  const uint32_t scroll_kids[] = {kTall};
  w.set_children(kScroll, scroll_kids);
  w.create(kOverflow);  // a plain view with horizontal overflow scroll
  w.set(kOverflow, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_OVERFLOW_X, NVE_OVERFLOW_SCROLL);
  sized(w, kOverflow, 320.0f, 40.0f);
  w.create(kWide);
  sized(w, kWide, 900.0f, 10.0f);
  const uint32_t wide[] = {kWide};
  w.set_children(kOverflow, wide);
  w.create(8u);  // filler: the outer container overflows by 100pt
  sized(w, 8u, 320.0f, 160.0f);
  const uint32_t outer_kids[] = {kScroll, kOverflow, 8u};
  w.set_children(kOuter, outer_kids);
  const uint32_t kids[] = {kOuter};
  w.set_children(1u, kids);

  NveWinHost host(WinHostConfig{.width = 320.0f, .height = 240.0f});
  int changes = 0;
  host.set_on_change([&changes] { ++changes; });
  NVE_CHECK(host.attach_interface(&ScriptedApp::get_interface));
  const NveWinVisualTree& tree = host.visual_tree();
  NVE_CHECK(node(host, kScroll).scrollable() && node(host, kOverflow).scrollable() && !node(host, kTall).scrollable());
  const auto [cw, ch] = tree.content_size(node(host, kScroll));
  NVE_CHECK(approx(cw, 320.0, 0.5) && approx(ch, 300.0, 0.5));
  // The button is below the fold: clipped out, not hit.
  NVE_CHECK(host.press_at(50.0f, 160.0f).node_id == 0u);

  // Wheel -> the runtime offset, the painted shift, a Scroll event with the content size.
  const int before = changes;
  NVE_CHECK(host.wheel_at(50.0f, 50.0f, 0.0f, 120.0f));
  NVE_CHECK(changes > before && approx(node(host, kScroll).scroll_y, 120.0, 0.5));
  NVE_CHECK(app.count(NVE_EVENT_SCROLL, kScroll) == 1u);
  const auto* ev = app.last(NVE_EVENT_SCROLL);
  NVE_CHECK(approx(ev->ev.y, 120.0, 0.5) && approx(ev->ev.height, 300.0, 1.0) && approx(ev->ev.width, 320.0, 1.0));
  float rx = -1.0f, ry = -1.0f;
  NVE_CHECK(host.session().runtime().scroll_by(kScroll, 0.0f, 0.0f, &rx, &ry) == NVE_RUNTIME_OK);
  NVE_CHECK(approx(ry, 120.0, 0.5) && approx(rx, 0.0, 0.01));
  // The scrolled button is now on screen at its shifted position.
  NVE_CHECK(approx(abs_rect(host, kButton).y, 150.0 - 120.0, 0.5));
  NVE_CHECK(host.press_at(50.0f, 45.0f).node_id == kButton);
  NVE_CHECK(approx(app.last(NVE_EVENT_PRESS)->ev.y, 15.0, 0.5));

  // Clamped at the end of the content (max 200); past it, the wheel chains to the outer container.
  NVE_CHECK(host.wheel_at(50.0f, 50.0f, 0.0f, 500.0f));
  NVE_CHECK(approx(node(host, kScroll).scroll_y, 200.0, 0.5) && app.count(NVE_EVENT_SCROLL, kScroll) == 2u);
  NVE_CHECK(host.wheel_at(50.0f, 50.0f, 0.0f, 40.0f));
  NVE_CHECK(approx(node(host, kScroll).scroll_y, 200.0, 0.5) && approx(node(host, kOuter).scroll_y, 40.0, 0.5));
  NVE_CHECK(app.count(NVE_EVENT_SCROLL, kScroll) == 2u);  // the outer one does not subscribe
  NVE_CHECK(host.wheel_at(50.0f, 50.0f, 0.0f, -1000.0f));  // back up: inner first
  NVE_CHECK(approx(node(host, kScroll).scroll_y, 0.0, 0.5) && approx(node(host, kOuter).scroll_y, 40.0, 0.5));
  NVE_CHECK(host.wheel_at(50.0f, 10.0f, 0.0f, -1000.0f));  // inner at top: chains outward
  NVE_CHECK(approx(node(host, kOuter).scroll_y, 0.0, 0.5));
  NVE_CHECK(!host.wheel_at(50.0f, 10.0f, 0.0f, -10.0f));   // nothing can move
  NVE_CHECK(!host.wheel_at(50.0f, 50.0f, NAN, 1.0f));

  // Horizontal overflow view.
  const auto [ox, oy] = center(host, kOverflow);
  NVE_CHECK(host.wheel_at(ox, oy, 100.0f, 0.0f) && approx(node(host, kOverflow).scroll_x, 100.0, 0.5));
  NVE_CHECK(approx(abs_rect(host, kWide).x, -100.0, 0.5));

  // Programmatic scroll is applied by the runtime and not echoed as a user Scroll event.
  const size_t scrolls = app.count(NVE_EVENT_SCROLL, kScroll);
  w.set_f32(kScroll, NVE_TABLE_SCROLL_DESC, NVE_TABLE_SCROLL_REQUESTED_SCROLL_Y, 80.0f);
  flush(host);
  NVE_CHECK(approx(node(host, kScroll).scroll_y, 80.0, 0.5) && app.count(NVE_EVENT_SCROLL, kScroll) == scrolls);
  NVE_CHECK(host.dispatch_scroll(kScroll, 0.0f, 10.0f));  // the user scrolls away; an unrelated frame keeps it
  w.set(kTall, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, h::rgba(9, 9, 9));
  flush(host);
  NVE_CHECK(approx(node(host, kScroll).scroll_y, 10.0, 0.5) && app.count(NVE_EVENT_SCROLL, kScroll) == scrolls + 1u);
  NVE_CHECK(!host.dispatch_scroll(99u, 0.0f, 0.0f));
  NVE_CHECK(!host.dispatch_scroll(kTall, 0.0f, 10.0f));  // not a scroll container

  // A growing child grows the scroll extent without touching the scroll node.
  w.set(kTall, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_HEIGHT, h::length_points(500.0f));
  flush(host);
  NVE_CHECK(approx(tree.content_size(node(host, kScroll)).second, 500.0, 0.5));
  NVE_CHECK(host.wheel_at(50.0f, 50.0f, 0.0f, 1000.0f) && approx(node(host, kScroll).scroll_y, 400.0, 0.5));
}

void test_paired_lifecycle() {
  NveWinHost host;
  std::string error;
  NVE_CHECK(!host.connect_paired("not a uri", "token", &error) && !error.empty());
  NVE_CHECK(!host.is_paired() && !host.is_loaded());
  // A well-formed endpoint connects in the background; nothing listens, so it never pairs.
  error.clear();
  if (host.connect_paired("nve://127.0.0.1:9", "token", &error)) {
    NVE_CHECK(!host.is_paired());
    host.disconnect_paired();
  }
  NVE_CHECK(!host.is_paired());
  ScriptedApp app;
  app.w.create(1u);
  text_node(app.w, 2u, "local again");
  const uint32_t kids[] = {2u};
  app.w.set_children(1u, kids);
  NVE_CHECK(host.attach_interface(&ScriptedApp::get_interface));
  NVE_CHECK(node(host, 2u).text == "local again");
}

void test_window_manager() {
  NveWinWindowManager mgr;
  bool allow_primary = false;
  std::vector<uint32_t> closed_order;
  int all_closed_calls = 0;
  mgr.set_should_close_callback([&](uint32_t sid) { return sid == 1u ? allow_primary : true; });
  mgr.set_on_window_closed_callback([&](uint32_t sid) { closed_order.push_back(sid); });
  mgr.set_on_all_windows_closed_callback([&]() { ++all_closed_calls; });

  NveWinHost* h1 = mgr.open_headless_window({.surface_id = 1u, .title = "Primary", .width = 640, .height = 420});
  NveWinHost* h2 = mgr.open_headless_window({.surface_id = 2u, .title = "Inspector", .width = 360, .height = 300});
  NVE_CHECK(h1 != nullptr && h2 != nullptr && h1 != h2 && h2->surface_id() == 2u);
  NVE_CHECK(mgr.window_count() == 2u && mgr.focused_surface_id() == 2u && mgr.host_for_surface(1u) == h1);
  NVE_CHECK(mgr.open_headless_window({.surface_id = 1u, .title = "Dup"}) == nullptr);
  NVE_CHECK(mgr.open_headless_window({.surface_id = 0u}) == nullptr);
  NVE_CHECK(mgr.open_headless_window({.surface_id = 9u, .parent_surface_id = 42u}) == nullptr);

  NVE_CHECK(mgr.set_title(1u, "Primary — Renamed") && mgr.spec_for_surface(1u)->title == "Primary — Renamed");
  NVE_CHECK(mgr.set_bounds(1u, 40, 50, 800, 500) && !mgr.set_bounds(1u, 0, 0, 0, 10));
  NVE_CHECK(mgr.spec_for_surface(1u)->width == 800 && h1->config().width == 800.0f && h1->config().height == 500.0f);

  NveWinHost* h3 = mgr.open_headless_window(
      {.surface_id = 3u, .title = "Modal", .width = 320, .height = 180, .parent_surface_id = 1u, .modal = true});
  NveWinHost* h4 = mgr.open_headless_window({.surface_id = 4u,
                                             .title = "Popover",
                                             .width = 200,
                                             .height = 120,
                                             .parent_surface_id = 3u,
                                             .cascade_close = true});
  NVE_CHECK(h3 != nullptr && h4 != nullptr && mgr.active_modal_blocker(1u) == 3u);
  NVE_CHECK(!mgr.focus_window(1u) && mgr.focus_window(2u) && mgr.focused_surface_id() == 2u);

  NVE_CHECK(!mgr.request_close(1u) && mgr.window_count() == 4u && !mgr.quit_posted());
  allow_primary = true;
  NVE_CHECK(mgr.request_close(1u));
  NVE_CHECK(mgr.window_count() == 1u && !mgr.quit_posted() && all_closed_calls == 0);
  NVE_CHECK((closed_order == std::vector<uint32_t>{4u, 3u, 1u}));
  NVE_CHECK(mgr.request_close(2u) && mgr.window_count() == 0u && mgr.quit_posted() && all_closed_calls == 1);
  NVE_CHECK(!mgr.request_close(2u) && mgr.host_for_surface(2u) == nullptr);
}

}  // namespace

void nve_test_win_canvas_scroll_and_windows() {
  test_canvas_nodes();
  test_scroll_containers();
  test_paired_lifecycle();
  test_window_manager();
}
