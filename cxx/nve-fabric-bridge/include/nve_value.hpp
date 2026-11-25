#pragma once

#include "nve_abi.generated.h"

#include <algorithm>
#include <cstddef>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <new>
#include <optional>
#include <string_view>
#include <type_traits>
#include <utility>
#include <vector>

namespace nve {

// ============================================================================
// NveStrSlice & NveValue Lightweight Helpers
// ============================================================================

inline NveStrSlice make_str_slice(std::string_view sv) noexcept {
  return NveStrSlice{
      sv.empty() ? nullptr : reinterpret_cast<const uint8_t*>(sv.data()),
      sv.size(),
  };
}

inline std::string_view str_slice_view(NveStrSlice s) noexcept {
  if (s.ptr == nullptr || s.len == 0) {
    return {};
  }
  return std::string_view(reinterpret_cast<const char*>(s.ptr), s.len);
}

inline NveValue make_null_value() noexcept {
  NveValue v{};
  v.tag = NVE_VAL_NULL;
  return v;
}

inline NveValue make_bool_value(bool b) noexcept {
  NveValue v{};
  v.tag = NVE_VAL_BOOL;
  v.data.bool_val = b ? 1u : 0u;
  return v;
}

inline NveValue make_i64_value(int64_t i) noexcept {
  NveValue v{};
  v.tag = NVE_VAL_I64;
  v.data.i64_val = i;
  return v;
}

inline NveValue make_f64_value(double d) noexcept {
  NveValue v{};
  v.tag = NVE_VAL_F64;
  v.data.f64_val = d;
  return v;
}

inline NveValue make_str_value(NveStrSlice s) noexcept {
  NveValue v{};
  v.tag = NVE_VAL_STR;
  v.data.str_val = s;
  return v;
}

inline NveValue make_str_value(std::string_view sv) noexcept {
  return make_str_value(make_str_slice(sv));
}

inline NveValue make_bytes_value(NveBufferSlice b) noexcept {
  NveValue v{};
  v.tag = NVE_VAL_BYTES;
  v.data.bytes_val = b;
  return v;
}

inline NveValue make_array_value(const NveValue* ptr, size_t len) noexcept {
  NveValue v{};
  v.tag = NVE_VAL_ARRAY;
  v.data.array_val = NveValueArray{ptr, len};
  return v;
}

inline NveValue make_map_value(const NveMapEntry* ptr, size_t len) noexcept {
  NveValue v{};
  v.tag = NVE_VAL_MAP;
  v.data.map_val = NveValueMap{ptr, len};
  return v;
}

// ============================================================================
// Aligned Linear Bump Arena (Zero-Malloc Steady-State Frame Allocator)
// ============================================================================

/// Fast aligned linear slab allocator designed for per-frame zero-allocation FFI capture.
///
/// After warmup, `reset()` rewinds `slab_index_` and `cur_` in O(1) while retaining all
/// allocated slabs so subsequent frames execute with zero `malloc` / `free` calls.
class BumpArena {
 public:
  static constexpr size_t kDefaultInitialSlabBytes = 64u * 1024u;
  static constexpr size_t kSlabAlignment = 64u;

  explicit BumpArena(size_t initial_slab_bytes = kDefaultInitialSlabBytes)
      : initial_slab_bytes_(std::max<size_t>(initial_slab_bytes, 1024u)) {
    allocate_slab(initial_slab_bytes_);
    slab_index_ = 0;
    cur_ = slabs_[0].data;
    end_ = slabs_[0].data + slabs_[0].cap;
  }

  ~BumpArena() {
    for (auto& slab : slabs_) {
      ::operator delete(slab.data, std::align_val_t(kSlabAlignment));
    }
  }

  BumpArena(const BumpArena&) = delete;
  BumpArena& operator=(const BumpArena&) = delete;

  BumpArena(BumpArena&& other) noexcept
      : slabs_(std::move(other.slabs_)),
        slab_index_(other.slab_index_),
        cur_(other.cur_),
        end_(other.end_),
        initial_slab_bytes_(other.initial_slab_bytes_) {
    other.slab_index_ = 0;
    other.cur_ = nullptr;
    other.end_ = nullptr;
  }

