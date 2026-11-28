// NVE host runtime: one published table bound to a scene for O(1) per-node reads.
//
// `BoundTable::bind` resolves a generated `NveTableDesc` to the scene's `NveTableView` by key once
// per acquired scene (scene slots are runtime-assigned): O(tables), independent of node capacity —
// sparse rows resolve through the runtime's published row index (`TableReader::row_of_node`).
// `RowRef` reads a field from the node's row, or from the table defaults when the node has no row,
// so absent sparse rows and present rows read uniformly.
#pragma once

#include <bit>
#include <cstdint>
#include <optional>
#include <span>
#include <string_view>

#include "runtime_session.hpp"
#include "table_reader.hpp"
#include "table_word.h"

namespace nve::host {

/// The root is node index 1 / generation 0 (`NodeId::ROOT`).
inline constexpr uint32_t kRootIndex = 1u;

/// One field read from a node's row, or from the table defaults when the node has no row.
class RowRef {
 public:
  RowRef(const TableReader* reader, uint32_t row, const NveTableDesc* desc) noexcept
      : reader_(reader), row_(row), desc_(desc) {}

  [[nodiscard]] bool present() const noexcept { return reader_ != nullptr; }

  /// Decoded field bits (respecting `f.opt`); `nullopt` when absent or invalid.
  [[nodiscard]] std::optional<uint32_t> opt_bits(FieldSpec f) const noexcept {
    if (reader_ != nullptr) return reader_->bits(row_, f);
    uint32_t out = 0u;
    if (desc_ == nullptr || nve_field_opt_bits(desc_->defaults, desc_->words, f, &out) == 0) return std::nullopt;
    return out;
  }
  [[nodiscard]] std::optional<uint32_t> opt_u32(FieldSpec f) const noexcept { return opt_bits(f); }
  [[nodiscard]] std::optional<int32_t> opt_i32(FieldSpec f) const noexcept {
    if (reader_ != nullptr) return reader_->i32(row_, f);
    int32_t out = 0;
    if (desc_ == nullptr || nve_field_opt_i32(desc_->defaults, desc_->words, f, &out) == 0) return std::nullopt;
    return out;
  }
  [[nodiscard]] std::optional<float> opt_f32(FieldSpec f) const noexcept {
    const std::optional<uint32_t> b = opt_bits(f);
    return b ? std::optional<float>(std::bit_cast<float>(*b)) : std::nullopt;
  }
  [[nodiscard]] std::optional<bool> opt_bool(FieldSpec f) const noexcept {
    if (reader_ != nullptr) return reader_->boolean(row_, f);
    const std::optional<uint32_t> b = opt_bits(f);
    if (!b) return std::nullopt;
    if (f.opt == static_cast<uint8_t>(NVE_OPT_PLUS_ONE)) {
      return *b <= 1u ? std::optional<bool>(*b == 1u) : std::nullopt;
    }
    return (*b & 1u) != 0u;
  }
  [[nodiscard]] std::optional<Rgba> opt_color(FieldSpec f) const noexcept {
    const std::optional<uint32_t> b = opt_bits(f);
    return b ? std::optional<Rgba>(Rgba{*b}) : std::nullopt;
  }
  [[nodiscard]] std::optional<LengthValue> opt_length(FieldSpec f) const noexcept {
    const std::optional<uint32_t> b = opt_bits(f);
    return b ? std::optional<LengthValue>(LengthValue::decode(*b)) : std::nullopt;
  }
  [[nodiscard]] std::optional<GridLineValue> opt_grid_line(FieldSpec f) const noexcept {
    const std::optional<uint32_t> b = opt_bits(f);
    return b ? std::optional<GridLineValue>(GridLineValue::decode(*b)) : std::nullopt;
  }

  [[nodiscard]] uint32_t bits(FieldSpec f) const noexcept { return opt_bits(f).value_or(0u); }
  [[nodiscard]] float f32(FieldSpec f) const noexcept { return opt_f32(f).value_or(0.0f); }
  [[nodiscard]] int32_t i32(FieldSpec f) const noexcept { return opt_i32(f).value_or(0); }
  [[nodiscard]] bool boolean(FieldSpec f) const noexcept { return opt_bool(f).value_or(false); }
  [[nodiscard]] LengthValue length(FieldSpec f) const noexcept {
    return opt_length(f).value_or(LengthValue{NVE_LENGTH_AUTO, 0.0f});
  }
  [[nodiscard]] GridLineValue grid_line(FieldSpec f) const noexcept {
    return opt_grid_line(f).value_or(GridLineValue{NVE_GRID_LINE_AUTO, 0, 0u});
  }

