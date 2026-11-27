// NVE host runtime: decoded per-node view of a runtime scene for retained-view hosts.
//
// `SceneTables` binds every core table a native view host renders (generated `NveTableDesc`s,
// no hand-written layouts). `read_node` decodes one node into `NodeData`: kind (from
// `NveNodeRow::primary_slot`), absolute and parent-relative rects, visual style, inherited
// typography, text, image, text input, scroll, interaction, accessibility, and link fields.
// String views borrow the scene and are valid only while its `RuntimeSceneGuard` is held.
#pragma once

#include <cmath>
#include <cstdint>
#include <span>
#include <string_view>

#include "bound_table.hpp"
#include "../generated/nve_abi.generated.h"
#include "../generated/nve_tables.generated.h"
#include "runtime_session.hpp"

namespace nve::host {

/// Realized view kind: the node's `primary_slot` table (`RealizationOrder`), else `View`.
enum class NodeKind : uint8_t { View, Text, Image, TextInput, Scroll, Link };

/// Subscription bit of `kind` in `nve.interaction.event_mask` (`EventMask` = `1 << (kind - 1)`).
[[nodiscard]] constexpr uint32_t event_bit(NveEventKind kind) noexcept {
  return uint32_t{1} << (static_cast<uint32_t>(kind) - 1u);
}

/// Fully resolved (inherited) text style.
struct TextStyle {
  uint32_t color = 0xFF000000u;
  float font_size = 16.0f;
  LengthValue line_height{NVE_LENGTH_AUTO, 0.0f};
  float letter_spacing = 0.0f;
  std::string_view font_family;
  uint32_t max_lines = 0u;
  uint16_t font_weight = 400u;
  uint8_t font_style = NVE_FONT_STYLE_NORMAL;
  uint8_t white_space = NVE_WHITE_SPACE_NORMAL;
  uint8_t text_align = NVE_TEXT_ALIGN_START;
  uint8_t text_decoration = 0u;
  uint8_t vertical_align = NVE_VERTICAL_ALIGN_BASELINE;

  friend bool operator==(const TextStyle&, const TextStyle&) noexcept = default;
};

struct VisualStyle {
  uint32_t background = 0u;
  uint32_t border_color = 0u;
  float border_width = 0.0f;
  float border_radius = 0.0f;
  float opacity = 1.0f;
  int32_t z_index = 0;
  uint8_t overflow_x = NVE_OVERFLOW_VISIBLE;
  uint8_t overflow_y = NVE_OVERFLOW_VISIBLE;
  uint8_t visibility = NVE_VISIBILITY_INHERIT;
  uint32_t box_shadows = 0u;     // `ResourceId` (`nve.res.box_shadows`), 0 = none
  uint32_t background_fill = 0u; // `ResourceId` (gradient fills), 0 = none
};

struct ImageData {
  uint32_t src = 0u;  // `ResourceId` (`nve.res.image_src`)
  std::string_view alt;
  uint8_t fit = NVE_IMAGE_FIT_COVER;
  bool tint_enabled = false;
  uint32_t tint = 0u;
  uint16_t intrinsic_width = 0u;
  uint16_t intrinsic_height = 0u;
};

struct TextInputData {
  std::string_view value;
  std::string_view placeholder;
  bool multiline = false;
  bool secure = false;
  bool read_only = false;
  bool required = false;
  uint8_t input_kind = NVE_INPUT_KIND_TEXT;
  uint32_t max_length = 0u;
};

struct ScrollData {
  uint8_t axes = NVE_SCROLL_AXES_VERTICAL;
  uint8_t scrollbars = NVE_SCROLLBARS_AUTO;
  bool paging = false;
  bool bounce = false;
  float requested_x = NAN;  // programmatic target, NaN = unchanged
  float requested_y = NAN;
};

struct InteractionData {
  uint32_t event_mask = 0u;
  uint8_t cursor = 0u;
  bool disabled = false;
  bool focusable = false;
};

struct AccessibilityData {
  bool present = false;
  uint8_t role = NVE_ARIA_ROLE_NONE;
  uint8_t level = 0u;
  bool hidden = false;
  uint8_t checked = NVE_ARIA_TRI_STATE_UNSET;
  uint8_t selected = NVE_ARIA_TRI_STATE_UNSET;
  uint8_t expanded = NVE_ARIA_TRI_STATE_UNSET;
  uint8_t pressed = NVE_ARIA_TRI_STATE_UNSET;
  std::string_view label;
  std::string_view hint;
  std::string_view value_text;
  float value_now = NAN;
  float value_min = NAN;
  float value_max = NAN;
};

/// One decoded node. Views borrow the scene (valid while its guard is held).
struct NodeData {
  uint32_t id = 0u;      // packed `NodeId`
  uint32_t index = 0u;
  uint32_t parent = 0u;  // packed `NodeId`, 0 for the root
  NodeKind kind = NodeKind::View;
  NveLayoutRect rect{};   // absolute (not scroll-shifted)
  NveLayoutRect local{};  // relative to the parent's rect
  std::span<const uint32_t> children;  // packed `NodeId`s
  VisualStyle visual;
  TextStyle text_style;
  std::string_view text;
  ImageData image;
  TextInputData input;
  ScrollData scroll;
  InteractionData interaction;
  AccessibilityData a11y;
  std::string_view href;

