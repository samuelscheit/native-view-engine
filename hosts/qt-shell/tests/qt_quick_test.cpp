// Qt Quick host (`NveQtQuickItem`): the retained scene graph (pixels for backgrounds, borders,
// z-index, opacity, visibility, rounded overflow clips, shadows; wrapper reuse, O(changed)
// updates, texture reuse, reparenting and removal), the QML API (properties, signals), app
// modules with hot reload / focus survival / state files, queued wakes, and the live kitchen sink.
#include "qt_quick_helpers.hpp"

#include <nve/host/demo_counter_app.hpp>

#include <QGuiApplication>
#include <QSGClipNode>
#include <QSGNode>
#include <QSGRendererInterface>

#include <nve/host/testing/scratch_dir.hpp>

#include <filesystem>

namespace {

using namespace nve::qt;
using namespace nve::qt::test;
namespace h = nve::host;
using nve::host::demo::CounterApp;

const QSGNode* first_of_type(const QSGNode* node, QSGNode::NodeType type) {
  if (node == nullptr || node->type() == type) return node;
  for (const QSGNode* c = node->firstChild(); c != nullptr; c = c->nextSibling()) {
    if (const QSGNode* hit = first_of_type(c, type)) return hit;
  }
  return nullptr;
}

bool is_ancestor(const QSGNode* ancestor, const QSGNode* node) {
  for (const QSGNode* n = node != nullptr ? node->parent() : nullptr; n != nullptr; n = n->parent()) {
    if (n == ancestor) return true;
  }
  return false;
}

void test_scene_graph_pixels_and_retention() {
  constexpr uint32_t kBox = 2u, kLow = 3u, kHigh = 4u, kClip = 5u, kOverflow = 6u, kFaded = 7u, kHidden = 8u,
                     kShadowed = 9u, kLabel = 10u;
  ScriptedApp app;
  h::Nve2Writer& w = app.w;
  w.create(1u);
  w.set(1u, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, h::rgba(255, 255, 255));
  w.set(1u, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_FLEX_DIRECTION, NVE_FLEX_DIRECTION_ROW);
  w.set(1u, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_FLEX_WRAP, NVE_FLEX_WRAP_WRAP);
  w.create(kBox);  // 60x40 red with a 4px blue border
  size_node(w, kBox, 60.0f, 40.0f);
  w.set(kBox, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, h::rgba(255, 0, 0));
  w.set(kBox, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BORDER_COLOR, h::rgba(0, 0, 255));
  w.set_f32(kBox, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BORDER_WIDTH, 4.0f);
  // kLow (green) is later in scene order but z 1 < kHigh's z 2: kHigh (black) paints on top.
  w.create(kHigh);
  size_node(w, kHigh, 40.0f, 40.0f);
  w.set(kHigh, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, h::rgba(0, 0, 0));
  w.set(kHigh, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_Z_INDEX, 2u);
  w.create(kLow);
  size_node(w, kLow, 40.0f, 40.0f);
  w.set(kLow, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, h::rgba(0, 255, 0));
  w.set(kLow, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_Z_INDEX, 1u);
  w.set(kLow, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_MARGIN_LEFT, h::length_points(-20.0f));
  // kClip: 50x50 rounded overflow:hidden; its 200x200 yellow child is clipped to it.
  w.create(kClip);
  size_node(w, kClip, 50.0f, 50.0f);
  w.set(kClip, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_OVERFLOW_X, NVE_OVERFLOW_HIDDEN);
  w.set(kClip, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_OVERFLOW_Y, NVE_OVERFLOW_HIDDEN);
  w.set_f32(kClip, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BORDER_RADIUS, 25.0f);
  w.create(kOverflow);
  size_node(w, kOverflow, 200.0f, 200.0f);
  w.set(kOverflow, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, h::rgba(255, 255, 0));
  const uint32_t clipped[] = {kOverflow};
  w.set_children(kClip, clipped);
  w.create(kFaded);  // 50% black
  size_node(w, kFaded, 30.0f, 30.0f);
  w.set(kFaded, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, h::rgba(0, 0, 0));
  w.set_f32(kFaded, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_OPACITY, 0.5f);
  w.create(kHidden);
  size_node(w, kHidden, 30.0f, 30.0f);
  w.set(kHidden, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, h::rgba(255, 0, 255));
  w.set(kHidden, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_VISIBILITY, NVE_VISIBILITY_HIDDEN);
  w.create(kShadowed);
  size_node(w, kShadowed, 30.0f, 30.0f);
  w.set(kShadowed, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_MARGIN_LEFT, h::length_points(20.0f));
  w.set(kShadowed, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, h::rgba(255, 255, 255));
  const h::BoxShadow shadows[] = {{12.0f, 0.0f, 0.0f, 0.0f, h::rgba(0, 0, 0), false}};
  w.define_resource(40u, h::kBoxShadowsKind, h::encode_box_shadows(shadows));
  w.set(kShadowed, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BOX_SHADOWS, 40u);
  text_node(w, kLabel, "Quick label");
  const uint32_t kids[] = {kBox, kHigh, kLow, kClip, kFaded, kHidden, kShadowed, kLabel};
  w.set_children(1u, kids);

  QuickHarness q(QSize(400, 200));
  ProbeItem& item = *q.item;
  NVE_CHECK(item.host().attach_interface(&ScriptedApp::get_interface));
  NVE_CHECK(item.isAppLoaded() && item.nodeCount() == 9u && item.nodeText(kLabel) == "Quick label");
  NVE_CHECK(item.nodeRect(kBox) == QRectF(0, 0, 60, 40) && item.nodeRect(kHigh) == QRectF(60, 0, 40, 40));
  NVE_CHECK(item.nodeRect(kLow) == QRectF(80, 0, 40, 40));
  NVE_CHECK(item.nodeRect(kOverflow) == QRectF(120, 0, 200, 200));  // layout rects ignore clips

  QImage img = q.grab();
  NVE_CHECK(same_rgb(img.pixelColor(30, 20), 255, 0, 0) && same_rgb(img.pixelColor(1, 20), 0, 0, 255));
  NVE_CHECK(same_rgb(img.pixelColor(90, 20), 0, 0, 0));       // z-index: kHigh over kLow
  NVE_CHECK(same_rgb(img.pixelColor(110, 20), 0, 255, 0));    // kLow's uncovered part
  NVE_CHECK(same_rgb(img.pixelColor(145, 25), 255, 255, 0));  // inside the clip
  NVE_CHECK(same_rgb(img.pixelColor(145, 80), 255, 255, 255)); // below the clip box
  // Rounded clip: a stencil clip with rounded geometry. The software adaptation honours only
  // `clipRect` (docs/UPSTREAM.md), so the corner pixel is checked on RHI backends only.
  const auto* round_clip = static_cast<const QSGClipNode*>(first_of_type(q.sg(kClip), QSGNode::ClipNodeType));
  NVE_CHECK(round_clip != nullptr && !round_clip->isRectangular() && round_clip->geometry() != nullptr);
  NVE_CHECK(round_clip->clipRect() == QRectF(0, 0, 50, 50) && round_clip->geometry()->vertexCount() > 8);
  if (q.window.rendererInterface()->graphicsApi() != QSGRendererInterface::Software) {
    NVE_CHECK(same_rgb(img.pixelColor(121, 1), 255, 255, 255));
  }
  NVE_CHECK(same_rgb(img.pixelColor(185, 15), 128, 128, 128, 8));  // 50% black on white
  NVE_CHECK(same_rgb(img.pixelColor(215, 15), 255, 255, 255));     // hidden: not painted
  NVE_CHECK(same_rgb(img.pixelColor(275, 15), 255, 255, 255));     // shadowed box itself
  NVE_CHECK(same_rgb(img.pixelColor(290, 15), 0, 0, 0));           // its shadow, 12px right

  // Retention: every node has a wrapper; an unrelated change updates only that node.
  const SceneGraphStats first = item.sceneGraphStats();
  NVE_CHECK(first.retainedNodes == 9u && first.syncCount >= 1u && first.solidRectNodes >= 5u);
  NVE_CHECK(first.textRasterizations == 1u && first.boxRasterizations >= 2u);
  const QSGNode* box_sg = q.sg(kBox);
  const QSGNode* label_sg = q.sg(kLabel);
  NVE_CHECK(box_sg != nullptr && label_sg != nullptr && is_ancestor(q.sg(kClip), q.sg(kOverflow)));
  w.set(kFaded, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, h::rgba(0, 0, 255));
  flush(item.host());
  img = q.grab();
  NVE_CHECK(same_rgb(img.pixelColor(185, 15), 128, 128, 255, 8));
  const SceneGraphStats second = item.sceneGraphStats();
  NVE_CHECK(second.syncCount == first.syncCount + 1u);
  NVE_CHECK(second.textRasterizations == first.textRasterizations);  // the label texture was kept
  NVE_CHECK(q.sg(kBox) == box_sg && q.sg(kLabel) == label_sg);
  // Nothing changed: a grab does not touch the scene graph.
  (void)q.grab();
  NVE_CHECK(item.sceneGraphStats().syncCount == second.syncCount);

  // Text change: re-rasterized in place; the wrapper stays.
  (void)w.set_str(kLabel, NVE_TABLE_TEXT_DESC, NVE_TABLE_TEXT_TEXT, "Changed");
  flush(item.host());
  (void)q.grab();
  NVE_CHECK(item.nodeText(kLabel) == "Changed" && q.sg(kLabel) == label_sg);
  NVE_CHECK(item.sceneGraphStats().textRasterizations == first.textRasterizations + 1u);

  // Reparent kBox into kClip: same wrapper, new parent; remove kHidden: its wrapper is gone.
  const uint32_t clip_kids[] = {kBox, kOverflow};
  w.set_children(kClip, clip_kids);
  w.remove(kHidden);
  const uint32_t rest[] = {kHigh, kLow, kClip, kFaded, kShadowed, kLabel};
  w.set_children(1u, rest);
  flush(item.host());
  img = q.grab();
  NVE_CHECK(q.sg(kBox) == box_sg && is_ancestor(q.sg(kClip), box_sg) && q.sg(kHidden) == nullptr);
  NVE_CHECK(item.nodeCount() == 8u && item.sceneGraphStats().retainedNodes == 8u);
  const QRectF moved = item.nodeRect(kBox);
  NVE_CHECK(moved.topLeft() == item.nodeRect(kClip).topLeft() && moved.size() == QSizeF(60, 40));
  NVE_CHECK(same_rgb(img.pixelColor(static_cast<int>(moved.x()) + 30, static_cast<int>(moved.y()) + 20), 255, 0, 0));
  NVE_CHECK(!same_rgb(img.pixelColor(static_cast<int>(moved.x()) + 55, static_cast<int>(moved.y()) + 20), 255, 0, 0));

  // Root background change repaints the root only.
  w.set(1u, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, h::rgba(10, 20, 30));
  flush(item.host());
  NVE_CHECK(same_rgb(q.at(395, 195), 10, 20, 30));
}

void test_qml_api_modules_hot_reload_and_state() {
  QuickHarness q;
  ProbeItem& item = *q.item;
  int loaded_signals = 0, path_signals = 0, scene_signals = 0;
  QObject::connect(&item, &NveQtQuickItem::appLoadedChanged, [&] { ++loaded_signals; });
  QObject::connect(&item, &NveQtQuickItem::dylibPathChanged, [&] { ++path_signals; });
  QObject::connect(&item, &NveQtQuickItem::sceneChanged, [&] { ++scene_signals; });
  NVE_CHECK(!item.loadDylib(QString()) && !item.loadDylib(QStringLiteral("/nonexistent/libnope.so")));
  NVE_CHECK(!item.loadDylib(QStringLiteral(NVE_TEST_NO_SURFACE_MODULE)) && !item.isAppLoaded());
  NVE_CHECK(loaded_signals == 0 && path_signals == 0 && item.dylibPath().isEmpty() && !item.hotReload());

  const nve::host::testing::ScratchDir dir("nve-qt-quick-app");
  const std::string app_path = dir.file("app" + std::filesystem::path(NVE_TEST_COUNTER_MODULE).extension().string());
  const auto build = [&](const char* fixture) {
    std::filesystem::copy_file(fixture, app_path, std::filesystem::copy_options::overwrite_existing);
  };
  build(NVE_TEST_COUNTER_MODULE);
  item.setDylibPath(QString::fromStdString(app_path));  // QML property write loads
  NVE_CHECK(item.isAppLoaded() && item.dylibPath() == QString::fromStdString(app_path));
  NVE_CHECK(loaded_signals == 1 && path_signals == 1 && scene_signals >= 1 && item.nodeCount() == 5u);
  NVE_CHECK(item.nodeText(CounterApp::kCount) == "Count: 0" && item.sceneGeneration() > 0u);

  // A real click on the increment button (hit test -> Press) counts.
  q.click(q.center(CounterApp::kIncrement));
  q.click(q.center(CounterApp::kIncrement));
  NVE_CHECK(item.nodeText(CounterApp::kCount) == "Count: 2");
  NVE_CHECK(item.focusedNodeId() == CounterApp::kIncrement);  // buttons are focusable
  const QSGNode* inc_sg = (q.grab(), q.sg(CounterApp::kIncrement));
  NVE_CHECK(inc_sg != nullptr);

  // Failed reload keeps the app; a rebuilt file reloads with new code, state, nodes and focus.
  NVE_CHECK(!item.hotReload(QStringLiteral(NVE_TEST_NO_ENTRY_MODULE)) && item.isAppLoaded());
  build(NVE_TEST_COUNTER_DOUBLE_MODULE);
  NVE_CHECK(item.hotReload());
  NVE_CHECK(item.nodeText(CounterApp::kCount) == "Count: 2" && item.nodeCount() == 5u);
  NVE_CHECK(item.focusedNodeId() == CounterApp::kIncrement);
  (void)q.grab();
  NVE_CHECK(q.sg(CounterApp::kIncrement) == inc_sg);  // retained across the reload
  q.key(Qt::Key_Space, QStringLiteral(" "));         // keyboard activation of the focused button
  NVE_CHECK(item.nodeText(CounterApp::kCount) == "Count: 4");

  // State file round trip through the host.
  QtHost& host = item.host();
  const std::string path = dir.file("state.nvestate");
  NVE_CHECK(host.save_state(path, 2u));
  q.click(q.center(CounterApp::kReset));
  NVE_CHECK(item.nodeText(CounterApp::kCount) == "Count: 0");
  NVE_CHECK(!host.restore_state(path, 1u) && host.restore_state(path, 2u));
  NVE_CHECK(item.nodeText(CounterApp::kCount) == "Count: 4");
  NVE_CHECK(host.clear_state(path) && !std::filesystem::exists(path));

  // A new surface id drops the app (and its nodes); the same id is a no-op.
  int surface_signals = 0;
  QObject::connect(&item, &NveQtQuickItem::surfaceIdChanged, [&] { ++surface_signals; });
  item.setSurfaceId(item.surfaceId());
  NVE_CHECK(surface_signals == 0 && item.isAppLoaded());
  item.setSurfaceId(7u);
  NVE_CHECK(surface_signals == 1 && item.surfaceId() == 7u && !item.isAppLoaded() && item.nodeCount() == 0u);
  NVE_CHECK(loaded_signals == 2 && item.dylibPath().isEmpty() && item.focusedNodeId() == 0u);
  (void)q.grab();
  NVE_CHECK(item.sceneGraphStats().retainedNodes == 0u);

  // Pairing: a malformed endpoint is refused; a well-formed one never pairs (nothing listens).
  NVE_CHECK(!item.connectPaired(QStringLiteral("not a uri"), QStringLiteral("t")) && !item.isAppLoaded());
  if (item.connectPaired(QStringLiteral("nve://127.0.0.1:9"), QStringLiteral("t"))) {
    spin();
    NVE_CHECK(!host.is_paired());
    item.disconnectPaired();
  }
  NVE_CHECK(!host.is_paired() && !item.isAppLoaded());
}

void test_queued_wakes_never_outlive_the_item() {
  ScriptedApp app;
  app.w.create(1u);
  app.w.create(2u);
  app.w.attach(2u, NVE_TABLE_SCROLL_DESC);
  size_node(app.w, 2u, 100.0f, 50.0f);
  app.w.create(3u);
  size_node(app.w, 3u, 100.0f, 400.0f);
  const uint32_t inner[] = {3u};
  app.w.set_children(2u, inner);
  const uint32_t kids[] = {2u};
  app.w.set_children(1u, kids);
  {
    auto q = std::make_unique<QuickHarness>();
    NVE_CHECK(q->host().attach_interface(&ScriptedApp::get_interface));
    NVE_CHECK(q->host().scroll_by(2u, 0.0f, 30.0f).has_value());
    NVE_CHECK(q->item->scrollOffsetForNode(2u) == QPointF());  // published by the queued pump
    spin();
    NVE_CHECK(approx(q->item->scrollOffsetForNode(2u).y(), 30.0, 0.01));
    NVE_CHECK(q->host().scroll_by(2u, 0.0f, 10.0f).has_value());
    q.reset();  // the queued wake is dropped
    spin();
  }
  NVE_CHECK(app.creates == 1u && app.destroys == 1u);
}

void test_live_kitchen_sink_if_configured() {
  const char* path = std::getenv("NVE_TEST_KITCHEN_SINK_DYLIB");
  if (path == nullptr || *path == '\0') return;
  A11yRecorder recorder;
  QuickHarness q(QSize(1280, 800));
  ProbeItem& item = *q.item;
  NVE_CHECK(item.loadDylib(QString::fromUtf8(path)) && item.nodeCount() > 10u);
  const QImage img = q.grab();
  NVE_CHECK(!img.isNull() && item.sceneGraphStats().retainedNodes == item.nodeCount());
  NVE_CHECK(item.sceneGraphStats().textRasterizations > 5u);

  // Find texts through the accessibility tree, press through real hit testing.
  QAccessibleInterface* root = QAccessible::queryAccessibleInterface(&item);
  NVE_CHECK(root != nullptr && root->childCount() > 0);
  NVE_CHECK(find_accessible(root, QStringLiteral("NVE Kitchen Sink")) != nullptr);
  QAccessibleInterface* count = find_accessible(root, QStringLiteral("Count: 0"));
  NVE_CHECK(count != nullptr);
  const uint32_t count_id = item.hitTestNode(item.mapFromGlobal(QPointF(count->rect().center())));
  NVE_CHECK(count_id != 0u && item.nodeText(count_id) == "Count: 0");
  // Buttons: the nearest accessible ancestor (or self) of a label that has the press action.
  const auto pressable = [&](const QString& label) {
    QAccessibleInterface* a = find_accessible(root, label);
    while (a != nullptr && a != root && a->actionInterface()->actionNames().isEmpty()) a = a->parent();
    return a != root ? a : nullptr;
  };
  QAccessibleInterface* inc = pressable(QStringLiteral("Increment +"));
  QAccessibleInterface* reset = pressable(QStringLiteral("Reset"));
  NVE_CHECK(inc != nullptr && reset != nullptr && inc != reset && inc->role() == QAccessible::Button);
  q.click(item.mapFromGlobal(QPointF(inc->rect().center())));  // a real click through hit testing
  NVE_CHECK(item.nodeText(count_id) == "Count: 1");
  inc->actionInterface()->doAction(QAccessibleActionInterface::pressAction());
  NVE_CHECK(item.nodeText(count_id) == "Count: 2");
  NVE_CHECK(A11yRecorder::count(QAccessible::NameChanged, QStringLiteral("Count: 2")) >= 1u);
  reset->actionInterface()->doAction(QAccessibleActionInterface::pressAction());
  NVE_CHECK(item.nodeText(count_id) == "Count: 0");
  for (int i = 0; i < 3; ++i) inc->actionInterface()->doAction(QAccessibleActionInterface::pressAction());
  (void)q.grab();
  NVE_CHECK(item.sceneGraphStats().retainedNodes == item.nodeCount());

  // Hot reload: state, records, scene-graph wrappers and accessible interfaces are kept.
  const quint32 nodes = item.nodeCount();
  const QSGNode* count_sg = q.sg(count_id);
  NVE_CHECK(item.hotReload());
  (void)q.grab();
  NVE_CHECK(item.nodeText(count_id) == "Count: 3" && item.nodeCount() == nodes && q.sg(count_id) == count_sg);
  NVE_CHECK(inc->isValid() && find_accessible(root, QStringLiteral("Count: 3")) == count);
}

}  // namespace

