#pragma once

#include "nve_rn_component.hpp"
#include "nve_scene_abi.h"

#include <algorithm>
#include <cmath>
#include <cstddef>
#include <cstdint>
#include <cstring>
#include <limits>
#include <string_view>
#include <unordered_map>
#include <vector>

namespace nve {

/// Incremental C++20 builder translating `nve::RenderedNodeSnapshot` frames into a zero-decode
/// 64-byte `NveRenderNode[]` + flat slabs (`text_slab`, `children_slab`, `dirty_indices`,
/// `removed_ids`) exposed via `NveSceneView`.
class SceneTableBuilder {
 public:
  SceneTableBuilder() = default;

  static constexpr uint8_t map_component_kind(uint16_t kind) noexcept {
    return kind <= 7u ? static_cast<uint8_t>(kind) : NVE_NODE_KIND_CUSTOM;
  }

  static uint8_t quantize_opacity(float opacity) noexcept {
    if (!std::isfinite(opacity)) {
      return 255u;
    }
    const float clamped = std::clamp(opacity, 0.0f, 1.0f);
    return static_cast<uint8_t>(std::lround(clamped * 255.0f));
  }

  static constexpr bool verify_view_schema(const NveSceneView& view) noexcept {
    return view.schema_hash == NVE_SCENE_SCHEMA_HASH;
  }

  void clear() noexcept {
    frame_id_ = 0;
    nodes_.clear();
    id_to_index_.clear();
    text_slab_.clear();
    children_slab_.clear();
    dirty_indices_.clear();
    dirty_marks_.clear();
    seen_marks_.clear();
    removed_ids_.clear();
    mark_epoch_ = 0;
    live_text_bytes_ = 0;
    live_children_len_ = 0;
  }

  /// Synchronizes `snapshots` for `frame_id`, populating exact `dirty_indices` and `removed_ids`
  /// in `O(nodes)` with zero per-frame JSON or string re-allocation for unchanged text.
  void sync(uint64_t frame_id, const std::vector<RenderedNodeSnapshot>& snapshots) {
    frame_id_ = frame_id;
    mark_epoch_ += 1u;
    if (mark_epoch_ == 0u) {
      std::fill(dirty_marks_.begin(), dirty_marks_.end(), 0u);
      std::fill(seen_marks_.begin(), seen_marks_.end(), 0u);
      mark_epoch_ = 1u;
    }
    dirty_indices_.clear();
    removed_ids_.clear();

    for (const auto& snap : snapshots) {
      if (snap.node_id == 0u) {
        continue;
      }
      upsert_snapshot(snap);
    }

    // Swap-remove any previously live node not seen in this frame.
    size_t i = 0;
    while (i < nodes_.size()) {
      if (seen_marks_[i] != mark_epoch_) {
        remove_at_index(i);
      } else {
        ++i;
      }
    }

    maybe_compact_slabs();
  }

  /// Forces immediate compaction of dead slab bytes while keeping live offsets valid.
  void compact_slabs() {
    compact_text_slab();
    compact_children_slab();
  }

  /// Returns a zero-copy C ABI `NveSceneView` pointing directly into this builder's slabs.
  NveSceneView view() const noexcept {
    return NveSceneView{
        .schema_hash = NVE_SCENE_SCHEMA_HASH,
        .frame_id = frame_id_,
        .node_count = static_cast<uint32_t>(nodes_.size()),
        .dirty_count = static_cast<uint32_t>(dirty_indices_.size()),
        .removed_count = static_cast<uint32_t>(removed_ids_.size()),
        .text_slab_len = static_cast<uint32_t>(text_slab_.size()),
        .children_slab_len = static_cast<uint32_t>(children_slab_.size()),
        ._pad = 0u,
        .nodes = nodes_.empty() ? nullptr : nodes_.data(),
        .text_slab = text_slab_.empty() ? nullptr : text_slab_.data(),
        .children_slab = children_slab_.empty() ? nullptr : children_slab_.data(),
        .dirty_indices = dirty_indices_.empty() ? nullptr : dirty_indices_.data(),
        .removed_ids = removed_ids_.empty() ? nullptr : removed_ids_.data(),
    };
  }