  BumpArena& operator=(BumpArena&& other) noexcept {
    if (this != &other) {
      for (auto& slab : slabs_) {
        ::operator delete(slab.data, std::align_val_t(kSlabAlignment));
      }
      slabs_ = std::move(other.slabs_);
      slab_index_ = other.slab_index_;
      cur_ = other.cur_;
      end_ = other.end_;
      initial_slab_bytes_ = other.initial_slab_bytes_;
      other.slab_index_ = 0;
      other.cur_ = nullptr;
      other.end_ = nullptr;
    }
    return *this;
  }

  /// O(1) frame reset: rewinds pointer to start of slab 0 and reuses all retained slabs.
  void reset() noexcept {
    if (!slabs_.empty()) {
      slab_index_ = 0;
      cur_ = slabs_[0].data;
      end_ = slabs_[0].data + slabs_[0].cap;
    }
  }

  /// Allocate `bytes` aligned to `align` (power of two). Fast path is a single pointer bump.
  void* alloc(size_t bytes, size_t align = alignof(std::max_align_t)) {
    const size_t eff_align = align < 1u ? 1u : align;
    const size_t mask = eff_align - 1u;
    const uintptr_t cur_addr = reinterpret_cast<uintptr_t>(cur_);
    const uintptr_t aligned_addr = (cur_addr + mask) & ~mask;
    uint8_t* aligned_ptr = reinterpret_cast<uint8_t*>(aligned_addr);
    if (aligned_ptr >= cur_ && static_cast<size_t>(end_ - aligned_ptr) >= bytes) {
      cur_ = aligned_ptr + bytes;
      return aligned_ptr;
    }
    return alloc_slow(bytes, eff_align);
  }

  template <typename T>
  T* alloc_array(size_t count) {
    static_assert(std::is_trivially_destructible_v<T>, "BumpArena requires trivially destructible T");
    if (count == 0) {
      return nullptr;
    }
    return static_cast<T*>(alloc(sizeof(T) * count, alignof(T)));
  }

  template <typename T, typename... Args>
  T* make(Args&&... args) {
    static_assert(std::is_trivially_destructible_v<T>, "BumpArena::make requires trivially destructible T");
    void* mem = alloc(sizeof(T), alignof(T));
    return ::new (mem) T(std::forward<Args>(args)...);
  }

  /// Placement-new without trivial-destructor check (for arena-owned polymorphic wrappers whose
  /// destructor is a no-op, e.g. `facebook::jsi::Runtime::PointerValue`).
  template <typename T, typename... Args>
  T* emplace(Args&&... args) {
    void* mem = alloc(sizeof(T), alignof(T));
    return ::new (mem) T(std::forward<Args>(args)...);
  }

  NveStrSlice copy_str(std::string_view sv) {
    if (sv.empty()) {
      return NveStrSlice{nullptr, 0};
    }
    auto* dst = static_cast<uint8_t*>(alloc(sv.size(), 1u));
    std::memcpy(dst, sv.data(), sv.size());
    return NveStrSlice{dst, sv.size()};
  }

  NveBufferSlice copy_bytes(const uint8_t* data, size_t len) {
    if (data == nullptr || len == 0) {
      return NveBufferSlice{nullptr, 0};
    }
    auto* dst = static_cast<uint8_t*>(alloc(len, 1u));
    std::memcpy(dst, data, len);
    return NveBufferSlice{dst, len};
  }

  size_t slab_count() const noexcept { return slabs_.size(); }

  size_t total_capacity() const noexcept {
    size_t total = 0;
    for (const auto& s : slabs_) {
      total += s.cap;
    }
    return total;
  }

  size_t used_bytes() const noexcept {
    if (slabs_.empty()) return 0;
    size_t used = 0;
    for (size_t i = 0; i < slab_index_; ++i) {
      used += slabs_[i].cap;
    }
    used += static_cast<size_t>(cur_ - slabs_[slab_index_].data);
    return used;
  }

 private:
  struct Slab {
    uint8_t* data{nullptr};
    size_t cap{0};
  };

  void allocate_slab(size_t min_bytes) {
    const size_t rounded = (min_bytes + (kSlabAlignment - 1u)) & ~(kSlabAlignment - 1u);
    auto* ptr = static_cast<uint8_t*>(::operator new(rounded, std::align_val_t(kSlabAlignment)));
    slabs_.push_back(Slab{ptr, rounded});
  }

