// NVE host runtime: `OwnedEvents`, a deep copy of a borrowed `NveDirectEvent` batch.
//
// `drain_emitted_events` batches borrow app memory until the next runtime call: event names,
// text, and `NveValue` strings / bytes / arrays / maps all point into the app (often into the
// loaded module's static data). A host that hands events to arbitrary callbacks, which may
// dispatch, pump, reload, or detach the app, first copies them here; every slice of the copy
// points into this object.
//
// Storage is four flat slabs (events, bytes, values, map entries) sized by a counting pass, so a
// reused `OwnedEvents` allocates only when a batch outgrows every earlier one: steady-state
// copies allocate nothing. Moving keeps every slice valid (vector moves keep their buffers).
#pragma once

#include <cstddef>
#include <cstdint>
#include <span>
#include <type_traits>
#include <vector>

#include "../generated/nve_abi.generated.h"
#include "bin_codec.hpp"

namespace nve::host {

class OwnedEvents {
 public:
  OwnedEvents() = default;
  // Slices point into this object's slabs: a copy would alias them.
  OwnedEvents(const OwnedEvents&) = delete;
  OwnedEvents& operator=(const OwnedEvents&) = delete;
  OwnedEvents(OwnedEvents&&) noexcept = default;
  OwnedEvents& operator=(OwnedEvents&&) noexcept = default;

  /// Replaces the contents with a deep copy of `events`. A slice with a NULL pointer (whatever
  /// its length) copies as empty; values nested deeper than `kBinValueMaxDepth` (never produced
  /// by a valid app) copy as `Null`.
  void assign(std::span<const NveDirectEvent> events) {
    clear();
    Counts n;
    for (const NveDirectEvent& ev : events) {
      n.bytes += len(ev.name.ptr, ev.name.len) + len(ev.text.ptr, ev.text.len);
      count(ev.value, 0u, n);
    }
    // Exact reservations: the fill pass never reallocates, so every rebased pointer stays valid.
    events_.reserve(events.size());
    bytes_.reserve(n.bytes);
    values_.reserve(n.values);
    entries_.reserve(n.entries);
    events_.assign(events.begin(), events.end());
    for (NveDirectEvent& ev : events_) {
      ev.name = own(ev.name);
      ev.text = own(ev.text);
      ev.value = own(ev.value, 0u);
    }
  }

  /// Empties the copy, keeping every slab's capacity.
  void clear() noexcept {
    events_.clear();
    bytes_.clear();
    values_.clear();
    entries_.clear();
  }

  [[nodiscard]] std::span<const NveDirectEvent> events() const noexcept { return events_; }
  [[nodiscard]] bool empty() const noexcept { return events_.empty(); }

 private:
  struct Counts {
    size_t bytes = 0u;
    size_t values = 0u;
    size_t entries = 0u;
  };

  [[nodiscard]] static size_t len(const void* ptr, size_t n) noexcept { return ptr != nullptr ? n : 0u; }

  /// Mirrors `own(value, depth)` exactly: what it appends to each slab.
  static void count(const NveValue& v, uint32_t depth, Counts& n) {
    switch (v.tag) {
      case NVE_VAL_STR: n.bytes += len(v.data.str_val.ptr, v.data.str_val.len); break;
      case NVE_VAL_BYTES: n.bytes += len(v.data.bytes_val.ptr, v.data.bytes_val.len); break;
      case NVE_VAL_ARRAY: {
        if (depth >= kBinValueMaxDepth) break;
        const NveValueArray a = v.data.array_val;
        n.values += len(a.ptr, a.len);
        for (size_t i = 0u; i < len(a.ptr, a.len); ++i) count(a.ptr[i], depth + 1u, n);
        break;
      }
      case NVE_VAL_MAP: {
        if (depth >= kBinValueMaxDepth) break;
        const NveValueMap m = v.data.map_val;
        n.entries += len(m.ptr, m.len);
        for (size_t i = 0u; i < len(m.ptr, m.len); ++i) {
          n.bytes += len(m.ptr[i].key.ptr, m.ptr[i].key.len);
          count(m.ptr[i].value, depth + 1u, n);
        }
        break;
      }
      default: break;
    }
  }

  /// Appends `n` bytes at `ptr` to the byte slab; NULL for none.
  [[nodiscard]] const uint8_t* own_bytes(const uint8_t* ptr, size_t n) {
    if (ptr == nullptr || n == 0u) return nullptr;
    const size_t at = bytes_.size();
    bytes_.insert(bytes_.end(), ptr, ptr + n);
    return bytes_.data() + at;
  }

  NveStrSlice own(NveStrSlice s) {
    const uint8_t* p = own_bytes(s.ptr, s.len);
    return NveStrSlice{p, p != nullptr ? s.len : 0u};
  }

  NveValue own(const NveValue& v, uint32_t depth) {
    NveValue out = v;
    switch (v.tag) {
      case NVE_VAL_STR: out.data.str_val = own(v.data.str_val); break;
      case NVE_VAL_BYTES: {
        const uint8_t* p = own_bytes(v.data.bytes_val.ptr, v.data.bytes_val.len);
        out.data.bytes_val = NveBufferSlice{p, p != nullptr ? v.data.bytes_val.len : 0u};
        break;
      }
      case NVE_VAL_ARRAY: {
        if (depth >= kBinValueMaxDepth) return NveValue{};
        const NveValueArray a = v.data.array_val;
        const size_t n = len(a.ptr, a.len);
        // The elements stay contiguous: their slots are taken first, nested containers after.
        const size_t at = values_.size();
        values_.resize(at + n);
        for (size_t i = 0u; i < n; ++i) {
          const NveValue item = own(a.ptr[i], depth + 1u);
          values_[at + i] = item;
        }
        out.data.array_val = NveValueArray{n != 0u ? values_.data() + at : nullptr, n};
        break;
      }
      case NVE_VAL_MAP: {
        if (depth >= kBinValueMaxDepth) return NveValue{};
        const NveValueMap m = v.data.map_val;
        const size_t n = len(m.ptr, m.len);
        const size_t at = entries_.size();
        entries_.resize(at + n);
        for (size_t i = 0u; i < n; ++i) {
          const NveStrSlice key = own(m.ptr[i].key);
          const NveValue value = own(m.ptr[i].value, depth + 1u);
          entries_[at + i] = NveMapEntry{key, value};
        }
        out.data.map_val = NveValueMap{n != 0u ? entries_.data() + at : nullptr, n};
        break;
      }
      default: break;
    }
    return out;
  }

  std::vector<NveDirectEvent> events_;
  std::vector<uint8_t> bytes_;
  std::vector<NveValue> values_;
  std::vector<NveMapEntry> entries_;
};

static_assert(std::is_nothrow_move_constructible_v<OwnedEvents> && std::is_nothrow_move_assignable_v<OwnedEvents>,
              "HostSession::pump hands batches around by move");

}  // namespace nve::host