  const std::vector<NveRenderNode>& nodes() const noexcept { return nodes_; }
  const std::vector<uint8_t>& text_slab() const noexcept { return text_slab_; }
  const std::vector<uint32_t>& children_slab() const noexcept { return children_slab_; }
  const std::vector<uint32_t>& dirty_indices() const noexcept { return dirty_indices_; }
  const std::vector<uint32_t>& removed_ids() const noexcept { return removed_ids_; }
  uint64_t frame_id() const noexcept { return frame_id_; }

  std::string_view text_for(const NveRenderNode& node) const noexcept {
    if (node.text_len == 0u) {
      return {};
    }
    const size_t start = static_cast<size_t>(node.text_offset);
    const size_t len = static_cast<size_t>(node.text_len);
    if (start > text_slab_.size() || len > text_slab_.size() - start) {
      return {};
    }
    return std::string_view(reinterpret_cast<const char*>(text_slab_.data() + start), len);
  }

 private:
  static float sanitize_finite(float v) noexcept { return std::isfinite(v) ? v : 0.0f; }

  static float sanitize_positive(float v, float fallback) noexcept {
    return (std::isfinite(v) && v > 0.0f) ? v : fallback;
  }

  void mark_dirty(size_t idx) {
    if (dirty_marks_[idx] != mark_epoch_) {
      dirty_marks_[idx] = mark_epoch_;
      dirty_indices_.push_back(static_cast<uint32_t>(idx));
    }
  }

  static NveRenderNode build_node_record(
      const RenderedNodeSnapshot& snap,
      uint32_t text_offset,
      uint16_t text_len,
      uint32_t child_offset,
      uint16_t child_count) noexcept {
    const float border_radius = std::max(0.0f, sanitize_finite(snap.border_radius));
    const float border_width = std::max(0.0f, sanitize_finite(snap.border_width));
    uint8_t flags = 0u;
    if (snap.kind == NVE_COMP_TEXT && child_count == 0u) {
      flags |= NVE_NODE_LEAF_TEXT;
    }
    if (snap.kind == NVE_COMP_PRESSABLE || snap.event_mask != 0u) {
      flags |= NVE_NODE_INTERACTIVE;
    }
    if (border_radius > 0.0f || snap.kind == NVE_COMP_SCROLL_VIEW) {
      flags |= NVE_NODE_CLIP_BOUNDS;
    }

    NveRenderNode node{};
    node.node_id = snap.node_id;
    node.kind = map_component_kind(snap.kind);
    node.flags = flags;
    node.opacity_u8 = quantize_opacity(snap.opacity);
    node._pad0 = 0u;
    node.x = sanitize_finite(snap.layout.x);
    node.y = sanitize_finite(snap.layout.y);
    node.width = std::max(0.0f, sanitize_finite(snap.layout.width));
    node.height = std::max(0.0f, sanitize_finite(snap.layout.height));
    node.bg_rgba = snap.background_rgba;
    node.text_rgba = snap.text_color_rgba;
    node.border_rgba = snap.border_color_rgba;
    node.border_radius = border_radius;
    node.border_width = border_width;
    node.font_size = sanitize_positive(snap.font_size, 16.0f);
    node.text_offset = text_offset;
    node.text_len = text_len;
    node.child_count = child_count;
    node.child_offset = child_offset;
    node.z_index = snap.z_index;
    return node;
  }

