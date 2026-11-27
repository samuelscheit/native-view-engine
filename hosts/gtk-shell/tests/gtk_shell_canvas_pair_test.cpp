// GTK host: cairo `NVE_DRAW1` rendering (pixels, clips, corrupt payloads), `nve.canvas` nodes
// (`GtkDrawingArea`, resource bytes, generation-driven redraws, table detach), native scroll views
// (content extents, user scroll -> event + runtime offset, programmatic scroll, overflow views),
// and paired thin-client lifecycle.
#include "gtk_test_helpers.hpp"

#include <nve/canvas/draw_list_encoder.hpp>

#include <vector>

namespace {

using namespace nve::gtk;
using namespace nve::gtk::test;
using namespace nve::canvas;
namespace h = nve::host;

uint32_t pixel(cairo_surface_t* s, int x, int y) {
  cairo_surface_flush(s);
  const unsigned char* data = cairo_image_surface_get_data(s);
  const int stride = cairo_image_surface_get_stride(s);
  return *reinterpret_cast<const uint32_t*>(data + y * stride + x * 4);  // premultiplied ARGB32
}

void test_cairo_renderer() {
  const DrawPoint line[] = {{0.0f, 18.0f}, {39.0f, 18.0f}};
  DrawListEncoder e;
  e.clear(DrawColorRgba::rgb(255, 255, 255))
      .fill_rect(0.0f, 0.0f, 10.0f, 10.0f, DrawColorRgba::rgb(255, 0, 0))
      .clip_rect(20.0f, 0.0f, 10.0f, 10.0f)
      .fill_rect(15.0f, 0.0f, 20.0f, 10.0f, DrawColorRgba::rgb(0, 0, 255))
      .restore_clip()
      .fill_circle(35.0f, 5.0f, 3.0f, DrawColorRgba::rgb(0, 255, 0))
      .stroke_polyline(line, 2.0f, DrawColorRgba::rgb(0, 0, 0));
  const std::vector<uint8_t> bytes = e.finish();
  cairo_surface_t* s = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, 40, 20);
  cairo_t* cr = cairo_create(s);
  const DrawListDecodeResult res = render_draw_list_to_cairo(cr, 40.0, 20.0, bytes);
  NVE_CHECK(res.ok() && res.ops_decoded == 7u);
  NVE_CHECK(pixel(s, 5, 5) == 0xFFFF0000u);   // red rect
  NVE_CHECK(pixel(s, 17, 5) == 0xFFFFFFFFu);  // blue clipped away
  NVE_CHECK(pixel(s, 25, 5) == 0xFF0000FFu);  // blue inside the clip
  NVE_CHECK(pixel(s, 35, 5) == 0xFF00FF00u);  // circle centre
  NVE_CHECK(pixel(s, 20, 18) == 0xFF000000u); // polyline

