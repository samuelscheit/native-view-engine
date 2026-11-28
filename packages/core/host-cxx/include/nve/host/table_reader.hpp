// NVE host runtime: zero-copy reader over one published word table (`NveTableView`).
//
// Dense tables: row `i` belongs to node index `i` and `owners` is empty. Sparse tables: row `r`
// belongs to raw `NodeId` `owners[r]` (unsorted, swap-removed), and a node's row resolves in O(1)
// through the store's published paged row index (`row_of_node`). Every read is bounds-checked and
// returns `std::nullopt` for out-of-range rows, malformed specs, or absent optional fields.
#pragma once

#include <bit>
#include <cstddef>
#include <cstdint>
#include <optional>
#include <span>

#include "../generated/nve_abi.generated.h"
#include "table_word.h"
#include "xxh64.hpp"

namespace nve::host {

using FieldSpec = NveFieldSpec;
using FieldKind = NveFieldKind;
using OptEncoding = NveOptEncoding;

/// Packed `NodeId` (`index:24 | generation:8`), as carried by rows, owners, events, and `removed_ids`.
[[nodiscard]] constexpr uint32_t node_index(uint32_t id) noexcept { return id & 0x00FFFFFFu; }
[[nodiscard]] constexpr uint8_t node_generation(uint32_t id) noexcept { return static_cast<uint8_t>(id >> 24u); }
[[nodiscard]] constexpr uint32_t make_node_id(uint32_t index, uint8_t generation) noexcept {
  return node_index(index) | (uint32_t{generation} << 24u);
}

namespace detail {
/// Deliberately not `constexpr`: reaching it during constant evaluation is a compile error.
void field_spec_is_invalid();
}  // namespace detail

/// `true` when `count` elements of `elem_size` bytes stay within `max_bytes` (pass `PTRDIFF_MAX`
/// for this host; tests pass 32-bit limits so 64-bit CI exercises 32-bit address spaces).
[[nodiscard]] constexpr bool count_fits(uint64_t count, uint64_t elem_size, uint64_t max_bytes) noexcept {
  return elem_size != 0u && count <= max_bytes / elem_size;
}

/// `count_fits` against this host's `PTRDIFF_MAX` (the largest valid `std::span` byte extent).
[[nodiscard]] constexpr bool count_fits_host(uint64_t count, uint64_t elem_size) noexcept {
  return count_fits(count, elem_size, static_cast<uint64_t>(PTRDIFF_MAX));
}

/// Compile-time `FieldSpec` literal `field(word, shift, bits, NVE_FIELD_*, ...)`: an invalid spec
/// (`bits` outside `1..=32`, `shift + bits > 32`, past word 31, or mismatched optional metadata)
/// fails to compile.
[[nodiscard]] consteval FieldSpec field(uint8_t word, uint8_t shift, uint8_t bits, FieldKind kind,
                                        OptEncoding opt = NVE_OPT_REQUIRED,
                                        uint8_t val_word = NVE_NO_VAL_WORD,
                                        uint8_t explicit_bit = NVE_NO_EXPLICIT_BIT,
                                        uint8_t inherits = 0u) {
  const FieldSpec f{word, shift, bits, static_cast<uint8_t>(kind), static_cast<uint8_t>(opt),
                    val_word, explicit_bit, inherits};
  if (nve_field_valid(f) == 0) detail::field_spec_is_invalid();
  return f;
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

/// Decoded `nve_store::GridLine`.
struct GridLineValue {
  NveGridLineKind kind = NVE_GRID_LINE_AUTO;
  int16_t line = 0;
  uint16_t span = 0u;

  [[nodiscard]] static constexpr GridLineValue decode(uint32_t bits) noexcept {
    const NveGridLine g = nve_grid_line_decode(bits);
    return GridLineValue{static_cast<NveGridLineKind>(g.kind), g.line, g.span};
  }
  [[nodiscard]] constexpr uint32_t encode() const noexcept {
    return nve_grid_line_encode(NveGridLine{static_cast<uint8_t>(kind), line, span});
  }
  friend constexpr bool operator==(GridLineValue, GridLineValue) noexcept = default;
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

  /// Returns `std::nullopt` when the spans cannot hold `row_count * words` rows (or the owners), or
  /// when a sparse table (owners) with rows publishes no row index: its rows could never be
  /// resolved. `row_index_pages` is truncated to whole `NVE_ROW_INDEX_PAGE_LEN` pages.
  [[nodiscard]] static std::optional<TableReader> make(Key64 key, uint32_t slot, uint32_t words, uint32_t row_count,
                                                       std::span<const uint32_t> rows,
                                                       std::span<const uint32_t> owners,
                                                       std::span<const uint32_t> dirty_rows,
                                                       std::span<const uint32_t> row_index_dir = {},
                                                       std::span<const uint32_t> row_index_pages = {}) noexcept {
    if (words == 0u || words > NVE_MAX_TABLE_WORDS) return std::nullopt;
    if (rows.size() / words < row_count) return std::nullopt;
    if (!owners.empty() && owners.size() < row_count) return std::nullopt;
    if (!owners.empty() && row_count > 0u && (row_index_dir.empty() || row_index_pages.size() < kPageLen)) {
      return std::nullopt;
    }
    TableReader t;
    t.key_ = key;
    t.slot_ = slot;
    t.words_ = words;
    t.row_count_ = row_count;
    t.rows_ = rows.first(static_cast<size_t>(row_count) * words);
    t.owners_ = owners.empty() ? owners : owners.first(row_count);
    t.dirty_rows_ = dirty_rows;
    t.row_index_dir_ = row_index_dir;
    t.row_index_pages_ = row_index_pages.first(row_index_pages.size() - row_index_pages.size() % kPageLen);
    return t;
  }

  /// Wraps any `NveTableView`-shaped struct, rejecting bad word counts, spans the address space
  /// cannot hold (32-bit hosts), and null spans with non-zero counts — before forming a span.
  template <class View>
  [[nodiscard]] static std::optional<TableReader> from_view(const View& v) noexcept {
    if (v.words == 0u || v.words > NVE_MAX_TABLE_WORDS) return std::nullopt;
    const uint64_t row_words64 = static_cast<uint64_t>(v.row_count) * v.words;               // <= 2^37, exact
    const uint64_t page_words64 = static_cast<uint64_t>(v.row_index_page_count) * kPageLen;  // <= 2^40, exact
    if (!count_fits_host(row_words64, sizeof(uint32_t)) || !count_fits_host(v.dirty_count, sizeof(uint32_t)) ||
        !count_fits_host(v.row_index_dir_len, sizeof(uint32_t)) || !count_fits_host(page_words64, sizeof(uint32_t))) {
      return std::nullopt;
    }
    const size_t row_words = static_cast<size_t>(row_words64);
    const size_t page_words = static_cast<size_t>(page_words64);
    if ((row_words > 0u && v.rows == nullptr) || (v.dirty_count > 0u && v.dirty_rows == nullptr) ||
        (v.row_index_dir_len > 0u && v.row_index_dir == nullptr) || (page_words > 0u && v.row_index_pages == nullptr)) {
      return std::nullopt;
    }
    const auto span = [](const uint32_t* p, size_t n) {
      return n > 0u ? std::span<const uint32_t>(p, n) : std::span<const uint32_t>();
    };
    return make(Key64::from_halves(v.key_lo, v.key_hi), v.slot, v.words, v.row_count, span(v.rows, row_words),
                span(v.owners, v.owners != nullptr ? v.row_count : 0u), span(v.dirty_rows, v.dirty_count),
                span(v.row_index_dir, v.row_index_dir_len), span(v.row_index_pages, page_words));
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

  /// Raw un-decoded field bits in `row[f.word]`.
  [[nodiscard]] std::optional<uint32_t> raw_bits(uint32_t r, FieldSpec f) const noexcept {
    if (r >= row_count_ || nve_field_fits(f, words_) == 0) return std::nullopt;
    return nve_field_bits(rows_[static_cast<size_t>(r) * words_ + f.word], f);
  }

  /// Decoded field bits of row `r` (respecting `f.opt`); `nullopt` when absent, out of range, or invalid.
  [[nodiscard]] std::optional<uint32_t> bits(uint32_t r, FieldSpec f) const noexcept {
    if (r >= row_count_) return std::nullopt;
    uint32_t out = 0u;
    if (nve_field_opt_bits(row_ptr(r), words_, f, &out) == 0) return std::nullopt;
    return out;
  }

  [[nodiscard]] std::optional<uint32_t> u32(uint32_t r, FieldSpec f) const noexcept { return bits(r, f); }
  [[nodiscard]] std::optional<uint32_t> enum_value(uint32_t r, FieldSpec f) const noexcept { return bits(r, f); }
  [[nodiscard]] std::optional<uint32_t> resource(uint32_t r, FieldSpec f) const noexcept { return bits(r, f); }
  [[nodiscard]] std::optional<uint32_t> node(uint32_t r, FieldSpec f) const noexcept { return bits(r, f); }

  [[nodiscard]] std::optional<int32_t> i32(uint32_t r, FieldSpec f) const noexcept {
    if (r >= row_count_) return std::nullopt;
    int32_t out = 0;
    if (nve_field_opt_i32(row_ptr(r), words_, f, &out) == 0) return std::nullopt;
    return out;
  }

  [[nodiscard]] std::optional<float> f32(uint32_t r, FieldSpec f) const noexcept {
    const std::optional<uint32_t> b = bits(r, f);
    if (!b) return std::nullopt;
    return std::bit_cast<float>(*b);
  }

  [[nodiscard]] std::optional<bool> boolean(uint32_t r, FieldSpec f) const noexcept {
    const std::optional<uint32_t> b = bits(r, f);
    if (!b) return std::nullopt;
    if (f.opt == static_cast<uint8_t>(NVE_OPT_PLUS_ONE)) {
      if (*b > 1u) return std::nullopt;  // `Option<bool>`: 1 -> false (0), 2 -> true (1), 3 -> None
      return *b == 1u;
    }
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

  [[nodiscard]] std::optional<GridLineValue> grid_line(uint32_t r, FieldSpec f) const noexcept {
    const std::optional<uint32_t> b = bits(r, f);
    if (!b) return std::nullopt;
    return GridLineValue::decode(*b);
  }

  [[nodiscard]] std::optional<uint64_t> u64(uint32_t r, FieldSpec f) const noexcept {
    if (r >= row_count_ || f.kind == static_cast<uint8_t>(NVE_FIELD_STR) ||
        nve_field_is_two_word(f.kind) == 0 || nve_field_fits(f, words_) == 0) {
      return std::nullopt;
    }
    return nve_field_u64(row_ptr(r), words_, f);
  }

  [[nodiscard]] std::optional<int64_t> i64(uint32_t r, FieldSpec f) const noexcept {
    const std::optional<uint64_t> v = u64(r, f);
    return v ? std::optional<int64_t>(static_cast<int64_t>(*v)) : std::nullopt;
  }

  [[nodiscard]] std::optional<double> f64(uint32_t r, FieldSpec f) const noexcept {
    const std::optional<uint64_t> v = u64(r, f);
    return v ? std::optional<double>(std::bit_cast<double>(*v)) : std::nullopt;
  }

  /// `{offset, len}` of a `Str` field; `nullopt` for a non-`Str` / malformed spec or past the row.
  [[nodiscard]] std::optional<StrSpan> str(uint32_t r, FieldSpec f) const noexcept {
    if (f.kind != static_cast<uint8_t>(NVE_FIELD_STR) || nve_field_fits(f, words_) == 0) return std::nullopt;
    const std::optional<uint32_t> offset = word(r, f.word);
    const std::optional<uint32_t> len = word(r, static_cast<uint32_t>(f.word) + 1u);
    if (!offset || !len) return std::nullopt;
    return StrSpan{*offset, *len};
  }

  /// `true` when `f` is explicitly set in row `r` (non-inherited fields are always explicit when in range).
  [[nodiscard]] bool is_explicit(uint32_t r, FieldSpec f) const noexcept {
    if (r >= row_count_) return false;
    return nve_field_explicit(row_ptr(r), words_, f) != 0;
  }

  /// Per-table `EventSet` listener mask word (`0` when `events_word` is `NVE_NO_EVENTS_WORD` or OOB).
  [[nodiscard]] uint32_t events(uint32_t r, uint8_t events_word) const noexcept {
    if (r >= row_count_) return 0u;
    return nve_row_events(row_ptr(r), words_, events_word);
  }

  /// Raw `NodeId` owning sparse row `r`, or the row index itself for dense tables.
  [[nodiscard]] std::optional<uint32_t> owner(uint32_t r) const noexcept {
    if (r >= row_count_) return std::nullopt;
    return owners_.empty() ? r : owners_[r];
  }

  /// Row of node slot `index` in O(1): the slot itself (dense), else the published row index.
  [[nodiscard]] std::optional<uint32_t> row_of_node(uint32_t index) const noexcept {
    if (owners_.empty()) return index < row_count_ ? std::optional<uint32_t>(index) : std::nullopt;
    const size_t dir_slot = index >> NVE_ROW_INDEX_PAGE_BITS;
    if (dir_slot >= row_index_dir_.size()) return std::nullopt;
    const size_t page = row_index_dir_[dir_slot];  // `NVE_ROW_INDEX_NO_PAGE` fails the bound below
    if (page >= row_index_pages_.size() / kPageLen) return std::nullopt;
    const uint32_t row_plus_one = row_index_pages_[page * kPageLen + (index & (kPageLen - 1u))];
    if (row_plus_one == 0u || row_plus_one > row_count_) return std::nullopt;
    return row_plus_one - 1u;
  }

  /// Row of `owner` (raw `NodeId` for sparse tables, node index for dense ones) in O(1).
  [[nodiscard]] std::optional<uint32_t> row_of(uint32_t owner_id) const noexcept {
    if (owners_.empty()) return row_of_node(owner_id);
    const std::optional<uint32_t> r = row_of_node(node_index(owner_id));
    return r && owners_[*r] == owner_id ? r : std::nullopt;
  }

 private:
  static constexpr size_t kPageLen = NVE_ROW_INDEX_PAGE_LEN;

  [[nodiscard]] const uint32_t* row_ptr(uint32_t r) const noexcept {
    return rows_.data() + static_cast<size_t>(r) * words_;
  }

  Key64 key_{};
  uint32_t slot_ = 0u;
  uint32_t words_ = 0u;
  uint32_t row_count_ = 0u;
  std::span<const uint32_t> rows_;
  std::span<const uint32_t> owners_;
  std::span<const uint32_t> dirty_rows_;
  std::span<const uint32_t> row_index_dir_;
  std::span<const uint32_t> row_index_pages_;
};

}  // namespace nve::host
