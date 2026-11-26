#include "NveRnwComponentView.hpp"

#include <algorithm>
#include <cctype>
#include <cmath>
#include <cstring>

namespace nve::win {

namespace {

std::string ascii_lower(std::string_view sv) {
  std::string out;
  out.reserve(sv.size());
  for (unsigned char ch : sv) {
    out.push_back(static_cast<char>(std::tolower(ch)));
  }
  return out;
}

ImageResizeMode extract_resize_mode(const nve::RawPropMap* props) {
  if (props == nullptr) {
    return ImageResizeMode::Cover;
  }
  for (const char* key : {"resizeMode", "resize_mode"}) {
    const auto it = props->find(key);
    if (it != props->end()) {
      if (const auto* s = std::get_if<std::string>(&it->second)) {
        return parse_image_resize_mode(*s);
      }
    }
  }
  return ImageResizeMode::Cover;
}

nve::LayoutRect intersect_rects(const nve::LayoutRect& a, const nve::LayoutRect& b) noexcept {
  const float x0 = std::max(a.x, b.x);
  const float y0 = std::max(a.y, b.y);
  const float x1 = std::min(a.x + a.width, b.x + b.width);
  const float y1 = std::min(a.y + a.height, b.y + b.height);
  return nve::LayoutRect{x0, y0, std::max(0.0f, x1 - x0), std::max(0.0f, y1 - y0)};
}

bool rect_contains(const nve::LayoutRect& r, float px, float py) noexcept {
  return r.width > 0.0f && r.height > 0.0f &&
         px >= r.x && px <= (r.x + r.width) &&
         py >= r.y && py <= (r.y + r.height);
}

} // namespace

ImageResizeMode parse_image_resize_mode(std::string_view raw) noexcept {
  if (raw.empty()) {
    return ImageResizeMode::Cover;
  }
  const std::string lower = ascii_lower(raw);
  if (lower == "contain") {
    return ImageResizeMode::Contain;
  }
  if (lower == "stretch" || lower == "fill") {
    return ImageResizeMode::Stretch;
  }
  if (lower == "center") {
    return ImageResizeMode::Center;
  }
  return ImageResizeMode::Cover;
}

void NveWinVisualTree::Adapter::update_root(
    const NveSceneView& /*view*/,
    const NveRenderNode& node) {
  root_background_rgba = node.bg_rgba;
  root_layout = nve::LayoutRect{node.x, node.y, node.width, node.height};
}

NveWinNodeVisual NveWinVisualTree::Adapter::create_view(
    const NveSceneView& /*view*/,
    const NveRenderNode& node) {
  ++created_count;
  NveWinNodeVisual visual{};
  visual.node_id = node.node_id;
  visual.kind = node.kind;
  visual.view_tag = nve::view_tag_for_render_node(node);
  return visual;
}

void NveWinVisualTree::Adapter::update_view(
    const NveSceneView& view,
    NveWinNodeVisual& visual,
    const NveRenderNode& node) {
  ++updated_count;
  visual.node_id = node.node_id;
  visual.kind = node.kind;
  visual.view_tag = nve::view_tag_for_render_node(node);
  visual.is_text_leaf = (node.flags & NVE_NODE_LEAF_TEXT) != 0u;
  visual.interactive =
      (node.kind == NVE_NODE_KIND_PRESSABLE) || ((node.flags & NVE_NODE_INTERACTIVE) != 0u);
  visual.clip_bounds =
      (node.kind == NVE_NODE_KIND_SCROLL_VIEW) || ((node.flags & NVE_NODE_CLIP_BOUNDS) != 0u);

  const std::string_view payload = nve::scene_node_text(view, node);
  if (node.kind == NVE_NODE_KIND_IMAGE) {
    visual.image_src.assign(payload.data(), payload.size());
    visual.text.clear();
    visual.draw_list.clear();
    if (raw_props != nullptr) {
      const auto it = raw_props->find(node.node_id);
      visual.resize_mode =
          (it != raw_props->end()) ? extract_resize_mode(&it->second) : ImageResizeMode::Cover;
    }
  } else if (node.kind == NVE_NODE_KIND_SKIA_CANVAS) {
    const auto* raw_ptr = reinterpret_cast<const uint8_t*>(payload.data());
    const bool same =
        visual.draw_list.size() == payload.size() &&
        (payload.empty() ||
         std::memcmp(visual.draw_list.data(), raw_ptr, payload.size()) == 0);
    if (!same) {
      visual.draw_list.assign(raw_ptr, raw_ptr + payload.size());
    }
    visual.text.clear();
    visual.image_src.clear();
  } else {
    visual.text.assign(payload.data(), payload.size());
    visual.image_src.clear();
    visual.draw_list.clear();
  }

  visual.layout = nve::LayoutRect{node.x, node.y, node.width, node.height};
  visual.background_rgba = node.bg_rgba;
  visual.text_color_rgba = node.text_rgba;
  visual.border_color_rgba = node.border_rgba;
  visual.border_width = node.border_width;
  visual.border_radius = node.border_radius;
  visual.opacity = static_cast<float>(node.opacity_u8) / 255.0f;
  visual.font_size = node.font_size;
  visual.z_index = node.z_index;
}

void NveWinVisualTree::Adapter::destroy_view(uint32_t /*node_id*/, NveWinNodeVisual& /*visual*/) {
  ++destroyed_count;
}

bool NveWinVisualTree::sync(const NveSceneView& view, const RawPropByNode* raw_props) {
  adapter_.raw_props = raw_props;
  const bool ok = reconciler_.sync(view, adapter_);
  if (ok && raw_props != nullptr) {
    for (const auto& [nid, rp] : *raw_props) {
      if (NveWinNodeVisual* visual = reconciler_.find(nid);
          visual != nullptr && visual->kind == NVE_NODE_KIND_IMAGE) {
        visual->resize_mode = extract_resize_mode(&rp);
      }
    }
  }
  adapter_.raw_props = nullptr;
  return ok;
}

void NveWinVisualTree::clear() {
  reconciler_.clear(adapter_);
  adapter_.root_children.clear();
  adapter_.root_background_rgba = 0u;
  adapter_.root_layout = nve::LayoutRect{};
}

bool NveWinVisualTree::set_scroll_offset(uint32_t node_id, float offset_x, float offset_y) {
  NveWinNodeVisual* node = reconciler_.find(node_id);
  if (node == nullptr || node->kind != NVE_NODE_KIND_SCROLL_VIEW) {
    return false;
  }
  const float max_x = std::max(0.0f, node->content_width - node->layout.width);
  const float max_y = std::max(0.0f, node->content_height - node->layout.height);
  const float nx = std::clamp(std::isfinite(offset_x) ? offset_x : 0.0f, 0.0f, max_x);
  const float ny = std::clamp(std::isfinite(offset_y) ? offset_y : 0.0f, 0.0f, max_y);
  node->scroll_offset_x = nx;
  node->scroll_offset_y = ny;
  return true;
}

std::optional<std::pair<float, float>> NveWinVisualTree::scroll_offset(
    uint32_t node_id) const noexcept {
  const NveWinNodeVisual* node = reconciler_.find(node_id);
  if (node == nullptr || node->kind != NVE_NODE_KIND_SCROLL_VIEW) {
    return std::nullopt;
  }
  return std::make_pair(node->scroll_offset_x, node->scroll_offset_y);
}

std::optional<nve::LayoutRect> NveWinVisualTree::absolute_rect(uint32_t node_id) const {
  if (node_id == NVE_ROOT_NODE_ID) {
    return adapter_.root_layout;
  }
  const NveWinNodeVisual* node = reconciler_.find(node_id);
  if (node == nullptr) {
    return std::nullopt;
  }

  float abs_x = node->layout.x;
  float abs_y = node->layout.y;
  uint32_t cur_parent = node->parent_id;
  uint32_t hops = 0;
  while (cur_parent != 0u && cur_parent != NVE_ROOT_NODE_ID && hops < 512u) {
    const NveWinNodeVisual* parent = reconciler_.find(cur_parent);
    if (parent == nullptr) {
      break;
    }
    if (parent->kind == NVE_NODE_KIND_SCROLL_VIEW) {
      abs_x -= parent->scroll_offset_x;
      abs_y -= parent->scroll_offset_y;
    }
    abs_x += parent->layout.x;
    abs_y += parent->layout.y;
    cur_parent = parent->parent_id;
    ++hops;
  }
  return nve::LayoutRect{abs_x, abs_y, node->layout.width, node->layout.height};
}

void NveWinVisualTree::append_draw_order(
    uint32_t node_id,
    float origin_x,
    float origin_y,
    float inherited_opacity,
    bool has_clip,
    nve::LayoutRect clip_rect,
    std::vector<FlattenedVisualNode>& out,
    uint32_t depth) const {
  if (depth > 512u) {
    return;
  }
  const NveWinNodeVisual* node = reconciler_.find(node_id);
  if (node == nullptr) {
    return;
  }

  const float clamped_opacity = std::clamp(node->opacity, 0.0f, 1.0f);
  const float effective_opacity = inherited_opacity * clamped_opacity;
  const float abs_x = origin_x + node->layout.x;
  const float abs_y = origin_y + node->layout.y;

  out.push_back(FlattenedVisualNode{
      .node_id = node_id,
      .abs_x = abs_x,
      .abs_y = abs_y,
      .effective_opacity = effective_opacity,
      .has_clip = has_clip,
      .clip_rect = clip_rect,
  });

  if (node->children.empty()) {
    return;
  }

  const nve::LayoutRect self_rect{abs_x, abs_y, node->layout.width, node->layout.height};
  const bool next_has_clip = has_clip || node->clip_bounds;
  const nve::LayoutRect next_clip =
      node->clip_bounds ? (has_clip ? intersect_rects(clip_rect, self_rect) : self_rect) : clip_rect;

  const float child_origin_x =
      abs_x - (node->kind == NVE_NODE_KIND_SCROLL_VIEW ? node->scroll_offset_x : 0.0f);
  const float child_origin_y =
      abs_y - (node->kind == NVE_NODE_KIND_SCROLL_VIEW ? node->scroll_offset_y : 0.0f);

  std::vector<uint32_t> ordered = node->children;
  std::stable_sort(ordered.begin(), ordered.end(), [this](uint32_t a, uint32_t b) {
    const NveWinNodeVisual* va = reconciler_.find(a);
    const NveWinNodeVisual* vb = reconciler_.find(b);
    return (va ? va->z_index : 0) < (vb ? vb->z_index : 0);
  });

  for (uint32_t child_id : ordered) {
    append_draw_order(
        child_id,
        child_origin_x,
        child_origin_y,
        effective_opacity,
        next_has_clip,
        next_clip,
        out,
        depth + 1u);
  }
}

std::vector<FlattenedVisualNode> NveWinVisualTree::build_draw_order() const {
  std::vector<FlattenedVisualNode> out;
  out.reserve(reconciler_.size());

  std::vector<uint32_t> ordered_roots = adapter_.root_children;
  std::stable_sort(ordered_roots.begin(), ordered_roots.end(), [this](uint32_t a, uint32_t b) {
    const NveWinNodeVisual* va = reconciler_.find(a);
    const NveWinNodeVisual* vb = reconciler_.find(b);
    return (va ? va->z_index : 0) < (vb ? vb->z_index : 0);
  });

  for (uint32_t child_id : ordered_roots) {
    append_draw_order(child_id, 0.0f, 0.0f, 1.0f, false, nve::LayoutRect{}, out, 0u);
  }
  return out;
}

std::optional<HitTestResult> NveWinVisualTree::hit_test_interactive(
    float surface_x,
    float surface_y) const {
  const std::vector<FlattenedVisualNode> order = build_draw_order();
  for (auto it = order.rbegin(); it != order.rend(); ++it) {
    const NveWinNodeVisual* node = reconciler_.find(it->node_id);
    if (node == nullptr || !node->interactive || it->effective_opacity <= 0.0f) {
      continue;
    }
    if (it->has_clip && !rect_contains(it->clip_rect, surface_x, surface_y)) {
      continue;
    }
    const nve::LayoutRect bounds{it->abs_x, it->abs_y, node->layout.width, node->layout.height};
    if (rect_contains(bounds, surface_x, surface_y)) {
      return HitTestResult{
          .node_id = node->node_id,
          .local_x = surface_x - it->abs_x,
          .local_y = surface_y - it->abs_y,
      };
    }
  }
  return std::nullopt;
}

std::optional<uint32_t> NveWinVisualTree::hit_test_scrollable(
    float surface_x,
    float surface_y) const {
  const std::vector<FlattenedVisualNode> order = build_draw_order();
  for (auto it = order.rbegin(); it != order.rend(); ++it) {
    const NveWinNodeVisual* node = reconciler_.find(it->node_id);
    if (node == nullptr || node->kind != NVE_NODE_KIND_SCROLL_VIEW ||
        it->effective_opacity <= 0.0f) {
      continue;
    }
    if (it->has_clip && !rect_contains(it->clip_rect, surface_x, surface_y)) {
      continue;
    }
    const nve::LayoutRect bounds{it->abs_x, it->abs_y, node->layout.width, node->layout.height};
    if (rect_contains(bounds, surface_x, surface_y)) {
      return node->node_id;
    }
  }
  return std::nullopt;
}

} // namespace nve::win
