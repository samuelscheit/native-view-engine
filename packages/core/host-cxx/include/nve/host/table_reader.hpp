// NVE host runtime: zero-copy reader over one published word table (`NveTableView`).
//
// Dense tables: row `i` belongs to node index `i` and `owners` is empty. Sparse tables: row `r`
// belongs to raw `NodeId` `owners[r]` (unsorted, swap-removed). Every read is bounds-checked and
// returns `std::nullopt` instead of touching memory outside the published spans.
#pragma once

#include <bit>
#include <cstddef>
#include <cstdint>
#include <optional>
#include <span>

#include "table_word.h"
#include "xxh64.hpp"

namespace nve::host {

using FieldSpec = NveFieldSpec;
using FieldKind = NveFieldKind;

/// `FieldSpec` literal: `field(word, shift, bits, NVE_FIELD_*)`.
[[nodiscard]] constexpr FieldSpec field(uint8_t word, uint8_t shift, uint8_t bits, FieldKind kind) noexcept {
  return FieldSpec{word, shift, bits, static_cast<uint8_t>(kind)};
}

/// Decoded `nve_store::Length`.
struct LengthValue {
  NveLengthUnit unit = NVE_LENGTH_AUTO;
  float value = 0.0f;

  [[nodiscard]] static constexpr LengthValue decode(uint32_t word) noexcept {
    return LengthValue{nve_length_unit(word), std::bit_cast<float>(nve_length_value_bits(word))};
  }
  friend constexpr bool operator==(LengthValue, LengthValue) noexcept = default;
};

/// Packed `RGBA8888` colour (`r | g<<8 | b<<16 | a<<24`).
struct Rgba {
  uint32_t packed = 0u;

  [[nodiscard]] constexpr uint8_t r() const noexcept { return nve_color_channel(packed, 0u); }
  [[nodiscard]] constexpr uint8_t g() const noexcept { return nve_color_channel(packed, 1u); }
  [[nodiscard]] constexpr uint8_t b() const noexcept { return nve_color_channel(packed, 2u); }
  [[nodiscard]] constexpr uint8_t a() const noexcept { return nve_color_channel(packed, 3u); }
  friend constexpr bool operator==(Rgba, Rgba) noexcept = default;
};

/// Two-word string reference `{offset, len}` into the scene text slab.
struct StrSpan {
  uint32_t offset = 0u;
  uint32_t len = 0u;
  friend constexpr bool operator==(StrSpan, StrSpan) noexcept = default;
};

class TableReader {
 public:
  TableReader() = default;

  /// Returns `std::nullopt` when the spans cannot hold `row_count * words` rows (or the owners).
  [[nodiscard]] static std::optional<TableReader> make(Key64 key, uint32_t slot, uint32_t words, uint32_t row_count,
                                                       std::span<const uint32_t> rows,
                                                       std::span<const uint32_t> owners,
                                                       std::span<const uint32_t> dirty_rows) noexcept {
    if (words == 0u || words > NVE_MAX_TABLE_WORDS) return std::nullopt;
    if (rows.size() / words < row_count) return std::nullopt;
    if (!owners.empty() && owners.size() < row_count) return std::nullopt;
    TableReader t;
    t.key_ = key;
    t.slot_ = slot;
    t.words_ = words;
    t.row_count_ = row_count;
    t.rows_ = rows.first(static_cast<size_t>(row_count) * words);
    t.owners_ = owners.empty() ? owners : owners.first(row_count);
    t.dirty_rows_ = dirty_rows;
    return t;
  }

  /// Wraps any `NveTableView`-shaped struct (`key_lo`, `key_hi`, `slot`, `words`, `row_count`,
  /// `dirty_count`, `rows`, `owners`, `dirty_rows`), rejecting null spans with non-zero counts.
  template <class View>
  [[nodiscard]] static std::optional<TableReader> from_view(const View& v) noexcept {
    const size_t row_words = static_cast<size_t>(v.row_count) * v.words;
    if ((row_words > 0u && v.rows == nullptr) || (v.dirty_count > 0u && v.dirty_rows == nullptr)) return std::nullopt;
    const std::span<const uint32_t> rows = row_words > 0u ? std::span<const uint32_t>(v.rows, row_words)
                                                          : std::span<const uint32_t>();
    const std::span<const uint32_t> owners = v.owners != nullptr && v.row_count > 0u
                                                 ? std::span<const uint32_t>(v.owners, v.row_count)
                                                 : std::span<const uint32_t>();
    const std::span<const uint32_t> dirty = v.dirty_count > 0u ? std::span<const uint32_t>(v.dirty_rows, v.dirty_count)
                                                               : std::span<const uint32_t>();
    return make(Key64::from_halves(v.key_lo, v.key_hi), v.slot, v.words, v.row_count, rows, owners, dirty);
  }

