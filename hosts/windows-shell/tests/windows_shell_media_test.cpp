// Windows host model (portable): accessibility semantics, inherited typography, text inputs
// (controlled echo, submit, focus, secure / multiline), images (URL, inline bytes, invalid, fit
// placement), box shadows, visibility, z-order, hit testing, cursors, hover, and links.
#include "windows_test_helpers.hpp"

#include <nve/host/core_resources.hpp>

#include <algorithm>
#include <vector>

namespace {

using namespace nve::win;
using namespace nve::win::test;
namespace h = nve::host;

constexpr uint32_t kColumn = 2u, kHeading = 3u, kEntry = 4u, kSecret = 5u, kNotes = 6u, kImgUrl = 7u, kImgBytes = 8u,
                   kImgBad = 9u, kHidden = 10u, kShown = 11u, kLink = 12u, kLinkText = 13u, kGrab = 14u, kHover = 15u,
                   kCard = 16u, kBelow = 17u, kAbove = 18u, kDisabled = 19u;

void a11y(h::Nve2Writer& w, uint32_t id, uint32_t role, std::string_view label) {
  w.set(id, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_ROLE, role);
  if (!label.empty()) (void)w.set_str(id, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_LABEL, label);
}

void image(h::Nve2Writer& w, uint32_t id, uint32_t res, std::span<const uint8_t> src, uint32_t fit) {
  w.create(id);
  w.define_resource(res, h::kImageSrcKind, src);
  w.set(id, NVE_TABLE_IMAGE_DESC, NVE_TABLE_IMAGE_SRC, res);
  w.set(id, NVE_TABLE_IMAGE_DESC, NVE_TABLE_IMAGE_FIT, fit);
  sized(w, id, 32.0f, 16.0f);
}

void input(h::Nve2Writer& w, uint32_t id, std::string_view value) {
  w.create(id);
  (void)w.set_str(id, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, value);
  events(w, id, {NVE_EVENT_TEXT_CHANGE, NVE_EVENT_SUBMIT, NVE_EVENT_FOCUS, NVE_EVENT_BLUR});
  sized(w, id, 160.0f, 24.0f);
}

void absolute(h::Nve2Writer& w, uint32_t id, float left, float top, float width, float height, int32_t z) {
  w.create(id);
  w.set(id, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_POSITION, NVE_POSITION_ABSOLUTE);
  w.set(id, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_INSET_LEFT, h::length_points(left));
  w.set(id, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_INSET_TOP, h::length_points(top));
  sized(w, id, width, height);
  w.set(id, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_Z_INDEX, static_cast<uint32_t>(z));
}

void test_pure_helpers() {
  // Image placement for every `NVE_IMAGE_FIT_*` (100x50 box, 20x20 image).
  const Rect box{10.0f, 10.0f, 100.0f, 50.0f};
  const auto fill = place_image(NVE_IMAGE_FIT_FILL, box, 20.0f, 20.0f);
  NVE_CHECK(fill && fill->dst == box && (fill->src == Rect{0.0f, 0.0f, 20.0f, 20.0f}));
  const auto contain = place_image(NVE_IMAGE_FIT_CONTAIN, box, 20.0f, 20.0f);
  NVE_CHECK(contain && (contain->dst == Rect{35.0f, 10.0f, 50.0f, 50.0f}));
  const auto cover = place_image(NVE_IMAGE_FIT_COVER, box, 20.0f, 20.0f);
  NVE_CHECK(cover && cover->dst == box && (cover->src == Rect{0.0f, 5.0f, 20.0f, 10.0f}));
  const auto none = place_image(NVE_IMAGE_FIT_NONE, box, 20.0f, 20.0f);
  NVE_CHECK(none && (none->dst == Rect{50.0f, 25.0f, 20.0f, 20.0f}) && (none->src == Rect{0.0f, 0.0f, 20.0f, 20.0f}));
  const auto down_small = place_image(NVE_IMAGE_FIT_SCALE_DOWN, box, 20.0f, 20.0f);
  NVE_CHECK(down_small && down_small->dst == none->dst);
  const auto down_big = place_image(NVE_IMAGE_FIT_SCALE_DOWN, box, 400.0f, 400.0f);
  NVE_CHECK(down_big && (down_big->dst == Rect{35.0f, 10.0f, 50.0f, 50.0f}));
  const auto none_big = place_image(NVE_IMAGE_FIT_NONE, box, 400.0f, 400.0f);  // centred crop
  NVE_CHECK(none_big && none_big->dst == box && (none_big->src == Rect{150.0f, 175.0f, 100.0f, 50.0f}));
  NVE_CHECK(!place_image(NVE_IMAGE_FIT_COVER, Rect{}, 20.0f, 20.0f));
  NVE_CHECK(!place_image(NVE_IMAGE_FIT_COVER, box, 0.0f, 20.0f) && !place_image(NVE_IMAGE_FIT_COVER, box, NAN, 1.0f));

  // Shadow passes: a hard shadow is one offset pass; a blurred one ramps outwards, the
  // composited alphas (1/n .. 1) reach full colour at the innermost pass; inset / clear: none.
  const Rect card{0.0f, 0.0f, 40.0f, 20.0f};
  const auto hard = outer_shadow_passes(h::BoxShadow{2.0f, 3.0f, 0.0f, 1.0f, h::rgba(0, 0, 0, 128), false}, card, 4.0f);
  NVE_CHECK(hard.size() == 1u && (hard[0].rect == Rect{1.0f, 2.0f, 42.0f, 22.0f}) && hard[0].radius == 5.0f);
  const auto soft = outer_shadow_passes(h::BoxShadow{0.0f, 4.0f, 8.0f, 0.0f, h::rgba(0, 0, 0, 255), false}, card, 0.0f);
  NVE_CHECK(soft.size() == 4u && soft.front().rect.width > soft.back().rect.width);
  NVE_CHECK(approx(soft.front().rect.x, -6.0, 0.01) && approx(soft.back().alpha, 1.0, 1e-6) &&
            approx(soft.front().alpha, 0.25, 1e-6));
  NVE_CHECK(outer_shadow_passes(h::BoxShadow{0, 0, 4, 0, h::rgba(0, 0, 0), true}, card, 0.0f).empty());
  NVE_CHECK(outer_shadow_passes(h::BoxShadow{0, 0, 4, 0, h::rgba(0, 0, 0, 0), false}, card, 0.0f).empty());
}

void test_scene_semantics_inputs_and_pointer() {
  const std::vector<uint8_t> png_bytes = {0x89, 'P', 'N', 'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3};
  const std::vector<uint8_t> url = h::encode_image_url("data:image/png;base64,iVBORw0KGgo=");
  const std::vector<uint8_t> bytes = h::encode_image_bytes(png_bytes);
  const std::vector<uint8_t> empty_url = h::encode_image_url("");  // decodes, but names nothing
  const h::BoxShadow shadows[] = {{0.0f, 2.0f, 6.0f, 0.0f, h::rgba(0, 0, 0, 64), false},
                                  {0.0f, 0.0f, 0.0f, 1.0f, h::rgba(255, 0, 0), true}};

  ScriptedApp app;
  h::Nve2Writer& w = app.w;
  w.create(1u);
  w.create(kColumn);
  w.set(kColumn, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_SET,
        NVE_TYPOGRAPHY_SET_FONT_STYLE | NVE_TYPOGRAPHY_SET_FONT_FAMILY | NVE_TYPOGRAPHY_SET_TEXT_DECORATION |
            NVE_TYPOGRAPHY_SET_COLOR | NVE_TYPOGRAPHY_SET_FONT_WEIGHT);
  w.set(kColumn, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_FONT_STYLE, NVE_FONT_STYLE_ITALIC);
  (void)w.set_str(kColumn, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_FONT_FAMILY, "Inter");
  w.set(kColumn, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_TEXT_DECORATION, NVE_TEXT_DECORATION_UNDERLINE);
  w.set(kColumn, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_COLOR, h::rgba(10, 20, 30));
  w.set(kColumn, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_FONT_WEIGHT, 700u);

  text_node(w, kHeading, "Settings");
  a11y(w, kHeading, NVE_ARIA_ROLE_HEADING, "");
  w.set(kHeading, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_LEVEL, 2u);
  input(w, kEntry, "hi");
  (void)w.set_str(kEntry, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_PLACEHOLDER, "Name");
  w.set(kEntry, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_MAX_LENGTH, 8u);
  input(w, kSecret, "");
  w.set(kSecret, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_INPUT_KIND, NVE_INPUT_KIND_PASSWORD);
  input(w, kNotes, "line");
  w.set_bool(kNotes, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_MULTILINE, true);
  w.set_bool(kNotes, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_READ_ONLY, true);
  image(w, kImgUrl, 40u, url, NVE_IMAGE_FIT_CONTAIN);
  (void)w.set_str(kImgUrl, NVE_TABLE_IMAGE_DESC, NVE_TABLE_IMAGE_ALT, "Logo");
  image(w, kImgBytes, 41u, bytes, NVE_IMAGE_FIT_COVER);
  image(w, kImgBad, 42u, empty_url, NVE_IMAGE_FIT_COVER);
  w.create(kHidden);  // hidden parent, explicitly visible child
  sized(w, kHidden, 40.0f, 10.0f);
  w.set(kHidden, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_VISIBILITY, NVE_VISIBILITY_HIDDEN);
  text_node(w, kShown, "shown");
  w.set(kShown, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_VISIBILITY, NVE_VISIBILITY_VISIBLE);
  const uint32_t hidden_kids[] = {kShown};
  w.set_children(kHidden, hidden_kids);
  w.create(kLink);
  (void)w.set_str(kLink, NVE_TABLE_LINK_DESC, NVE_TABLE_LINK_HREF, "https://example.com/docs");
  sized(w, kLink, 80.0f, 20.0f);
  text_node(w, kLinkText, "Docs");
  const uint32_t link_kids[] = {kLinkText};
  w.set_children(kLink, link_kids);
  w.create(kGrab);
  sized(w, kGrab, 50.0f, 20.0f);
  w.set(kGrab, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_CURSOR, NVE_CURSOR_KIND_GRAB);
  w.create(kHover);
  sized(w, kHover, 50.0f, 20.0f);
  events(w, kHover, {NVE_EVENT_POINTER_ENTER, NVE_EVENT_POINTER_LEAVE, NVE_EVENT_POINTER_MOVE});
  w.create(kCard);
  sized(w, kCard, 60.0f, 30.0f);
  w.define_resource(43u, h::kBoxShadowsKind, h::encode_box_shadows(shadows));
  w.set(kCard, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BOX_SHADOWS, 43u);
  w.set_f32(kCard, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BORDER_RADIUS, 6.0f);
  w.set_f32(kCard, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BORDER_WIDTH, 2.0f);
  w.set(kCard, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BORDER_COLOR, h::rgba(1, 2, 3));
  w.set_f32(kCard, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_OPACITY, 0.5f);
  w.create(kDisabled);
  sized(w, kDisabled, 50.0f, 20.0f);
  events(w, kDisabled, {NVE_EVENT_PRESS});
  w.set_bool(kDisabled, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_DISABLED, true);
  const uint32_t column_kids[] = {kHeading, kEntry, kSecret, kNotes, kImgUrl, kImgBytes, kImgBad,
                                  kHidden, kLink, kGrab, kHover, kCard, kDisabled};
  w.set_children(kColumn, column_kids);
  // Overlapping absolute siblings: the earlier one has the higher z-index, so it paints on top.
  absolute(w, kAbove, 300.0f, 10.0f, 50.0f, 50.0f, 5);
  events(w, kAbove, {NVE_EVENT_PRESS});
  absolute(w, kBelow, 320.0f, 20.0f, 50.0f, 50.0f, 0);
  events(w, kBelow, {NVE_EVENT_PRESS});
  const uint32_t root_kids[] = {kColumn, kAbove, kBelow};
  w.set_children(1u, root_kids);
  // Controlled input: the app upper-cases what it receives.
  app.on_event = [](ScriptedApp& a, const h::testing::RecordedEvent& e) {
    if (e.ev.kind != NVE_EVENT_TEXT_CHANGE || e.ev.node_id != kEntry) return;
    std::string upper = e.text;
    for (char& c : upper) c = static_cast<char>(c >= 'a' && c <= 'z' ? c - 32 : c);
    (void)a.w.set_str(kEntry, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, upper);
  };

  NveWinHost host(WinHostConfig{.width = 480.0f, .height = 900.0f});
  NVE_CHECK(host.attach_interface(&ScriptedApp::get_interface));
  const NveWinVisualTree& tree = host.visual_tree();

  // Semantics.
  const WinNodeVisual& heading = node(host, kHeading);
  NVE_CHECK(heading.semantics.role == NVE_ARIA_ROLE_HEADING && heading.semantics.level == 2u &&
            heading.semantics.label == "Settings");
  NVE_CHECK(node(host, kEntry).semantics.role == NVE_ARIA_ROLE_TEXT_BOX);
  NVE_CHECK(node(host, kImgUrl).semantics.role == NVE_ARIA_ROLE_IMAGE && node(host, kImgUrl).semantics.label == "Logo");
  NVE_CHECK(node(host, kLink).kind == h::NodeKind::Link && node(host, kLink).semantics.role == NVE_ARIA_ROLE_LINK);
  NVE_CHECK(node(host, kDisabled).semantics.disabled && !node(host, kDisabled).wants(NVE_EVENT_PRESS));

  // Inherited typography reaches every text-bearing descendant; explicit values win.
  for (const uint32_t id : {kHeading, kLinkText, kEntry}) {
    const TextPaint& t = node(host, id).text_style;
    NVE_CHECK(t.font_style == NVE_FONT_STYLE_ITALIC && t.font_family == "Inter" && t.font_weight == 700u);
    NVE_CHECK(t.text_decoration == NVE_TEXT_DECORATION_UNDERLINE && t.color == h::rgba(10, 20, 30));
  }

  // Text inputs: owned rows, secure / multiline / read-only, the live-input list.
  NVE_CHECK(tree.text_inputs().size() == 3u);
  const WinNodeVisual& entry = node(host, kEntry);
  NVE_CHECK(entry.kind == h::NodeKind::TextInput && entry.input.value == "hi" && entry.input.placeholder == "Name" &&
            entry.input.max_length == 8u && !entry.input.secure);
  NVE_CHECK(node(host, kSecret).input.secure && node(host, kNotes).input.multiline && node(host, kNotes).input.read_only);
  NVE_CHECK(host.dispatch_text_change(kEntry, "hey"));
  NVE_CHECK(app.last(NVE_EVENT_TEXT_CHANGE)->text == "hey" && node(host, kEntry).input.value == "HEY");
  NVE_CHECK(!host.dispatch_submit(kEntry, "HEY") && app.last(NVE_EVENT_SUBMIT)->text == "HEY");
  (void)host.dispatch_focus(kEntry, true);
  (void)host.dispatch_focus(kEntry, false);
  NVE_CHECK(app.count(NVE_EVENT_FOCUS, kEntry) == 1u && app.count(NVE_EVENT_BLUR, kEntry) == 1u);

  // Images: URL, inline bytes (content-hash key), an empty source -> nothing to draw.
  NVE_CHECK(node(host, kImgUrl).image.url == "data:image/png;base64,iVBORw0KGgo=" &&
            node(host, kImgUrl).image.key == node(host, kImgUrl).image.url && node(host, kImgUrl).image_fit == NVE_IMAGE_FIT_CONTAIN);
  NVE_CHECK(node(host, kImgBytes).image.encoded == png_bytes && node(host, kImgBytes).image.key.rfind("bytes:", 0) == 0u);
  NVE_CHECK(node(host, kImgBad).image.empty());

  // Box shadows decoded, visual style.
  const WinNodeVisual& card = node(host, kCard);
  NVE_CHECK(card.shadows.size() == 2u && card.shadows[0] == shadows[0] && card.shadows[1].inset);
  NVE_CHECK(card.visual.border_width == 2.0f && card.visual.border_color == h::rgba(1, 2, 3));
  NVE_CHECK(approx(tree.frame_of(kCard)->opacity, 0.5, 1e-6));

  // Visibility: a hidden parent is not painted / hit, its visible child is.
  NVE_CHECK(!tree.frame_of(kHidden)->visible && tree.frame_of(kShown)->visible);
  const auto [hx, hy] = center(host, kHidden);
  const WinNodeVisual* shown_hit = tree.hit_test(hx, hy);
  NVE_CHECK(shown_hit == nullptr || shown_hit->id != kHidden);

  // Z-order: paint order follows z-index; the topmost overlapping node takes the press.
  const std::vector<uint32_t> order = paint_order(host);
  const auto pos = [&order](uint32_t id) { return std::find(order.begin(), order.end(), id) - order.begin(); };
  NVE_CHECK(pos(kBelow) < pos(kAbove) && pos(kColumn) < pos(kBelow));
  NVE_CHECK(host.press_at(330.0f, 30.0f).node_id == kAbove);
  NVE_CHECK(host.press_at(360.0f, 65.0f).node_id == kBelow);
  NVE_CHECK(app.count(NVE_EVENT_PRESS, kAbove) == 1u && app.count(NVE_EVENT_PRESS, kBelow) == 1u);
  const auto* press = app.last(NVE_EVENT_PRESS);
  NVE_CHECK(approx(press->ev.x, 40.0, 0.5) && approx(press->ev.y, 45.0, 0.5));  // node-local

  // Disabled nodes take no press; links report their href (the window opens it natively).
  const auto [dx, dy] = center(host, kDisabled);
  NVE_CHECK(host.press_at(dx, dy).node_id == 0u);
  const auto [lx, ly] = center(host, kLinkText);
  const PressResult link = host.press_at(lx, ly);
  NVE_CHECK(link.node_id == kLink && !link.dispatched && link.href == "https://example.com/docs");

  // Cursors: explicit kind, POINTER over links, AUTO elsewhere.
  const auto [gx, gy] = center(host, kGrab);
  NVE_CHECK(host.cursor_at(gx, gy) == NVE_CURSOR_KIND_GRAB);
  NVE_CHECK(host.cursor_at(lx, ly) == NVE_CURSOR_KIND_POINTER);
  NVE_CHECK(host.cursor_at(470.0f, 890.0f) == NVE_CURSOR_KIND_AUTO);

  // Hover: enter once, move while inside, leave on exit / window leave.
  const auto [vx, vy] = center(host, kHover);
  host.pointer_move(vx, vy);
  host.pointer_move(vx + 1.0f, vy);
  NVE_CHECK(host.hovered_node() == kHover && app.count(NVE_EVENT_POINTER_ENTER, kHover) == 1u);
  NVE_CHECK(app.count(NVE_EVENT_POINTER_MOVE, kHover) == 2u && approx(app.last(NVE_EVENT_POINTER_MOVE)->ev.x, 26.0, 0.5));
  host.pointer_move(gx, gy);
  NVE_CHECK(host.hovered_node() == 0u && app.count(NVE_EVENT_POINTER_LEAVE, kHover) == 1u);
  host.pointer_move(vx, vy);
  host.pointer_leave();
  NVE_CHECK(app.count(NVE_EVENT_POINTER_ENTER, kHover) == 2u && app.count(NVE_EVENT_POINTER_LEAVE, kHover) == 2u);

  // Removal drops the view and its text-input registration.
  w.remove(kSecret);
  const uint32_t fewer[] = {kHeading, kEntry, kNotes, kImgUrl, kImgBytes, kImgBad, kHidden, kLink, kGrab, kHover, kCard, kDisabled};
  w.set_children(kColumn, fewer);
  flush(host);
  NVE_CHECK(tree.find(kSecret) == nullptr && tree.text_inputs().size() == 2u);
  // A kind change (view -> text input) recreates the view.
  (void)w.set_str(kGrab, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, "now an input");
  flush(host);
  NVE_CHECK(node(host, kGrab).kind == h::NodeKind::TextInput && tree.text_inputs().size() == 3u);
}

}  // namespace

void nve_test_win_media_and_input() {
  test_pure_helpers();
  test_scene_semantics_inputs_and_pointer();
}
