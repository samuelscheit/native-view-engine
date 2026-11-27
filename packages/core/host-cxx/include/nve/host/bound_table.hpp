// NVE host runtime: one published table bound to a scene for O(1) per-node reads.
//
// `BoundTable::bind` resolves a generated `NveTableDesc` to the scene's `NveTableView` by key once
// per acquired scene (scene slots are runtime-assigned) and, for sparse tables, builds a dense
// node-index -> row map in reused storage. `RowRef` reads a field from the node's row, or from the
// table defaults when the node has no row, so absent sparse rows and present rows read uniformly.
#pragma once

#include <bit>
#include <cstdint>
#include <optional>
#include <span>
#include <string_view>
#include <vector>

#include "runtime_session.hpp"
#include "table_reader.hpp"
#include "table_word.h"

namespace nve::host {

/// Packed `NodeId` (`index:24 | generation:8`), as carried by rows, events, and `removed_ids`.
[[nodiscard]] constexpr uint32_t node_index(uint32_t id) noexcept { return id & 0x00FFFFFFu; }
[[nodiscard]] constexpr uint8_t node_generation(uint32_t id) noexcept { return static_cast<uint8_t>(id >> 24u); }
[[nodiscard]] constexpr uint32_t make_node_id(uint32_t index, uint8_t generation) noexcept {
  return (index & 0x00FFFFFFu) | (uint32_t{generation} << 24u);
}
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
  [[nodiscard]] std::string_view str(FieldSpec f, const RuntimeSceneReader& scene) const noexcept {
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
  static constexpr uint32_t kNoRow = UINT32_MAX;

  /// Resolves `desc` in `scene`; afterwards `at()` / `row_of()` read this scene until rebound.
  void bind(const RuntimeSceneReader& scene, const NveTableDesc& desc) {
    desc_ = &desc;
    reader_ = scene.find_table(Key64{desc.key});
    rows_by_index_.clear();
    if (!reader_ || !reader_->is_sparse()) return;
    rows_by_index_.assign(scene.node_capacity(), kNoRow);
    for (uint32_t r = 0; r < reader_->row_count(); ++r) {
      const uint32_t index = node_index(reader_->owner(r).value_or(0u));
      if (index < rows_by_index_.size()) rows_by_index_[index] = r;
    }
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

  /// Row of node `index`, if it has one.
  [[nodiscard]] std::optional<uint32_t> row_of(uint32_t index) const noexcept {
    if (!reader_) return std::nullopt;
    if (!reader_->is_sparse()) return index < reader_->row_count() ? std::optional<uint32_t>(index) : std::nullopt;
    if (index >= rows_by_index_.size() || rows_by_index_[index] == kNoRow) return std::nullopt;
    return rows_by_index_[index];
  }

  [[nodiscard]] bool has(uint32_t index) const noexcept { return row_of(index).has_value(); }

  /// Node `index`'s row, or the defaults when it has none.
  [[nodiscard]] RowRef at(uint32_t index) const noexcept {
    if (const std::optional<uint32_t> r = row_of(index)) return RowRef(&*reader_, *r, desc_);
    return RowRef(nullptr, 0u, desc_);
  }

 private:
  const NveTableDesc* desc_ = nullptr;
  std::optional<TableReader> reader_;
  std::vector<uint32_t> rows_by_index_;
};

}  // namespace nve::host
