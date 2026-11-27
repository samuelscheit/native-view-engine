// NVE host runtime: bounds-checked little-endian `BinReader` / `BinWriter` plus the compact
// tagged `NveBinValue` format (byte-identical to Rust `nve_abi::encode_bin_value`).
//
// Wire layout (all integers little-endian):
//   Null [0] | Bool [1, 0|1] | I64 [2, i64] | F64 [3, f64] | Str [4, len u32, utf8]
//   Bytes [5, len u32, raw] | Array [6, payload u32, count u32, elems...]
//   Map [7, payload u32, count u32, (klen u32, key utf8, value)...]
// `payload` counts every byte after the payload field, so containers skip in O(1).
//
// Errors are sticky: the first failure is recorded, later reads return zero values, and callers
// check `ok()` once after decoding. Reads never allocate and never throw.
#pragma once

#include <bit>
#include <cstddef>
#include <cstdint>
#include <limits>
#include <span>
#include <string_view>
#include <utility>
#include <vector>

#include "api.h"
#include "utf8.h"

namespace nve::host {

/// `NveBinValue` tag (Rust `nve_abi::NveValueTag`).
enum class BinTag : uint8_t { Null = 0, Bool = 1, I64 = 2, F64 = 3, Str = 4, Bytes = 5, Array = 6, Map = 7 };

/// Maximum container nesting depth (Rust `NVE_BIN_VALUE_MAX_DEPTH`).
inline constexpr uint32_t kBinValueMaxDepth = 64;

enum class BinError : uint8_t {
  None = 0,
  Truncated,      ///< read past the end of the buffer
  UnexpectedTag,  ///< value has a different tag than requested
  InvalidTag,     ///< tag byte outside `0..=7`
  InvalidBool,    ///< bool byte other than 0 / 1
  InvalidUtf8,    ///< string or map key is not well-formed UTF-8
  TooLarge,       ///< length does not fit the `u32` wire field
  MaxDepth,       ///< container nesting deeper than `kBinValueMaxDepth`
  TrailingBytes,  ///< container payload length disagrees with its contents
  InvalidMark,    ///< `BinWriter::end` got a mark that is not an open container in this buffer
};

/// Stable name of `e` (`"Truncated"`), for error messages.
[[nodiscard]] constexpr const char* to_string(BinError e) noexcept {
  switch (e) {
    case BinError::None: return "None";
    case BinError::Truncated: return "Truncated";
    case BinError::UnexpectedTag: return "UnexpectedTag";
    case BinError::InvalidTag: return "InvalidTag";
    case BinError::InvalidBool: return "InvalidBool";
    case BinError::InvalidUtf8: return "InvalidUtf8";
    case BinError::TooLarge: return "TooLarge";
    case BinError::MaxDepth: return "MaxDepth";
    case BinError::TrailingBytes: return "TrailingBytes";
    case BinError::InvalidMark: return "InvalidMark";
  }
  return "Unknown";
}

/// Bounds-checked zero-copy cursor over little-endian bytes.
class BinReader {
 public:
  explicit BinReader(std::span<const uint8_t> data) noexcept : data_(data), limit_(data.size()) {}

  [[nodiscard]] bool ok() const noexcept { return error_ == BinError::None; }
  [[nodiscard]] BinError error() const noexcept { return error_; }
  [[nodiscard]] size_t position() const noexcept { return pos_; }
  [[nodiscard]] size_t remaining() const noexcept { return limit_ - pos_; }
  [[nodiscard]] bool at_end() const noexcept { return pos_ == limit_; }

  // ---- raw little-endian primitives -------------------------------------------------------
  uint8_t u8() noexcept {
    const uint8_t* p = take(1);
    return p != nullptr ? p[0] : 0u;
  }
  uint16_t u16() noexcept {
    const uint8_t* p = take(2);
    return p != nullptr ? nve_load_u16_le(p) : 0u;
  }
  uint32_t u32() noexcept {
    const uint8_t* p = take(4);
    return p != nullptr ? nve_load_u32_le(p) : 0u;
  }
  uint64_t u64() noexcept {
    const uint8_t* p = take(8);
    return p != nullptr ? nve_load_u64_le(p) : 0u;
  }
  int32_t i32() noexcept { return std::bit_cast<int32_t>(u32()); }
  int64_t i64() noexcept { return std::bit_cast<int64_t>(u64()); }
  float f32() noexcept { return std::bit_cast<float>(u32()); }
  double f64() noexcept { return std::bit_cast<double>(u64()); }

  /// Next `n` bytes, borrowed (empty on failure).
  std::span<const uint8_t> bytes(size_t n) noexcept {
    const uint8_t* p = take(n);
    return p != nullptr ? std::span<const uint8_t>(p, n) : std::span<const uint8_t>();
  }