  void* alloc_slow(size_t bytes, size_t align) {
    // Reuse already-allocated subsequent slabs if any fit.
    while (slab_index_ + 1 < slabs_.size()) {
      ++slab_index_;
      cur_ = slabs_[slab_index_].data;
      end_ = slabs_[slab_index_].data + slabs_[slab_index_].cap;
      const size_t mask = align - 1u;
      const uintptr_t aligned_addr = (reinterpret_cast<uintptr_t>(cur_) + mask) & ~mask;
      auto* aligned_ptr = reinterpret_cast<uint8_t*>(aligned_addr);
      if (aligned_ptr >= cur_ && static_cast<size_t>(end_ - aligned_ptr) >= bytes) {
        cur_ = aligned_ptr + bytes;
        return aligned_ptr;
      }
    }

    const size_t prev_cap = slabs_.empty() ? initial_slab_bytes_ : slabs_.back().cap;
    const size_t next_cap = std::max<size_t>(prev_cap * 2u, bytes + align);
    allocate_slab(next_cap);
    slab_index_ = slabs_.size() - 1;
    cur_ = slabs_[slab_index_].data;
    end_ = slabs_[slab_index_].data + slabs_[slab_index_].cap;
    const size_t mask = align - 1u;
    const uintptr_t aligned_addr = (reinterpret_cast<uintptr_t>(cur_) + mask) & ~mask;
    auto* aligned_ptr = reinterpret_cast<uint8_t*>(aligned_addr);
    cur_ = aligned_ptr + bytes;
    return aligned_ptr;
  }

