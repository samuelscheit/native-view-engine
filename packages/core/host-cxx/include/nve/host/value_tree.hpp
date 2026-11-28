// NVE host runtime: a C `NveValue` view of a `DynamicValue`, for C signatures that take
// `NveValue` (hybrid `Value` parameters).
#pragma once

#include <cstdint>
#include <deque>
#include <vector>

#include "../generated/nve_abi.generated.h"
#include "bin_codec.hpp"
#include "hybrid_runtime.hpp"

namespace nve::host {

/// `NveValue` tree of a `DynamicValue`. Strings, bytes, and map keys borrow the value's bytes and
/// container slices borrow this tree, so both must outlive every use of `root()`. The value is
/// valid by construction (at most `kBinValueMaxDepth` deep), so building it cannot fail.
class NveValueTree {
 public:
  NveValueTree() = default;

  explicit NveValueTree(const DynamicValue& value) {
    BinReader r = value.reader();
    root_ = build(r);
  }

  NveValueTree(const NveValueTree&) = delete;
  NveValueTree& operator=(const NveValueTree&) = delete;

  /// Resets the tree for reuse while keeping warm container vector capacities (trims past 64 slots
  /// after oversized spikes). Invalidates any previously returned `NveValue` slices.
  void clear() noexcept {
    constexpr size_t kMaxRetainedContainers = 64u;
    array_used_ = 0u;
    map_used_ = 0u;
    if (arrays_.size() > kMaxRetainedContainers) arrays_.resize(kMaxRetainedContainers);
    if (maps_.size() > kMaxRetainedContainers) maps_.resize(kMaxRetainedContainers);
    root_ = NveValue{};
  }

  /// Validates and materializes the next value from `r` into this tree, returning its `NveValue`
  /// root. Strings, bytes, and map keys borrow `r`'s buffer; container slices borrow this tree.
  /// On malformed input, `r` records the error (sticky) and this returns `Null`.
  NveValue read_value(BinReader& r) {
    const std::span<const uint8_t> raw = r.read_checked_value();
    if (!r.ok()) return NveValue{};
    BinReader sub(raw);
    return build(sub);
  }

  [[nodiscard]] const NveValue& root() const noexcept { return root_; }

 private:
  std::vector<NveValue>& next_array() {
    if (array_used_ == arrays_.size()) {
      arrays_.emplace_back();
    }
    std::vector<NveValue>& items = arrays_[array_used_++];
    items.clear();
    return items;
  }

  std::vector<NveMapEntry>& next_map() {
    if (map_used_ == maps_.size()) {
      maps_.emplace_back();
    }
    std::vector<NveMapEntry>& entries = maps_[map_used_++];
    entries.clear();
    return entries;
  }

  NveValue build(BinReader& r) {
    NveValue out{};
    switch (r.peek_tag()) {
      case BinTag::Null:
        (void)r.read_null();
        out.tag = NVE_VAL_NULL;
        break;
      case BinTag::Bool:
        out.tag = NVE_VAL_BOOL;
        out.data.bool_val = r.read_bool() ? 1u : 0u;
        break;
      case BinTag::I64:
        out.tag = NVE_VAL_I64;
        out.data.i64_val = r.read_i64();
        break;
      case BinTag::F64:
        out.tag = NVE_VAL_F64;
        out.data.f64_val = r.read_f64();
        break;
      case BinTag::Str: {
        const std::string_view s = r.read_str();
        out.tag = NVE_VAL_STR;
        out.data.str_val = NveStrSlice{reinterpret_cast<const uint8_t*>(s.data()), s.size()};
        break;
      }
      case BinTag::Bytes: {
        const std::span<const uint8_t> b = r.read_bytes();
        out.tag = NVE_VAL_BYTES;
        out.data.bytes_val = NveBufferSlice{b.data(), b.size()};
        break;
      }
      case BinTag::Array: {
        const BinReader::ReadMark mark = r.begin_array();
        // `std::deque` keeps earlier containers in place while children append more.
        std::vector<NveValue>& items = next_array();
        items.reserve(mark.count);
        for (uint32_t i = 0; i < mark.count && r.ok(); ++i) items.push_back(build(r));
        r.end(mark);
        out.tag = NVE_VAL_ARRAY;
        out.data.array_val = NveValueArray{items.data(), items.size()};
        break;
      }
      case BinTag::Map: {
        const BinReader::ReadMark mark = r.begin_map();
        std::vector<NveMapEntry>& entries = next_map();
        entries.reserve(mark.count);
        for (uint32_t i = 0; i < mark.count && r.ok(); ++i) {
          const std::string_view key = r.read_map_key();
          NveMapEntry entry{};
          entry.key = NveStrSlice{reinterpret_cast<const uint8_t*>(key.data()), key.size()};
          entry.value = build(r);
          entries.push_back(entry);
        }
        r.end(mark);
        out.tag = NVE_VAL_MAP;
        out.data.map_val = NveValueMap{entries.data(), entries.size()};
        break;
      }
    }
    return out;
  }

  std::deque<std::vector<NveValue>> arrays_;
  std::deque<std::vector<NveMapEntry>> maps_;
  size_t array_used_ = 0u;
  size_t map_used_ = 0u;
  NveValue root_{};
};

}  // namespace nve::host