  /// Next `n` bytes as validated UTF-8, borrowed (empty on failure).
  std::string_view utf8(size_t n) noexcept {
    const std::span<const uint8_t> b = bytes(n);
    if (!ok()) return {};
    if (nve_utf8_valid(b.data(), b.size()) == 0) return fail<std::string_view>(BinError::InvalidUtf8);
    return {reinterpret_cast<const char*>(b.data()), b.size()};
  }

  // ---- tagged NveBinValue -----------------------------------------------------------------
  /// Tag of the next value without consuming it (`BinTag::Null` and an error when truncated).
  BinTag peek_tag() noexcept {
    if (pos_ >= limit_) return fail<BinTag>(BinError::Truncated);
    const uint8_t t = data_[pos_];
    if (t > static_cast<uint8_t>(BinTag::Map)) return fail<BinTag>(BinError::InvalidTag);
    return static_cast<BinTag>(t);
  }

  /// Consumes a `Null` and returns `true`; leaves any other value in place.
  bool read_null() noexcept {
    if (!ok() || peek_tag() != BinTag::Null || !ok()) return false;
    ++pos_;
    return true;
  }

  bool read_bool() noexcept {
    if (!expect(BinTag::Bool)) return false;
    const uint8_t b = u8();
    if (b > 1u) return fail<bool>(BinError::InvalidBool);
    return b == 1u;
  }

  int64_t read_i64() noexcept { return expect(BinTag::I64) ? i64() : 0; }

  /// `F64`, or an `I64` widened to `double` (Rust `BinValueView::as_number`).
  double read_f64() noexcept {
    const BinTag t = peek_tag();
    if (t == BinTag::I64) return static_cast<double>(read_i64());
    return expect(BinTag::F64) ? f64() : 0.0;
  }

  std::string_view read_str() noexcept { return expect(BinTag::Str) ? utf8(u32()) : std::string_view(); }

  std::span<const uint8_t> read_bytes() noexcept {
    return expect(BinTag::Bytes) ? bytes(u32()) : std::span<const uint8_t>();
  }

  /// A container being read: its entry count, and the bounds `end` checks and restores.
  struct ReadMark {
    uint32_t count = 0;
    size_t end = 0;
    size_t outer_limit = 0;
  };

  /// Opens an `Array` (count 0 on failure; counts the payload cannot hold are rejected). Until
  /// `end(mark)`, no read can pass its payload, so a crafted entry never reads its parent's bytes.
  ReadMark begin_array() noexcept { return open(BinTag::Array, 1u); }

  /// Opens a `Map` like `begin_array`; read each entry as `read_map_key()` then a value.
  ReadMark begin_map() noexcept { return open(BinTag::Map, 5u); }

  /// Closes `mark`'s container: its payload must have been read exactly (`TrailingBytes`).
  void end(const ReadMark& mark) noexcept {
    limit_ = mark.outer_limit;
    if (ok() && pos_ != mark.end) (void)fail<bool>(BinError::TrailingBytes);
  }

  std::string_view read_map_key() noexcept { return utf8(u32()); }

  /// Skips one value in O(1) (containers jump over their payload).
  void skip_value() noexcept {
    const BinTag t = peek_tag();
    if (!ok()) return;
    ++pos_;
    switch (t) {
      case BinTag::Null:
        return;
      case BinTag::Bool:
        (void)bytes(1);
        return;
      case BinTag::I64:
      case BinTag::F64:
        (void)bytes(8);
        return;
      case BinTag::Str:
      case BinTag::Bytes:
      case BinTag::Array:
      case BinTag::Map:
        (void)bytes(u32());
        return;
    }
  }

  /// The next value, fully validated like `check_value`, borrowed (empty on failure).
  std::span<const uint8_t> read_checked_value() noexcept {
    const size_t start = pos_;
    check_value();
    return ok() ? data_.subspan(start, pos_ - start) : std::span<const uint8_t>();
  }

  /// The next complete encoded value, borrowed, skipped in O(1) without validating its contents.
  std::span<const uint8_t> read_raw_value() noexcept {
    const size_t start = pos_;
    skip_value();
    return ok() ? data_.subspan(start, pos_ - start) : std::span<const uint8_t>();
  }

  /// Fully validates the next value (structure, bool range, UTF-8, depth, container lengths)
  /// and consumes it, exactly like Rust `BinValueView::parse`.
  void check_value() noexcept { check_value_at(0u); }

  /// `BinError::None` when `bytes` holds exactly one valid `NveBinValue`.
  [[nodiscard]] static BinError validate(std::span<const uint8_t> bytes) noexcept {
    BinReader r(bytes);
    r.check_value();
    if (r.ok() && !r.at_end()) return BinError::TrailingBytes;
    return r.error();
  }