  void upsert_snapshot(const RenderedNodeSnapshot& snap) {
    const size_t clamped_text_len =
        std::min<size_t>(snap.text.size(), std::numeric_limits<uint16_t>::max());
    const uint16_t text_len_u16 = static_cast<uint16_t>(clamped_text_len);

    const size_t clamped_child_count =
        std::min<size_t>(snap.children.size(), std::numeric_limits<uint16_t>::max());
    const uint16_t child_count_u16 = static_cast<uint16_t>(clamped_child_count);

    auto it = id_to_index_.find(snap.node_id);
    if (it != id_to_index_.end()) {
      const size_t idx = static_cast<size_t>(it->second);
      const NveRenderNode prev = nodes_[idx];
      seen_marks_[idx] = mark_epoch_;

      uint32_t text_offset = 0u;
      uint16_t text_len = 0u;
      {
        const size_t prev_start = static_cast<size_t>(prev.text_offset);
        const size_t prev_len = static_cast<size_t>(prev.text_len);
        const bool same_text =
            prev_len == clamped_text_len &&
            (prev_len == 0u ||
             (prev_start <= text_slab_.size() && prev_len <= text_slab_.size() - prev_start &&
              std::memcmp(text_slab_.data() + prev_start, snap.text.data(), prev_len) == 0));
        if (same_text) {
          text_offset = prev.text_offset;
          text_len = prev.text_len;
        } else {
          if (live_text_bytes_ >= prev_len) {
            live_text_bytes_ -= prev_len;
          } else {
            live_text_bytes_ = 0u;
          }
          live_text_bytes_ += clamped_text_len;
          if (clamped_text_len > 0u) {
            text_offset = static_cast<uint32_t>(text_slab_.size());
            text_len = text_len_u16;
            const auto* bytes = reinterpret_cast<const uint8_t*>(snap.text.data());
            text_slab_.insert(text_slab_.end(), bytes, bytes + clamped_text_len);
          }
        }
      }

      uint32_t child_offset = 0u;
      uint16_t child_count = 0u;
      {
        const size_t prev_start = static_cast<size_t>(prev.child_offset);
        const size_t prev_len = static_cast<size_t>(prev.child_count);
        const bool same_children =
            prev_len == clamped_child_count &&
            (prev_len == 0u ||
             (prev_start <= children_slab_.size() &&
              prev_len <= children_slab_.size() - prev_start &&
              std::memcmp(
                  children_slab_.data() + prev_start,
                  snap.children.data(),
                  prev_len * sizeof(uint32_t)) == 0));
        if (same_children) {
          child_offset = prev.child_offset;
          child_count = prev.child_count;
        } else {
          if (live_children_len_ >= prev_len) {
            live_children_len_ -= prev_len;
          } else {
            live_children_len_ = 0u;
          }
          live_children_len_ += clamped_child_count;
          if (clamped_child_count > 0u) {
            child_offset = static_cast<uint32_t>(children_slab_.size());
            child_count = child_count_u16;
            children_slab_.insert(
                children_slab_.end(),
                snap.children.begin(),
                snap.children.begin() + static_cast<std::ptrdiff_t>(clamped_child_count));
          }
        }
      }

      const NveRenderNode next =
          build_node_record(snap, text_offset, text_len, child_offset, child_count);
      if (std::memcmp(&prev, &next, sizeof(NveRenderNode)) != 0) {
        nodes_[idx] = next;
        mark_dirty(idx);
      }
      return;
    }

    uint32_t text_offset = 0u;
    if (clamped_text_len > 0u) {
      text_offset = static_cast<uint32_t>(text_slab_.size());
      const auto* bytes = reinterpret_cast<const uint8_t*>(snap.text.data());
      text_slab_.insert(text_slab_.end(), bytes, bytes + clamped_text_len);
      live_text_bytes_ += clamped_text_len;
    }

    uint32_t child_offset = 0u;
    if (clamped_child_count > 0u) {
      child_offset = static_cast<uint32_t>(children_slab_.size());
      children_slab_.insert(
          children_slab_.end(),
          snap.children.begin(),
          snap.children.begin() + static_cast<std::ptrdiff_t>(clamped_child_count));
      live_children_len_ += clamped_child_count;
    }

    const size_t idx = nodes_.size();
    nodes_.push_back(
        build_node_record(snap, text_offset, text_len_u16, child_offset, child_count_u16));
    dirty_marks_.push_back(0u);
    seen_marks_.push_back(mark_epoch_);
    id_to_index_.emplace(snap.node_id, static_cast<uint32_t>(idx));
    mark_dirty(idx);
  }

