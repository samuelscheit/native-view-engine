// NVE host runtime: O(changed) retained native-view reconciler over `NveRuntimeSceneView`.
//
// Mirrors the published scene into one native view per live node through an `Adapter`:
//
//   using Handle = ...;                                        // default-constructible, movable
//   uint32_t tag(const NodeData&);                             // view type; a change recreates it
//   Handle create(const NodeData&);                            // every non-root node
//   void update(Handle&, const NodeData&);                     // after create and on every change
//   void update_root(const NodeData&);                         // root node (the host's own view)
//   void destroy(uint32_t id, Handle&);
//   void set_children(const NodeData& parent, Handle* parent_view /* null for root */,
//                     std::span<Handle* const> kids);           // in scene child order
//
// The first sync (and any `frame_id` regression, e.g. a new app) visits every live node;
// afterwards only `removed_ids`, `dirty_indices` (row, tables, children, or layout rect changed),
// and descendants whose inherited typography changed. Views are keyed by node index and checked
// against the packed `NodeId` generation, so a recycled slot recreates its view.
#pragma once

#include <cstdint>
#include <span>
#include <vector>

#include "bound_table.hpp"
#include "runtime_session.hpp"
#include "scene_nodes.hpp"

namespace nve::host {

template <class Adapter>
class RetainedTree {
 public:
  using Handle = typename Adapter::Handle;

  /// Reconciles `scene` (bound into `tables`); returns the number of views created or updated.
  size_t sync(const RuntimeSceneReader& scene, SceneTables& tables, Adapter& adapter) {
    tables.bind(scene);
    if (++epoch_ == 0u) {
      for (Entry& e : entries_) e.stamp = 0u;
      epoch_ = 1u;
    }
    const bool full = !initialized_ || scene.frame_id() < last_frame_;
    if (!full && scene.frame_id() == last_frame_) return 0u;  // this frame is already mirrored
    initialized_ = true;
    last_frame_ = scene.frame_id();
    if (entries_.size() < scene.node_capacity()) entries_.resize(scene.node_capacity());
    updated_ = 0u;
    work_.clear();

    if (full) {
      for (uint32_t i = 0; i < entries_.size(); ++i) {
        const NveNodeRow* row = scene.is_live(i) ? scene.row(i) : nullptr;
        if (entries_[i].live && (row == nullptr || node_generation(entries_[i].id) != row->generation)) {
          destroy(i, adapter);
        }
        if (row != nullptr && i != 0u) work_.push_back(i);
      }
    } else {
      for (const uint32_t id : scene.removed_ids()) {
        const uint32_t i = node_index(id);
        if (i < entries_.size() && entries_[i].live && entries_[i].id == id) destroy(i, adapter);
      }
      for (const uint32_t i : scene.dirty_indices()) {
        if (i < entries_.size() && scene.is_live(i)) work_.push_back(i);
      }
    }

    for (const uint32_t i : work_) visit(scene, tables, adapter, i);
    // Inherited typography: a changed resolved style re-renders descendants until it stops changing.
    for (size_t q = 0; q < cascade_.size(); ++q) {
      const uint32_t i = cascade_[q];
      if (entries_[i].stamp != epoch_ && scene.is_live(i)) visit(scene, tables, adapter, i);
    }
    cascade_.clear();
    // Children of every dirty node, plus the parents of views (re)created under a clean parent.
    for (const uint32_t i : work_) relink_.push_back(i);
    for (const uint32_t i : relink_) {
      if (i < entries_.size() && entries_[i].linked != epoch_) {
        entries_[i].linked = epoch_;
        sync_children(scene, tables, adapter, i);
      }
    }
    relink_.clear();
    return updated_;
  }

  /// Destroys every view (e.g. before attaching a new app).
  void clear(Adapter& adapter) {
    for (uint32_t i = 0; i < entries_.size(); ++i) {
      if (entries_[i].live) destroy(i, adapter);
    }
    initialized_ = false;
    last_frame_ = 0u;
  }

  /// View of packed `NodeId` `id` (generation-checked).
  [[nodiscard]] Handle* find(uint32_t id) noexcept {
    const uint32_t i = node_index(id);
    return i < entries_.size() && entries_[i].live && entries_[i].id == id && i != kRootIndex
               ? &entries_[i].handle
               : nullptr;
  }

