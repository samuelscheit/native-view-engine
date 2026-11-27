// GTK host: O(changed) reconciliation. Geometry-only changes (`moved_indices`) reach
// `Adapter::move` — position, size and scroll extents, no CSS / semantics / label work — while
// content changes keep a full `update`; parent-relative positions stay exact across reparents.
#include "gtk_test_helpers.hpp"

#include <string>
#include <vector>

namespace {

using namespace nve::gtk;
using namespace nve::gtk::test;
namespace h = nve::host;

void set_height(h::Nve2Writer& w, uint32_t id, float height) {
  w.set(id, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_HEIGHT, h::length_points(height));
  w.set_f32(id, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_FLEX_SHRINK, 0.0f);
}

void test_moves_skip_update_and_refresh_scroll_extents() {
  // ROOT -> [scroll(2, h=100)[a(3) h=30, b(4) h=40, c(5) h=50], p1(6, h=20, padding-top 20)[x(8)],
  //          p2(7, h=20)]
  constexpr uint32_t kScroll = 2u, kA = 3u, kB = 4u, kC = 5u, kP1 = 6u, kP2 = 7u, kX = 8u;
  ScriptedApp app;
  h::Nve2Writer& w = app.w;
  w.create(1u);
  w.create(kScroll);
  w.attach(kScroll, NVE_TABLE_SCROLL_DESC);
  set_height(w, kScroll, 100.0f);
  for (const uint32_t id : {kA, kB, kC}) text_node(w, id, "row " + std::to_string(id));
  set_height(w, kA, 30.0f);
  set_height(w, kB, 40.0f);
  set_height(w, kC, 50.0f);
  w.set(kB, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, h::rgba(1, 2, 3));
  w.set_children(kScroll, std::vector<uint32_t>{kA, kB, kC});
  w.create(kP1);
  set_height(w, kP1, 20.0f);
  w.set(kP1, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_PADDING_TOP, h::length_points(20.0f));
  w.create(kP2);
  set_height(w, kP2, 20.0f);
  text_node(w, kX, "x");
  set_height(w, kX, 10.0f);
  w.set_children(kP1, std::vector<uint32_t>{kX});
  w.set_children(1u, std::vector<uint32_t>{kScroll, kP1, kP2});

  NveGtkHost host(GtkHostConfig{.width = 320.0f, .height = 400.0f});
  NVE_CHECK(host.attach_interface(&ScriptedApp::get_interface));
  GtkWidget* scroll = host.widget_for_node_id(kScroll);
  GtkWidget* content = gtk_scrolled_window_get_child(GTK_SCROLLED_WINDOW(scroll));
  if (GTK_IS_VIEWPORT(content)) content = gtk_viewport_get_child(GTK_VIEWPORT(content));
  GtkWidget* b = host.widget_for_node_id(kB);
  NVE_CHECK(request_height(content) == 120);
  NVE_CHECK(approx(child_position(content, b).second, 30.0, 0.1));

  // `a` grows: b and c only move. One update (a), two moves, no stylesheet rebuild.
  const uint64_t updates = host.reconcile_update_count();
  const uint64_t moves = host.reconcile_move_count();
  const std::string css = host.active_css();
  set_height(w, kA, 60.0f);
  flush(host);
  NVE_CHECK(host.reconcile_update_count() - updates == 1u);
  NVE_CHECK(host.reconcile_move_count() - moves == 2u);
  NVE_CHECK(host.active_css() == css);
  NVE_CHECK(host.widget_for_node_id(kB) == b);  // retained
  NVE_CHECK(approx(child_position(content, b).second, 60.0, 0.1));
  NVE_CHECK(approx(child_position(content, host.widget_for_node_id(kC)).second, 100.0, 0.1));
  NVE_CHECK(request_height(content) == 150);  // extents follow moved children

  // Content + move: b gets a full update (new background) at its new position.
  set_height(w, kA, 30.0f);
  w.set(kB, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, h::rgba(7, 8, 9));
  flush(host);
  NVE_CHECK(host.active_css().find("rgba(7, 8, 9, 1)") != std::string::npos);
  NVE_CHECK(approx(child_position(content, b).second, 30.0, 0.1));
  NVE_CHECK(request_height(content) == 120);

  // Reparent with an unchanged absolute rect: x keeps y=120 but sits at 0 inside p2 (was 20 in p1).
  GtkWidget* x = host.widget_for_node_id(kX);
  GtkWidget* p2 = host.widget_for_node_id(kP2);
  NVE_CHECK(approx(child_position(host.widget_for_node_id(kP1), x).second, 20.0, 0.1));
  w.set_children(kP2, std::vector<uint32_t>{kX});
  flush(host);
  NVE_CHECK(gtk_widget_get_parent(x) == p2 && host.widget_for_node_id(kX) == x);
  NVE_CHECK(approx(child_position(p2, x).second, 0.0, 0.1));

  // A removed child shrinks its scroll view's extent.
  w.remove(kC);
  w.set_children(kScroll, std::vector<uint32_t>{kA, kB});
  flush(host);
  NVE_CHECK(request_height(content) == 70);
}

}  // namespace

void nve_test_gtk_reconcile() { test_moves_skip_update_and_refresh_scroll_extents(); }