  [[nodiscard]] bool is_root() const noexcept { return index == kRootIndex; }
  [[nodiscard]] bool hidden() const noexcept {
    return visual.visibility == NVE_VISIBILITY_HIDDEN || visual.opacity <= 0.001f;
  }
  [[nodiscard]] bool wants(NveEventKind kind) const noexcept {
    return !interaction.disabled && (interaction.event_mask & event_bit(kind)) != 0u;
  }
  [[nodiscard]] bool scrollable() const noexcept {
    return kind == NodeKind::Scroll || visual.overflow_x >= NVE_OVERFLOW_SCROLL ||
           visual.overflow_y >= NVE_OVERFLOW_SCROLL;
  }
};

/// Every core table a retained-view host renders, bound to one scene.
struct SceneTables {
  BoundTable visual, typography, text, image, text_input, scroll, accessibility, interaction, link;

  void bind(const RuntimeSceneReader& scene) {
    visual.bind(scene, NVE_TABLE_VISUAL_DESC);
    typography.bind(scene, NVE_TABLE_TYPOGRAPHY_DESC);
    text.bind(scene, NVE_TABLE_TEXT_DESC);
    image.bind(scene, NVE_TABLE_IMAGE_DESC);
    text_input.bind(scene, NVE_TABLE_TEXT_INPUT_DESC);
    scroll.bind(scene, NVE_TABLE_SCROLL_DESC);
    accessibility.bind(scene, NVE_TABLE_ACCESSIBILITY_DESC);
    interaction.bind(scene, NVE_TABLE_INTERACTION_DESC);
    link.bind(scene, NVE_TABLE_LINK_DESC);
  }

