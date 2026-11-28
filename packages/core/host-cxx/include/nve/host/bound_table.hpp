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

  [[nodiscard]] uint32_t bits(FieldSpec f) const noexcept {
    if (reader_ != nullptr) {
      if (const std::optional<uint32_t> v = reader_->bits(row_, f)) return *v;
    }
    return desc_ != nullptr && nve_field_fits(f, desc_->words) != 0 ? nve_field_bits(desc_->defaults[f.word], f)
                                                                    : 0u;
  }
  [[nodiscard]] float f32(FieldSpec f) const noexcept { return std::bit_cast<float>(bits(f)); }
  [[nodiscard]] int32_t i32(FieldSpec f) const noexcept { return nve_bits_sign_extend(bits(f), f.bits); }
  [[nodiscard]] bool boolean(FieldSpec f) const noexcept { return (bits(f) & 1u) != 0u; }
  [[nodiscard]] LengthValue length(FieldSpec f) const noexcept { return LengthValue::decode(bits(f)); }

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