 private:
  template <class T>
  T fail(BinError e) noexcept {
    if (error_ == BinError::None) error_ = e;
    return T{};
  }

  const uint8_t* take(size_t n) noexcept {
    if (!ok()) return nullptr;
    if (n > limit_ - pos_) return fail<const uint8_t*>(BinError::Truncated);
    const uint8_t* p = data_.data() + pos_;
    pos_ += n;
    return p;
  }

  bool expect(BinTag tag) noexcept {
    const BinTag t = peek_tag();
    if (!ok()) return false;
    if (t != tag) return fail<bool>(BinError::UnexpectedTag);
    ++pos_;
    return true;
  }

  /// Opens a container; `end` receives the offset one past its payload.
  uint32_t open_container(BinTag tag, uint32_t min_entry_bytes, size_t* end) noexcept {
    if (!expect(tag)) return 0u;
    const uint32_t payload = u32();
    if (!ok()) return 0u;
    if (payload < 4u || payload > remaining()) return fail<uint32_t>(BinError::Truncated);
    *end = pos_ + payload;
    const uint32_t count = u32();
    if (count > (payload - 4u) / min_entry_bytes) return fail<uint32_t>(BinError::Truncated);
    return count;
  }

  ReadMark open(BinTag tag, uint32_t min_entry_bytes) noexcept {
    ReadMark mark{0u, pos_, limit_};
    size_t end = 0;
    const uint32_t count = open_container(tag, min_entry_bytes, &end);
    if (!ok()) return mark;
    mark.count = count;
    mark.end = end;
    limit_ = end;
    return mark;
  }

  void check_value_at(uint32_t depth) noexcept {
    const BinTag t = peek_tag();
    if (!ok()) return;
    switch (t) {
      case BinTag::Null:
      case BinTag::I64:
      case BinTag::F64:
        skip_value();
        return;
      case BinTag::Bool:
        (void)read_bool();
        return;
      case BinTag::Str:
        (void)read_str();
        return;
      case BinTag::Bytes:
        (void)read_bytes();
        return;
      case BinTag::Array:
      case BinTag::Map: {
        if (depth >= kBinValueMaxDepth) {
          (void)fail<bool>(BinError::MaxDepth);
          return;
        }
        const bool is_map = t == BinTag::Map;
        size_t end = 0;
        const uint32_t count = open_container(t, is_map ? 5u : 1u, &end);
        if (!ok()) return;
        // Like Rust's `&bytes[..container_end]`: no child read may escape its container.
        const size_t outer_limit = limit_;
        limit_ = end;
        for (uint32_t i = 0; i < count && ok(); ++i) {
          if (is_map) (void)read_map_key();
          check_value_at(depth + 1u);
        }
        limit_ = outer_limit;
        if (ok() && pos_ != end) (void)fail<bool>(BinError::TrailingBytes);
        return;
      }
    }
  }

  std::span<const uint8_t> data_;
  size_t pos_ = 0;
  size_t limit_;  ///< reads stop here: `data_.size()`, or the enclosing container's end while validating
  BinError error_ = BinError::None;
};

/// Little-endian writer; reuse one instance with `clear()` for allocation-free steady state.
class BinWriter {
 public:
  /// An open container: its payload-length field and the writer generation that opened it.
  struct Mark {
    size_t len_pos;
    uint32_t generation;
  };

  BinWriter() = default;
  explicit BinWriter(size_t capacity) { buf_.reserve(capacity); }

  [[nodiscard]] bool ok() const noexcept { return error_ == BinError::None; }
  [[nodiscard]] BinError error() const noexcept { return error_; }
  [[nodiscard]] std::span<const uint8_t> bytes() const noexcept { return buf_; }
  [[nodiscard]] size_t size() const noexcept { return buf_.size(); }
  [[nodiscard]] std::vector<uint8_t> take() && noexcept { return std::move(buf_); }
  /// Empties the buffer (keeping capacity) and invalidates every outstanding `Mark`.
  void clear() noexcept {
    buf_.clear();
    error_ = BinError::None;
    if (++generation_ == 0u) generation_ = 1u;  // never 0: a zero-initialized `Mark{}` must not match
  }

  // ---- raw little-endian primitives -------------------------------------------------------
  void u8(uint8_t v) { buf_.push_back(v); }
  void u16(uint16_t v) { put<2>([&](uint8_t* p) { nve_store_u16_le(p, v); }); }
  void u32(uint32_t v) { put<4>([&](uint8_t* p) { nve_store_u32_le(p, v); }); }
  void u64(uint64_t v) { put<8>([&](uint8_t* p) { nve_store_u64_le(p, v); }); }
  void i32(int32_t v) { u32(std::bit_cast<uint32_t>(v)); }
  void i64(int64_t v) { u64(std::bit_cast<uint64_t>(v)); }
  void f32(float v) { u32(std::bit_cast<uint32_t>(v)); }
  void f64(double v) { u64(std::bit_cast<uint64_t>(v)); }
  void raw(std::span<const uint8_t> b) { buf_.insert(buf_.end(), b.begin(), b.end()); }