  [[nodiscard]] NodeKind kind_of(uint8_t primary_slot) const noexcept {
    const auto is = [primary_slot](const BoundTable& t) { return t.slot() == std::optional<uint32_t>(primary_slot); };
    if (is(image)) return NodeKind::Image;
    if (is(text_input)) return NodeKind::TextInput;
    if (is(scroll)) return NodeKind::Scroll;
    if (is(link)) return NodeKind::Link;
    if (is(text)) return NodeKind::Text;
    return NodeKind::View;
  }
};

/// `parent` with node `index`'s explicit typography (bits in `nve.typography.set`) applied.
[[nodiscard]] inline TextStyle inherit_text_style(const TextStyle& parent, const SceneTables& t,
                                                  const RuntimeSceneReader& scene, uint32_t index) noexcept {
  if (!t.typography.has(index)) return parent;
  const RowRef r = t.typography.at(index);
  const uint32_t set = r.bits(NVE_TABLE_TYPOGRAPHY_SET);
  TextStyle s = parent;
  if ((set & NVE_TYPOGRAPHY_SET_COLOR) != 0u) s.color = r.bits(NVE_TABLE_TYPOGRAPHY_COLOR);
  if ((set & NVE_TYPOGRAPHY_SET_FONT_SIZE) != 0u) s.font_size = r.f32(NVE_TABLE_TYPOGRAPHY_FONT_SIZE);
  if ((set & NVE_TYPOGRAPHY_SET_LINE_HEIGHT) != 0u) s.line_height = r.length(NVE_TABLE_TYPOGRAPHY_LINE_HEIGHT);
  if ((set & NVE_TYPOGRAPHY_SET_LETTER_SPACING) != 0u) s.letter_spacing = r.f32(NVE_TABLE_TYPOGRAPHY_LETTER_SPACING);
  if ((set & NVE_TYPOGRAPHY_SET_FONT_FAMILY) != 0u) s.font_family = r.str(NVE_TABLE_TYPOGRAPHY_FONT_FAMILY, scene);
  if ((set & NVE_TYPOGRAPHY_SET_MAX_LINES) != 0u) s.max_lines = r.bits(NVE_TABLE_TYPOGRAPHY_MAX_LINES);
  if ((set & NVE_TYPOGRAPHY_SET_FONT_WEIGHT) != 0u) {
    s.font_weight = static_cast<uint16_t>(r.bits(NVE_TABLE_TYPOGRAPHY_FONT_WEIGHT));
  }
  if ((set & NVE_TYPOGRAPHY_SET_FONT_STYLE) != 0u) {
    s.font_style = static_cast<uint8_t>(r.bits(NVE_TABLE_TYPOGRAPHY_FONT_STYLE));
  }
  if ((set & NVE_TYPOGRAPHY_SET_WHITE_SPACE) != 0u) {
    s.white_space = static_cast<uint8_t>(r.bits(NVE_TABLE_TYPOGRAPHY_WHITE_SPACE));
  }
  if ((set & NVE_TYPOGRAPHY_SET_TEXT_ALIGN) != 0u) {
    s.text_align = static_cast<uint8_t>(r.bits(NVE_TABLE_TYPOGRAPHY_TEXT_ALIGN));
  }
  if ((set & NVE_TYPOGRAPHY_SET_TEXT_DECORATION) != 0u) {
    s.text_decoration = static_cast<uint8_t>(r.bits(NVE_TABLE_TYPOGRAPHY_TEXT_DECORATION));
  }
  if ((set & NVE_TYPOGRAPHY_SET_VERTICAL_ALIGN) != 0u) {
    s.vertical_align = static_cast<uint8_t>(r.bits(NVE_TABLE_TYPOGRAPHY_VERTICAL_ALIGN));
  }
  return s;
}

/// Decodes live node `index`. `text_style` is its resolved (inherited) style.
[[nodiscard]] inline NodeData read_node(const RuntimeSceneReader& scene, const SceneTables& t, uint32_t index,
                                        const TextStyle& text_style) noexcept {
  NodeData n;
  const NveNodeRow* row = scene.row(index);
  if (row == nullptr) return n;
  n.index = index;
  n.id = make_node_id(index, row->generation);
  n.parent = index == kRootIndex ? 0u : row->parent;
  n.kind = t.kind_of(row->primary_slot);
  n.rect = *scene.rect(index);
  n.local = n.rect;
  if (const NveLayoutRect* p = n.parent != 0u ? scene.rect(node_index(n.parent)) : nullptr) {
    n.local.x -= p->x;
    n.local.y -= p->y;
  }
  n.children = scene.node_children(index).value_or(std::span<const uint32_t>());
  n.text_style = text_style;

  const RowRef v = t.visual.at(index);
  n.visual = VisualStyle{v.bits(NVE_TABLE_VISUAL_BACKGROUND),
                         v.bits(NVE_TABLE_VISUAL_BORDER_COLOR),
                         v.f32(NVE_TABLE_VISUAL_BORDER_WIDTH),
                         v.f32(NVE_TABLE_VISUAL_BORDER_RADIUS),
                         v.f32(NVE_TABLE_VISUAL_OPACITY),
                         v.i32(NVE_TABLE_VISUAL_Z_INDEX),
                         static_cast<uint8_t>(v.bits(NVE_TABLE_VISUAL_OVERFLOW_X)),
                         static_cast<uint8_t>(v.bits(NVE_TABLE_VISUAL_OVERFLOW_Y)),
                         static_cast<uint8_t>(v.bits(NVE_TABLE_VISUAL_VISIBILITY)),
                         v.bits(NVE_TABLE_VISUAL_BOX_SHADOWS),
                         v.bits(NVE_TABLE_VISUAL_BACKGROUND_FILL)};

  n.text = t.text.at(index).str(NVE_TABLE_TEXT_TEXT, scene);

  if (t.image.has(index)) {
    const RowRef r = t.image.at(index);
    n.image = ImageData{r.bits(NVE_TABLE_IMAGE_SRC),
                        r.str(NVE_TABLE_IMAGE_ALT, scene),
                        static_cast<uint8_t>(r.bits(NVE_TABLE_IMAGE_FIT)),
                        r.boolean(NVE_TABLE_IMAGE_TINT_ENABLED),
                        r.bits(NVE_TABLE_IMAGE_TINT),
                        static_cast<uint16_t>(r.bits(NVE_TABLE_IMAGE_INTRINSIC_WIDTH)),
                        static_cast<uint16_t>(r.bits(NVE_TABLE_IMAGE_INTRINSIC_HEIGHT))};
  }
  if (t.text_input.has(index)) {
    const RowRef r = t.text_input.at(index);
    n.input = TextInputData{r.str(NVE_TABLE_TEXT_INPUT_VALUE, scene),
                            r.str(NVE_TABLE_TEXT_INPUT_PLACEHOLDER, scene),
                            r.boolean(NVE_TABLE_TEXT_INPUT_MULTILINE),
                            r.boolean(NVE_TABLE_TEXT_INPUT_SECURE),
                            r.boolean(NVE_TABLE_TEXT_INPUT_READ_ONLY),
                            r.boolean(NVE_TABLE_TEXT_INPUT_REQUIRED),
                            static_cast<uint8_t>(r.bits(NVE_TABLE_TEXT_INPUT_INPUT_KIND)),
                            r.bits(NVE_TABLE_TEXT_INPUT_MAX_LENGTH)};
  }
  if (t.scroll.has(index)) {
    const RowRef r = t.scroll.at(index);
    n.scroll = ScrollData{static_cast<uint8_t>(r.bits(NVE_TABLE_SCROLL_AXES)),
                          static_cast<uint8_t>(r.bits(NVE_TABLE_SCROLL_SCROLLBARS)),
                          r.boolean(NVE_TABLE_SCROLL_PAGING),
                          r.boolean(NVE_TABLE_SCROLL_BOUNCE),
                          r.f32(NVE_TABLE_SCROLL_REQUESTED_SCROLL_X),
                          r.f32(NVE_TABLE_SCROLL_REQUESTED_SCROLL_Y)};
  }
  if (t.interaction.has(index)) {
    const RowRef r = t.interaction.at(index);
    n.interaction = InteractionData{r.bits(NVE_TABLE_INTERACTION_EVENT_MASK),
                                    static_cast<uint8_t>(r.bits(NVE_TABLE_INTERACTION_CURSOR)),
                                    r.boolean(NVE_TABLE_INTERACTION_DISABLED),
                                    r.boolean(NVE_TABLE_INTERACTION_FOCUSABLE)};
  }
  if (t.accessibility.has(index)) {
    const RowRef r = t.accessibility.at(index);
    n.a11y = AccessibilityData{true,
                               static_cast<uint8_t>(r.bits(NVE_TABLE_ACCESSIBILITY_ROLE)),
                               static_cast<uint8_t>(r.bits(NVE_TABLE_ACCESSIBILITY_LEVEL)),
                               r.boolean(NVE_TABLE_ACCESSIBILITY_HIDDEN),
                               static_cast<uint8_t>(r.bits(NVE_TABLE_ACCESSIBILITY_CHECKED)),
                               static_cast<uint8_t>(r.bits(NVE_TABLE_ACCESSIBILITY_SELECTED)),
                               static_cast<uint8_t>(r.bits(NVE_TABLE_ACCESSIBILITY_EXPANDED)),
                               static_cast<uint8_t>(r.bits(NVE_TABLE_ACCESSIBILITY_PRESSED)),
                               r.str(NVE_TABLE_ACCESSIBILITY_LABEL, scene),
                               r.str(NVE_TABLE_ACCESSIBILITY_HINT, scene),
                               r.str(NVE_TABLE_ACCESSIBILITY_VALUE_TEXT, scene),
                               r.f32(NVE_TABLE_ACCESSIBILITY_VALUE_NOW),
                               r.f32(NVE_TABLE_ACCESSIBILITY_VALUE_MIN),
                               r.f32(NVE_TABLE_ACCESSIBILITY_VALUE_MAX)};
  }
  n.href = t.link.at(index).str(NVE_TABLE_LINK_HREF, scene);
  return n;
}

}  // namespace nve::host
