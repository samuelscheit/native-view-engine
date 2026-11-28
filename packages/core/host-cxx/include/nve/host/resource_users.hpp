// NVE host runtime: reverse index resource id -> retained views that read it.
//
// Each view owns `kSlots` fixed link slots (one per resource it may depend on: image source, box
// shadows, background fill, one adapter-specific id). A link sits in an intrusive doubly-linked
// list per resource id; list heads live in an open-addressed (linear probing, backward-shift
// delete) map holding only ids some view references, so memory is O(views + referenced ids) for
// any id value. Re-pointing a slot and finding a list are O(1) expected; visiting every user of a
// resource whose bytes changed (`NveSceneView::dirty_resources`) is O(users). Storage grows
// only with node capacity and the number of referenced ids: steady state allocates nothing.
#pragma once

#include <algorithm>
#include <array>
#include <bit>
#include <cstddef>
#include <cstdint>
#include <vector>

namespace nve::host {

class ResourceUsers {
 public:
  static constexpr uint32_t kSlots = 4u;
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
      if (res_[link] == ids[k]) continue;
      unlink(link);
      if (ids[k] != 0u) push(link, ids[k]);
    }
  }

  /// Drops every dependency of view `view`.
  void clear(uint32_t view) { set(view, Ids{}); }

  /// Forgets every dependency (a new scene epoch: every id is stale). The head map shrinks back
  /// when it is far larger than what the previous epoch referenced.
  void reset() {
    std::fill(res_.begin(), res_.end(), 0u);
    std::fill(next_.begin(), next_.end(), kNone);
    std::fill(prev_.begin(), prev_.end(), kNone);
    const size_t want = std::bit_ceil(std::max<size_t>(kMinCapacity, count_ * 2u));
    if (keys_.size() > want * 4u) {
      keys_.assign(want, 0u);
      heads_.assign(want, kNone);
    } else {
      std::fill(keys_.begin(), keys_.end(), 0u);
    }
    count_ = 0u;
  }

  /// Calls `f(view)` for every slot that reads resource `id` (a view reading `id` through two
  /// slots is reported twice).
  template <class F>
  void for_each_user(uint32_t id, F&& f) const {
    const size_t at = find(id);
    if (at == kAbsent) return;
    for (uint32_t link = heads_[at]; link != kNone; link = next_[link]) f(link / kSlots);
  }

  /// Distinct resource ids with at least one user.
  [[nodiscard]] size_t referenced() const noexcept { return count_; }
  /// Head-map slots allocated (O(referenced ids), independent of id values).
  [[nodiscard]] size_t capacity() const noexcept { return keys_.size(); }

 private:
  static constexpr uint32_t kNone = UINT32_MAX;
  static constexpr size_t kAbsent = SIZE_MAX;
  static constexpr size_t kMinCapacity = 16u;

  [[nodiscard]] size_t home(uint32_t id) const noexcept {
    return static_cast<size_t>((uint64_t{id} * 0x9E3779B97F4A7C15ull) >> 32u) & (keys_.size() - 1u);
  }

  [[nodiscard]] size_t find(uint32_t id) const noexcept {
    if (id == 0u || keys_.empty()) return kAbsent;
    for (size_t i = home(id);; i = (i + 1u) & (keys_.size() - 1u)) {
      if (keys_[i] == id) return i;
      if (keys_[i] == 0u) return kAbsent;
    }
  }

  /// Slot of `id`, inserting an empty list (load factor <= 3/4).
  size_t find_or_insert(uint32_t id) {
    if ((count_ + 1u) * 4u > keys_.size() * 3u) grow();
    size_t i = home(id);
    for (; keys_[i] != 0u; i = (i + 1u) & (keys_.size() - 1u)) {
      if (keys_[i] == id) return i;
    }
    keys_[i] = id;
    heads_[i] = kNone;
    ++count_;
    return i;
  }

  void grow() {
    std::vector<uint32_t> keys(std::max(kMinCapacity, keys_.size() * 2u), 0u);
    std::vector<uint32_t> heads(keys.size(), kNone);
    keys.swap(keys_);
    heads.swap(heads_);
    for (size_t j = 0u; j < keys.size(); ++j) {
      if (keys[j] == 0u) continue;
      size_t i = home(keys[j]);
      while (keys_[i] != 0u) i = (i + 1u) & (keys_.size() - 1u);
      keys_[i] = keys[j];
      heads_[i] = heads[j];
    }
  }

  /// Removes slot `i` and shifts back the probe chain behind it (no tombstones).
  void erase_at(size_t i) {
    const size_t mask = keys_.size() - 1u;
    for (size_t j = (i + 1u) & mask; keys_[j] != 0u; j = (j + 1u) & mask) {
      const size_t k = home(keys_[j]);
      // `j`'s entry may fill hole `i` when its home is not cyclically within `(i, j]`.
      if (((j - k) & mask) >= ((j - i) & mask)) {
        keys_[i] = keys_[j];
        heads_[i] = heads_[j];
        i = j;
      }
    }
    keys_[i] = 0u;
    --count_;
  }

  void push(uint32_t link, uint32_t id) {
    const size_t at = find_or_insert(id);
    res_[link] = id;
    prev_[link] = kNone;
    next_[link] = heads_[at];
    if (heads_[at] != kNone) prev_[heads_[at]] = link;
    heads_[at] = link;
  }

  void unlink(uint32_t link) {
    const uint32_t id = res_[link];
    if (id == 0u) return;
    if (prev_[link] != kNone) {
      next_[prev_[link]] = next_[link];
    } else if (const size_t at = find(id); at != kAbsent) {
      heads_[at] = next_[link];
      if (heads_[at] == kNone) erase_at(at);
    }
    if (next_[link] != kNone) prev_[next_[link]] = prev_[link];
    res_[link] = 0u;
    next_[link] = prev_[link] = kNone;
  }

  std::vector<uint32_t> keys_;   // resource id (0 = empty slot)
  std::vector<uint32_t> heads_;  // first link of that id's list
  size_t count_ = 0u;
  std::vector<uint32_t> res_;  // link -> resource id (0 = unused)
  std::vector<uint32_t> next_;
  std::vector<uint32_t> prev_;
};

}  // namespace nve::host
