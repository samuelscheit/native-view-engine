// NVE host runtime: `OwnedEvents`, a deep copy of a borrowed `NveDirectEvent` batch.
//
// `drain_emitted_events` batches borrow app memory until the next runtime call: event names,
// text, and `NveValue` strings / bytes / arrays / maps all point into the app (often into the
// loaded module's static data). A host that hands events to arbitrary callbacks, which may
// dispatch, pump, reload, or detach the app, first copies them here; every slice of the copy
// points into this object.
#pragma once

#include <cstdint>
#include <deque>
#include <span>
#include <vector>

#include "../generated/nve_abi.generated.h"
#include "bin_codec.hpp"

namespace nve::host {

class OwnedEvents {
 public:
  OwnedEvents() = default;
  // Slices point into this object's storage: copying would alias it.
  OwnedEvents(const OwnedEvents&) = delete;
  OwnedEvents& operator=(const OwnedEvents&) = delete;
  OwnedEvents(OwnedEvents&&) noexcept = default;  // deque / vector moves keep element addresses
  OwnedEvents& operator=(OwnedEvents&&) noexcept = default;

  /// Replaces the contents with a deep copy of `events`. Values nested deeper than
  /// `kBinValueMaxDepth` (never produced by a valid app) are copied as `Null`.
  void assign(std::span<const NveDirectEvent> events) {
    clear();
    events_.assign(events.begin(), events.end());
    for (NveDirectEvent& ev : events_) {
      ev.name = own(ev.name);
      ev.text = own(ev.text);
      ev.value = own(ev.value, 0u);
    }
  }

  void clear() noexcept {
    events_.clear();
    bytes_.clear();
    arrays_.clear();
    maps_.clear();
  }

  [[nodiscard]] std::span<const NveDirectEvent> events() const noexcept { return events_; }
  [[nodiscard]] bool empty() const noexcept { return events_.empty(); }

 private:
  [[nodiscard]] const uint8_t* own_bytes(const uint8_t* ptr, size_t len) {
    if (ptr == nullptr || len == 0u) return nullptr;
    return bytes_.emplace_back(ptr, ptr + len).data();
  }

  NveStrSlice own(NveStrSlice s) {
    const uint8_t* p = own_bytes(s.ptr, s.len);
    return NveStrSlice{p, p != nullptr ? s.len : 0u};
  }

  NveValue own(NveValue v, uint32_t depth) {
    NveValue out = v;
    switch (v.tag) {
      case NVE_VAL_STR:
        out.data.str_val = own(v.data.str_val);
        break;
      case NVE_VAL_BYTES: {
        const uint8_t* p = own_bytes(v.data.bytes_val.ptr, v.data.bytes_val.len);
        out.data.bytes_val = NveBufferSlice{p, p != nullptr ? v.data.bytes_val.len : 0u};
        break;
      }
      case NVE_VAL_ARRAY: {
        if (depth >= kBinValueMaxDepth) return NveValue{};
        const NveValueArray a = v.data.array_val;
        // `std::deque` keeps earlier containers in place while children append more.
        std::vector<NveValue>& items = arrays_.emplace_back();
        if (a.ptr != nullptr) {
          items.reserve(a.len);
          for (size_t i = 0u; i < a.len; ++i) items.push_back(own(a.ptr[i], depth + 1u));
        }
        out.data.array_val = NveValueArray{items.empty() ? nullptr : items.data(), items.size()};
        break;
      }
      case NVE_VAL_MAP: {
        if (depth >= kBinValueMaxDepth) return NveValue{};
        const NveValueMap m = v.data.map_val;
        std::vector<NveMapEntry>& entries = maps_.emplace_back();
        if (m.ptr != nullptr) {
          entries.reserve(m.len);
          for (size_t i = 0u; i < m.len; ++i) {
            entries.push_back(NveMapEntry{own(m.ptr[i].key), own(m.ptr[i].value, depth + 1u)});
          }
        }
        out.data.map_val = NveValueMap{entries.empty() ? nullptr : entries.data(), entries.size()};
        break;
      }
      default:
        break;
    }
    return out;
  }

  std::vector<NveDirectEvent> events_;
  std::deque<std::vector<uint8_t>> bytes_;
  std::deque<std::vector<NveValue>> arrays_;
  std::deque<std::vector<NveMapEntry>> maps_;
};

}  // namespace nve::host