  std::vector<Slab> slabs_;
  size_t slab_index_{0};
  uint8_t* cur_{nullptr};
  uint8_t* end_{nullptr};
  size_t initial_slab_bytes_{kDefaultInitialSlabBytes};
};

// ============================================================================
// Compact Binary `NveBinValue` Wire Codec & Zero-Copy Borrowed `BinValueView`
// ============================================================================

namespace detail {

/// Strict RFC 3629 UTF-8 validator (zero heap allocations).
inline bool is_valid_utf8(const uint8_t* data, size_t len) noexcept {
  size_t i = 0;
  while (i < len) {
    const uint8_t b0 = data[i];
    if (b0 < 0x80u) {
      ++i;
      continue;
    }
    if ((b0 & 0xE0u) == 0xC0u) {
      if (i + 1 >= len) return false;
      const uint8_t b1 = data[i + 1];
      if ((b1 & 0xC0u) != 0x80u) return false;
      if (b0 < 0xC2u) return false; // overlong
      i += 2;
    } else if ((b0 & 0xF0u) == 0xE0u) {
      if (i + 2 >= len) return false;
      const uint8_t b1 = data[i + 1];
      const uint8_t b2 = data[i + 2];
      if ((b1 & 0xC0u) != 0x80u || (b2 & 0xC0u) != 0x80u) return false;
      if (b0 == 0xE0u && b1 < 0xA0u) return false; // overlong
      if (b0 == 0xEDu && b1 >= 0xA0u) return false; // UTF-16 surrogates
      i += 3;
    } else if ((b0 & 0xF8u) == 0xF0u) {
      if (i + 3 >= len) return false;
      const uint8_t b1 = data[i + 1];
      const uint8_t b2 = data[i + 2];
      const uint8_t b3 = data[i + 3];
      if ((b1 & 0xC0u) != 0x80u || (b2 & 0xC0u) != 0x80u || (b3 & 0xC0u) != 0x80u) {
        return false;
      }
      if (b0 == 0xF0u && b1 < 0x90u) return false; // overlong
      if (b0 > 0xF4u || (b0 == 0xF4u && b1 >= 0x90u)) return false; // > U+10FFFF
      i += 4;
    } else {
      return false;
    }
  }
  return true;
}

inline bool read_u32_le(const uint8_t* data, size_t len, size_t pos, uint32_t& out) noexcept {
  if (pos > len || len - pos < 4u) {
    return false;
  }
  std::memcpy(&out, data + pos, sizeof(uint32_t));
  return true;
}

inline void append_u32_le(std::vector<uint8_t>& out, uint32_t v) {
  const auto* p = reinterpret_cast<const uint8_t*>(&v);
  out.insert(out.end(), p, p + sizeof(uint32_t));
}

inline void append_i64_le(std::vector<uint8_t>& out, int64_t v) {
  const auto* p = reinterpret_cast<const uint8_t*>(&v);
  out.insert(out.end(), p, p + sizeof(int64_t));
}

inline void append_f64_le(std::vector<uint8_t>& out, double v) {
  const auto* p = reinterpret_cast<const uint8_t*>(&v);
  out.insert(out.end(), p, p + sizeof(double));
}

inline bool encode_bin_value_impl(
    std::vector<uint8_t>& out,
    const NveValue& value,
    uint32_t depth,
    uint32_t max_depth) {
  switch (value.tag) {
    case NVE_VAL_NULL:
      out.push_back(static_cast<uint8_t>(NVE_VAL_NULL));
      return true;
    case NVE_VAL_BOOL:
      out.push_back(static_cast<uint8_t>(NVE_VAL_BOOL));
      out.push_back(value.data.bool_val ? 1u : 0u);
      return true;
    case NVE_VAL_I64:
      out.push_back(static_cast<uint8_t>(NVE_VAL_I64));
      append_i64_le(out, value.data.i64_val);
      return true;
    case NVE_VAL_F64:
      out.push_back(static_cast<uint8_t>(NVE_VAL_F64));
      append_f64_le(out, value.data.f64_val);
      return true;
    case NVE_VAL_STR: {
      const NveStrSlice s = value.data.str_val;
      if (s.len > 0xFFFFFFFFu || (s.len > 0 && s.ptr == nullptr)) {
        return false;
      }
      out.push_back(static_cast<uint8_t>(NVE_VAL_STR));
      append_u32_le(out, static_cast<uint32_t>(s.len));
      if (s.len > 0) {
        out.insert(out.end(), s.ptr, s.ptr + s.len);
      }
      return true;
    }
    case NVE_VAL_BYTES: {
      const NveBufferSlice b = value.data.bytes_val;
      if (b.len > 0xFFFFFFFFu || (b.len > 0 && b.ptr == nullptr)) {
        return false;
      }
      out.push_back(static_cast<uint8_t>(NVE_VAL_BYTES));
      append_u32_le(out, static_cast<uint32_t>(b.len));
      if (b.len > 0) {
        out.insert(out.end(), b.ptr, b.ptr + b.len);
      }
      return true;
    }
    case NVE_VAL_ARRAY: {
      if (depth >= max_depth) {
        return false;
      }
      const NveValueArray arr = value.data.array_val;
      if (arr.len > 0xFFFFFFFFu || (arr.len > 0 && arr.ptr == nullptr)) {
        return false;
      }
      out.push_back(static_cast<uint8_t>(NVE_VAL_ARRAY));
      const size_t byte_len_pos = out.size();
      append_u32_le(out, 0u);
      const size_t payload_start = out.size();
      append_u32_le(out, static_cast<uint32_t>(arr.len));
      for (size_t i = 0; i < arr.len; ++i) {
        if (!encode_bin_value_impl(out, arr.ptr[i], depth + 1u, max_depth)) {
          return false;
        }
      }
      const size_t payload_bytes = out.size() - payload_start;
      if (payload_bytes > 0xFFFFFFFFu) {
        return false;
      }
      const uint32_t payload_u32 = static_cast<uint32_t>(payload_bytes);
      std::memcpy(out.data() + byte_len_pos, &payload_u32, sizeof(uint32_t));
      return true;
    }
    case NVE_VAL_MAP: {
      if (depth >= max_depth) {
        return false;
      }
      const NveValueMap map = value.data.map_val;
      if (map.len > 0xFFFFFFFFu || (map.len > 0 && map.ptr == nullptr)) {
        return false;
      }
      out.push_back(static_cast<uint8_t>(NVE_VAL_MAP));
      const size_t byte_len_pos = out.size();
      append_u32_le(out, 0u);
      const size_t payload_start = out.size();
      append_u32_le(out, static_cast<uint32_t>(map.len));
      for (size_t i = 0; i < map.len; ++i) {
        const NveStrSlice k = map.ptr[i].key;
        if (k.len > 0xFFFFFFFFu || (k.len > 0 && k.ptr == nullptr)) {
          return false;
        }
        append_u32_le(out, static_cast<uint32_t>(k.len));
        if (k.len > 0) {
          out.insert(out.end(), k.ptr, k.ptr + k.len);
        }
        if (!encode_bin_value_impl(out, map.ptr[i].value, depth + 1u, max_depth)) {
          return false;
        }
      }
      const size_t payload_bytes = out.size() - payload_start;
      if (payload_bytes > 0xFFFFFFFFu) {
        return false;
      }
      const uint32_t payload_u32 = static_cast<uint32_t>(payload_bytes);
      std::memcpy(out.data() + byte_len_pos, &payload_u32, sizeof(uint32_t));
      return true;
    }
    default:
      return false;
  }
}

inline bool validate_bin_value_at(
    const uint8_t* data,
    size_t len,
    size_t start,
    uint32_t depth,
    uint32_t max_depth,
    NveValueTag& out_tag,
    size_t& out_end) noexcept {
  if (start >= len) {
    return false;
  }
  const uint8_t raw_tag = data[start];
  if (raw_tag > NVE_VAL_MAP) {
    return false;
  }
  out_tag = static_cast<NveValueTag>(raw_tag);
  size_t pos = start + 1u;

  switch (out_tag) {
    case NVE_VAL_NULL:
      out_end = pos;
      return true;
    case NVE_VAL_BOOL:
      if (pos >= len || data[pos] > 1u) {
        return false;
      }
      out_end = pos + 1u;
      return true;
    case NVE_VAL_I64:
    case NVE_VAL_F64:
      if (len - pos < 8u) {
        return false;
      }
      out_end = pos + 8u;
      return true;
    case NVE_VAL_STR: {
      uint32_t slen = 0;
      if (!read_u32_le(data, len, pos, slen)) {
        return false;
      }
      pos += 4u;
      if (len - pos < slen) {
        return false;
      }
      if (!is_valid_utf8(data + pos, slen)) {
        return false;
      }
      out_end = pos + slen;
      return true;
    }
    case NVE_VAL_BYTES: {
      uint32_t blen = 0;
      if (!read_u32_le(data, len, pos, blen)) {
        return false;
      }
      pos += 4u;
      if (len - pos < blen) {
        return false;
      }
      out_end = pos + blen;
      return true;
    }
    case NVE_VAL_ARRAY: {
      if (depth >= max_depth) {
        return false;
      }
      uint32_t payload_len = 0;
      if (!read_u32_le(data, len, pos, payload_len)) {
        return false;
      }
      pos += 4u;
      if (payload_len < 4u || len - pos < payload_len) {
        return false;
      }
      const size_t container_end = pos + payload_len;
      uint32_t count = 0;
      if (!read_u32_le(data, container_end, pos, count)) {
        return false;
      }
      pos += 4u;
      if (count > container_end - pos) {
        return false;
      }
      for (uint32_t i = 0; i < count; ++i) {
        NveValueTag child_tag = NVE_VAL_NULL;
        size_t next_pos = 0;
        if (!validate_bin_value_at(
                data, container_end, pos, depth + 1u, max_depth, child_tag, next_pos)) {
          return false;
        }
        pos = next_pos;
      }
      if (pos != container_end) {
        return false;
      }
      out_end = container_end;
      return true;
    }
    case NVE_VAL_MAP: {
      if (depth >= max_depth) {
        return false;
      }
      uint32_t payload_len = 0;
      if (!read_u32_le(data, len, pos, payload_len)) {
        return false;
      }
      pos += 4u;
      if (payload_len < 4u || len - pos < payload_len) {
        return false;
      }
      const size_t container_end = pos + payload_len;
      uint32_t count = 0;
      if (!read_u32_le(data, container_end, pos, count)) {
        return false;
      }
      pos += 4u;
      if (count > (container_end - pos) / 5u) {
        return false;
      }
      for (uint32_t i = 0; i < count; ++i) {
        uint32_t klen = 0;
        if (!read_u32_le(data, container_end, pos, klen)) {
          return false;
        }
        pos += 4u;
        if (container_end - pos < klen || !is_valid_utf8(data + pos, klen)) {
          return false;
        }
        pos += klen;
        NveValueTag child_tag = NVE_VAL_NULL;
        size_t next_pos = 0;
        if (!validate_bin_value_at(
                data, container_end, pos, depth + 1u, max_depth, child_tag, next_pos)) {
          return false;
        }
        pos = next_pos;
      }
      if (pos != container_end) {
        return false;
      }
      out_end = container_end;
      return true;
    }
  }
  return false;
}

inline bool skip_validated_bin_value(
    const uint8_t* data,
    size_t len,
    size_t start,
    NveValueTag& out_tag,
    size_t& out_end) noexcept {
  if (start >= len || data[start] > NVE_VAL_MAP) {
    return false;
  }
  out_tag = static_cast<NveValueTag>(data[start]);
  const size_t pos = start + 1u;
  switch (out_tag) {
    case NVE_VAL_NULL:
      out_end = pos;
      return out_end <= len;
    case NVE_VAL_BOOL:
      out_end = pos + 1u;
      return out_end <= len;
    case NVE_VAL_I64:
    case NVE_VAL_F64:
      out_end = pos + 8u;
      return out_end <= len;
    case NVE_VAL_STR:
    case NVE_VAL_BYTES:
    case NVE_VAL_ARRAY:
    case NVE_VAL_MAP: {
      uint32_t blen = 0;
      if (!read_u32_le(data, len, pos, blen)) {
        return false;
      }
      if (len - (pos + 4u) < blen) {
        return false;
      }
      out_end = pos + 4u + blen;
      return true;
    }
  }
  return false;
}

} // namespace detail

/// Encode an `NveValue` into the compact binary `NveBinValue` format directly into `out`.
/// On failure (invalid tag or nesting > `max_depth`), rolls `out` back to its previous length.
inline bool encode_bin_value(
    std::vector<uint8_t>& out,
    const NveValue& value,
    uint32_t max_depth = NVE_BIN_VALUE_MAX_DEPTH) {
  const size_t start_len = out.size();
  if (!detail::encode_bin_value_impl(out, value, 0u, max_depth)) {
    out.resize(start_len);
    return false;
  }
  return true;
}

/// Zero-copy, zero-allocation borrowed reader over a validated binary `NveBinValue` slice.
class BinValueView {
 public:
  BinValueView() noexcept = default;