  void remove_at_index(size_t idx) {
    const NveRenderNode removed = nodes_[idx];
    id_to_index_.erase(removed.node_id);
    removed_ids_.push_back(removed.node_id);

    if (live_text_bytes_ >= removed.text_len) {
      live_text_bytes_ -= removed.text_len;
    } else {
      live_text_bytes_ = 0u;
    }
    if (live_children_len_ >= removed.child_count) {
      live_children_len_ -= removed.child_count;
    } else {
      live_children_len_ = 0u;
    }

    const size_t last_idx = nodes_.size() - 1u;
    if (idx != last_idx) {
      nodes_[idx] = nodes_[last_idx];
      seen_marks_[idx] = seen_marks_[last_idx];
      dirty_marks_[idx] = 0u;
      id_to_index_[nodes_[idx].node_id] = static_cast<uint32_t>(idx);
    }
    nodes_.pop_back();
    dirty_marks_.pop_back();
    seen_marks_.pop_back();

    const uint32_t new_len = static_cast<uint32_t>(nodes_.size());
    dirty_indices_.erase(
        std::remove_if(
            dirty_indices_.begin(),
            dirty_indices_.end(),
            [new_len](uint32_t di) { return di >= new_len; }),
        dirty_indices_.end());
    if (idx < nodes_.size()) {
      mark_dirty(idx);
    }
  }

  void maybe_compact_slabs() {
    if (text_slab_.size() > 65536u && text_slab_.size() > live_text_bytes_ * 2u) {
      compact_text_slab();
    }
    if (children_slab_.size() > 16384u && children_slab_.size() > live_children_len_ * 2u) {
      compact_children_slab();
    }
  }

  void compact_text_slab() {
    std::vector<uint8_t> compacted;
    compacted.reserve(live_text_bytes_);
    for (auto& node : nodes_) {
      if (node.text_len == 0u) {
        node.text_offset = 0u;
        continue;
      }
      const size_t start = static_cast<size_t>(node.text_offset);
      const size_t len = static_cast<size_t>(node.text_len);
      const uint32_t new_off = static_cast<uint32_t>(compacted.size());
      compacted.insert(
          compacted.end(),
          text_slab_.begin() + static_cast<std::ptrdiff_t>(start),
          text_slab_.begin() + static_cast<std::ptrdiff_t>(start + len));
      node.text_offset = new_off;
    }
    live_text_bytes_ = compacted.size();
    text_slab_ = std::move(compacted);
  }

  void compact_children_slab() {
    std::vector<uint32_t> compacted;
    compacted.reserve(live_children_len_);
    for (auto& node : nodes_) {
      if (node.child_count == 0u) {
        node.child_offset = 0u;
        continue;
      }
      const size_t start = static_cast<size_t>(node.child_offset);
      const size_t len = static_cast<size_t>(node.child_count);
      const uint32_t new_off = static_cast<uint32_t>(compacted.size());
      compacted.insert(
          compacted.end(),
          children_slab_.begin() + static_cast<std::ptrdiff_t>(start),
          children_slab_.begin() + static_cast<std::ptrdiff_t>(start + len));
      node.child_offset = new_off;
    }
    live_children_len_ = compacted.size();
    children_slab_ = std::move(compacted);
  }

  uint64_t frame_id_{0};
  std::vector<NveRenderNode> nodes_;
  std::unordered_map<uint32_t, uint32_t> id_to_index_;
  std::vector<uint8_t> text_slab_;
  std::vector<uint32_t> children_slab_;
  std::vector<uint32_t> dirty_indices_;
  std::vector<uint64_t> dirty_marks_;
  std::vector<uint64_t> seen_marks_;
  std::vector<uint32_t> removed_ids_;
  uint64_t mark_epoch_{0};
  size_t live_text_bytes_{0};
  size_t live_children_len_{0};
};

} // namespace nve