  [[nodiscard]] uint64_t u64(FieldSpec f) const noexcept {
    if (reader_ != nullptr) return reader_->u64(row_, f).value_or(0u);
    return desc_ != nullptr && f.kind != static_cast<uint8_t>(NVE_FIELD_STR)
               ? nve_field_u64(desc_->defaults, desc_->words, f)
               : 0u;
  }
  [[nodiscard]] int64_t i64(FieldSpec f) const noexcept { return static_cast<int64_t>(u64(f)); }
  [[nodiscard]] double f64(FieldSpec f) const noexcept { return std::bit_cast<double>(u64(f)); }

  /// `true` when this node's row is present and `f` is explicitly set (word 0 explicit bit on
  /// inherited tables; always true for non-inherited fields on a present row).
  [[nodiscard]] bool is_explicit(FieldSpec f) const noexcept {
    return reader_ != nullptr && reader_->is_explicit(row_, f);
  }

  /// Per-table `EventSet` listener bitmask word (`0` when absent or `NVE_NO_EVENTS_WORD`).
  [[nodiscard]] uint32_t events(uint8_t events_word) const noexcept {
    return reader_ != nullptr ? reader_->events(row_, events_word) : 0u;
  }

  /// `Str` field text (empty when absent or out of the slab).
  [[nodiscard]] std::string_view str(FieldSpec f, const SceneReader& scene) const noexcept {
    if (reader_ == nullptr) return {};
    const std::optional<StrSpan> span = reader_->str(row_, f);
    if (!span || span->len == 0u) return {};
    return scene.text(span->offset, span->len).value_or(std::string_view());
  }

 private:
  const TableReader* reader_ = nullptr;
  uint32_t row_ = 0u;
  const NveTableDesc* desc_ = nullptr;
};

class BoundTable {
 public:
  /// Resolves `desc` in `scene`; afterwards `at()` / `row_of()` read this scene until rebound.
  void bind(const SceneReader& scene, const NveTableDesc& desc) noexcept {
    desc_ = &desc;
    reader_ = scene.find_table(Key64{desc.key});
    node_rows_ = scene.rows();
  }

  [[nodiscard]] bool published() const noexcept { return reader_.has_value(); }
  [[nodiscard]] const NveTableDesc* desc() const noexcept { return desc_; }
  [[nodiscard]] std::optional<uint32_t> slot() const noexcept {
    return reader_ ? std::optional<uint32_t>(reader_->slot()) : std::nullopt;
  }
  [[nodiscard]] std::span<const uint32_t> dirty_rows() const noexcept {
    return reader_ ? reader_->dirty_rows() : std::span<const uint32_t>();
  }
  [[nodiscard]] const std::optional<TableReader>& reader() const noexcept { return reader_; }

  /// Row of node `index`, if it has one (O(1)). A dense table stores a (zeroed) row at every
  /// index, so there the node's `table_set` bit decides whether the row is the node's.
  [[nodiscard]] std::optional<uint32_t> row_of(uint32_t index) const noexcept {
    if (!reader_ || (!reader_->is_sparse() && !attached(index))) return std::nullopt;
    return reader_->row_of_node(index);
  }

  [[nodiscard]] bool has(uint32_t index) const noexcept { return row_of(index).has_value(); }

  /// Node `index`'s row, or the defaults when it has none.
  [[nodiscard]] RowRef at(uint32_t index) const noexcept {
    if (const std::optional<uint32_t> r = row_of(index)) return RowRef(&*reader_, *r, desc_);
    return RowRef(nullptr, 0u, desc_);
  }

 private:
  [[nodiscard]] bool attached(uint32_t index) const noexcept {
    const uint32_t slot = reader_->slot();
    if (index >= node_rows_.size() || slot >= 128u) return false;
    return (node_rows_[index].table_set[slot >> 5u] & (uint32_t{1} << (slot & 31u))) != 0u;
  }

  const NveTableDesc* desc_ = nullptr;
  std::optional<TableReader> reader_;
  std::span<const NveNodeRow> node_rows_;  // borrowed from the bound scene
};

}  // namespace nve::host