  // Corrupt payload: rejected before painting, cairo state stays balanced.
  std::vector<uint8_t> bad = bytes;
  bad.pop_back();
  cairo_save(cr);
  NVE_CHECK(render_draw_list_to_cairo(cr, 40.0, 20.0, bad).error == DrawListError::Truncated);
  cairo_restore(cr);
  NVE_CHECK(cairo_status(cr) == CAIRO_STATUS_SUCCESS);
  // An unbalanced clip stack is unwound: drawing afterwards is unclipped.
  DrawListEncoder unbalanced;
  unbalanced.clip_rect(0.0f, 0.0f, 1.0f, 1.0f);
  NVE_CHECK(render_draw_list_to_cairo(cr, 40.0, 20.0, unbalanced.finish()).ok());
  cairo_set_source_rgb(cr, 0.0, 1.0, 0.0);
  cairo_paint(cr);
  NVE_CHECK(pixel(s, 30, 15) == 0xFF00FF00u);
  NVE_CHECK(render_draw_list_to_cairo(cr, 0.0, 20.0, bytes).ok());  // zero-size: no-op
  cairo_destroy(cr);
  cairo_surface_destroy(s);
}

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
  w.set(kCanvas, NVE_TABLE_LAYOUT_DESC, NVE_LAYOUT_WIDTH, h::length_points(120.0f));
  w.set(kCanvas, NVE_TABLE_LAYOUT_DESC, NVE_LAYOUT_HEIGHT, h::length_points(40.0f));
  const std::vector<uint8_t> first = sparkline(10.0f);
  w.define_resource(5u, kDrawListKind, first);
  w.set(kCanvas, NVE_TABLE_CANVAS_DESC, NVE_CANVAS_DRAW_LIST, 5u);
  w.set(kCanvas, NVE_TABLE_CANVAS_DESC, NVE_CANVAS_GENERATION, 1u);
  const uint32_t kids[] = {kCanvas};
  w.set_children(1u, kids);

  NveGtkHost host;
  NVE_CHECK(host.attach_interface(&ScriptedApp::get_interface));
  NVE_CHECK(app.renders == 1u);  // module resource kinds (`nve.res.draw_list`) reach the replica
  GtkWidget* area = host.widget_for_node_id(kCanvas);
  NVE_CHECK(area != nullptr && GTK_IS_DRAWING_AREA(area));
  NVE_CHECK(gtk_drawing_area_get_content_width(GTK_DRAWING_AREA(area)) == 120);
  NVE_CHECK(gtk_drawing_area_get_content_height(GTK_DRAWING_AREA(area)) == 40);
  const auto shown = host.canvas_draw_list_for_node_id(kCanvas);
  NVE_CHECK(std::vector<uint8_t>(shown.begin(), shown.end()) == first);
  NVE_CHECK(host.canvas_redraw_count_for_node_id(kCanvas) == 1u);

  // New frame with an unrelated change: no redraw.
  w.set(kCanvas, NVE_TABLE_VISUAL_DESC, NVE_VISUAL_BACKGROUND, h::rgba(1, 1, 1));
  flush(host);
  NVE_CHECK(host.canvas_redraw_count_for_node_id(kCanvas) == 1u);
  // New draw list + generation: one redraw with the new bytes.
  const std::vector<uint8_t> second = sparkline(30.0f);
  w.define_resource(6u, kDrawListKind, second);
  w.set(kCanvas, NVE_TABLE_CANVAS_DESC, NVE_CANVAS_DRAW_LIST, 6u);
  w.set(kCanvas, NVE_TABLE_CANVAS_DESC, NVE_CANVAS_GENERATION, 2u);
  w.release_resource(5u);
  flush(host);
  NVE_CHECK(host.canvas_redraw_count_for_node_id(kCanvas) == 2u);
  const auto now = host.canvas_draw_list_for_node_id(kCanvas);
  NVE_CHECK(std::vector<uint8_t>(now.begin(), now.end()) == second);
  NVE_CHECK(host.widget_for_node_id(kCanvas) == area);
  // Detaching the canvas row turns the node back into a plain view.
  w.detach(kCanvas, NVE_TABLE_CANVAS_DESC);
  flush(host);
  NVE_CHECK(GTK_IS_FIXED(host.widget_for_node_id(kCanvas)) && host.canvas_draw_list_for_node_id(kCanvas).empty());
}

