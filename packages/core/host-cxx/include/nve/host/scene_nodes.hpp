// NVE host runtime: decoded per-node view of a runtime scene for retained-view hosts.
//
// `SceneTables` binds every core table a native view host renders (generated `NveTableDesc`s,
// no hand-written layouts). `read_node` decodes one node into `NodeData`: kind (from
// `NveNodeRow::primary_slot`), absolute and parent-relative rects, visual style, inherited
// typography (via `#[nve_table(inherited)]` word 0 `explicit_bit` checks), text, image, text
// input, scroll, interaction, accessibility, and link fields. String views borrow the scene and
// are valid only while its `RuntimeSceneGuard` is held.
//
// Floats are sealed here, once, for every host: app-written NaN / Inf / negative extents become
// the field's neutral value (sizes 0, opacity 1, inherited typography, "no request" NaN for
// programmatic scroll and absent a11y values), so no native setter ever sees them and `NodeData`
// equality stays reflexive.
#pragma once

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <optional>
#include <span>
#include <string_view>

#include "../generated/nve_abi.generated.h"
#include "../generated/nve_tables.generated.h"
#include "bound_table.hpp"
#include "runtime_session.hpp"

namespace nve::host {

/// Realized view kind: the node's `primary_slot` table (`RealizationOrder`), else `View`. An
/// empty text node with children is a `View`: a Dioxus `text { "Count: " {n} }` publishes an
/// empty wrapper whose runs are child text leaves, so only leaves are realized as text (labels
/// hold no children). A text node with its own text stays `Text`.
enum class NodeKind : uint8_t { View, Text, Image, TextInput, Scroll, Link };

/// Listener bit of one per-table event in its table's `EventSet` word (`1u << event_id`).
[[nodiscard]] constexpr uint32_t event_set_bit(NveInteractionEvent ev) noexcept {
  const auto id = static_cast<uint32_t>(ev);
  return id <= static_cast<uint32_t>(NVE_INTERACTION_EVENT_WHEEL) ? (uint32_t{1} << id) : 0u;
}
[[nodiscard]] constexpr uint32_t event_set_bit(NveTextInputEvent ev) noexcept {
  const auto id = static_cast<uint32_t>(ev);
  return id <= static_cast<uint32_t>(NVE_TEXT_INPUT_EVENT_COMPOSITION_END) ? (uint32_t{1} << id) : 0u;
}
[[nodiscard]] constexpr uint32_t event_set_bit(NveScrollEvent ev) noexcept {
  const auto id = static_cast<uint32_t>(ev);
  return id <= static_cast<uint32_t>(NVE_SCROLL_EVENT_SCROLL) ? (uint32_t{1} << id) : 0u;
}
[[nodiscard]] constexpr uint32_t event_set_bit(NveLayoutEvent ev) noexcept {
  const auto id = static_cast<uint32_t>(ev);
  return id <= static_cast<uint32_t>(NVE_LAYOUT_EVENT_LAYOUT) ? (uint32_t{1} << id) : 0u;
}

/// `v` when finite, else `fallback`.
[[nodiscard]] inline float finite_or(float v, float fallback) noexcept { return std::isfinite(v) ? v : fallback; }
/// A size / thickness: finite and non-negative, else 0.
[[nodiscard]] inline float extent(float v) noexcept { return std::isfinite(v) && v > 0.0f ? v : 0.0f; }
[[nodiscard]] inline NveLayoutRect sealed_rect(NveLayoutRect r) noexcept {
  return NveLayoutRect{finite_or(r.x, 0.0f), finite_or(r.y, 0.0f), extent(r.width), extent(r.height)};
}

/// How a line breaker wraps text at the available width, from CSS `white-space`, `word-break`
/// and `overflow-wrap` (matching the runtime measurer's contract).
enum class TextWrap : uint8_t {
  None,            // `white-space: nowrap | pre`: lines break only at `\n`
  Word,            // at word boundaries; an over-long word overflows (min-content = longest word)
  Anywhere,        // `word-break: break-all`: between any two characters
  WordOrAnywhere,  // `overflow-wrap: anywhere | break-word`, `word-break: break-word`
};

[[nodiscard]] constexpr TextWrap text_wrap(uint32_t white_space, uint32_t word_break, uint32_t overflow_wrap) noexcept {
  if (white_space == NVE_WHITE_SPACE_NO_WRAP || white_space == NVE_WHITE_SPACE_PRE) return TextWrap::None;
  if (word_break == NVE_WORD_BREAK_BREAK_ALL) return TextWrap::Anywhere;
  if (word_break == NVE_WORD_BREAK_BREAK_WORD || overflow_wrap != NVE_OVERFLOW_WRAP_NORMAL) {
    return TextWrap::WordOrAnywhere;
  }
  return TextWrap::Word;
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
  uint8_t word_break = NVE_WORD_BREAK_NORMAL;
  uint8_t overflow_wrap = NVE_OVERFLOW_WRAP_NORMAL;
  uint8_t text_align = NVE_TEXT_ALIGN_START;
  uint8_t text_decoration = 0u;
  uint8_t vertical_align = NVE_VERTICAL_ALIGN_BASELINE;

  [[nodiscard]] constexpr TextWrap wrap() const noexcept { return text_wrap(white_space, word_break, overflow_wrap); }

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
  uint32_t box_shadows = 0u;      // `ResourceId` (`nve.res.box_shadows`), 0 = none
  uint32_t background_fill = 0u;  // `ResourceId` (gradient fills), 0 = none
};

struct ImageData {
  uint32_t src = 0u;  // `ResourceId` (`nve.res.image_src`)
  std::string_view alt;
  uint8_t fit = NVE_IMAGE_FIT_COVER;
  std::optional<uint32_t> tint;  // packed RGBA8888 when present
  uint16_t intrinsic_width = 0u;
  uint16_t intrinsic_height = 0u;
};

struct TextInputData {
  uint32_t events = 0u;  // `NVE_TEXT_INPUT_EVENT_SET_*` bits
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
  uint32_t events = 0u;  // `NVE_SCROLL_EVENT_SET_*` bits
  uint8_t axes = NVE_SCROLL_AXES_VERTICAL;
  uint8_t scrollbars = NVE_SCROLLBARS_AUTO;
  bool paging = false;
  bool bounce = false;
  float requested_x = NAN;  // programmatic target, NaN = unchanged
  float requested_y = NAN;
};

struct InteractionData {
  uint32_t events = 0u;  // `NVE_INTERACTION_EVENT_SET_*` bits
  uint8_t cursor = 0u;
  bool disabled = false;
  bool focusable = false;
  std::optional<int16_t> tab_index;
};

struct AccessibilityData {
  bool present = false;
  std::optional<uint8_t> role;  // `NVE_ARIA_ROLE_*` when set
  uint8_t level = 0u;
  bool hidden = false;
  std::optional<uint8_t> checked;   // `NVE_ARIA_TRI_STATE_*` when set
  std::optional<uint8_t> selected;  // `NVE_ARIA_TRI_STATE_*` when set
  std::optional<uint8_t> expanded;  // `NVE_ARIA_TRI_STATE_*` when set
  std::optional<uint8_t> pressed;   // `NVE_ARIA_TRI_STATE_*` when set
  std::string_view label;
  std::string_view hint;
  std::string_view value_text;
  float value_now = NAN;
  float value_min = NAN;
  float value_max = NAN;
};

/// Identity and geometry of one live node: all a geometry-only `move` needs.
struct NodeGeometry {
  uint32_t id = 0u;      // packed `NodeId`
  uint32_t index = 0u;
  uint32_t parent = 0u;  // packed `NodeId`, 0 for the root
  NodeKind kind = NodeKind::View;
  NveLayoutRect rect{};   // absolute (not scroll-shifted)
  NveLayoutRect local{};  // relative to the parent's rect
  /// Content box (inside border and padding) relative to `rect`'s origin: where a host draws
  /// text, images, inputs and canvases (layout rects are border boxes).
  NveLayoutRect content{};
};

/// Everything a view shows for one live node.
struct NodeData : NodeGeometry {
  std::span<const uint32_t> children;  // packed `NodeId`s
  uint32_t layout_events = 0u;         // `NVE_LAYOUT_EVENT_SET_*` bits
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
  [[nodiscard]] bool wants(NveInteractionEvent ev) const noexcept {
    return !interaction.disabled && (interaction.events & event_set_bit(ev)) != 0u;
  }
  [[nodiscard]] bool wants(NveTextInputEvent ev) const noexcept {
    return !interaction.disabled && (input.events & event_set_bit(ev)) != 0u;
  }
  [[nodiscard]] bool wants(NveScrollEvent ev) const noexcept {
    return !interaction.disabled && (scroll.events & event_set_bit(ev)) != 0u;
  }
  [[nodiscard]] bool wants(NveLayoutEvent ev) const noexcept {
    return !interaction.disabled && (layout_events & event_set_bit(ev)) != 0u;
  }
  [[nodiscard]] bool scrollable() const noexcept {
    return kind == NodeKind::Scroll || visual.overflow_x >= NVE_OVERFLOW_SCROLL ||
           visual.overflow_y >= NVE_OVERFLOW_SCROLL;
  }
};

/// `NVE_ARIA_ROLE_*` of `node`: its explicit `nve.accessibility.role`, else its kind's native role
/// (a view that takes presses is a button), or `std::nullopt` for a plain container.
[[nodiscard]] inline std::optional<uint8_t> aria_role(const NodeData& node) noexcept {
  if (node.a11y.present && node.a11y.role.has_value()) return node.a11y.role;
  switch (node.kind) {
    case NodeKind::Text: return static_cast<uint8_t>(NVE_ARIA_ROLE_TEXT);
    case NodeKind::Image: return static_cast<uint8_t>(NVE_ARIA_ROLE_IMAGE);
    case NodeKind::TextInput:
      return static_cast<uint8_t>(node.input.input_kind == NVE_INPUT_KIND_SEARCH ? NVE_ARIA_ROLE_SEARCH_BOX
                                                                                 : NVE_ARIA_ROLE_TEXT_BOX);
    case NodeKind::Link: return static_cast<uint8_t>(NVE_ARIA_ROLE_LINK);
    case NodeKind::Scroll: return static_cast<uint8_t>(NVE_ARIA_ROLE_SCROLL_AREA);
    case NodeKind::View: break;
  }
  return node.wants(NVE_INTERACTION_EVENT_PRESS) ? std::optional<uint8_t>(NVE_ARIA_ROLE_BUTTON) : std::nullopt;
}

/// Accessible name of `node`: its explicit label, else its text (Text) or alt text (Image).
[[nodiscard]] inline std::string_view accessible_label(const NodeData& node) noexcept {
  if (!node.a11y.label.empty()) return node.a11y.label;
  if (node.kind == NodeKind::Text) return node.text;
  if (node.kind == NodeKind::Image) return node.image.alt;
  return {};
}

/// Every core table a retained-view host renders, bound to one scene.
struct SceneTables {
  BoundTable layout, visual, typography, text, image, text_input, scroll, accessibility, interaction, link;

