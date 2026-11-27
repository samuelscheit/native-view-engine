// NVE host runtime: O(changed) retained native-view reconciler over `NveRuntimeSceneView`.
//
// Mirrors the published scene into one native view per live node through an `Adapter`:
//
//   using Handle = ...;                                        // default-constructible, movable
//   uint32_t tag(const NodeData&);                             // view type; a change recreates it
//   Handle create(const NodeData&);                            // every non-root node
//   void update(Handle&, const NodeData&);                     // after create and on every change
//   void move(Handle&, const NodeData&);                       // optional: geometry-only change
//   void update_root(const NodeData&);                         // root node (the host's own view)
//   void destroy(uint32_t id, Handle&);
//   void set_children(const NodeData& parent, Handle* parent_view /* null for root */,
//                     std::span<Handle* const> kids);           // in scene child order
//   uint32_t module_resource(const NodeData&);                 // optional: resource id a module
//                                                              // table attaches (canvas draw list)
//
// The first sync, any `frame_id` that is not the next one (a regression, or a skipped frame whose
// change sets were never seen), and any new `scene_epoch` (the runtime discarded its scene: app
// attach / swap / detach, pairing, keyframe; ids may reappear without removals) visit every live
// node, keeping views whose node, generation and tag still match. Afterwards a sync costs
// O(changed): `removed_ids`, `dirty_indices` (created, or row, tables or children changed), views
// reading a resource listed in `dirty_resources` (image source, box shadows, background fill,
// `module_resource`: same id, new bytes) and descendants whose inherited typography changed get
// `update`. Inside `update`, an adapter may keep what it decoded from a resource only when
// `resource_unchanged(scene, id, applied_id)`: the same id (`id == applied_id`), an incremental
// sync (no frame gap or new epoch, whose dirty ids were never seen), and `!scene.resource_dirty(id)`.
// `moved_indices` (only the box relative to the parent changed; a moved node's descendants are
// not listed, so adapters position views relative to their parent) get `move`, which receives
// `read_geometry` data (identity and geometry only). Adapters without `move` get a full `update`
// instead. Views are keyed by node index and checked against the packed `NodeId` generation, so a
// recycled slot recreates its view.
//
// `NodeData` (and its `TextStyle::font_family`) borrows the scene: it is valid only inside the
// adapter call. The tree itself keeps no view into the scene across syncs.
#pragma once

#include <cstdint>
#include <span>
#include <string>
#include <vector>

#include "bound_table.hpp"
#include "resource_users.hpp"
#include "runtime_session.hpp"
#include "scene_nodes.hpp"

namespace nve::host {

/// `Adapter` repositions a view whose geometry alone changed.
template <class Adapter>
concept MovingAdapter = requires(Adapter& a, typename Adapter::Handle& h, const NodeData& n) { a.move(h, n); };

/// `Adapter` reads one more resource per node than `NodeData` names (e.g. a canvas draw list).
template <class Adapter>
concept ModuleResourceAdapter = requires(Adapter& a, const NodeData& n) {
  { a.module_resource(n) } -> std::convertible_to<uint32_t>;
};

/// Resource ids live node `n`'s views read: image source, box shadows, background fill, and the
/// adapter's `module_resource`.
template <class Adapter>
[[nodiscard]] ResourceUsers::Ids node_resources(Adapter& adapter, const NodeData& n) {
  uint32_t module = 0u;
  if constexpr (ModuleResourceAdapter<Adapter>) module = adapter.module_resource(n);
  return {n.kind == NodeKind::Image ? n.image.src : 0u, n.visual.box_shadows, n.visual.background_fill, module};
}

template <class Adapter>
class RetainedTree {
 public:
  using Handle = typename Adapter::Handle;