  static std::optional<BinValueView> parse(
      const uint8_t* data,
      size_t len,
      uint32_t max_depth = NVE_BIN_VALUE_MAX_DEPTH) noexcept {
    if (data == nullptr || len == 0) {
      return std::nullopt;
    }
    NveValueTag tag = NVE_VAL_NULL;
    size_t end = 0;
    if (!detail::validate_bin_value_at(data, len, 0u, 0u, max_depth, tag, end)) {
      return std::nullopt;
    }
    if (end != len) {
      return std::nullopt;
    }
    return BinValueView(data, len, tag);
  }

  static std::optional<BinValueView> parse(
      NveBufferSlice slice,
      uint32_t max_depth = NVE_BIN_VALUE_MAX_DEPTH) noexcept {
    return parse(slice.ptr, slice.len, max_depth);
  }

  NveValueTag tag() const noexcept { return tag_; }
  bool is_null() const noexcept { return tag_ == NVE_VAL_NULL; }
  NveBufferSlice raw_bytes() const noexcept { return NveBufferSlice{data_, len_}; }

  std::optional<bool> as_bool() const noexcept {
    if (tag_ != NVE_VAL_BOOL || len_ < 2u) return std::nullopt;
    return data_[1] != 0u;
  }

  std::optional<int64_t> as_i64() const noexcept {
    if (tag_ != NVE_VAL_I64 || len_ < 9u) return std::nullopt;
    int64_t v = 0;
    std::memcpy(&v, data_ + 1, sizeof(int64_t));
    return v;
  }

