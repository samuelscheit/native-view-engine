// Qt Quick host media and semantics: images (data URL / bytes, fit, tint, broken sources, the
// content-keyed cache, texture reuse), `nve.canvas` draw lists (pixels, generation redraws,
// corrupt payloads, detach), typography, and the accessible node tree (roles, names, states,
// values, heading attributes, press action, geometry, change / focus / destroy events).
#include "qt_quick_helpers.hpp"

#include <nve/canvas/draw_list_encoder.hpp>
#include <nve/host/core_resources.hpp>

#include <algorithm>

namespace {

using namespace nve::qt;
using namespace nve::qt::test;
namespace h = nve::host;
namespace c = nve::canvas;

void image(h::Nve2Writer& w, uint32_t id, uint32_t res, std::span<const uint8_t> src) {
  w.create(id);
  w.define_resource(res, h::kImageSrcKind, src);
  w.set(id, NVE_TABLE_IMAGE_DESC, NVE_TABLE_IMAGE_SRC, res);
  size_node(w, id, 40.0f, 20.0f);
}

std::vector<uint8_t> two_rects(c::DrawColorRgba second) {
  c::DrawListEncoder e;
  e.clear(c::DrawColorRgba::rgb(255, 255, 255))
      .fill_rect(0.0f, 0.0f, 20.0f, 20.0f, c::DrawColorRgba::rgb(255, 0, 0))
      .fill_rect(20.0f, 0.0f, 20.0f, 20.0f, second);
  return e.finish();
}

void test_images_and_canvas() {
  constexpr uint32_t kUrl = 2u, kBytes = 3u, kTint = 4u, kBad = 5u, kContain = 6u, kCanvas = 7u, kSame = 8u;
  ScriptedApp app;
  h::Nve2Writer& w = app.w;
  w.create(1u);
  w.set(1u, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, h::rgba(255, 255, 255));
  w.set(1u, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_FLEX_DIRECTION, NVE_FLEX_DIRECTION_ROW);
  w.set(1u, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_ALIGN_ITEMS, NVE_ALIGN_ITEMS_FLEX_START);
  const std::vector<uint8_t> red = png(4, 4, QColor(255, 0, 0));
  image(w, kUrl, 60u, h::encode_image_url(data_url(png(2, 2, QColor(0, 255, 0)))));
  image(w, kBytes, 61u, h::encode_image_bytes(red));
  image(w, kTint, 62u, h::encode_image_bytes(red));
  w.set_bool(kTint, NVE_TABLE_IMAGE_DESC, NVE_TABLE_IMAGE_TINT_ENABLED, true);
  w.set(kTint, NVE_TABLE_IMAGE_DESC, NVE_TABLE_IMAGE_TINT, h::rgba(0, 0, 255));
  image(w, kBad, 63u, h::encode_image_url("data:image/png;base64,AAAA"));
  image(w, kContain, 64u, h::encode_image_bytes(red));  // square image in a 40x20 box: pillarboxed
  w.set(kContain, NVE_TABLE_IMAGE_DESC, NVE_TABLE_IMAGE_FIT, NVE_IMAGE_FIT_CONTAIN);
  w.create(kCanvas);
  size_node(w, kCanvas, 40.0f, 20.0f);
  w.define_resource(70u, c::kDrawListKind, two_rects(c::DrawColorRgba::rgb(0, 0, 255)));
  w.set(kCanvas, NVE_TABLE_CANVAS_DESC, NVE_TABLE_CANVAS_DRAW_LIST, 70u);
  w.set(kCanvas, NVE_TABLE_CANVAS_DESC, NVE_TABLE_CANVAS_GENERATION, 1u);
  image(w, kSame, 65u, h::encode_image_bytes(red));  // same bytes as kBytes: one cached decode
  const uint32_t kids[] = {kUrl, kBytes, kTint, kBad, kContain, kCanvas, kSame};
  w.set_children(1u, kids);

  QuickHarness q(QSize(320, 60));
  ProbeItem& item = *q.item;
  NVE_CHECK(item.host().attach_interface(&ScriptedApp::get_interface));
  QImage img = q.grab();
  NVE_CHECK(same_rgb(img.pixelColor(20, 10), 0, 255, 0));    // data URL, cover
  NVE_CHECK(same_rgb(img.pixelColor(60, 10), 255, 0, 0));    // bytes
  NVE_CHECK(same_rgb(img.pixelColor(100, 10), 0, 0, 255));   // tinted
  NVE_CHECK(same_rgb(img.pixelColor(140, 10), 255, 255, 255));  // broken: nothing painted
  NVE_CHECK(same_rgb(img.pixelColor(180, 10), 255, 0, 0) && same_rgb(img.pixelColor(162, 10), 255, 255, 255));
  NVE_CHECK(same_rgb(img.pixelColor(205, 10), 255, 0, 0) && same_rgb(img.pixelColor(235, 10), 0, 0, 255));
  NVE_CHECK(same_rgb(img.pixelColor(260, 10), 255, 0, 0));
  const SceneGraphStats first = item.sceneGraphStats();
  NVE_CHECK(first.imageRasterizations == 6u && first.canvasRasterizations == 1u);
  // Content-keyed: the green URL + the red bytes (decoded once for kBytes / kTint / kContain /
  // kSame); the broken source is not cached.
  NVE_CHECK(item.host().images().size() == 2u && item.host().images().hits() >= 3u);

  // An unrelated change: no image / canvas re-rasterization.
  w.set(1u, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, h::rgba(250, 250, 250));
  flush(item.host());
  (void)q.grab();
  NVE_CHECK(item.sceneGraphStats().imageRasterizations == first.imageRasterizations);
  NVE_CHECK(item.sceneGraphStats().canvasRasterizations == first.canvasRasterizations);
  // Tint change: one image re-rasterized.
  w.set(kTint, NVE_TABLE_IMAGE_DESC, NVE_TABLE_IMAGE_TINT, h::rgba(0, 255, 0));
  flush(item.host());
  NVE_CHECK(same_rgb(q.at(100, 10), 0, 255, 0));
  NVE_CHECK(item.sceneGraphStats().imageRasterizations == first.imageRasterizations + 1u);

  // Canvas: a new draw list + generation redraws; a corrupt one paints only the background.
  w.define_resource(71u, c::kDrawListKind, two_rects(c::DrawColorRgba::rgb(0, 255, 0)));
  w.set(kCanvas, NVE_TABLE_CANVAS_DESC, NVE_TABLE_CANVAS_DRAW_LIST, 71u);
  w.set(kCanvas, NVE_TABLE_CANVAS_DESC, NVE_TABLE_CANVAS_GENERATION, 2u);
  w.release_resource(70u);
  flush(item.host());
  NVE_CHECK(same_rgb(q.at(235, 10), 0, 255, 0));
  NVE_CHECK(item.sceneGraphStats().canvasRasterizations == first.canvasRasterizations + 1u);
  std::vector<uint8_t> corrupt = two_rects(c::DrawColorRgba::rgb(0, 0, 0));
  corrupt.resize(corrupt.size() - 3u);
  w.define_resource(72u, c::kDrawListKind, corrupt);
  w.set(kCanvas, NVE_TABLE_CANVAS_DESC, NVE_TABLE_CANVAS_DRAW_LIST, 72u);
  w.set(kCanvas, NVE_TABLE_CANVAS_DESC, NVE_TABLE_CANVAS_GENERATION, 3u);
  w.set(kCanvas, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, h::rgba(9, 9, 9));
  flush(item.host());
  NVE_CHECK(same_rgb(q.at(205, 10), 9, 9, 9) && same_rgb(q.at(235, 10), 9, 9, 9));
  // Detaching the canvas row: a plain box again (solid rect, no texture).
  const uint32_t solid = item.sceneGraphStats().solidRectNodes;
  w.detach(kCanvas, NVE_TABLE_CANVAS_DESC);
  flush(item.host());
  NVE_CHECK(same_rgb(q.at(205, 10), 9, 9, 9) && item.sceneGraphStats().solidRectNodes == solid + 1u);
}

/// O(changed) sync contracts: moved-only records get a new box (and are re-rasterized only when
/// their size changed); resources republished under the same id reach exactly their users.
void test_moves_and_republished_resources() {
  constexpr uint32_t kBox = 2u, kTop = 3u, kText = 4u, kImg = 5u, kCanvas = 6u;
  ScriptedApp app;
  h::Nve2Writer& w = app.w;
  w.create(1u);
  w.set(1u, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, h::rgba(255, 255, 255));
  w.set(1u, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_ALIGN_ITEMS, NVE_ALIGN_ITEMS_FLEX_START);
  w.create(kBox);
  w.set(kBox, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_WIDTH, h::length_points(200.0f));
  w.create(kTop);
  size_node(w, kTop, 50.0f, 20.0f);
  w.set(kTop, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, h::rgba(255, 255, 255));
  const h::BoxShadow near[] = {{4.0f, 0.0f, 0.0f, 0.0f, h::rgba(0, 0, 0), false}};
  w.define_resource(80u, h::kBoxShadowsKind, h::encode_box_shadows(near));
  w.set(kTop, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BOX_SHADOWS, 80u);
  text_node(w, kText, "moving text");
  w.set(kText, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_PADDING_LEFT, h::length_percent(10.0f));
  image(w, kImg, 60u, h::encode_image_bytes(png(4, 4, QColor(255, 0, 0))));
  w.create(kCanvas);
  size_node(w, kCanvas, 40.0f, 20.0f);
  w.define_resource(70u, c::kDrawListKind, two_rects(c::DrawColorRgba::rgb(0, 0, 255)));
  w.set(kCanvas, NVE_TABLE_CANVAS_DESC, NVE_TABLE_CANVAS_DRAW_LIST, 70u);
  const uint32_t inner[] = {kTop, kText, kImg, kCanvas};
  w.set_children(kBox, inner);
  const uint32_t kids[] = {kBox};
  w.set_children(1u, kids);

  QuickHarness q(QSize(400, 200));
  ProbeItem& item = *q.item;
  NVE_CHECK(item.host().attach_interface(&ScriptedApp::get_interface));
  (void)q.grab();
  NVE_CHECK(item.nodeRect(kText).width() == 200.0 && same_rgb(q.at(52, 10), 0, 0, 0));  // shadow 4px right

  // A wider container: the text moves (wider: re-rasterized once), siblings keep their textures.
  SceneGraphStats before = item.sceneGraphStats();
  w.set(kBox, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_WIDTH, h::length_points(300.0f));
  flush(item.host());
  (void)q.grab();
  SceneGraphStats after = item.sceneGraphStats();
  NVE_CHECK(after.nodesMoved >= before.nodesMoved + 1u && item.nodeRect(kText).width() == 300.0);
  NVE_CHECK(after.textRasterizations == before.textRasterizations + 1u);
  NVE_CHECK(after.imageRasterizations == before.imageRasterizations);
  NVE_CHECK(after.canvasRasterizations == before.canvasRasterizations);
  // A taller first child: later siblings move down, re-rasterizing nothing.
  const QRectF img = item.nodeRect(kImg);
  before = after;
  w.set(kTop, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_HEIGHT, h::length_points(40.0f));
  flush(item.host());
  const QImage moved = q.grab();
  after = item.sceneGraphStats();
  NVE_CHECK(after.nodesMoved >= before.nodesMoved + 3u && item.nodeRect(kImg).y() == img.y() + 20.0);
  NVE_CHECK(after.textRasterizations == before.textRasterizations);
  NVE_CHECK(after.imageRasterizations == before.imageRasterizations);
  NVE_CHECK(after.canvasRasterizations == before.canvasRasterizations);
  const QRectF canvas = item.nodeRect(kCanvas);
  const QPoint red_px = canvas.topLeft().toPoint() + QPoint(5, 5);
  NVE_CHECK(same_rgb(moved.pixelColor(red_px), 255, 0, 0));  // drawn at its new place
  NVE_CHECK(same_rgb(moved.pixelColor(img.center().toPoint() + QPoint(0, 20)), 255, 0, 0));

  // Same id, new bytes: exactly their readers re-decode.
  before = after;
  w.define_resource(70u, c::kDrawListKind, two_rects(c::DrawColorRgba::rgb(0, 255, 0)));
  w.define_resource(60u, h::kImageSrcKind, h::encode_image_bytes(png(4, 4, QColor(0, 0, 255))));
  const h::BoxShadow far[] = {{12.0f, 0.0f, 0.0f, 0.0f, h::rgba(0, 0, 0), false}};
  w.define_resource(80u, h::kBoxShadowsKind, h::encode_box_shadows(far));
  flush(item.host());
  const QImage republished = q.grab();
  after = item.sceneGraphStats();
  NVE_CHECK(after.canvasRasterizations == before.canvasRasterizations + 1u);
  NVE_CHECK(after.imageRasterizations == before.imageRasterizations + 1u);
  NVE_CHECK(after.textRasterizations == before.textRasterizations);
  NVE_CHECK(same_rgb(republished.pixelColor(canvas.topLeft().toPoint() + QPoint(30, 5)), 0, 255, 0));
  NVE_CHECK(same_rgb(republished.pixelColor(img.center().toPoint() + QPoint(0, 20)), 0, 0, 255));
  NVE_CHECK(same_rgb(republished.pixelColor(60, 10), 0, 0, 0));  // the shadow now reaches 12px
  // A new id carrying the bytes already shown: no redraw.
  before = after;
  w.define_resource(71u, c::kDrawListKind, two_rects(c::DrawColorRgba::rgb(0, 255, 0)));
  w.set(kCanvas, NVE_TABLE_CANVAS_DESC, NVE_TABLE_CANVAS_DRAW_LIST, 71u);
  w.release_resource(70u);
  flush(item.host());
  (void)q.grab();
  NVE_CHECK(item.sceneGraphStats().canvasRasterizations == before.canvasRasterizations);
}

void test_typography_and_accessibility() {
  constexpr uint32_t kHeading = 2u, kButton = 3u, kCheck = 4u, kSlider = 5u, kHidden = 6u, kDisabled = 7u,
                     kGroup = 8u, kInner = 9u, kSecret = 10u;
  ScriptedApp app;
  h::Nve2Writer& w = app.w;
  w.create(1u);
  w.set(1u, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_SET, NVE_TYPOGRAPHY_SET_COLOR);  // inherited
  w.set(1u, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_COLOR, h::rgba(200, 0, 0));
  text_node(w, kHeading, "Title");
  w.set(kHeading, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_ROLE, NVE_ARIA_ROLE_HEADING);
  w.set(kHeading, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_LEVEL, 2u);
  w.set(kHeading, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_SET, NVE_TYPOGRAPHY_SET_FONT_SIZE);
  w.set_f32(kHeading, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_FONT_SIZE, 24.0f);
  text_node(w, kButton, "Go");
  w.set(kButton, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_ROLE, NVE_ARIA_ROLE_BUTTON);
  (void)w.set_str(kButton, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_HINT, "Starts it");
  subscribe(w, kButton, {NVE_EVENT_PRESS});
  w.create(kCheck);
  size_node(w, kCheck, 20.0f, 20.0f);
  w.set(kCheck, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_ROLE, NVE_ARIA_ROLE_CHECKBOX);
  (void)w.set_str(kCheck, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_LABEL, "Agree");
  w.set(kCheck, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_CHECKED, NVE_ARIA_TRI_STATE_TRUE);
  w.create(kSlider);
  size_node(w, kSlider, 20.0f, 20.0f);
  w.set(kSlider, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_ROLE, NVE_ARIA_ROLE_SLIDER);
  w.set_f32(kSlider, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_VALUE_NOW, 30.0f);
  w.set_f32(kSlider, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_VALUE_MAX, 100.0f);
  text_node(w, kHidden, "hidden");
  w.set(kHidden, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_VISIBILITY, NVE_VISIBILITY_HIDDEN);
  text_node(w, kDisabled, "Off");
  subscribe(w, kDisabled, {NVE_EVENT_PRESS});
  w.set_bool(kDisabled, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_DISABLED, true);
  w.create(kGroup);
  text_node(w, kInner, "Inner");
  const uint32_t inner[] = {kInner};
  w.set_children(kGroup, inner);
  w.create(kSecret);
  size_node(w, kSecret, 100.0f, 20.0f);
  w.set_bool(kSecret, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_SECURE, true);
  (void)w.set_str(kSecret, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, "pass");
  const uint32_t kids[] = {kHeading, kButton, kCheck, kSlider, kHidden, kDisabled, kGroup, kSecret};
  w.set_children(1u, kids);

  A11yRecorder recorder;
  QuickHarness q(QSize(300, 300));
  ProbeItem& item = *q.item;
  NVE_CHECK(item.host().attach_interface(&ScriptedApp::get_interface));
  NVE_CHECK(item.nodeRect(kHeading).height() > item.nodeRect(kButton).height());  // 24px font measured
  const QImage img = q.grab();
  const QRectF head = item.nodeRect(kHeading);
  bool red_ink = false;  // inherited text color reached the heading's pixels
  for (int x = static_cast<int>(head.left()); x < static_cast<int>(head.right()) && !red_ink; ++x) {
    for (int y = static_cast<int>(head.top()); y < static_cast<int>(head.bottom()) && !red_ink; ++y) {
      const QColor px = img.pixelColor(x, y);
      red_ink = px.red() > 150 && px.green() < 60 && px.blue() < 60;
    }
  }
  NVE_CHECK(red_ink);

  // The tree mirrors the scene: item -> nodes, parents and indexes consistent.
  QAccessibleInterface* root = QAccessible::queryAccessibleInterface(&item);
  NVE_CHECK(root != nullptr && root->object() == &item && root->role() == QAccessible::Pane);
  NVE_CHECK(root->childCount() == 8);
  for (int i = 0; i < root->childCount(); ++i) {
    QAccessibleInterface* child = root->child(i);
    NVE_CHECK(child != nullptr && child->isValid() && child->parent() == root && root->indexOfChild(child) == i);
  }
  QAccessibleInterface* heading = root->child(0);
  NVE_CHECK(heading->role() == QAccessible::Heading && heading->text(QAccessible::Name) == "Title");
  NVE_CHECK(heading->attributesInterface() != nullptr);
  NVE_CHECK(heading->attributesInterface()->attributeValue(QAccessible::Attribute::Level).toInt() == 2);
  NVE_CHECK(heading->rect() == QRectF(item.mapToGlobal(head.topLeft()), head.size()).toAlignedRect());
  QAccessibleInterface* button = root->child(1);
  NVE_CHECK(button->role() == QAccessible::Button && button->text(QAccessible::Description) == "Starts it");
  NVE_CHECK(button->actionInterface()->actionNames() == QStringList{QAccessibleActionInterface::pressAction()});
  button->actionInterface()->doAction(QAccessibleActionInterface::pressAction());
  NVE_CHECK(app.count(NVE_EVENT_PRESS, kButton) == 1u);
  button->actionInterface()->doAction(QStringLiteral("bogus"));
  NVE_CHECK(app.count(NVE_EVENT_PRESS, kButton) == 1u);
  NVE_CHECK(root->childAt(button->rect().center().x(), button->rect().center().y()) == button);
  QAccessibleInterface* check = root->child(2);
  NVE_CHECK(check->role() == QAccessible::CheckBox && check->state().checkable && check->state().checked);
  QAccessibleInterface* slider = root->child(3);
  NVE_CHECK(slider->valueInterface() != nullptr && slider->valueInterface()->currentValue().toDouble() == 30.0);
  NVE_CHECK(slider->valueInterface()->maximumValue().toDouble() == 100.0);
  NVE_CHECK(root->child(4)->state().invisible);
  QAccessibleInterface* disabled = root->child(5);
  NVE_CHECK(disabled->state().disabled && disabled->actionInterface()->actionNames().isEmpty());
  disabled->actionInterface()->doAction(QAccessibleActionInterface::pressAction());
  q.click(q.center(kDisabled));
  NVE_CHECK(app.count(NVE_EVENT_PRESS, kDisabled) == 0u);
  QAccessibleInterface* group = root->child(6);
  NVE_CHECK(group->childCount() == 1 && group->child(0)->text(QAccessible::Name) == "Inner");
  NVE_CHECK(group->child(0)->parent() == group);
  QAccessibleInterface* secret = root->child(7);
  NVE_CHECK(secret->role() == QAccessible::EditableText && secret->text(QAccessible::Value) == "••••");
  NVE_CHECK(item.accessibilityInfoForNodeId(kSecret)->secure && !item.accessibilityInfoForNodeId(999u));

  // Focus: a Focus event, the focused state and the focus child.
  q.click(q.center(kSecret));
  NVE_CHECK(A11yRecorder::count(QAccessible::Focus, QString()) >= 1u || secret->state().focused);
  NVE_CHECK(secret->state().focused && root->focusChild() == secret);

  // Semantics changes post events; removal invalidates the interface and posts ObjectDestroyed.
  (void)w.set_str(kCheck, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_LABEL, "Agreed");
  w.set(kCheck, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_CHECKED, NVE_ARIA_TRI_STATE_FALSE);
  flush(item.host());
  NVE_CHECK(A11yRecorder::count(QAccessible::NameChanged, QStringLiteral("Agreed")) == 1u);
  NVE_CHECK(A11yRecorder::count(QAccessible::StateChanged, QStringLiteral("Agreed")) == 1u);
  NVE_CHECK(check->text(QAccessible::Name) == "Agreed" && !check->state().checked);
  const QAccessible::Id check_id = QAccessible::uniqueId(check);
  w.remove(kCheck);
  const uint32_t rest[] = {kHeading, kButton, kSlider, kHidden, kDisabled, kGroup, kSecret};
  w.set_children(1u, rest);
  flush(item.host());
  NVE_CHECK(A11yRecorder::count(QAccessible::ObjectDestroyed, QStringLiteral("Agreed")) == 1u);  // posted by Qt
  NVE_CHECK(QAccessible::accessibleInterface(check_id) == nullptr && root->childCount() == 7);
  NVE_CHECK(root->child(2) == slider);  // surviving interfaces are kept
}

}  // namespace

void nve_test_qt_quick_media_canvas_a11y() {
  test_images_and_canvas();
  test_moves_and_republished_resources();
  test_typography_and_accessibility();
}