  /// Reconciles `scene` (bound into `tables`); returns the number of views created or updated
  /// (`moved()` counts the geometry-only `move`s).
  size_t sync(const RuntimeSceneReader& scene, SceneTables& tables, Adapter& adapter) {
    const bool same_epoch = initialized_ && scene.scene_epoch() == scene_epoch_;
    if (same_epoch && scene.frame_id() == last_frame_) {
      moved_ = 0u;
      return 0u;  // this frame is already mirrored
    }
    const bool full = !same_epoch || scene.frame_id() != last_frame_ + 1u;
    incremental_ = !full;
    scene_epoch_ = scene.scene_epoch();
    tables.bind(scene);
    if (++epoch_ == 0u) {
      for (Entry& e : entries_) e.stamp = e.resolved = e.linked = 0u;
      epoch_ = 1u;
    }
    initialized_ = true;
    last_frame_ = scene.frame_id();
    if (entries_.size() < scene.node_capacity()) entries_.resize(scene.node_capacity());
    users_.reserve_views(entries_.size());
    updated_ = 0u;
    moved_ = 0u;
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
    if (!full) {
      // Same id, new bytes: re-apply every clean view that read it (collected first: `visit`
      // re-points the very lists being walked).
      for (const uint32_t id : scene.dirty_resources()) {
        users_.for_each_user(id, [&](uint32_t i) { resource_work_.push_back(i); });
      }
      for (const uint32_t i : resource_work_) {
        if (i < entries_.size() && entries_[i].live && entries_[i].stamp != epoch_ && scene.is_live(i)) {
          visit(scene, tables, adapter, i);
        }
      }
      resource_work_.clear();
    }
    // Inherited typography: a changed resolved style re-renders descendants until it stops changing.
    for (size_t q = 0; q < cascade_.size(); ++q) {
      const uint32_t i = cascade_[q];
      if (i < entries_.size() && entries_[i].stamp != epoch_ && scene.is_live(i)) visit(scene, tables, adapter, i);
    }
    cascade_.clear();
    if (!full) {
      for (const uint32_t i : scene.moved_indices()) {
        if (i < entries_.size() && entries_[i].stamp != epoch_ && scene.is_live(i)) move(scene, tables, adapter, i);
      }
    }
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

  /// Destroys every view.
  void clear(Adapter& adapter) {
    for (uint32_t i = 0; i < entries_.size(); ++i) {
      if (entries_[i].live) destroy(i, adapter);
    }
    initialized_ = false;
    incremental_ = false;
    last_frame_ = 0u;
  }

  /// View of packed `NodeId` `id` (generation-checked).
  [[nodiscard]] Handle* find(uint32_t id) noexcept {
    const uint32_t i = node_index(id);
    return i < entries_.size() && entries_[i].live && entries_[i].id == id && i != kRootIndex
               ? &entries_[i].handle
               : nullptr;
  }

  /// During (or after) `sync` of `scene`: whether a view may keep what it decoded from resource
  /// `applied_id` now that its node reads `id` (same id, incremental sync, bytes not redefined).
  [[nodiscard]] bool resource_unchanged(const RuntimeSceneReader& scene, uint32_t id,
                                        uint32_t applied_id) const noexcept {
    return incremental_ && id == applied_id && !scene.resource_dirty(id);
  }

  /// Geometry-only `move`s of the last sync.
  [[nodiscard]] size_t moved() const noexcept { return moved_; }

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
    TextStyle style;             // resolved in epoch `resolved` (borrows that sync's scene)
    TextStyle applied;           // last style handed to the adapter, without `font_family`
    std::string applied_family;  // ... whose family is owned here
  };

  [[nodiscard]] static bool same_applied(const Entry& e, const TextStyle& s) noexcept {
    TextStyle bare = s;
    bare.font_family = {};
    return e.applied == bare && e.applied_family == s.font_family;
  }

  static void store_applied(Entry& e, const TextStyle& s) {
    e.applied = s;
    e.applied.font_family = {};
    e.applied_family.assign(s.font_family);
  }

  void destroy(uint32_t i, Adapter& adapter) {
    Entry& e = entries_[i];
    if (i != kRootIndex) adapter.destroy(e.id, e.handle);
    users_.clear(i);
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
    const bool style_changed = !e.live || !same_applied(e, style);
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
        // A new view joins its parent and adopts its children (also when recreated by a cascade).
        relink_.push_back(i);
        if (node.parent != 0u) relink_.push_back(node_index(node.parent));
      }
      adapter.update(e.handle, node);
    }
    users_.set(i, node_resources(adapter, node));
    store_applied(e, style);
    ++updated_;
    if (style_changed) {
      for (const uint32_t child : node.children) cascade_.push_back(node_index(child));  // bounds-checked on pop
    }
  }

  /// A node whose geometry alone changed: `move` when its view is current, else a full visit.
  void move(const RuntimeSceneReader& scene, const SceneTables& tables, Adapter& adapter, uint32_t i) {
    Entry& e = entries_[i];
    if constexpr (MovingAdapter<Adapter>) {
      const NodeData node = read_geometry(scene, tables, i);
      if (i != kRootIndex && e.live && e.id == node.id) {
        e.stamp = epoch_;
        adapter.move(e.handle, node);
        ++moved_;
        return;
      }
    }
    visit(scene, tables, adapter, i);
  }

  void sync_children(const RuntimeSceneReader& scene, const SceneTables& tables, Adapter& adapter, uint32_t i) {
    Entry& e = entries_[i];
    if (!e.live) return;
    const NodeData node = read_node(scene, tables, i, resolve_style(scene, tables, i));
    kids_.clear();
    for (const uint32_t child : node.children) {
      if (Handle* h = find(child)) kids_.push_back(h);
    }
    adapter.set_children(node, i == kRootIndex ? nullptr : &e.handle, std::span<Handle* const>(kids_));
  }

  std::vector<Entry> entries_;
  ResourceUsers users_;
  std::vector<uint32_t> work_;
  std::vector<uint32_t> resource_work_;
  std::vector<uint32_t> cascade_;
  std::vector<uint32_t> relink_;
  std::vector<uint32_t> chain_;
  std::vector<Handle*> kids_;
  uint64_t last_frame_ = 0u;
  uint32_t scene_epoch_ = 0u;
  uint32_t epoch_ = 0u;
  size_t updated_ = 0u;
  size_t moved_ = 0u;
  bool initialized_ = false;
  bool incremental_ = false;
};

}  // namespace nve::host
