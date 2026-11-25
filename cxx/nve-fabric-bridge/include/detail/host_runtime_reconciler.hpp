#pragma once

#include "nve_abi.generated.h"
#include "nve_rn_component.hpp"

#include <cstddef>
#include <cstdint>
#include <span>
#include <string_view>
#include <unordered_map>
#include <unordered_set>
#include <vector>

namespace nve {

/// Compute a retained-view type tag distinguishing wrapper `NVE_COMP_TEXT` (`!children.empty()`)
/// from leaf `NVE_COMP_TEXT`.
inline uint32_t view_tag_for_snapshot(const RenderedNodeSnapshot& snap) noexcept {
  const uint32_t is_wrapper_text =
      (snap.kind == NVE_COMP_TEXT && !snap.children.empty()) ? 0x10000u : 0u;
  return static_cast<uint32_t>(snap.kind) | is_wrapper_text;
}

/// Platform-agnostic 3-pass retained native view reconciler shared across UIKit/AppKit,
/// WinUI/RNW, Qt 6, and GTK4 host shells.
///
/// `Adapter` must provide:
/// - `void update_root(const RenderedNodeSnapshot& snap)`
/// - `ViewHandle create_view(const RenderedNodeSnapshot& snap)`
/// - `void update_view(ViewHandle& view, const RenderedNodeSnapshot& snap)`
/// - `void destroy_view(uint32_t node_id, ViewHandle& view)`
/// - `template <typename LookupFn> void sync_children(uint32_t parent_id, ViewHandle* parent_view,
///       const std::vector<uint32_t>& child_ids, LookupFn&& lookup_fn)`
template <typename ViewHandle, typename Adapter>
class RetainedViewReconciler {
 public:
  void sync(const std::vector<RenderedNodeSnapshot>& snapshots, Adapter& adapter) {
    // Pass 1: Prune removed node IDs.
    std::unordered_set<uint32_t> live_ids;
    live_ids.reserve(snapshots.size());
    for (const auto& snap : snapshots) {
      live_ids.insert(snap.node_id);
    }
    for (auto it = views_.begin(); it != views_.end();) {
      if (live_ids.find(it->first) == live_ids.end()) {
        adapter.destroy_view(it->first, it->second);
        tags_.erase(it->first);
        it = views_.erase(it);
      } else {
        ++it;
      }
    }

    // Pass 2: Update root and create/reuse/update retained views.
    for (const auto& snap : snapshots) {
      if (snap.node_id == NVE_ROOT_NODE_ID) {
        adapter.update_root(snap);
        continue;
      }

      const uint32_t tag = view_tag_for_snapshot(snap);
      auto existing = views_.find(snap.node_id);
      auto tag_it = tags_.find(snap.node_id);
      if (existing == views_.end() || tag_it == tags_.end() || tag_it->second != tag) {
        if (existing != views_.end()) {
          adapter.destroy_view(snap.node_id, existing->second);
          views_.erase(existing);
          tags_.erase(snap.node_id);
        }
        auto [inserted, ok] = views_.emplace(snap.node_id, adapter.create_view(snap));
        (void)ok;
        tags_[snap.node_id] = tag;
        existing = inserted;
      }

      adapter.update_view(existing->second, snap);
    }

    // Pass 3: Reconcile parent-child hierarchy in snapshot child order.
    auto lookup_fn = [this](uint32_t child_id) -> ViewHandle* { return this->find(child_id); };
    for (const auto& snap : snapshots) {
      ViewHandle* parent_view =
          (snap.node_id == NVE_ROOT_NODE_ID) ? nullptr : find(snap.node_id);
      adapter.sync_children(snap.node_id, parent_view, snap.children, lookup_fn);
    }
  }

  void clear(Adapter& adapter) {
    for (auto& [node_id, view] : views_) {
      adapter.destroy_view(node_id, view);
    }
    views_.clear();
    tags_.clear();
  }

  ViewHandle* find(uint32_t node_id) noexcept {
    auto it = views_.find(node_id);
    return (it != views_.end()) ? &it->second : nullptr;
  }

