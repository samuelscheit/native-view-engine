#include "fabric_internal.hpp"
#include "nve_scene_builder.hpp"

namespace nve {

namespace {

inline float sanitize_finite(float v) noexcept {
  return std::isfinite(v) ? v : 0.0f;
}

inline float sanitize_positive(float v, float fallback) noexcept {
  return (std::isfinite(v) && v > 0.0f) ? v : fallback;
}

inline std::string_view effective_node_text(const ShadowNodeEntry& entry) noexcept {
  if (entry.kind == NVE_COMP_IMAGE && !entry.image_src.empty()) {
    return std::string_view(entry.image_src);
  }
  if (entry.kind == NVE_COMP_SKIA_CANVAS && !entry.draw_list.empty()) {
    return std::string_view(
        reinterpret_cast<const char*>(entry.draw_list.data()), entry.draw_list.size());
  }
  return std::string_view(entry.text);
}

inline NveRenderNode build_render_node(
    const ShadowNodeEntry& entry,
    uint32_t text_offset,
    uint16_t text_len,
    uint32_t child_offset,
    uint16_t child_count) noexcept {
  const float border_radius = std::max(0.0f, sanitize_finite(entry.style.border_radius));
  const float border_width = std::max(0.0f, sanitize_finite(entry.style.border_width));
  uint8_t flags = 0u;
  if (entry.kind == NVE_COMP_TEXT && child_count == 0u) {
    flags |= NVE_NODE_LEAF_TEXT;
  }
  if (entry.kind == NVE_COMP_PRESSABLE || entry.event_mask != 0u) {
    flags |= NVE_NODE_INTERACTIVE;
  }
  if (border_radius > 0.0f || entry.kind == NVE_COMP_SCROLL_VIEW) {
    flags |= NVE_NODE_CLIP_BOUNDS;
  }

  NveRenderNode node{};
  node.node_id = entry.node_id;
  node.kind = SceneTableBuilder::map_component_kind(entry.kind);
  node.flags = flags;
  node.opacity_u8 = SceneTableBuilder::quantize_opacity(entry.style.opacity);
  node._pad0 = 0u;
  node.x = sanitize_finite(entry.computed_layout.x);
  node.y = sanitize_finite(entry.computed_layout.y);
  node.width = std::max(0.0f, sanitize_finite(entry.computed_layout.width));
  node.height = std::max(0.0f, sanitize_finite(entry.computed_layout.height));
  node.bg_rgba = entry.style.background_rgba;
  node.text_rgba = entry.style.text_color_rgba;
  node.border_rgba = entry.style.border_color_rgba;
  node.border_radius = border_radius;
  node.border_width = border_width;
  node.font_size = sanitize_positive(entry.style.font_size, 16.0f);
  node.text_offset = text_offset;
  node.text_len = text_len;
  node.child_count = child_count;
  node.child_offset = child_offset;
  node.z_index = entry.style.z_index;
  return node;
}

} // namespace

void FabricHost::mark_scene_index_dirty(size_t idx) {
  if (scene_.dirty_marks[idx] != scene_.mark_epoch) {
    scene_.dirty_marks[idx] = scene_.mark_epoch;
    scene_.dirty_indices.push_back(static_cast<uint32_t>(idx));
  }
}

void FabricHost::remove_scene_node(uint32_t node_id) {
  const uint32_t idx_u32 = scene_.find_index(node_id);
  if (idx_u32 == kInvalidSlot) {
    return;
  }
  const size_t idx = static_cast<size_t>(idx_u32);
  const NveRenderNode removed = scene_.nodes[idx];
  scene_.clear_index(node_id);
  scene_.removed_ids.push_back(node_id);

  if (scene_.live_text_bytes >= removed.text_len) {
    scene_.live_text_bytes -= removed.text_len;
  } else {
    scene_.live_text_bytes = 0u;
  }
  if (scene_.live_children_len >= removed.child_count) {
    scene_.live_children_len -= removed.child_count;
  } else {
    scene_.live_children_len = 0u;
  }

  const size_t last_idx = scene_.nodes.size() - 1u;
  if (idx != last_idx) {
    scene_.nodes[idx] = scene_.nodes[last_idx];
    scene_.dirty_marks[idx] = 0u;
    scene_.set_index(scene_.nodes[idx].node_id, static_cast<uint32_t>(idx));
  }
  scene_.nodes.pop_back();
  scene_.dirty_marks.pop_back();

  if (!scene_.dirty_indices.empty()) {
    const uint32_t new_len = static_cast<uint32_t>(scene_.nodes.size());
    scene_.dirty_indices.erase(
        std::remove_if(
            scene_.dirty_indices.begin(),
            scene_.dirty_indices.end(),
            [new_len](uint32_t di) { return di >= new_len; }),
        scene_.dirty_indices.end());
  }
}

void FabricHost::upsert_scene_node(ShadowNodeEntry& entry) {
  if (entry.node_id == 0u) {
    return;
  }

  const uint8_t dirty_mask = entry.scene_dirty_mask;
  entry.scene_dirty_mask = 0u;

  const uint32_t existing_idx = scene_.find_index(entry.node_id);
  if (existing_idx != kInvalidSlot) {
    const size_t idx = static_cast<size_t>(existing_idx);
    NveRenderNode& dst = scene_.nodes[idx];

    // Fast path: layout-only change skips text_slab & children_slab checks completely.
    if (dirty_mask == kSceneDirtyLayout) {
      const float nx = sanitize_finite(entry.computed_layout.x);
      const float ny = sanitize_finite(entry.computed_layout.y);
      const float nw = std::max(0.0f, sanitize_finite(entry.computed_layout.width));
      const float nh = std::max(0.0f, sanitize_finite(entry.computed_layout.height));
      if (dst.x != nx || dst.y != ny || dst.width != nw || dst.height != nh) {
        dst.x = nx;
        dst.y = ny;
        dst.width = nw;
        dst.height = nh;
        mark_scene_index_dirty(idx);
      }
      return;
    }

    const NveRenderNode prev = dst;
    uint32_t text_offset = prev.text_offset;
    uint16_t text_len = prev.text_len;
    if ((dirty_mask & kSceneDirtyText) != 0u) {
      const std::string_view text = effective_node_text(entry);
      const size_t clamped_text_len =
          std::min<size_t>(text.size(), std::numeric_limits<uint16_t>::max());
      const uint16_t text_len_u16 = static_cast<uint16_t>(clamped_text_len);
      const size_t prev_start = static_cast<size_t>(prev.text_offset);
      const size_t prev_len = static_cast<size_t>(prev.text_len);
      const bool same_text =
          prev_len == clamped_text_len &&
          (prev_len == 0u ||
           (prev_start <= scene_.text_slab.size() &&
            prev_len <= scene_.text_slab.size() - prev_start &&
            std::memcmp(scene_.text_slab.data() + prev_start, text.data(), prev_len) == 0));
      if (!same_text) {
        if (scene_.live_text_bytes >= prev_len) {
          scene_.live_text_bytes -= prev_len;
        } else {
          scene_.live_text_bytes = 0u;
        }
        scene_.live_text_bytes += clamped_text_len;
        if (clamped_text_len > 0u) {
          text_offset = static_cast<uint32_t>(scene_.text_slab.size());
          text_len = text_len_u16;
          const auto* bytes = reinterpret_cast<const uint8_t*>(text.data());
          scene_.text_slab.insert(scene_.text_slab.end(), bytes, bytes + clamped_text_len);
        } else {
          text_offset = 0u;
          text_len = 0u;
        }
      }
    }

    uint32_t child_offset = prev.child_offset;
    uint16_t child_count = prev.child_count;
    if ((dirty_mask & kSceneDirtyChildren) != 0u) {
      const size_t clamped_child_count =
          std::min<size_t>(entry.children.size(), std::numeric_limits<uint16_t>::max());
      const uint16_t child_count_u16 = static_cast<uint16_t>(clamped_child_count);
      const size_t prev_start = static_cast<size_t>(prev.child_offset);
      const size_t prev_len = static_cast<size_t>(prev.child_count);
      const bool same_children =
          prev_len == clamped_child_count &&
          (prev_len == 0u ||
           (prev_start <= scene_.children_slab.size() &&
            prev_len <= scene_.children_slab.size() - prev_start &&
            std::memcmp(
                scene_.children_slab.data() + prev_start,
                entry.children.data(),
                prev_len * sizeof(uint32_t)) == 0));
      if (!same_children) {
        if (scene_.live_children_len >= prev_len) {
          scene_.live_children_len -= prev_len;
        } else {
          scene_.live_children_len = 0u;
        }
        scene_.live_children_len += clamped_child_count;
        if (clamped_child_count > 0u) {
          child_offset = static_cast<uint32_t>(scene_.children_slab.size());
          child_count = child_count_u16;
          scene_.children_slab.insert(
              scene_.children_slab.end(),
              entry.children.begin(),
              entry.children.begin() + static_cast<std::ptrdiff_t>(clamped_child_count));
        } else {
          child_offset = 0u;
          child_count = 0u;
        }
      }
    }

    const NveRenderNode next =
        build_render_node(entry, text_offset, text_len, child_offset, child_count);
    if (std::memcmp(&prev, &next, sizeof(NveRenderNode)) != 0) {
      dst = next;
      mark_scene_index_dirty(idx);
    }
    return;
  }

  const std::string_view text = effective_node_text(entry);
  const size_t clamped_text_len =
      std::min<size_t>(text.size(), std::numeric_limits<uint16_t>::max());
  const uint16_t text_len_u16 = static_cast<uint16_t>(clamped_text_len);

  const size_t clamped_child_count =
      std::min<size_t>(entry.children.size(), std::numeric_limits<uint16_t>::max());
  const uint16_t child_count_u16 = static_cast<uint16_t>(clamped_child_count);

  uint32_t text_offset = 0u;
  if (clamped_text_len > 0u) {
    text_offset = static_cast<uint32_t>(scene_.text_slab.size());
    const auto* bytes = reinterpret_cast<const uint8_t*>(text.data());
    scene_.text_slab.insert(scene_.text_slab.end(), bytes, bytes + clamped_text_len);
    scene_.live_text_bytes += clamped_text_len;
  }

  uint32_t child_offset = 0u;
  if (clamped_child_count > 0u) {
    child_offset = static_cast<uint32_t>(scene_.children_slab.size());
    scene_.children_slab.insert(
        scene_.children_slab.end(),
        entry.children.begin(),
        entry.children.begin() + static_cast<std::ptrdiff_t>(clamped_child_count));
    scene_.live_children_len += clamped_child_count;
  }

  const size_t idx = scene_.nodes.size();
  scene_.nodes.push_back(
      build_render_node(entry, text_offset, text_len_u16, child_offset, child_count_u16));
  scene_.dirty_marks.push_back(0u);
  scene_.set_index(entry.node_id, static_cast<uint32_t>(idx));
  mark_scene_index_dirty(idx);
}

void FabricHost::compact_scene_text_slab() {
  std::vector<uint8_t> compacted;
  compacted.reserve(scene_.live_text_bytes);
  for (auto& node : scene_.nodes) {
    if (node.text_len == 0u) {
      node.text_offset = 0u;
      continue;
    }
    const size_t start = static_cast<size_t>(node.text_offset);
    const size_t len = static_cast<size_t>(node.text_len);
    const uint32_t new_off = static_cast<uint32_t>(compacted.size());
    compacted.insert(
        compacted.end(),
        scene_.text_slab.begin() + static_cast<std::ptrdiff_t>(start),
        scene_.text_slab.begin() + static_cast<std::ptrdiff_t>(start + len));
    node.text_offset = new_off;
  }
  scene_.live_text_bytes = compacted.size();
  scene_.text_slab = std::move(compacted);
}

void FabricHost::compact_scene_children_slab() {
  std::vector<uint32_t> compacted;
  compacted.reserve(scene_.live_children_len);
  for (auto& node : scene_.nodes) {
    if (node.child_count == 0u) {
      node.child_offset = 0u;
      continue;
    }
    const size_t start = static_cast<size_t>(node.child_offset);
    const size_t len = static_cast<size_t>(node.child_count);
    const uint32_t new_off = static_cast<uint32_t>(compacted.size());
    compacted.insert(
        compacted.end(),
        scene_.children_slab.begin() + static_cast<std::ptrdiff_t>(start),
        scene_.children_slab.begin() + static_cast<std::ptrdiff_t>(start + len));
    node.child_offset = new_off;
  }
  scene_.live_children_len = compacted.size();
  scene_.children_slab = std::move(compacted);
}

void FabricHost::maybe_compact_scene_slabs() {
  if (scene_.text_slab.size() > 65536u && scene_.text_slab.size() > scene_.live_text_bytes * 2u) {
    compact_scene_text_slab();
  }
  if (scene_.children_slab.size() > 16384u &&
      scene_.children_slab.size() > scene_.live_children_len * 2u) {
    compact_scene_children_slab();
  }
}

void FabricHost::publish_scene_frame(uint64_t frame_id) {
  scene_pending_publish_ = false;
  scene_.frame_id = frame_id;
  scene_.mark_epoch += 1u;
  if (scene_.mark_epoch == 0u) {
    std::fill(scene_.dirty_marks.begin(), scene_.dirty_marks.end(), 0u);
    scene_.mark_epoch = 1u;
  }
  scene_.dirty_indices.clear();
  scene_.removed_ids.clear();
  scene_.initialized = true;

  for (uint32_t rid : frame_removed_nodes_) {
    if (nodes_.find(rid) == nullptr) {
      remove_scene_node(rid);
    }
  }
  frame_removed_nodes_.clear();

  for (uint32_t nid : frame_dirty_nodes_) {
    if (ShadowNodeEntry* entry = nodes_.find(nid)) {
      upsert_scene_node(*entry);
    }
  }
  frame_dirty_nodes_.clear();

  ++scene_dirty_epoch_;
  if (scene_dirty_epoch_ == 0u) {
    for (auto& entry : nodes_.entries()) {
      entry.scene_dirty_gen = 0u;
    }
    scene_dirty_epoch_ = 1u;
  }

  maybe_compact_scene_slabs();
}

NveSceneView FabricHost::scene_view() const noexcept {
  if (scene_pending_publish_) {
    const_cast<FabricHost*>(this)->publish_scene_frame(scene_.frame_id);
  }
  return NveSceneView{
      .schema_hash = NVE_SCENE_SCHEMA_HASH,
      .frame_id = scene_.frame_id,
      .node_count = static_cast<uint32_t>(scene_.nodes.size()),
      .dirty_count = static_cast<uint32_t>(scene_.dirty_indices.size()),
      .removed_count = static_cast<uint32_t>(scene_.removed_ids.size()),
      .text_slab_len = static_cast<uint32_t>(scene_.text_slab.size()),
      .children_slab_len = static_cast<uint32_t>(scene_.children_slab.size()),
      ._pad = 0u,
      .nodes = scene_.nodes.empty() ? nullptr : scene_.nodes.data(),
      .text_slab = scene_.text_slab.empty() ? nullptr : scene_.text_slab.data(),
      .children_slab = scene_.children_slab.empty() ? nullptr : scene_.children_slab.data(),
      .dirty_indices = scene_.dirty_indices.empty() ? nullptr : scene_.dirty_indices.data(),
      .removed_ids = scene_.removed_ids.empty() ? nullptr : scene_.removed_ids.data(),
  };
}

NveSceneView fabric_host_scene_view(NveFabricHostHandle host) noexcept {
  if (!host) {
    return NveSceneView{
        .schema_hash = NVE_SCENE_SCHEMA_HASH,
        .frame_id = 0u,
        .node_count = 0u,
        .dirty_count = 0u,
        .removed_count = 0u,
        .text_slab_len = 0u,
        .children_slab_len = 0u,
        ._pad = 0u,
        .nodes = nullptr,
        .text_slab = nullptr,
        .children_slab = nullptr,
        .dirty_indices = nullptr,
        .removed_ids = nullptr,
    };
  }
  return static_cast<const FabricHost*>(host)->scene_view();
}

} // namespace nve
