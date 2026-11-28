#import "NveAppleSceneNodes.h"

#include <algorithm>
#include <cmath>

namespace nve::apple {

using nve::host::NodeData;

static ImageResizeMode resize_mode_for_fit(uint32_t fit) noexcept {
  switch (fit) {
    case NVE_IMAGE_FIT_CONTAIN: return ImageResizeMode::Contain;
    case NVE_IMAGE_FIT_FILL: return ImageResizeMode::Stretch;
    case NVE_IMAGE_FIT_NONE: return ImageResizeMode::Center;
    case NVE_IMAGE_FIT_SCALE_DOWN: return ImageResizeMode::ScaleDown;
    default: return ImageResizeMode::Cover;
  }
}

static A11yRole role_for_aria(uint8_t aria) noexcept {
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
    case NVE_ARIA_ROLE_PRESENTATION: return A11yRole::None;
    default: return A11yRole::Group;
  }
}

static uint8_t line_height_unit_from_length(uint8_t unit) noexcept {
  if (unit == NVE_LENGTH_POINTS) return NVE_LINE_HEIGHT_POINTS;
  if (unit == NVE_LENGTH_PERCENT) return NVE_LINE_HEIGHT_PERCENT;
  return NVE_LINE_HEIGHT_AUTO;
}

NodeKind node_kind(const NodeData &n, bool has_canvas) noexcept {
  switch (n.kind) {
    case nve::host::NodeKind::TextInput: return NodeKind::TextInput;
    case nve::host::NodeKind::Image: return NodeKind::Image;
    case nve::host::NodeKind::Scroll: return NodeKind::Scroll;
    case nve::host::NodeKind::Text: return NodeKind::Text;
    case nve::host::NodeKind::View:
      return has_canvas ? NodeKind::Canvas : NodeKind::Container;
    case nve::host::NodeKind::Link: return NodeKind::Container;
  }
  return NodeKind::Container;
}

NodeView node_view(const NodeData &n, const nve::canvas::CanvasData *canvas) {
  NodeView v;
  v.node_id = n.id;
  v.frame = CGRectMake(n.rect.x, n.rect.y, n.rect.width, n.rect.height);

  NodeStyle &s = v.style;
  s.background_rgba = n.visual.background;
  s.border_rgba = n.visual.border_color;
  s.border_width = std::max(0.0f, n.visual.border_width);
  s.border_radius = std::max(0.0f, n.visual.border_radius);
  s.opacity = std::clamp(n.visual.opacity, 0.0f, 1.0f);
  s.z_index = n.visual.z_index;
  s.clips_children = n.kind == nve::host::NodeKind::Scroll || n.visual.overflow_x != NVE_OVERFLOW_VISIBLE ||
                     n.visual.overflow_y != NVE_OVERFLOW_VISIBLE;
  s.visible = n.visual.visibility != NVE_VISIBILITY_HIDDEN;
  s.text_rgba = n.text_style.color;
  s.font_family.assign(n.text_style.font_family);
  if (std::isfinite(n.text_style.font_size) && n.text_style.font_size > 0.0f) s.font_size = n.text_style.font_size;
  s.font_weight = n.text_style.font_weight;
  s.italic = n.text_style.font_style != NVE_FONT_STYLE_NORMAL;
  s.letter_spacing = std::isfinite(n.text_style.letter_spacing) ? n.text_style.letter_spacing : 0.0f;
  s.line_height_unit = line_height_unit_from_length(n.text_style.line_height.unit);
  s.line_height = std::isfinite(n.text_style.line_height.value) ? n.text_style.line_height.value : 0.0f;
  s.text_align = n.text_style.text_align;
  s.max_lines = n.text_style.max_lines;
  s.text_decoration = n.text_style.text_decoration;
  s.white_space = n.text_style.white_space;
  s.word_break = n.text_style.word_break;
  s.overflow_wrap = n.text_style.overflow_wrap;

  NodeSemantics &a = v.semantics;
  switch (n.kind) {
    case nve::host::NodeKind::TextInput:
      v.kind = NodeKind::TextInput;
      v.text = n.input.value;
      a.placeholder = n.input.placeholder;
      a.secure_text_entry = n.input.secure;
      a.multiline = n.input.multiline;
      a.read_only = n.input.read_only;
      a.max_length = n.input.max_length;
      a.role = A11yRole::TextBox;
      break;
    case nve::host::NodeKind::Image:
      v.kind = NodeKind::Image;
      v.image_resource = n.image.src;
      v.image_fit = resize_mode_for_fit(n.image.fit);
      if (n.image.tint.has_value()) v.image_tint_rgba = *n.image.tint;
      a.role = A11yRole::Image;
      a.label = n.image.alt;
      a.has_explicit_label = !a.label.empty();
      break;
    case nve::host::NodeKind::Scroll:
      v.kind = NodeKind::Scroll;
      a.role = A11yRole::ScrollArea;
      break;
    case nve::host::NodeKind::Text:
      v.kind = NodeKind::Text;
      v.text = n.text;
      a.role = s.font_size >= 20.0f ? A11yRole::Header : A11yRole::Text;
      a.label = v.text;
      break;
    case nve::host::NodeKind::View:
      if (canvas != nullptr) {
        v.kind = NodeKind::Canvas;
        v.canvas_draw_list = canvas->draw_list;
        v.canvas_generation = canvas->generation;
      }
      break;
    case nve::host::NodeKind::Link:
      break;
  }
  const bool link = n.kind == nve::host::NodeKind::Link;
  v.interactive = link || n.wants(NVE_INTERACTION_EVENT_PRESS);
  if (v.interactive && v.kind == NodeKind::Container) a.role = link ? A11yRole::Link : A11yRole::Button;
  a.disabled = a.disabled || n.interaction.disabled;

  if (n.a11y.present) {
    if (n.a11y.role.has_value()) {
      a.role = role_for_aria(*n.a11y.role);
      a.explicit_role = true;
    }
    if (!n.a11y.label.empty()) {
      a.label = n.a11y.label;
      a.has_explicit_label = true;
    }
    a.hint = n.a11y.hint;
    a.value = n.a11y.value_text;
    a.has_explicit_value = !a.value.empty();
    a.hidden = n.a11y.hidden;
  }
  return v;
}

}  // namespace nve::apple