  const ViewHandle* find(uint32_t node_id) const noexcept {
    auto it = views_.find(node_id);
    return (it != views_.end()) ? &it->second : nullptr;
  }

  size_t size() const noexcept { return views_.size(); }
  bool empty() const noexcept { return views_.empty(); }

 private:
  std::unordered_map<uint32_t, ViewHandle> views_;
  std::unordered_map<uint32_t, uint32_t> tags_;
};

/// Zero-copy UTF-8 text (or image `src` when `kind == NVE_NODE_KIND_IMAGE`) slice from `NveSceneView`.
inline std::string_view scene_node_text(
    const NveSceneView& view,
    const NveRenderNode& node) noexcept {
  if (node.text_len == 0u || view.text_slab == nullptr) {
    return {};
  }
  const size_t start = static_cast<size_t>(node.text_offset);
  const size_t len = static_cast<size_t>(node.text_len);
  if (start > static_cast<size_t>(view.text_slab_len) ||
      len > static_cast<size_t>(view.text_slab_len) - start) {
    return {};
  }
  return std::string_view(reinterpret_cast<const char*>(view.text_slab + start), len);
}

/// Zero-copy child ID span from `NveSceneView`.
inline std::span<const uint32_t> scene_node_children(
    const NveSceneView& view,
    const NveRenderNode& node) noexcept {
  if (node.child_count == 0u || view.children_slab == nullptr) {
    return {};
  }
  const size_t start = static_cast<size_t>(node.child_offset);
  const size_t count = static_cast<size_t>(node.child_count);
  if (start > static_cast<size_t>(view.children_slab_len) ||
      count > static_cast<size_t>(view.children_slab_len) - start) {
    return {};
  }
  return std::span<const uint32_t>(view.children_slab + start, count);
}

/// Compute a retained-view type tag from a flat 64-byte `NveRenderNode`, distinguishing
/// wrapper `NVE_NODE_KIND_TEXT` (`(flags & NVE_NODE_LEAF_TEXT) == 0`) from leaf text.
inline uint32_t view_tag_for_render_node(const NveRenderNode& node) noexcept {
  const uint32_t is_wrapper_text =
      (node.kind == NVE_NODE_KIND_TEXT && (node.flags & NVE_NODE_LEAF_TEXT) == 0u) ? 0x10000u : 0u;
  return static_cast<uint32_t>(node.kind) | is_wrapper_text;
}

/// Incremental `O(dirty + removed)` retained native view reconciler operating directly over
/// `NveSceneView` with zero `RenderedNodeSnapshot` materialization.
///
/// `Adapter` must provide:
/// - `void update_root(const NveSceneView& view, const NveRenderNode& node)`
/// - `ViewHandle create_view(const NveSceneView& view, const NveRenderNode& node)`
/// - `void update_view(const NveSceneView& view, ViewHandle& view_handle, const NveRenderNode& node)`
/// - `void destroy_view(uint32_t node_id, ViewHandle& view_handle)`
/// - `template <typename LookupFn> void sync_children(uint32_t parent_id, ViewHandle* parent_view,
///       std::span<const uint32_t> child_ids, LookupFn&& lookup_fn)`
template <typename ViewHandle, typename Adapter>
class IncrementalSceneReconciler {
 public:
  bool sync(const NveSceneView& view, Adapter& adapter) {
    if (view.schema_hash != NVE_SCENE_SCHEMA_HASH) {
      return false;
    }
    if ((view.node_count > 0u && view.nodes == nullptr) ||
        (view.dirty_count > 0u && view.dirty_indices == nullptr) ||
        (view.removed_count > 0u && view.removed_ids == nullptr)) {
      return false;
    }

    auto lookup_fn = [this](uint32_t child_id) -> ViewHandle* { return this->find(child_id); };

    if (!initialized_ || view.frame_id < last_frame_id_) {
      // Full reconciliation pass on initial attach or host/frame reset (e.g. hot-reload).
      std::unordered_set<uint32_t> live_ids;
      live_ids.reserve(view.node_count);
      for (uint32_t i = 0u; i < view.node_count; ++i) {
        live_ids.insert(view.nodes[i].node_id);
      }
      for (auto it = views_.begin(); it != views_.end();) {
        if (live_ids.find(it->first) == live_ids.end()) {
          adapter.destroy_view(it->first, it->second);
          tags_.erase(it->first);
          it = views_.erase(it);
        } else {
          ++it;
        }
      }

      for (uint32_t i = 0u; i < view.node_count; ++i) {
        upsert_node(view, view.nodes[i], adapter);
      }
      for (uint32_t i = 0u; i < view.node_count; ++i) {
        const NveRenderNode& node = view.nodes[i];
        ViewHandle* parent_view =
            (node.node_id == NVE_ROOT_NODE_ID) ? nullptr : find(node.node_id);
        adapter.sync_children(node.node_id, parent_view, scene_node_children(view, node), lookup_fn);
      }
      initialized_ = true;
      last_frame_id_ = view.frame_id;
      return true;
    }

    // Incremental O(removed + dirty) steady-state reconciliation.
    // Pass 1: Destroy explicitly removed nodes.
    for (uint32_t i = 0u; i < view.removed_count; ++i) {
      const uint32_t rid = view.removed_ids[i];
      auto it = views_.find(rid);
      if (it != views_.end()) {
        adapter.destroy_view(it->first, it->second);
        tags_.erase(it->first);
        views_.erase(it);
      }
    }

    // Pass 2: Create/reuse/update only dirty nodes.
    for (uint32_t i = 0u; i < view.dirty_count; ++i) {
      const uint32_t idx = view.dirty_indices[i];
      if (idx >= view.node_count) {
        continue;
      }
      upsert_node(view, view.nodes[idx], adapter);
    }

    // Pass 3: Reconcile child order only for dirty parents.
    for (uint32_t i = 0u; i < view.dirty_count; ++i) {
      const uint32_t idx = view.dirty_indices[i];
      if (idx >= view.node_count) {
        continue;
      }
      const NveRenderNode& node = view.nodes[idx];
      ViewHandle* parent_view =
          (node.node_id == NVE_ROOT_NODE_ID) ? nullptr : find(node.node_id);
      adapter.sync_children(node.node_id, parent_view, scene_node_children(view, node), lookup_fn);
    }

    last_frame_id_ = view.frame_id;
    return true;
  }

