// Qt host: `QPainter` `NVE_DRAW1` rendering (all ops, pixels, clips, corrupt payloads, unbalanced
// clip stacks, HiDPI rasterization), `nve.canvas` nodes (resource bytes, generation-driven
// redraws, table detach), and the paired thin-client lifecycle.
#include "NveQtWidgetInternal.hpp"
#include "qt_test_helpers.hpp"

#include <nve/canvas/draw_list_encoder.hpp>

#include <vector>

namespace {

using namespace nve::qt;
using namespace nve::qt::test;
using namespace nve::canvas;
namespace h = nve::host;

std::vector<uint8_t> scene_ops() {
  const DrawPoint line[] = {{0.0f, 18.0f}, {39.0f, 18.0f}};
  DrawListEncoder e;
  e.clear(DrawColorRgba::rgb(255, 255, 255))
      .fill_rect(0.0f, 0.0f, 10.0f, 10.0f, DrawColorRgba::rgb(255, 0, 0))
      .clip_rect(20.0f, 0.0f, 10.0f, 10.0f)
      .fill_rect(15.0f, 0.0f, 20.0f, 10.0f, DrawColorRgba::rgb(0, 0, 255))
      .restore_clip()
      .fill_circle(35.0f, 5.0f, 3.0f, DrawColorRgba::rgb(0, 255, 0))
      .stroke_polyline(line, 2.0f, DrawColorRgba::rgb(0, 0, 0));
  return e.finish();
}

void test_qpainter_renderer() {
  const std::vector<uint8_t> bytes = scene_ops();
  QImage img(40, 20, QImage::Format_ARGB32_Premultiplied);
  img.fill(Qt::transparent);
  {
    QPainter p(&img);
    const DrawListDecodeResult res = paint_draw_list(p, QRectF(0, 0, 40, 20), bytes);
    NVE_CHECK(res.ok() && res.ops_decoded == 7u);
  }
  NVE_CHECK(img.pixelColor(5, 5) == QColor(255, 0, 0));      // red rect
  NVE_CHECK(img.pixelColor(17, 5) == QColor(255, 255, 255)); // blue clipped away
  NVE_CHECK(img.pixelColor(25, 5) == QColor(0, 0, 255));     // blue inside the clip
  NVE_CHECK(img.pixelColor(35, 5) == QColor(0, 255, 0));     // circle centre
  NVE_CHECK(img.pixelColor(20, 18) == QColor(0, 0, 0));      // polyline

  // Draw lists are painted at the bounds' origin (a content box inset in its node).
  QImage offset(50, 30, QImage::Format_ARGB32_Premultiplied);
  offset.fill(Qt::transparent);
  {
    QPainter p(&offset);
    NVE_CHECK(paint_draw_list(p, QRectF(10, 10, 40, 20), bytes).ok());
  }
  NVE_CHECK(offset.pixelColor(15, 15) == QColor(255, 0, 0) && offset.pixelColor(5, 5).alpha() == 0);

  // Clear replaces pixels on an image device (translucent clear keeps no old color).
  DrawListEncoder translucent;
  translucent.clear(DrawColorRgba{0, 0, 0, 0});
  {
    QPainter p(&img);
    NVE_CHECK(paint_draw_list(p, QRectF(0, 0, 40, 20), translucent.finish()).ok());
  }
  NVE_CHECK(img.pixelColor(5, 5).alpha() == 0);

  // Corrupt payload: rejected before painting; painter state stays balanced.
  std::vector<uint8_t> bad = bytes;
  bad.pop_back();
  img.fill(Qt::white);
  {
    QPainter p(&img);
    p.save();
    NVE_CHECK(paint_draw_list(p, QRectF(0, 0, 40, 20), bad).error == DrawListError::Truncated);
    p.restore();
    // An unbalanced clip stack is unwound: drawing afterwards is unclipped.
    DrawListEncoder unbalanced;
    unbalanced.clip_rect(0.0f, 0.0f, 1.0f, 1.0f);
    NVE_CHECK(paint_draw_list(p, QRectF(0, 0, 40, 20), unbalanced.finish()).ok());
    p.fillRect(QRect(0, 0, 40, 20), Qt::green);
    NVE_CHECK(paint_draw_list(p, QRectF(0, 0, 0, 20), bytes).ok());  // zero-size: no-op
  }
  NVE_CHECK(img.pixelColor(30, 15) == QColor(0, 255, 0) && img.pixelColor(5, 5) == QColor(0, 255, 0));
  const std::vector<uint8_t> empty;
  QImage scratch(4, 4, QImage::Format_ARGB32_Premultiplied);
  QPainter sp(&scratch);
  const DrawListDecodeResult none = paint_draw_list(sp, QRectF(0, 0, 4, 4), empty);
  NVE_CHECK(none.ok() && none.ops_decoded == 0u);  // no payload: nothing to draw
  sp.end();

  // HiDPI rasterization: device pixels scale, logical size stays.
  const QImage hi = rasterize_draw_list(bytes, QSizeF(40, 20), 2.0);
  NVE_CHECK(hi.width() == 80 && hi.height() == 40 && hi.devicePixelRatio() == 2.0);
  NVE_CHECK(hi.pixelColor(10, 10) == QColor(255, 0, 0) && hi.pixelColor(70, 10) == QColor(0, 255, 0));
  NVE_CHECK(rasterize_draw_list(bad, QSizeF(40, 20), 1.0).isNull());
  NVE_CHECK(rasterize_draw_list(bytes, QSizeF(0, 20), 1.0).isNull());
  NVE_CHECK(rasterize_draw_list(bytes, QSizeF(std::nan(""), 20), 1.0).isNull());
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
  size_node(w, kCanvas, 120.0f, 40.0f);
  w.set(kCanvas, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_PADDING_LEFT, h::length_points(10.0f));
  const std::vector<uint8_t> first = sparkline(10.0f);
  w.define_resource(5u, kDrawListKind, first);
  w.set(kCanvas, NVE_TABLE_CANVAS_DESC, NVE_TABLE_CANVAS_DRAW_LIST, 5u);
  w.set(kCanvas, NVE_TABLE_CANVAS_DESC, NVE_TABLE_CANVAS_GENERATION, 1u);
  const uint32_t kids[] = {kCanvas};
  w.set_children(1u, kids);

  NveQtWidget widget;
  QtHost& host = widget.host();
  NVE_CHECK(host.attach_interface(&ScriptedApp::get_interface));
  NVE_CHECK(app.renders == 1u);
  auto* node = dynamic_cast<NveQtNodeWidget*>(widget.nativeWidgetForNodeId(kCanvas));
  NVE_CHECK(node != nullptr && node->content() == NodeContent::Canvas && node->size() == QSize(120, 40));
  NVE_CHECK(node->content_box() == QRectF(10.0, 0.0, 110.0, 40.0));
  const auto shown = widget.canvasDrawListForNodeId(kCanvas);
  NVE_CHECK(std::vector<uint8_t>(shown.begin(), shown.end()) == first);
  NVE_CHECK(widget.canvasRedrawCountForNodeId(kCanvas) == 1u);
  const QImage img = render(node);
  NVE_CHECK(img.pixelColor(5, 20).alpha() == 0 && same_rgb(img.pixelColor(60, 30), 250, 250, 250));

  // New frame with an unrelated change: no redraw.
  w.set(kCanvas, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, h::rgba(1, 1, 1));
  flush(host);
  NVE_CHECK(widget.canvasRedrawCountForNodeId(kCanvas) == 1u);
  // New draw list + generation: one redraw with the new bytes (the old resource released).
  const std::vector<uint8_t> second = sparkline(30.0f);
  w.define_resource(6u, kDrawListKind, second);
  w.set(kCanvas, NVE_TABLE_CANVAS_DESC, NVE_TABLE_CANVAS_DRAW_LIST, 6u);
  w.set(kCanvas, NVE_TABLE_CANVAS_DESC, NVE_TABLE_CANVAS_GENERATION, 2u);
  w.release_resource(5u);
  flush(host);
  NVE_CHECK(widget.canvasRedrawCountForNodeId(kCanvas) == 2u);
  const auto now = widget.canvasDrawListForNodeId(kCanvas);
  NVE_CHECK(std::vector<uint8_t>(now.begin(), now.end()) == second);
  NVE_CHECK(widget.nativeWidgetForNodeId(kCanvas) == node);
  // A corrupt draw list is kept but paints nothing (and does not crash the paint).
  std::vector<uint8_t> corrupt = second;
  corrupt.resize(corrupt.size() - 3u);
  w.define_resource(7u, kDrawListKind, corrupt);
  w.set(kCanvas, NVE_TABLE_CANVAS_DESC, NVE_TABLE_CANVAS_DRAW_LIST, 7u);
  w.set(kCanvas, NVE_TABLE_CANVAS_DESC, NVE_TABLE_CANVAS_GENERATION, 3u);
  flush(host);
  NVE_CHECK(widget.canvasRedrawCountForNodeId(kCanvas) == 3u);
  NVE_CHECK(same_rgb(render(node).pixelColor(60, 30), 1, 1, 1));  // only the node background
  // Detaching the canvas row turns the node back into a plain view.
  w.detach(kCanvas, NVE_TABLE_CANVAS_DESC);
  flush(host);
  NVE_CHECK(node->content() == NodeContent::None && widget.canvasDrawListForNodeId(kCanvas).empty());
}

void test_paired_lifecycle() {
  NveQtWidget widget;
  QtHost& host = widget.host();
  std::string error;
  NVE_CHECK(!host.connect_paired("not a uri", "token", &error) && !error.empty());
  NVE_CHECK(!host.is_paired() && !host.is_loaded());
  // A well-formed endpoint connects in the background; nothing listens, so it never pairs.
  error.clear();
  if (host.connect_paired("nve://127.0.0.1:9", "token", &error)) {
    spin();
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
  auto* t = dynamic_cast<NveQtNodeWidget*>(widget.nativeWidgetForNodeId(2u));
  NVE_CHECK(t != nullptr && t->text() == "local again");
}

}  // namespace

void nve_test_qt_canvas_and_pair() {
  test_qpainter_renderer();
  test_canvas_nodes();
  test_paired_lifecycle();
}