  std::optional<double> as_f64() const noexcept {
    if (tag_ != NVE_VAL_F64 || len_ < 9u) return std::nullopt;
    double v = 0.0;
    std::memcpy(&v, data_ + 1, sizeof(double));
    return v;
  }

  std::optional<double> as_number() const noexcept {
    if (tag_ == NVE_VAL_F64) return as_f64();
    if (tag_ == NVE_VAL_I64) {
      const auto i = as_i64();
      return i ? std::optional<double>(static_cast<double>(*i)) : std::nullopt;
    }
    return std::nullopt;
  }

  std::optional<std::string_view> as_str() const noexcept {
    if (tag_ != NVE_VAL_STR || len_ < 5u) return std::nullopt;
    uint32_t slen = 0;
    if (!detail::read_u32_le(data_, len_, 1u, slen) || len_ - 5u < slen) {
      return std::nullopt;
    }
    return std::string_view(reinterpret_cast<const char*>(data_ + 5), slen);
  }

  std::optional<NveBufferSlice> as_bytes() const noexcept {
    if (tag_ != NVE_VAL_BYTES || len_ < 5u) return std::nullopt;
    uint32_t blen = 0;
    if (!detail::read_u32_le(data_, len_, 1u, blen) || len_ - 5u < blen) {
      return std::nullopt;
    }
    return NveBufferSlice{blen == 0 ? nullptr : data_ + 5, blen};
  }

