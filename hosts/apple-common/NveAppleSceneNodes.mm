#import "NveAppleSceneNodes.h"

#include <nve/generated/nve_tables.generated.h>

#include <algorithm>
#include <cmath>

namespace nve::apple {

using nve::host::Key64;
using nve::host::TableReader;

/// Packed RGBA of an optional color field (transparent when the read failed).
static uint32_t packed(std::optional<nve::host::Rgba> c) noexcept { return c.value_or(nve::host::Rgba{}).packed; }

SceneTables::SceneTables(const nve::host::RuntimeSceneReader &scene)
    : scene_(scene),
      visual_(scene.find_table(Key64{NVE_TABLE_VISUAL_KEY})),
      typography_(scene.find_table(Key64{NVE_TABLE_TYPOGRAPHY_KEY})),
      text_(scene.find_table(Key64{NVE_TABLE_TEXT_KEY})),
      image_(scene.find_table(Key64{NVE_TABLE_IMAGE_KEY})),
      scroll_(scene.find_table(Key64{NVE_TABLE_SCROLL_KEY})),
      accessibility_(scene.find_table(Key64{NVE_TABLE_ACCESSIBILITY_KEY})),
      text_input_(scene.find_table(Key64{NVE_TABLE_TEXT_INPUT_KEY})),
      link_(scene.find_table(Key64{NVE_TABLE_LINK_KEY})),
      interaction_(scene.find_table(Key64{NVE_TABLE_INTERACTION_KEY})) {}

std::optional<uint32_t> SceneTables::index_of(uint32_t node_id) const noexcept {
  const uint32_t index = node_id & 0x00FFFFFFu;
  const NveNodeRow *r = scene_.row(index);
  if (node_id == 0u || r == nullptr || (r->flags & NVE_NODE_ROW_FLAG_LIVE) == 0u || r->generation != (node_id >> 24u)) {
    return std::nullopt;
  }
  return index;
}

uint32_t SceneTables::node_id_at(uint32_t index) const noexcept {
  const NveNodeRow *r = scene_.row(index);
  if (r == nullptr || (r->flags & NVE_NODE_ROW_FLAG_LIVE) == 0u) return 0u;
  return index | (static_cast<uint32_t>(r->generation) << 24u);
}

std::optional<uint32_t> SceneTables::row(const std::optional<TableReader> &table, uint32_t index) const noexcept {
  // A node owns a row only when its `table_set` holds the table (unattached dense rows are zero).
  if (!table || !scene_.is_live(index) || table->slot() >= 128u ||
      !scene_.has_table(index, static_cast<uint8_t>(table->slot()))) {
    return std::nullopt;
  }
  return table->row_of_node(index);
}

std::string_view SceneTables::string(const std::optional<TableReader> &table, uint32_t index, NveFieldSpec f) const noexcept {
  const auto r = row(table, index);
  if (!r) return {};
  const auto span = table->str(*r, f);
  if (!span) return {};
  return scene_.text(span->offset, span->len).value_or(std::string_view());
}

int32_t SceneTables::z_index(uint32_t index) const noexcept {
  const auto r = row(visual_, index);
  return r ? visual_->i32(*r, NVE_TABLE_VISUAL_Z_INDEX).value_or(0) : 0;
}

void SceneTables::resolve_typography(uint32_t index, NodeStyle &style) const noexcept {
  auto def = [](NveFieldSpec f) { return nve_field_bits(NVE_TABLE_TYPOGRAPHY_DEFAULTS[f.word], f); };
  style.text_rgba = def(NVE_TABLE_TYPOGRAPHY_COLOR);
  style.font_size = std::bit_cast<float>(def(NVE_TABLE_TYPOGRAPHY_FONT_SIZE));
  style.font_weight = static_cast<uint16_t>(def(NVE_TABLE_TYPOGRAPHY_FONT_WEIGHT));
  uint32_t resolved = 0u;
  uint32_t node = index;
  for (uint32_t depth = 0u; depth < scene_.node_capacity() && scene_.is_live(node); ++depth) {
    if (const auto r = row(typography_, node)) {
      const uint32_t set = typography_->u32(*r, NVE_TABLE_TYPOGRAPHY_SET).value_or(0u);
      const uint32_t fresh = set & ~resolved;
      if (fresh & NVE_TYPOGRAPHY_SET_COLOR) style.text_rgba = packed(typography_->color(*r, NVE_TABLE_TYPOGRAPHY_COLOR));
      if (fresh & NVE_TYPOGRAPHY_SET_FONT_SIZE) {
        const float s = typography_->f32(*r, NVE_TABLE_TYPOGRAPHY_FONT_SIZE).value_or(16.0f);
        if (std::isfinite(s) && s > 0.0f) style.font_size = s;
      }
      if (fresh & NVE_TYPOGRAPHY_SET_FONT_WEIGHT) {
        style.font_weight = static_cast<uint16_t>(typography_->u32(*r, NVE_TABLE_TYPOGRAPHY_FONT_WEIGHT).value_or(400u));
      }
      if (fresh & NVE_TYPOGRAPHY_SET_FONT_STYLE) {
        style.italic = typography_->enum_value(*r, NVE_TABLE_TYPOGRAPHY_FONT_STYLE).value_or(0u) != NVE_FONT_STYLE_NORMAL;
      }
      if (fresh & NVE_TYPOGRAPHY_SET_TEXT_ALIGN) style.text_align = typography_->enum_value(*r, NVE_TABLE_TYPOGRAPHY_TEXT_ALIGN).value_or(0u);
      if (fresh & NVE_TYPOGRAPHY_SET_MAX_LINES) style.max_lines = typography_->u32(*r, NVE_TABLE_TYPOGRAPHY_MAX_LINES).value_or(0u);
      resolved |= set;
    }
    const NveNodeRow *nr = scene_.row(node);
    const auto parent = nr != nullptr ? index_of(nr->parent) : std::nullopt;
    if (!parent || *parent == node) break;
    node = *parent;
  }
}

static ImageResizeMode resize_mode_for_fit(uint32_t fit) noexcept {
  switch (fit) {
    case NVE_IMAGE_FIT_CONTAIN: return ImageResizeMode::Contain;
    case NVE_IMAGE_FIT_FILL: return ImageResizeMode::Stretch;
    case NVE_IMAGE_FIT_NONE: return ImageResizeMode::Center;
    case NVE_IMAGE_FIT_SCALE_DOWN: return ImageResizeMode::ScaleDown;
    default: return ImageResizeMode::Cover;
  }
}

static A11yRole role_for_aria(uint32_t aria) noexcept {
  switch (aria) {
    case NVE_ARIA_ROLE_BUTTON: return A11yRole::Button;
    case NVE_ARIA_ROLE_LINK: return A11yRole::Link;
    case NVE_ARIA_ROLE_HEADING: return A11yRole::Header;
    case NVE_ARIA_ROLE_TEXT: return A11yRole::Text;
    case NVE_ARIA_ROLE_IMAGE: return A11yRole::Image;
    case NVE_ARIA_ROLE_TEXT_BOX:
    case NVE_ARIA_ROLE_SEARCH_BOX: return A11yRole::TextBox;
    case NVE_ARIA_ROLE_SCROLL_AREA: return A11yRole::ScrollArea;
    case NVE_ARIA_ROLE_LIST: return A11yRole::List;
    case NVE_ARIA_ROLE_LIST_ITEM: return A11yRole::ListItem;
    case NVE_ARIA_ROLE_CHECKBOX: return A11yRole::CheckBox;
    case NVE_ARIA_ROLE_NONE: return A11yRole::None;
    default: return A11yRole::Group;
  }
}

NodeView SceneTables::describe(uint32_t index) const {
  NodeView v;
  v.node_id = node_id_at(index);
  if (const NveLayoutRect *r = scene_.rect(index)) v.frame = CGRectMake(r->x, r->y, r->width, r->height);
  if (const auto r = row(visual_, index)) {
    v.style.background_rgba = packed(visual_->color(*r, NVE_TABLE_VISUAL_BACKGROUND));
    v.style.border_rgba = packed(visual_->color(*r, NVE_TABLE_VISUAL_BORDER_COLOR));
    v.style.border_width = std::max(0.0f, visual_->f32(*r, NVE_TABLE_VISUAL_BORDER_WIDTH).value_or(0.0f));
    v.style.border_radius = std::max(0.0f, visual_->f32(*r, NVE_TABLE_VISUAL_BORDER_RADIUS).value_or(0.0f));
    v.style.opacity = std::clamp(visual_->f32(*r, NVE_TABLE_VISUAL_OPACITY).value_or(1.0f), 0.0f, 1.0f);
    v.style.z_index = visual_->i32(*r, NVE_TABLE_VISUAL_Z_INDEX).value_or(0);
    v.style.clips_children = visual_->enum_value(*r, NVE_TABLE_VISUAL_OVERFLOW_X).value_or(0u) != NVE_OVERFLOW_VISIBLE ||
                             visual_->enum_value(*r, NVE_TABLE_VISUAL_OVERFLOW_Y).value_or(0u) != NVE_OVERFLOW_VISIBLE;
    v.style.visible = visual_->enum_value(*r, NVE_TABLE_VISUAL_VISIBILITY).value_or(0u) != NVE_VISIBILITY_HIDDEN;
  }
  resolve_typography(index, v.style);

  const auto input_row = row(text_input_, index);
  const auto image_row = row(image_, index);
  const bool is_scroll = row(scroll_, index).has_value();
  const std::string_view text = string(text_, index, NVE_TABLE_TEXT_TEXT);
  if (input_row) {
    v.kind = NodeKind::TextInput;
    v.text = string(text_input_, index, NVE_TABLE_TEXT_INPUT_VALUE);
    v.semantics.placeholder = string(text_input_, index, NVE_TABLE_TEXT_INPUT_PLACEHOLDER);
    v.semantics.secure_text_entry = text_input_->boolean(*input_row, NVE_TABLE_TEXT_INPUT_SECURE).value_or(false);
    v.semantics.multiline = text_input_->boolean(*input_row, NVE_TABLE_TEXT_INPUT_MULTILINE).value_or(false);
    v.semantics.disabled = text_input_->boolean(*input_row, NVE_TABLE_TEXT_INPUT_READ_ONLY).value_or(false);
    v.semantics.role = A11yRole::TextBox;
  } else if (image_row) {
    v.kind = NodeKind::Image;
    v.image_resource = image_->resource(*image_row, NVE_TABLE_IMAGE_SRC).value_or(0u);
    v.image_fit = resize_mode_for_fit(image_->enum_value(*image_row, NVE_TABLE_IMAGE_FIT).value_or(0u));
    if (image_->boolean(*image_row, NVE_TABLE_IMAGE_TINT_ENABLED).value_or(false)) {
      v.image_tint_rgba = packed(image_->color(*image_row, NVE_TABLE_IMAGE_TINT));
    }
    v.semantics.role = A11yRole::Image;
    v.semantics.label = string(image_, index, NVE_TABLE_IMAGE_ALT);
    v.semantics.has_explicit_label = !v.semantics.label.empty();
  } else if (is_scroll) {
    v.kind = NodeKind::Scroll;
    v.style.clips_children = true;
    v.semantics.role = A11yRole::ScrollArea;
  } else if (!text.empty() || row(text_, index)) {
    v.kind = NodeKind::Text;
    v.text = text;
    v.semantics.role = v.style.font_size >= 20.0f ? A11yRole::Header : A11yRole::Text;
    v.semantics.label = v.text;
  }
  v.interactive = row(interaction_, index).has_value() || row(link_, index).has_value();
  if (v.interactive && v.kind == NodeKind::Container) v.semantics.role = row(link_, index) ? A11yRole::Link : A11yRole::Button;

  if (const auto r = row(accessibility_, index)) {
    const uint32_t aria = accessibility_->enum_value(*r, NVE_TABLE_ACCESSIBILITY_ROLE).value_or(0u);
    if (aria != NVE_ARIA_ROLE_NONE) {
      v.semantics.role = role_for_aria(aria);
      v.semantics.explicit_role = true;
    }
    const std::string_view label = string(accessibility_, index, NVE_TABLE_ACCESSIBILITY_LABEL);
    if (!label.empty()) {
      v.semantics.label = label;
      v.semantics.has_explicit_label = true;
    }
    v.semantics.hint = string(accessibility_, index, NVE_TABLE_ACCESSIBILITY_HINT);
    const std::string_view value = string(accessibility_, index, NVE_TABLE_ACCESSIBILITY_VALUE_TEXT);
    v.semantics.value = value;
    v.semantics.has_explicit_value = !value.empty();
    v.semantics.hidden = accessibility_->boolean(*r, NVE_TABLE_ACCESSIBILITY_HIDDEN).value_or(false);
  }
  return v;
}

std::vector<uint32_t> SceneTables::paint_ordered_children(uint32_t index) const {
  std::vector<uint32_t> out;
  const auto kids = scene_.node_children(index);
  if (!kids) return out;
  out.reserve(kids->size());
  for (const uint32_t raw : *kids) {
    if (const auto c = index_of(raw)) out.push_back(*c);
  }
  std::stable_sort(out.begin(), out.end(), [&](uint32_t a, uint32_t b) { return z_index(a) < z_index(b); });
  return out;
}

}  // namespace nve::apple