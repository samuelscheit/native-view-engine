// NVE host runtime: reverse index resource id -> retained views that read it.
//
// Each view owns `kSlots` fixed link slots (one per resource it may depend on: image source, box
// shadows, background fill, one adapter-specific id). A link sits in an intrusive doubly-linked
// list headed per resource id, so re-pointing a slot is O(1) and visiting every user of a resource
// whose bytes changed (`NveRuntimeSceneView::dirty_resources`) is O(users). Storage is flat arrays
// grown only with node capacity and the largest resource id seen: steady state allocates nothing.
#pragma once

#include <array>
#include <cstdint>
#include <vector>

namespace nve::host {

class ResourceUsers {
 public:
  static constexpr uint32_t kSlots = 4u;
  /// Resource ids at or above this are not tracked (bounds the head table at 64 MiB).
  static constexpr uint32_t kMaxTrackedId = 1u << 24u;
  using Ids = std::array<uint32_t, kSlots>;

  /// Makes room for `views` view slots (never shrinks).
  void reserve_views(size_t views) {
    if (res_.size() >= views * kSlots) return;
    res_.resize(views * kSlots, 0u);
    next_.resize(views * kSlots, kNone);
    prev_.resize(views * kSlots, kNone);
  }

  /// Points view `view`'s slots at `ids` (`0` = none).
  void set(uint32_t view, const Ids& ids) {
    for (uint32_t k = 0u; k < kSlots; ++k) {
      const uint32_t link = view * kSlots + k;
      const uint32_t id = ids[k] < kMaxTrackedId ? ids[k] : 0u;
      if (res_[link] == id) continue;
      unlink(link);
      if (id != 0u) push(link, id);
    }
  }

  /// Drops every dependency of view `view`.
  void clear(uint32_t view) { set(view, Ids{}); }

  /// Calls `f(view)` for every slot that reads resource `id` (a view reading `id` through two
  /// slots is reported twice).
  template <class F>
  void for_each_user(uint32_t id, F&& f) const {
    if (id == 0u || id >= head_.size()) return;
    for (uint32_t link = head_[id]; link != kNone; link = next_[link]) f(link / kSlots);
  }

 private:
  static constexpr uint32_t kNone = UINT32_MAX;

  void push(uint32_t link, uint32_t id) {
    if (head_.size() <= id) head_.resize(static_cast<size_t>(id) + 1u, kNone);
    res_[link] = id;
    prev_[link] = kNone;
    next_[link] = head_[id];
    if (head_[id] != kNone) prev_[head_[id]] = link;
    head_[id] = link;
  }

  void unlink(uint32_t link) {
    const uint32_t id = res_[link];
    if (id == 0u) return;
    if (prev_[link] != kNone) next_[prev_[link]] = next_[link]; else head_[id] = next_[link];
    if (next_[link] != kNone) prev_[next_[link]] = prev_[link];
    res_[link] = 0u;
    next_[link] = prev_[link] = kNone;
  }

  std::vector<uint32_t> head_;  // resource id -> first link
  std::vector<uint32_t> res_;   // link -> resource id (0 = unused)
  std::vector<uint32_t> next_;
  std::vector<uint32_t> prev_;
};

}  // namespace nve::host