  std::optional<size_t> array_len() const noexcept {
    if (tag_ != NVE_VAL_ARRAY || len_ < 9u) return std::nullopt;
    uint32_t count = 0;
    if (!detail::read_u32_le(data_, len_, 5u, count)) return std::nullopt;
    return static_cast<size_t>(count);
  }

  std::optional<BinValueView> array_at(size_t index) const noexcept {
    const auto count = array_len();
    if (!count || index >= *count) {
      return std::nullopt;
    }
    size_t pos = 9u;
    for (size_t i = 0; i <= index; ++i) {
      NveValueTag elem_tag = NVE_VAL_NULL;
      size_t elem_end = 0;
      if (!detail::skip_validated_bin_value(data_, len_, pos, elem_tag, elem_end)) {
        return std::nullopt;
      }
      if (i == index) {
        return BinValueView(data_ + pos, elem_end - pos, elem_tag);
      }
      pos = elem_end;
    }
    return std::nullopt;
  }

  template <typename Fn>
  bool for_each_array(Fn&& fn) const {
    const auto count = array_len();
    if (!count) return false;
    size_t pos = 9u;
    for (size_t i = 0; i < *count; ++i) {
      NveValueTag elem_tag = NVE_VAL_NULL;
      size_t elem_end = 0;
      if (!detail::skip_validated_bin_value(data_, len_, pos, elem_tag, elem_end)) {
        return false;
      }
      fn(BinValueView(data_ + pos, elem_end - pos, elem_tag));
      pos = elem_end;
    }
    return true;
  }

  std::optional<size_t> map_len() const noexcept {
    if (tag_ != NVE_VAL_MAP || len_ < 9u) return std::nullopt;
    uint32_t count = 0;
    if (!detail::read_u32_le(data_, len_, 5u, count)) return std::nullopt;
    return static_cast<size_t>(count);
  }

  std::optional<BinValueView> map_get(std::string_view key) const noexcept {
    const auto count = map_len();
    if (!count) return std::nullopt;
    size_t pos = 9u;
    for (size_t i = 0; i < *count; ++i) {
      uint32_t klen = 0;
      if (!detail::read_u32_le(data_, len_, pos, klen)) return std::nullopt;
      pos += 4u;
      if (len_ - pos < klen) return std::nullopt;
      const std::string_view k(reinterpret_cast<const char*>(data_ + pos), klen);
      pos += klen;
      NveValueTag vtag = NVE_VAL_NULL;
      size_t vend = 0;
      if (!detail::skip_validated_bin_value(data_, len_, pos, vtag, vend)) {
        return std::nullopt;
      }
      if (k == key) {
        return BinValueView(data_ + pos, vend - pos, vtag);
      }
      pos = vend;
    }
    return std::nullopt;
  }

  template <typename Fn>
  bool for_each_map(Fn&& fn) const {
    const auto count = map_len();
    if (!count) return false;
    size_t pos = 9u;
    for (size_t i = 0; i < *count; ++i) {
      uint32_t klen = 0;
      if (!detail::read_u32_le(data_, len_, pos, klen)) return false;
      pos += 4u;
      if (len_ - pos < klen) return false;
      const std::string_view k(reinterpret_cast<const char*>(data_ + pos), klen);
      pos += klen;
      NveValueTag vtag = NVE_VAL_NULL;
      size_t vend = 0;
      if (!detail::skip_validated_bin_value(data_, len_, pos, vtag, vend)) {
        return false;
      }
      fn(k, BinValueView(data_ + pos, vend - pos, vtag));
      pos = vend;
    }
    return true;
  }

 private:
  BinValueView(const uint8_t* data, size_t len, NveValueTag tag) noexcept
      : data_(data), len_(len), tag_(tag) {}

  const uint8_t* data_{nullptr};
  size_t len_{0};
  NveValueTag tag_{NVE_VAL_NULL};
};

} // namespace nve