void test_scroll_views() {
  constexpr uint32_t kScroll = 2u, kTall = 3u, kOverflow = 4u, kWide = 5u;
  ScriptedApp app;
  h::Nve2Writer& w = app.w;
  w.create(1u);
  w.create(kScroll);
  w.attach(kScroll, NVE_TABLE_SCROLL_DESC);
  w.set(kScroll, NVE_TABLE_LAYOUT_DESC, NVE_LAYOUT_HEIGHT, h::length_points(100.0f));
  w.set_f32(kScroll, NVE_TABLE_LAYOUT_DESC, NVE_LAYOUT_FLEX_SHRINK, 0.0f);
  w.set(kScroll, NVE_TABLE_INTERACTION_DESC, NVE_INTERACTION_EVENT_MASK, h::event_bit(NVE_EVENT_SCROLL));
  w.create(kTall);
  w.set(kTall, NVE_TABLE_LAYOUT_DESC, NVE_LAYOUT_HEIGHT, h::length_points(300.0f));
  w.set_f32(kTall, NVE_TABLE_LAYOUT_DESC, NVE_LAYOUT_FLEX_SHRINK, 0.0f);
  const uint32_t tall[] = {kTall};
  w.set_children(kScroll, tall);
  w.create(kOverflow);  // a plain view with overflow: scroll is a scroll view too
  w.set(kOverflow, NVE_TABLE_VISUAL_DESC, NVE_VISUAL_OVERFLOW_X, NVE_OVERFLOW_SCROLL);
  w.set(kOverflow, NVE_TABLE_LAYOUT_DESC, NVE_LAYOUT_HEIGHT, h::length_points(40.0f));
  w.create(kWide);
  w.set(kWide, NVE_TABLE_LAYOUT_DESC, NVE_LAYOUT_WIDTH, h::length_points(900.0f));
  w.set(kWide, NVE_TABLE_LAYOUT_DESC, NVE_LAYOUT_HEIGHT, h::length_points(10.0f));
  const uint32_t wide[] = {kWide};
  w.set_children(kOverflow, wide);
  const uint32_t kids[] = {kScroll, kOverflow};
  w.set_children(1u, kids);

  NveGtkHost host(GtkHostConfig{.width = 320.0f, .height = 240.0f});
  NVE_CHECK(host.attach_interface(&ScriptedApp::get_interface));
  GtkWidget* window = present(host, 320, 240);
  GtkWidget* scroll = host.widget_for_node_id(kScroll);
  NVE_CHECK(GTK_IS_SCROLLED_WINDOW(scroll) && GTK_IS_SCROLLED_WINDOW(host.widget_for_node_id(kOverflow)));
  GtkWidget* content = gtk_scrolled_window_get_child(GTK_SCROLLED_WINDOW(scroll));
  if (GTK_IS_VIEWPORT(content)) content = gtk_viewport_get_child(GTK_VIEWPORT(content));
  NVE_CHECK(gtk_widget_get_parent(host.widget_for_node_id(kTall)) == content);
  NVE_CHECK(request_height(content) == 300);  // content extent = tallest child
  GtkPolicyType hp, vp;
  gtk_scrolled_window_get_policy(GTK_SCROLLED_WINDOW(scroll), &hp, &vp);
  NVE_CHECK(hp == GTK_POLICY_NEVER && vp == GTK_POLICY_AUTOMATIC);
  gtk_scrolled_window_get_policy(GTK_SCROLLED_WINDOW(host.widget_for_node_id(kOverflow)), &hp, &vp);
  NVE_CHECK(hp == GTK_POLICY_AUTOMATIC && vp == GTK_POLICY_NEVER);
  spin();

  // User scroll -> Scroll event with offset + content size, and the runtime offset follows.
  GtkAdjustment* vadj = gtk_scrolled_window_get_vadjustment(GTK_SCROLLED_WINDOW(scroll));
  NVE_CHECK(gtk_adjustment_get_upper(vadj) >= 299.0);
  gtk_adjustment_set_value(vadj, 50.0);
  NVE_CHECK(app.count(NVE_EVENT_SCROLL, kScroll) >= 1u);
  const auto* ev = app.last(NVE_EVENT_SCROLL);
  NVE_CHECK(approx(ev->ev.y, 50.0, 0.5) && approx(ev->ev.height, 300.0, 1.0));
  float rx = -1.0f, ry = -1.0f;
  NVE_CHECK(host.session().runtime().scroll_by(kScroll, 0.0f, 0.0f, &rx, &ry) == NVE_RUNTIME_OK);
  NVE_CHECK(approx(ry, 50.0, 0.5) && approx(rx, 0.0, 0.01));

  // Programmatic scroll is applied once and not echoed back as a user scroll.
  const size_t scrolls = app.count(NVE_EVENT_SCROLL, kScroll);
  w.set_f32(kScroll, NVE_TABLE_SCROLL_DESC, NVE_SCROLL_REQUESTED_SCROLL_Y, 120.0f);
  flush(host);
  NVE_CHECK(approx(gtk_adjustment_get_value(vadj), 120.0, 0.5));
  NVE_CHECK(app.count(NVE_EVENT_SCROLL, kScroll) == scrolls);
  gtk_adjustment_set_value(vadj, 10.0);  // the user scrolls away; an unrelated frame keeps it
  w.set(kTall, NVE_TABLE_VISUAL_DESC, NVE_VISUAL_BACKGROUND, h::rgba(9, 9, 9));
  flush(host);
  NVE_CHECK(approx(gtk_adjustment_get_value(vadj), 10.0, 0.5));

  // A growing child grows the scroll extent without touching the scroll node.
  w.set(kTall, NVE_TABLE_LAYOUT_DESC, NVE_LAYOUT_HEIGHT, h::length_points(500.0f));
  flush(host);
  NVE_CHECK(request_height(content) == 500);
  gtk_window_destroy(GTK_WINDOW(window));
  spin();
}

void test_paired_lifecycle() {
  NveGtkHost host;
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
  NVE_CHECK(label_text(host.widget_for_node_id(2u)) == "local again");
}

}  // namespace

void nve_test_gtk_canvas_scroll_and_pair() {
  test_cairo_renderer();
  test_canvas_nodes();
  test_scroll_views();
  test_paired_lifecycle();
}
