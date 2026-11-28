// Qt Widgets host: accessibility through `QAccessible` (roles, names, descriptions, states,
// values, heading levels, press actions, change events), images (data URL, inline bytes, file
// URL, invalid, LRU cache hits, fit, tint), box visuals (border, radius, opacity, outer / inset
// shadows), inherited typography, visibility, z-index restacking, rounded clipping, and removal.
#include "NveQtWidgetInternal.hpp"
#include "qt_test_helpers.hpp"

#include <nve/host/core_resources.hpp>
#include <nve/host/testing/scratch_dir.hpp>

#include <QAccessible>
#include <QGraphicsOpacityEffect>

#include <fstream>
#include <vector>

namespace {

using namespace nve::qt;
using namespace nve::qt::test;
namespace h = nve::host;

constexpr uint32_t kColumn = 2u, kHeading = 3u, kCheck = 4u, kDisabled = 5u, kSlider = 6u, kImgUrl = 7u,
                   kImgBytes = 8u, kImgFile = 9u, kImgBad = 10u, kImgTint = 11u, kHidden = 12u, kLow = 13u,
                   kHigh = 14u, kClip = 15u, kClipChild = 16u;

void a11y(h::Nve2Writer& w, uint32_t id, uint32_t role, std::string_view label) {
  w.set(id, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_ROLE, role);
  if (!label.empty()) (void)w.set_str(id, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_LABEL, label);
}

void image(h::Nve2Writer& w, uint32_t id, uint32_t res, std::span<const uint8_t> src, std::string_view alt) {
  w.create(id);
  w.define_resource(res, h::kImageSrcKind, src);
  w.set(id, NVE_TABLE_IMAGE_DESC, NVE_TABLE_IMAGE_SRC, res);
  if (!alt.empty()) (void)w.set_str(id, NVE_TABLE_IMAGE_DESC, NVE_TABLE_IMAGE_ALT, alt);
  size_node(w, id, 32.0f, 16.0f);
}

/// Records accessibility events posted for the host's widgets.
std::vector<QAccessible::Event> g_events;
void record_update(QAccessibleEvent* event) { g_events.push_back(event->type()); }
void record_root(QObject*) {}

void test_a11y_images_and_visuals() {
  const nve::host::testing::ScratchDir scratch("nve-qt-image");
  const std::string tmp = scratch.file("image.png");
  const std::vector<uint8_t> red = png(4, 3, QColor(255, 0, 0));
  {
    std::ofstream out(tmp, std::ios::binary);
    out.write(reinterpret_cast<const char*>(red.data()), static_cast<std::streamsize>(red.size()));
  }

  ScriptedApp app;
  h::Nve2Writer& w = app.w;
  w.create(1u);
  w.create(kColumn);
  w.set(kColumn, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_FONT_STYLE, NVE_FONT_STYLE_ITALIC);
  (void)w.set_str(kColumn, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_FONT_FAMILY, "Courier");
  w.set(kColumn, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_TEXT_DECORATION, NVE_TEXT_DECORATION_UNDERLINE);
  w.set(kColumn, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_COLOR, h::rgba(200, 0, 0));

  text_node(w, kHeading, "Settings and more settings that will not fit");
  a11y(w, kHeading, NVE_ARIA_ROLE_HEADING, "");
  w.set(kHeading, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_LEVEL, 2u);
  w.set(kHeading, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_MAX_LINES, 1u);
  w.set(kHeading, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_TEXT_ALIGN, NVE_TEXT_ALIGN_CENTER);

  w.create(kCheck);
  a11y(w, kCheck, NVE_ARIA_ROLE_CHECKBOX, "Agree");
  (void)w.set_str(kCheck, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_HINT, "Accept the terms");
  w.set(kCheck, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_CHECKED, NVE_ARIA_TRI_STATE_TRUE);
  subscribe(w, kCheck, {NVE_INTERACTION_EVENT_PRESS});
  w.set(kCheck, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, h::rgba(0, 0, 255));
  w.set(kCheck, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BORDER_COLOR, h::rgba(0, 255, 0));
  w.set_f32(kCheck, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BORDER_WIDTH, 3.0f);
  w.set_f32(kCheck, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BORDER_RADIUS, 6.0f);
  w.set_f32(kCheck, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_OPACITY, 0.5f);
  const h::BoxShadow shadows[] = {{4.0f, 6.0f, 8.0f, 1.0f, h::rgba(0, 0, 0, 128), false},
                                  {0.0f, 0.0f, 2.0f, 0.0f, h::rgba(255, 255, 0, 255), true}};
  w.define_resource(50u, h::kBoxShadowsKind, h::encode_box_shadows(shadows));
  w.set(kCheck, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BOX_SHADOWS, 50u);
  size_node(w, kCheck, 60.0f, 30.0f);

  w.create(kDisabled);
  a11y(w, kDisabled, NVE_ARIA_ROLE_BUTTON, "Disabled");
  subscribe(w, kDisabled, {NVE_INTERACTION_EVENT_PRESS});
  w.set_bool(kDisabled, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_DISABLED, true);
  size_node(w, kDisabled, 20.0f, 20.0f);

  w.create(kSlider);
  a11y(w, kSlider, NVE_ARIA_ROLE_SLIDER, "Volume");
  w.set_f32(kSlider, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_VALUE_NOW, 30.0f);
  w.set_f32(kSlider, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_VALUE_MIN, 0.0f);
  w.set_f32(kSlider, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_VALUE_MAX, 100.0f);
  size_node(w, kSlider, 20.0f, 20.0f);

  const std::string url = data_url(png(2, 2, QColor(0, 255, 0)));
  image(w, kImgUrl, 60u, h::encode_image_url(url), "Logo");
  w.set(kImgUrl, NVE_TABLE_IMAGE_DESC, NVE_TABLE_IMAGE_FIT, NVE_IMAGE_FIT_CONTAIN);
  image(w, kImgBytes, 61u, h::encode_image_bytes(red), "");
  image(w, kImgFile, 62u, h::encode_image_url("file://" + tmp), "");
  image(w, kImgBad, 63u, h::encode_image_url("data:image/png;base64,AAAA"), "");
  image(w, kImgTint, 64u, h::encode_image_bytes(red), "");
  w.set(kImgTint, NVE_TABLE_IMAGE_DESC, NVE_TABLE_IMAGE_TINT, h::rgba(0, 0, 255));
  text_node(w, kHidden, "hidden");
  w.set(kHidden, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_VISIBILITY, NVE_VISIBILITY_HIDDEN);
  w.set_bool(kHidden, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_HIDDEN, true);
  // Overlapping siblings: the earlier one has the higher z-index, so it paints on top.
  w.create(kLow);
  size_node(w, kLow, 40.0f, 40.0f);
  w.set(kLow, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, h::rgba(255, 0, 255));
  w.set(kLow, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_Z_INDEX, 5u);
  w.create(kHigh);
  size_node(w, kHigh, 40.0f, 40.0f);
  w.set(kHigh, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, h::rgba(0, 255, 255));
  w.set(kHigh, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_MARGIN_TOP, h::length_points(-40.0f));
  // Rounded `overflow: hidden` clips its child at the corners.
  w.create(kClip);
  size_node(w, kClip, 40.0f, 40.0f);
  w.set_f32(kClip, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BORDER_RADIUS, 20.0f);
  w.set(kClip, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_OVERFLOW_X, NVE_OVERFLOW_HIDDEN);
  w.set(kClip, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_OVERFLOW_Y, NVE_OVERFLOW_HIDDEN);
  w.create(kClipChild);
  size_node(w, kClipChild, 40.0f, 40.0f);
  w.set(kClipChild, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, h::rgba(255, 128, 0));
  const uint32_t clip_kids[] = {kClipChild};
  w.set_children(kClip, clip_kids);

  const uint32_t kids[] = {kHeading, kCheck, kDisabled, kSlider, kImgUrl, kImgBytes, kImgFile,
                           kImgBad,  kImgTint, kHidden, kLow,  kHigh,   kClip};
  w.set_children(kColumn, kids);
  w.set(kColumn, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_ALIGN_ITEMS, NVE_ALIGN_ITEMS_FLEX_START);
  const uint32_t root_kids[] = {kColumn};
  w.set_children(1u, root_kids);

  NveQtWidget widget(QtHostConfig{.width = 300.0f, .height = 600.0f});
  QtHost& host = widget.host();
  NVE_CHECK(host.attach_interface(&ScriptedApp::get_interface));
  NVE_CHECK(widget.viewCount() == 15u);
  widget.show();

  // --- Accessibility through QAccessible.
  const auto iface = [&](uint32_t id) { return QAccessible::queryAccessibleInterface(widget.nativeWidgetForNodeId(id)); };
  QAccessibleInterface* heading = iface(kHeading);
  NVE_CHECK(heading->role() == QAccessible::Heading && heading->text(QAccessible::Name) == heading->text(QAccessible::Name));
  NVE_CHECK(heading->text(QAccessible::Name).startsWith("Settings"));
  auto* attrs = static_cast<QAccessibleAttributesInterface*>(heading->interface_cast(QAccessible::AttributesInterface));
  NVE_CHECK(attrs != nullptr && attrs->attributeValue(QAccessible::Attribute::Level).toInt() == 2);
  QAccessibleInterface* check = iface(kCheck);
  NVE_CHECK(check->role() == QAccessible::CheckBox && check->text(QAccessible::Name) == "Agree");
  NVE_CHECK(check->text(QAccessible::Description) == "Accept the terms");
  NVE_CHECK(check->state().checkable && check->state().checked && !check->state().disabled);
  QAccessibleActionInterface* act = check->actionInterface();
  NVE_CHECK(act != nullptr && act->actionNames() == QStringList{QAccessibleActionInterface::pressAction()});
  act->doAction(QAccessibleActionInterface::pressAction());
  NVE_CHECK(app.count(NVE_INTERACTION_EVENT_PRESS, kCheck) == 1u);  // the accessible press reaches the app
  QAccessibleInterface* disabled = iface(kDisabled);
  NVE_CHECK(disabled->state().disabled && disabled->actionInterface()->actionNames().isEmpty());
  NVE_CHECK(!widget.nativeWidgetForNodeId(kDisabled)->isEnabled());
  click(widget.nativeWidgetForNodeId(kDisabled), QPointF(2, 2));
  NVE_CHECK(app.count(NVE_INTERACTION_EVENT_PRESS, kDisabled) == 0u);
  QAccessibleInterface* slider = iface(kSlider);
  NVE_CHECK(slider->role() == QAccessible::Slider && slider->valueInterface() != nullptr);
  NVE_CHECK(slider->valueInterface()->currentValue().toDouble() == 30.0);
  NVE_CHECK(slider->valueInterface()->maximumValue().toDouble() == 100.0);
  NVE_CHECK(iface(kImgUrl)->role() == QAccessible::Graphic && iface(kImgUrl)->text(QAccessible::Name) == "Logo");
  NVE_CHECK(widget.accessibilityInfoForNodeId(kHidden)->hidden && iface(kHidden)->state().invisible);
  NVE_CHECK(!widget.nativeWidgetForNodeId(kHidden)->isVisible());
  NVE_CHECK(!widget.accessibilityInfoForNodeId(999u));

  // Semantics changes post accessibility events.
  QAccessible::installUpdateHandler(&record_update);
  QAccessible::installRootObjectHandler(&record_root);
  QAccessible::setActive(true);
  g_events.clear();
  (void)w.set_str(kCheck, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_LABEL, "Agreed");
  w.set(kCheck, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_CHECKED, NVE_ARIA_TRI_STATE_FALSE);
  flush(host);
  QAccessible::installUpdateHandler(nullptr);
  QAccessible::installRootObjectHandler(nullptr);
  NVE_CHECK(std::count(g_events.begin(), g_events.end(), QAccessible::NameChanged) == 1);
  NVE_CHECK(std::count(g_events.begin(), g_events.end(), QAccessible::StateChanged) == 1);
  NVE_CHECK(iface(kCheck)->text(QAccessible::Name) == "Agreed" && !iface(kCheck)->state().checked);

  // --- Typography: inherited italic / family / underline / color; heading clamps to one line.
  auto* head = dynamic_cast<NveQtNodeWidget*>(widget.nativeWidgetForNodeId(kHeading));
  NVE_CHECK(head->text_style().font.italic() && head->text_style().font.underline());
  NVE_CHECK(head->text_style().font.family() == "Courier" && head->text_style().color == QColor(200, 0, 0));
  NVE_CHECK(head->text_style().max_lines == 1 && head->text_style().align == Qt::AlignHCenter);

  // --- Visuals.
  auto* chk = dynamic_cast<NveQtNodeWidget*>(widget.nativeWidgetForNodeId(kCheck));
  NVE_CHECK(chk->visual().border_width == 3.0 && chk->visual().border_color == QColor(0, 255, 0));
  NVE_CHECK(chk->visual().has_outer_shadow() && chk->visual().has_inset_shadow());
  auto* effect = qobject_cast<QGraphicsOpacityEffect*>(chk->graphicsEffect());
  NVE_CHECK(effect != nullptr && approx(effect->opacity(), 0.5, 0.001));
  // The outer shadow is a sibling right below the node, covering offset + spread + blur.
  NveQtShadowWidget* shadow = nullptr;
  for (QObject* o : chk->parentWidget()->children()) {
    if (auto* s = dynamic_cast<NveQtShadowWidget*>(o)) shadow = s;
  }
  NVE_CHECK(shadow != nullptr && shadow->testAttribute(Qt::WA_TransparentForMouseEvents));
  NVE_CHECK(shadow->geometry().contains(chk->geometry().translated(4, 6).adjusted(-1, -1, 1, 1)));
  {
    const QObjectList& siblings = chk->parentWidget()->children();
    NVE_CHECK(siblings.indexOf(shadow) + 1 == siblings.indexOf(chk));
  }
  // Pixels: border ring, inset shadow inside the border, fill in the middle (at half opacity).
  // (Rendered through the parent: a widget's own graphics effect applies when its parent paints it.)
  const QImage col = render(chk->parentWidget());
  const auto at = [&](int x, int y) { return col.pixelColor(chk->geometry().topLeft() + QPoint(x, y)); };
  NVE_CHECK(same_rgb(at(30, 1), 0, 255, 0, 10) && at(30, 1).alpha() < 200);  // half-opaque border
  NVE_CHECK(at(30, 15).blue() > 100 && at(30, 15).red() < 30);               // fill
  NVE_CHECK(at(30, 3).red() > 50);  // yellow inset glow just inside the border
  // Removing the shadows removes the sibling.
  w.set(kCheck, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BOX_SHADOWS, 0u);
  w.set_f32(kCheck, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_OPACITY, 1.0f);
  flush(host);
  NVE_CHECK(chk->graphicsEffect() == nullptr);
  int shadows_left = 0;
  for (QObject* o : chk->parentWidget()->children()) shadows_left += dynamic_cast<NveQtShadowWidget*>(o) ? 1 : 0;
  NVE_CHECK(shadows_left == 0);

  // z-index: the earlier, higher-z sibling is stacked above the later one.
  QWidget* low = widget.nativeWidgetForNodeId(kLow);
  QWidget* high = widget.nativeWidgetForNodeId(kHigh);
  NVE_CHECK(low->geometry() == high->geometry());
  const QImage column_img = render(widget.nativeWidgetForNodeId(kColumn));
  NVE_CHECK(same_rgb(column_img.pixelColor(low->geometry().center()), 255, 0, 255));
  w.set(kLow, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_Z_INDEX, 0u);
  flush(host);
  NVE_CHECK(same_rgb(render(widget.nativeWidgetForNodeId(kColumn)).pixelColor(low->geometry().center()), 0, 255, 255));
  // Rounded clip: the child is cut at the corner, intact at the centre.
  const QImage clip_img = render(widget.nativeWidgetForNodeId(kClip));
  NVE_CHECK(same_rgb(clip_img.pixelColor(20, 20), 255, 128, 0) && clip_img.pixelColor(1, 1).alpha() == 0);

  // --- Images: decode, fit, tint, cache.
  const auto img_of = [&](uint32_t id) { return dynamic_cast<NveQtNodeWidget*>(widget.nativeWidgetForNodeId(id)); };
  NVE_CHECK(img_of(kImgUrl)->content() == NodeContent::Image && img_of(kImgUrl)->image().width() == 2);
  NVE_CHECK(img_of(kImgUrl)->fit() == NVE_IMAGE_FIT_CONTAIN && img_of(kImgBytes)->fit() == NVE_IMAGE_FIT_COVER);
  NVE_CHECK(img_of(kImgBytes)->image().width() == 4 && img_of(kImgFile)->image().height() == 3);
  NVE_CHECK(img_of(kImgBad)->image().isNull());
  NVE_CHECK(same_rgb(img_of(kImgTint)->image().pixelColor(1, 1), 0, 0, 255));
  // Contain: 2x2 into 32x16 is centred 16x16; cover fills the box.
  const QImage url_px = render(img_of(kImgUrl));
  NVE_CHECK(url_px.pixelColor(2, 8).alpha() == 0 && same_rgb(url_px.pixelColor(16, 8), 0, 255, 0));
  NVE_CHECK(same_rgb(render(img_of(kImgBytes)).pixelColor(1, 1), 255, 0, 0));
  // url, red bytes, file, bad (not cached) and the tint (same red bytes: a hit).
  NVE_CHECK(host.images().size() == 3u && host.images().hits() == 1u && host.images().misses() == 4u);
  // The same bytes under another resource id hit the cache; unchanged images are not re-resolved.
  w.define_resource(65u, h::kImageSrcKind, h::encode_image_bytes(red));
  w.set(kImgBad, NVE_TABLE_IMAGE_DESC, NVE_TABLE_IMAGE_SRC, 65u);
  flush(host);
  NVE_CHECK(img_of(kImgBad)->image().cacheKey() == img_of(kImgBytes)->image().cacheKey());
  NVE_CHECK(host.images().hits() == 2u && host.images().misses() == 4u);

  // --- Removal drops the widget.
  w.remove(kImgFile);
  const uint32_t fewer[] = {kHeading, kCheck, kDisabled, kSlider, kImgUrl, kImgBytes, kImgBad, kImgTint,
                            kHidden,  kLow,   kHigh,     kClip};
  w.set_children(kColumn, fewer);
  flush(host);
  NVE_CHECK(widget.nativeWidgetForNodeId(kImgFile) == nullptr && widget.viewCount() == 14u);
  widget.hide();
}

/// Local image and link policy (`host::local_image_path` / `openable_link`): a readable local file
/// decodes by path or `file:` URL (percent-encoded too); remote, UNC, and percent-encoded UNC
/// `file:` URLs never reach the filesystem; links open only for http(s) / mailto.
void test_url_policy() {
  const nve::host::testing::ScratchDir scratch("nve-qt-url");
  const std::string tmp = scratch.file("a b.png");
  const std::vector<uint8_t> red = png(2, 2, QColor(255, 0, 0));
  {
    std::ofstream out(tmp, std::ios::binary);
    out.write(reinterpret_cast<const char*>(red.data()), static_cast<std::streamsize>(red.size()));
  }
  const auto decodes = [](const std::string& url) { return !decode_image(h::ImageSource::from_url(url)).isNull(); };
  std::string encoded = tmp;
  encoded.replace(encoded.find(' '), 1u, "%20");
  NVE_CHECK(decodes(tmp) && decodes("file://" + encoded) && decodes("FILE://localhost" + encoded));
  for (const char* url : {"file://host/share/a.png", "file:////host/share/a.png", "file:///%5c%5chost/share/a.png",
                          "file:///%2F%2Fhost/share/a.png", "//host/share/a.png", "\\\\host\\share\\a.png",
                          "https://example.com/a.png", "file:///tmp/%00a.png"}) {
    NVE_CHECK(!decodes(url));
  }
  NveQtWidget widget(QtHostConfig{.width = 100.0f, .height = 100.0f});
  for (const char* href : {"file:///etc/passwd", "javascript:alert(1)", "/etc/passwd", "nve-custom://x", ""}) {
    NVE_CHECK(!widget.host().activate(1u, false, href, 0.0f, 0.0f));
  }
}

}  // namespace

void nve_test_qt_a11y_and_media() {
  test_a11y_images_and_visuals();
  test_url_policy();
}