  void clear(Adapter& adapter) {
    for (auto& [node_id, view] : views_) {
      adapter.destroy_view(node_id, view);
    }
    views_.clear();
    tags_.clear();
    initialized_ = false;
    last_frame_id_ = 0u;
  }

  ViewHandle* find(uint32_t node_id) noexcept {
    auto it = views_.find(node_id);
    return (it != views_.end()) ? &it->second : nullptr;
  }

  const ViewHandle* find(uint32_t node_id) const noexcept {
    auto it = views_.find(node_id);
    return (it != views_.end()) ? &it->second : nullptr;
  }

  size_t size() const noexcept { return views_.size(); }
  bool empty() const noexcept { return views_.empty(); }
  uint64_t last_frame_id() const noexcept { return last_frame_id_; }

 private:
  void upsert_node(const NveSceneView& view, const NveRenderNode& node, Adapter& adapter) {
    if (node.node_id == NVE_ROOT_NODE_ID) {
      adapter.update_root(view, node);
      return;
    }

    const uint32_t tag = view_tag_for_render_node(node);
    auto existing = views_.find(node.node_id);
    auto tag_it = tags_.find(node.node_id);
    if (existing == views_.end() || tag_it == tags_.end() || tag_it->second != tag) {
      if (existing != views_.end()) {
        adapter.destroy_view(node.node_id, existing->second);
        views_.erase(existing);
        tags_.erase(node.node_id);
      }
      auto [inserted, ok] = views_.emplace(node.node_id, adapter.create_view(view, node));
      (void)ok;
      tags_[node.node_id] = tag;
      existing = inserted;
    }

    adapter.update_view(view, existing->second, node);
  }

  std::unordered_map<uint32_t, ViewHandle> views_;
  std::unordered_map<uint32_t, uint32_t> tags_;
  bool initialized_{false};
  uint64_t last_frame_id_{0};
};

} // namespace nve