  // ---- tagged NveBinValue -----------------------------------------------------------------
  void write_null() { tag(BinTag::Null); }
  void write_bool(bool v) {
    tag(BinTag::Bool);
    u8(v ? 1u : 0u);
  }
  void write_i64(int64_t v) {
    tag(BinTag::I64);
    i64(v);
  }
  void write_f64(double v) {
    tag(BinTag::F64);
    f64(v);
  }
  /// Writes a `Str`. Text that is not well-formed UTF-8 records `InvalidUtf8`, and a length above
  /// `u32` records `TooLarge`; either way nothing at all is written (no orphaned tag or length).
  void write_str(std::string_view s) {
    const auto* p = reinterpret_cast<const uint8_t*>(s.data());
    if (!admit(p, s.size(), true)) return;
    tag(BinTag::Str);
    sized(p, s.size());
  }
  /// Writes `Bytes`; a length above `u32` records `TooLarge` and writes nothing.
  void write_bytes(std::span<const uint8_t> b) {
    if (!admit(b.data(), b.size(), false)) return;
    tag(BinTag::Bytes);
    sized(b.data(), b.size());
  }
  /// Writes a map key; invalid UTF-8 or an oversized key records an error and writes nothing.
  void write_map_key(std::string_view key) {
    const auto* p = reinterpret_cast<const uint8_t*>(key.data());
    if (admit(p, key.size(), true)) sized(p, key.size());
  }

  /// Opens an `Array` of `count` values; close it with `end(mark)` after writing them.
  [[nodiscard]] Mark begin_array(uint32_t count) { return begin(BinTag::Array, count); }
  /// Opens a `Map` of `count` entries (`write_map_key` + value each); close it with `end(mark)`.
  [[nodiscard]] Mark begin_map(uint32_t count) { return begin(BinTag::Map, count); }

  /// Closes the container opened at `m`. A mark from before `clear()` (even once the buffer has
  /// refilled), past the buffer, not right after an `Array` / `Map` tag, or already closed (an open
  /// container's length field still holds 0; a closed one holds its payload >= 4) records
  /// `InvalidMark` and writes nothing.
  void end(Mark m) noexcept {
    if (m.generation != generation_ || m.len_pos < 1u || buf_.size() < 8u || m.len_pos > buf_.size() - 8u ||
        !is_container_tag(buf_[m.len_pos - 1u]) || nve_load_u32_le(buf_.data() + m.len_pos) != 0u) {
      return record(BinError::InvalidMark);
    }
    const size_t payload = buf_.size() - m.len_pos - 4u;
    if (payload > std::numeric_limits<uint32_t>::max()) return record(BinError::TooLarge);
    nve_store_u32_le(buf_.data() + m.len_pos, static_cast<uint32_t>(payload));
  }

 private:
  template <size_t N, class Fn>
  void put(Fn&& store) {
    const size_t at = buf_.size();
    buf_.resize(at + N);
    store(buf_.data() + at);
  }

  void tag(BinTag t) { u8(static_cast<uint8_t>(t)); }

  void record(BinError e) noexcept {
    if (error_ == BinError::None) error_ = e;
  }

  /// Checks a length-prefixed run before anything is written; records the error on rejection.
  bool admit(const uint8_t* p, size_t n, bool utf8) noexcept {
    if (n > std::numeric_limits<uint32_t>::max()) {
      record(BinError::TooLarge);
      return false;
    }
    if (utf8 && nve_utf8_valid(p, n) == 0) {
      record(BinError::InvalidUtf8);
      return false;
    }
    return true;
  }

  /// Length + bytes of an admitted run (`n` fits `u32`).
  void sized(const uint8_t* p, size_t n) {
    u32(static_cast<uint32_t>(n));
    buf_.insert(buf_.end(), p, p + n);
  }

  static constexpr bool is_container_tag(uint8_t b) noexcept {
    return b == static_cast<uint8_t>(BinTag::Array) || b == static_cast<uint8_t>(BinTag::Map);
  }

  Mark begin(BinTag t, uint32_t count) {
    tag(t);
    const Mark m{buf_.size(), generation_};
    u32(0u);
    u32(count);
    return m;
  }

  std::vector<uint8_t> buf_;
  BinError error_ = BinError::None;
  uint32_t generation_ = 1u;  ///< bumped by `clear()`; a zero-initialized `Mark{}` never matches
};

}  // namespace nve::host