  void bind(const SceneReader& scene) {
    layout.bind(scene, NVE_TABLE_LAYOUT_DESC);
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

/// `parent` with node `index`'s explicit typography (word 0 `explicit_bit` on `nve.typography`,
/// matching Rust `ResolvedRow<Typography>`) applied.
[[nodiscard]] inline TextStyle inherit_text_style(const TextStyle& parent, const SceneTables& t,
                                                  const SceneReader& scene, uint32_t index) noexcept {
  if (!t.typography.has(index)) return parent;
  const RowRef r = t.typography.at(index);
  TextStyle s = parent;
  if (r.is_explicit(NVE_TABLE_TYPOGRAPHY_COLOR)) s.color = r.bits(NVE_TABLE_TYPOGRAPHY_COLOR);
  // Invalid values keep the inherited one (as the runtime's `resolve_inherited_text`: a `0`
  // size is valid, an explicit `auto` line height resets to `normal`).
  if (r.is_explicit(NVE_TABLE_TYPOGRAPHY_FONT_SIZE)) {
    const float size = r.f32(NVE_TABLE_TYPOGRAPHY_FONT_SIZE);
    if (std::isfinite(size) && size >= 0.0f) s.font_size = size;
  }
  if (r.is_explicit(NVE_TABLE_TYPOGRAPHY_LINE_HEIGHT)) {
    const LengthValue lh = r.length(NVE_TABLE_TYPOGRAPHY_LINE_HEIGHT);
    if (lh.unit == NVE_LENGTH_AUTO) {
      s.line_height = LengthValue{NVE_LENGTH_AUTO, 0.0f};
    } else if (std::isfinite(lh.value) && lh.value >= 0.0f) {
      s.line_height = lh;
    }
  }
  if (r.is_explicit(NVE_TABLE_TYPOGRAPHY_LETTER_SPACING)) {
    s.letter_spacing = finite_or(r.f32(NVE_TABLE_TYPOGRAPHY_LETTER_SPACING), s.letter_spacing);
  }
  if (r.is_explicit(NVE_TABLE_TYPOGRAPHY_FONT_FAMILY)) s.font_family = r.str(NVE_TABLE_TYPOGRAPHY_FONT_FAMILY, scene);
  if (r.is_explicit(NVE_TABLE_TYPOGRAPHY_MAX_LINES)) {
    s.max_lines = r.opt_u32(NVE_TABLE_TYPOGRAPHY_MAX_LINES).value_or(0u);
  }
  if (r.is_explicit(NVE_TABLE_TYPOGRAPHY_FONT_WEIGHT)) {
    s.font_weight = static_cast<uint16_t>(r.bits(NVE_TABLE_TYPOGRAPHY_FONT_WEIGHT));
  }
  if (r.is_explicit(NVE_TABLE_TYPOGRAPHY_FONT_STYLE)) {
    s.font_style = static_cast<uint8_t>(r.bits(NVE_TABLE_TYPOGRAPHY_FONT_STYLE));
  }
  if (r.is_explicit(NVE_TABLE_TYPOGRAPHY_WHITE_SPACE)) {
    s.white_space = static_cast<uint8_t>(r.bits(NVE_TABLE_TYPOGRAPHY_WHITE_SPACE));
  }
  if (r.is_explicit(NVE_TABLE_TYPOGRAPHY_WORD_BREAK)) {
    s.word_break = static_cast<uint8_t>(r.bits(NVE_TABLE_TYPOGRAPHY_WORD_BREAK));
  }
  if (r.is_explicit(NVE_TABLE_TYPOGRAPHY_OVERFLOW_WRAP)) {
    s.overflow_wrap = static_cast<uint8_t>(r.bits(NVE_TABLE_TYPOGRAPHY_OVERFLOW_WRAP));
  }
  if (r.is_explicit(NVE_TABLE_TYPOGRAPHY_TEXT_ALIGN)) {
    s.text_align = static_cast<uint8_t>(r.bits(NVE_TABLE_TYPOGRAPHY_TEXT_ALIGN));
  }
  if (r.is_explicit(NVE_TABLE_TYPOGRAPHY_TEXT_DECORATION)) {
    s.text_decoration = static_cast<uint8_t>(r.bits(NVE_TABLE_TYPOGRAPHY_TEXT_DECORATION));
  }
  if (r.is_explicit(NVE_TABLE_TYPOGRAPHY_VERTICAL_ALIGN)) {
    s.vertical_align = static_cast<uint8_t>(r.bits(NVE_TABLE_TYPOGRAPHY_VERTICAL_ALIGN));
  }
  return s;
}

namespace detail {
/// Fills `g` from `row` of node `index` (`kind` before the text-wrapper rule).
inline void read_geometry_row(const SceneReader& scene, const SceneTables& t, uint32_t index,
                              const NveNodeRow& row, NodeGeometry& g) noexcept {
  g.index = index;
  g.id = make_node_id(index, row.generation);
  g.parent = index == kRootIndex ? 0u : row.parent;
  g.kind = t.kind_of(row.primary_slot);
  g.rect = sealed_rect(*scene.rect(index));
  g.local = g.rect;
  const NveLayoutRect* parent_rect = g.parent != 0u ? scene.rect(node_index(g.parent)) : nullptr;
  if (parent_rect != nullptr) {
    const NveLayoutRect parent = sealed_rect(*parent_rect);
    g.local.x = finite_or(g.local.x - parent.x, 0.0f);  // FLT_MAX - -FLT_MAX overflows
    g.local.y = finite_or(g.local.y - parent.y, 0.0f);
  }
  // Content box: border (`nve.visual.border_width`, all sides) + padding. Percent padding resolves
  // against the containing block's width on every side, as in CSS and the runtime's layout.
  const float basis = parent_rect != nullptr ? sealed_rect(*parent_rect).width : g.rect.width;  // root: viewport
  const RowRef l = t.layout.at(index);
  const auto pad = [&](NveFieldSpec f) {
    const LengthValue v = l.length(f);
    return extent(v.unit == NVE_LENGTH_POINTS    ? v.value
                  : v.unit == NVE_LENGTH_PERCENT ? basis * v.value / 100.0f
                                                 : 0.0f);
  };
  const float border = extent(t.visual.at(index).f32(NVE_TABLE_VISUAL_BORDER_WIDTH));
  const float left = border + pad(NVE_TABLE_LAYOUT_PADDING_LEFT);
  const float top = border + pad(NVE_TABLE_LAYOUT_PADDING_TOP);
  const float right = border + pad(NVE_TABLE_LAYOUT_PADDING_RIGHT);
  const float bottom = border + pad(NVE_TABLE_LAYOUT_PADDING_BOTTOM);
  g.content = NveLayoutRect{left, top, std::max(0.0f, g.rect.width - left - right),
                            std::max(0.0f, g.rect.height - top - bottom)};
}

/// An empty text wrapper of run leaves realizes as a container (see `NodeKind`).
[[nodiscard]] constexpr NodeKind realized_kind(NodeKind kind, bool has_children, std::string_view text) noexcept {
  return kind == NodeKind::Text && has_children && text.empty() ? NodeKind::View : kind;
}

[[nodiscard]] inline std::optional<uint8_t> opt_u8(const RowRef& r, FieldSpec f) noexcept {
  const std::optional<uint32_t> v = r.opt_bits(f);
  return v ? std::optional<uint8_t>(static_cast<uint8_t>(*v)) : std::nullopt;
}

[[nodiscard]] inline float opt_finite_or_nan(const RowRef& r, FieldSpec f) noexcept {
  const std::optional<float> v = r.opt_f32(f);
  return v ? finite_or(*v, NAN) : NAN;
}
}  // namespace detail

/// Identity and geometry of live node `index` (`std::nullopt` when it has no row).
[[nodiscard]] inline std::optional<NodeGeometry> read_geometry(const SceneReader& scene, const SceneTables& t,
                                                               uint32_t index) noexcept {
  const NveNodeRow* row = scene.row(index);
  if (row == nullptr) return std::nullopt;
  NodeGeometry g;
  detail::read_geometry_row(scene, t, index, *row, g);
  if (g.kind == NodeKind::Text && !scene.node_children(index).value_or(std::span<const uint32_t>()).empty()) {
    g.kind = detail::realized_kind(g.kind, true, t.text.at(index).str(NVE_TABLE_TEXT_TEXT, scene));
  }
  return g;
}

/// Decodes live node `index` (a default `NodeData` when it has no row). `text_style` is its
/// resolved (inherited) style.
[[nodiscard]] inline NodeData read_node(const SceneReader& scene, const SceneTables& t, uint32_t index,
                                        const TextStyle& text_style) noexcept {
  NodeData n;
  const NveNodeRow* row = scene.row(index);
  if (row == nullptr) return n;
  detail::read_geometry_row(scene, t, index, *row, n);
  n.children = scene.node_children(index).value_or(std::span<const uint32_t>());
  n.layout_events = t.layout.at(index).events(NVE_TABLE_LAYOUT_EVENTS_WORD);
  n.text = t.text.at(index).str(NVE_TABLE_TEXT_TEXT, scene);
  n.kind = detail::realized_kind(n.kind, !n.children.empty(), n.text);
  n.text_style = text_style;

  const RowRef v = t.visual.at(index);
  n.visual = VisualStyle{v.bits(NVE_TABLE_VISUAL_BACKGROUND),
                         v.bits(NVE_TABLE_VISUAL_BORDER_COLOR),
                         extent(v.f32(NVE_TABLE_VISUAL_BORDER_WIDTH)),
                         extent(v.f32(NVE_TABLE_VISUAL_BORDER_RADIUS)),
                         std::clamp(finite_or(v.f32(NVE_TABLE_VISUAL_OPACITY), 1.0f), 0.0f, 1.0f),
                         v.i32(NVE_TABLE_VISUAL_Z_INDEX),
                         static_cast<uint8_t>(v.bits(NVE_TABLE_VISUAL_OVERFLOW_X)),
                         static_cast<uint8_t>(v.bits(NVE_TABLE_VISUAL_OVERFLOW_Y)),
                         static_cast<uint8_t>(v.bits(NVE_TABLE_VISUAL_VISIBILITY)),
                         v.bits(NVE_TABLE_VISUAL_BOX_SHADOWS),
                         v.bits(NVE_TABLE_VISUAL_BACKGROUND_FILL)};

  if (t.image.has(index)) {
    const RowRef r = t.image.at(index);
    const std::optional<Rgba> tint = r.opt_color(NVE_TABLE_IMAGE_TINT);
    n.image = ImageData{r.bits(NVE_TABLE_IMAGE_SRC),
                        r.str(NVE_TABLE_IMAGE_ALT, scene),
                        static_cast<uint8_t>(r.bits(NVE_TABLE_IMAGE_FIT)),
                        tint ? std::optional<uint32_t>(tint->packed) : std::nullopt,
                        static_cast<uint16_t>(r.opt_u32(NVE_TABLE_IMAGE_INTRINSIC_WIDTH).value_or(0u)),
                        static_cast<uint16_t>(r.opt_u32(NVE_TABLE_IMAGE_INTRINSIC_HEIGHT).value_or(0u))};
  }
  if (t.text_input.has(index)) {
    const RowRef r = t.text_input.at(index);
    n.input = TextInputData{r.events(NVE_TABLE_TEXT_INPUT_EVENTS_WORD),
                            r.str(NVE_TABLE_TEXT_INPUT_VALUE, scene),
                            r.str(NVE_TABLE_TEXT_INPUT_PLACEHOLDER, scene),
                            r.boolean(NVE_TABLE_TEXT_INPUT_MULTILINE),
                            r.boolean(NVE_TABLE_TEXT_INPUT_SECURE),
                            r.boolean(NVE_TABLE_TEXT_INPUT_READ_ONLY),
                            r.boolean(NVE_TABLE_TEXT_INPUT_REQUIRED),
                            static_cast<uint8_t>(r.bits(NVE_TABLE_TEXT_INPUT_INPUT_KIND)),
                            r.opt_u32(NVE_TABLE_TEXT_INPUT_MAX_LENGTH).value_or(0u)};
  }
  if (t.scroll.has(index)) {
    const RowRef r = t.scroll.at(index);
    n.scroll = ScrollData{r.events(NVE_TABLE_SCROLL_EVENTS_WORD),
                          static_cast<uint8_t>(r.bits(NVE_TABLE_SCROLL_AXES)),
                          static_cast<uint8_t>(r.bits(NVE_TABLE_SCROLL_SCROLLBARS)),
                          r.boolean(NVE_TABLE_SCROLL_PAGING),
                          r.boolean(NVE_TABLE_SCROLL_BOUNCE),
                          detail::opt_finite_or_nan(r, NVE_TABLE_SCROLL_REQUESTED_SCROLL_X),
                          detail::opt_finite_or_nan(r, NVE_TABLE_SCROLL_REQUESTED_SCROLL_Y)};
  }
  if (t.interaction.has(index)) {
    const RowRef r = t.interaction.at(index);
    const std::optional<int32_t> tab = r.opt_i32(NVE_TABLE_INTERACTION_TAB_INDEX);
    n.interaction = InteractionData{r.events(NVE_TABLE_INTERACTION_EVENTS_WORD),
                                    static_cast<uint8_t>(r.bits(NVE_TABLE_INTERACTION_CURSOR)),
                                    r.boolean(NVE_TABLE_INTERACTION_DISABLED),
                                    r.boolean(NVE_TABLE_INTERACTION_FOCUSABLE),
                                    tab ? std::optional<int16_t>(static_cast<int16_t>(*tab)) : std::nullopt};
  }
  if (t.accessibility.has(index)) {
    const RowRef r = t.accessibility.at(index);
    n.a11y = AccessibilityData{true,
                               detail::opt_u8(r, NVE_TABLE_ACCESSIBILITY_ROLE),
                               static_cast<uint8_t>(r.opt_u32(NVE_TABLE_ACCESSIBILITY_LEVEL).value_or(0u)),
                               r.boolean(NVE_TABLE_ACCESSIBILITY_HIDDEN),
                               detail::opt_u8(r, NVE_TABLE_ACCESSIBILITY_CHECKED),
                               detail::opt_u8(r, NVE_TABLE_ACCESSIBILITY_SELECTED),
                               detail::opt_u8(r, NVE_TABLE_ACCESSIBILITY_EXPANDED),
                               detail::opt_u8(r, NVE_TABLE_ACCESSIBILITY_PRESSED),
                               r.str(NVE_TABLE_ACCESSIBILITY_LABEL, scene),
                               r.str(NVE_TABLE_ACCESSIBILITY_HINT, scene),
                               r.str(NVE_TABLE_ACCESSIBILITY_VALUE_TEXT, scene),
                               detail::opt_finite_or_nan(r, NVE_TABLE_ACCESSIBILITY_VALUE_NOW),
                               detail::opt_finite_or_nan(r, NVE_TABLE_ACCESSIBILITY_VALUE_MIN),
                               detail::opt_finite_or_nan(r, NVE_TABLE_ACCESSIBILITY_VALUE_MAX)};
  }
  n.href = t.link.at(index).str(NVE_TABLE_LINK_HREF, scene);
  return n;
}

}  // namespace nve::host