void nve_test_qt_quick_input();
void nve_test_qt_quick_media_canvas_a11y();

/// Wrapper ownership and clip shape across orphaning, a dropped scene graph, and radius changes:
/// a detached subtree keeps (owns) its wrappers and comes back intact; when the item leaves the
/// window Qt deletes the tree and every record, attached or not, is rebuilt; a clip that loses
/// its radius gets rectangular geometry (never the stale rounded one).
void test_scene_graph_orphans_rebuild_and_clip_shape() {
  constexpr uint32_t kHolder = 2u, kClip = 3u, kInner = 4u;
  ScriptedApp app;
  h::Nve2Writer& w = app.w;
  w.create(1u);
  w.set(1u, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, h::rgba(255, 255, 255));
  w.create(kHolder);
  size_node(w, kHolder, 100.0f, 100.0f);
  w.create(kClip);
  size_node(w, kClip, 60.0f, 60.0f);
  w.set(kClip, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_OVERFLOW_X, NVE_OVERFLOW_HIDDEN);
  w.set(kClip, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_OVERFLOW_Y, NVE_OVERFLOW_HIDDEN);
  w.set_f32(kClip, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BORDER_RADIUS, 30.0f);
  w.create(kInner);
  size_node(w, kInner, 200.0f, 200.0f);
  w.set(kInner, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, h::rgba(255, 0, 0));
  const uint32_t inner[] = {kInner};
  w.set_children(kClip, inner);
  const uint32_t clip[] = {kClip};
  w.set_children(kHolder, clip);
  const uint32_t holder[] = {kHolder};
  w.set_children(1u, holder);
  QuickHarness q;
  ProbeItem& item = *q.item;
  NVE_CHECK(item.host().attach_interface(&ScriptedApp::get_interface));
  (void)q.grab();
  const auto clip_node = [&] {
    return static_cast<const QSGClipNode*>(first_of_type(q.sg(kClip), QSGNode::ClipNodeType));
  };
  NVE_CHECK(clip_node() != nullptr && clip_node()->geometry()->vertexCount() > 8);

  // Radius 0: rectangular, with a 4-vertex rect geometry.
  w.set_f32(kClip, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BORDER_RADIUS, 0.0f);
  flush(item.host());
  NVE_CHECK(same_rgb(q.at(55, 55), 255, 0, 0));  // the corner is no longer clipped away
  NVE_CHECK(clip_node()->isRectangular() && clip_node()->geometry()->vertexCount() == 4);
  NVE_CHECK(clip_node()->geometry()->vertexDataAsPoint2D()[3].x == 60.0f);

  // Orphan kClip's subtree (alive, detached): its wrappers stay owned, out of the tree.
  const QSGNode* clip_sg = q.sg(kClip);
  w.set_children(kHolder, std::span<const uint32_t>());
  flush(item.host());
  NVE_CHECK(same_rgb(q.at(10, 10), 255, 255, 255));
  NVE_CHECK(q.sg(kClip) == clip_sg && clip_sg->parent() == nullptr);
  // Drop the whole scene graph while the subtree is orphaned, then re-attach both.
  item.setParentItem(nullptr);
  (void)q.grab();
  item.setParentItem(q.window.contentItem());
  w.set_children(kHolder, clip);
  flush(item.host());
  NVE_CHECK(same_rgb(q.at(10, 10), 255, 0, 0) && same_rgb(q.at(70, 70), 255, 255, 255));
  NVE_CHECK(q.sg(kClip) != nullptr && is_ancestor(q.sg(kHolder), q.sg(kInner)));
  NVE_CHECK(item.sceneGraphStats().retainedNodes == 3u);
}

int main(int argc, char** argv) {
  QGuiApplication app(argc, argv);
  test_scene_graph_pixels_and_retention();
  test_scene_graph_orphans_rebuild_and_clip_shape();
  test_qml_api_modules_hot_reload_and_state();
  test_queued_wakes_never_outlive_the_item();
  test_live_kitchen_sink_if_configured();
  nve_test_qt_quick_input();
  nve_test_qt_quick_media_canvas_a11y();
  std::cout << "nve_qt_quick_test: ok\n";
  return 0;
}