  [[nodiscard]] Key64 key() const noexcept { return key_; }
  [[nodiscard]] uint32_t slot() const noexcept { return slot_; }
  [[nodiscard]] uint32_t words() const noexcept { return words_; }
  [[nodiscard]] uint32_t row_count() const noexcept { return row_count_; }
  [[nodiscard]] bool is_sparse() const noexcept { return !owners_.empty(); }
  [[nodiscard]] std::span<const uint32_t> dirty_rows() const noexcept { return dirty_rows_; }

  /// All words of `row` (empty when out of range).
  [[nodiscard]] std::span<const uint32_t> row(uint32_t r) const noexcept {
    return r < row_count_ ? rows_.subspan(static_cast<size_t>(r) * words_, words_) : std::span<const uint32_t>();
  }

  [[nodiscard]] std::optional<uint32_t> word(uint32_t r, uint32_t w) const noexcept {
    if (r >= row_count_ || w >= words_) return std::nullopt;
    return rows_[static_cast<size_t>(r) * words_ + w];
  }

  [[nodiscard]] std::optional<uint32_t> bits(uint32_t r, FieldSpec f) const noexcept {
    const std::optional<uint32_t> w = word(r, f.word);
    if (!w) return std::nullopt;
    return nve_field_bits(*w, f);
  }

  [[nodiscard]] std::optional<uint32_t> u32(uint32_t r, FieldSpec f) const noexcept { return bits(r, f); }
  [[nodiscard]] std::optional<uint32_t> enum_value(uint32_t r, FieldSpec f) const noexcept { return bits(r, f); }
  [[nodiscard]] std::optional<uint32_t> resource(uint32_t r, FieldSpec f) const noexcept { return bits(r, f); }
  [[nodiscard]] std::optional<uint32_t> node(uint32_t r, FieldSpec f) const noexcept { return bits(r, f); }

  [[nodiscard]] std::optional<int32_t> i32(uint32_t r, FieldSpec f) const noexcept {
    const std::optional<uint32_t> w = word(r, f.word);
    if (!w) return std::nullopt;
    return nve_field_i32(*w, f);
  }

  [[nodiscard]] std::optional<float> f32(uint32_t r, FieldSpec f) const noexcept {
    const std::optional<uint32_t> b = bits(r, f);
    if (!b) return std::nullopt;
    return std::bit_cast<float>(*b);
  }

  [[nodiscard]] std::optional<bool> boolean(uint32_t r, FieldSpec f) const noexcept {
    const std::optional<uint32_t> b = bits(r, f);
    if (!b) return std::nullopt;
    return (*b & 1u) != 0u;
  }

  [[nodiscard]] std::optional<Rgba> color(uint32_t r, FieldSpec f) const noexcept {
    const std::optional<uint32_t> b = bits(r, f);
    if (!b) return std::nullopt;
    return Rgba{*b};
  }

  [[nodiscard]] std::optional<LengthValue> length(uint32_t r, FieldSpec f) const noexcept {
    const std::optional<uint32_t> b = bits(r, f);
    if (!b) return std::nullopt;
    return LengthValue::decode(*b);
  }

  [[nodiscard]] std::optional<StrSpan> str(uint32_t r, FieldSpec f) const noexcept {
    const std::optional<uint32_t> offset = word(r, f.word);
    const std::optional<uint32_t> len = word(r, static_cast<uint32_t>(f.word) + 1u);
    if (!offset || !len) return std::nullopt;
    return StrSpan{*offset, *len};
  }

  /// Raw `NodeId` owning sparse row `r`, or the row index itself for dense tables.
  [[nodiscard]] std::optional<uint32_t> owner(uint32_t r) const noexcept {
    if (r >= row_count_) return std::nullopt;
    return owners_.empty() ? r : owners_[r];
  }

  /// Row of `owner` (raw `NodeId` for sparse tables, node index for dense ones). O(rows) sparse.
  [[nodiscard]] std::optional<uint32_t> row_of(uint32_t owner_id) const noexcept {
    if (owners_.empty()) return owner_id < row_count_ ? std::optional<uint32_t>(owner_id) : std::nullopt;
    for (uint32_t r = 0; r < row_count_; ++r) {
      if (owners_[r] == owner_id) return r;
    }
    return std::nullopt;
  }

 private:
  Key64 key_{};
  uint32_t slot_ = 0u;
  uint32_t words_ = 0u;
  uint32_t row_count_ = 0u;
  std::span<const uint32_t> rows_;
  std::span<const uint32_t> owners_;
  std::span<const uint32_t> dirty_rows_;
};

}  // namespace nve::host