  /// Resolved text style last applied to node `id` (`nullptr` when unknown).
  [[nodiscard]] const TextStyle* text_style(uint32_t id) const noexcept {
    const uint32_t i = node_index(id);
    return i < entries_.size() && entries_[i].live && entries_[i].id == id ? &entries_[i].applied : nullptr;
  }

  [[nodiscard]] size_t size() const noexcept {
    size_t n = 0u;
    for (const Entry& e : entries_) n += (e.live && node_index(e.id) != kRootIndex) ? 1u : 0u;
    return n;
  }

 private:
  struct Entry {
    uint32_t id = 0u;
    uint32_t tag = 0u;
    uint32_t stamp = 0u;     // last epoch this node was visited
    uint32_t resolved = 0u;  // epoch of `style`
    uint32_t linked = 0u;    // last epoch its children were handed to the adapter
    bool live = false;
    Handle handle{};
    TextStyle style;    // resolved this epoch
    TextStyle applied;  // last style handed to the adapter
  };

  void destroy(uint32_t i, Adapter& adapter) {
    Entry& e = entries_[i];
    if (i != kRootIndex) adapter.destroy(e.id, e.handle);
    e.live = false;
    e.handle = Handle{};
  }

  /// Resolved inherited style of live node `i`, memoized per epoch (iterative: deep trees safe).
  const TextStyle& resolve_style(const RuntimeSceneReader& scene, const SceneTables& tables, uint32_t i) {
    chain_.clear();
    uint32_t cur = i;
    while (cur < entries_.size() && entries_[cur].resolved != epoch_ && scene.is_live(cur)) {
      chain_.push_back(cur);
      if (cur == kRootIndex || chain_.size() > entries_.size()) break;
      cur = node_index(scene.row(cur)->parent);
    }
    for (size_t k = chain_.size(); k-- > 0u;) {
      const uint32_t n = chain_[k];
      const uint32_t parent = n == kRootIndex ? 0u : node_index(scene.row(n)->parent);
      const bool has_parent = parent < entries_.size() && entries_[parent].resolved == epoch_ && parent != n;
      entries_[n].style = inherit_text_style(has_parent ? entries_[parent].style : TextStyle{}, tables, scene, n);
      entries_[n].resolved = epoch_;
    }
    return entries_[i].style;
  }

  void visit(const RuntimeSceneReader& scene, const SceneTables& tables, Adapter& adapter, uint32_t i) {
    Entry& e = entries_[i];
    e.stamp = epoch_;
    const TextStyle style = resolve_style(scene, tables, i);
    const NodeData node = read_node(scene, tables, i, style);
    const bool style_changed = !e.live || !(e.applied == style);
    if (i == kRootIndex) {
      adapter.update_root(node);
      e.id = node.id;
      e.live = true;
    } else {
      const uint32_t tag = adapter.tag(node);
      if (e.live && (e.id != node.id || e.tag != tag)) destroy(i, adapter);
      if (!e.live) {
        e.handle = adapter.create(node);
        e.id = node.id;
        e.tag = tag;
        e.live = true;
        if (node.parent != 0u) relink_.push_back(node_index(node.parent));
      }
      adapter.update(e.handle, node);
    }
    e.applied = style;
    ++updated_;
    if (style_changed) {
      for (const uint32_t child : node.children) cascade_.push_back(node_index(child));
    }
  }

  void sync_children(const RuntimeSceneReader& scene, const SceneTables& tables, Adapter& adapter, uint32_t i) {
    Entry& e = entries_[i];
    if (!e.live) return;
    const NodeData node = read_node(scene, tables, i, e.style);
    kids_.clear();
    for (const uint32_t child : node.children) {
      if (Handle* h = find(child)) kids_.push_back(h);
    }
    adapter.set_children(node, i == kRootIndex ? nullptr : &e.handle, std::span<Handle* const>(kids_));
  }

  std::vector<Entry> entries_;
  std::vector<uint32_t> work_;
  std::vector<uint32_t> cascade_;
  std::vector<uint32_t> relink_;
  std::vector<uint32_t> chain_;
  std::vector<Handle*> kids_;
  uint64_t last_frame_ = 0u;
  uint32_t epoch_ = 0u;
  size_t updated_ = 0u;
  bool initialized_ = false;
};

}  // namespace nve::host
