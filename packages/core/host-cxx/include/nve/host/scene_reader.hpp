// NVE host runtime: validated zero-copy `SceneReader` over a borrowed `NveSceneView` (the one
// scene ABI every host reads, published by `libnve_runtime` straight from `NodeStore` slabs).
//
// `open()` validates `NVE_RUNTIME_SCHEMA_HASH`, every borrowed slab pointer/count pair, UTF-8
// `text_slab`, and every published `NveTableView` once; afterwards reads are O(1) bounds checks
// that never re-validate.
#pragma once

#include <algorithm>
#include <cstddef>
#include <cstdint>
#include <optional>
#include <span>
#include <string_view>

#include "../generated/nve_runtime_abi.generated.h"
#include "table_reader.hpp"
#include "utf8.h"
#include "xxh64.hpp"

namespace nve::host {

enum class SceneError : uint8_t {
  None = 0,
  NullView,        ///< view pointer is null
  SchemaMismatch,  ///< `schema_hash` differs from the generated `NVE_RUNTIME_SCHEMA_HASH`
  NullSlab,        ///< a slab pointer is null while its count is non-zero
  MisalignedSlab,  ///< a non-empty slab pointer violates its element type alignment
  InvalidUtf8,     ///< `text_slab` is not well-formed UTF-8
  InvalidTable,    ///< a published `NveTableView` is malformed, its slot >= 128 or repeated
  InvalidIndex,    ///< a `dirty_indices` / `moved_indices` entry is >= `node_capacity`
  UnsortedScrollOffsets,  ///< `scroll_offsets` node ids are not strictly ascending
  UnsortedDirtyResources,  ///< `dirty_resources` ids are not strictly ascending
};

/// Validated zero-copy reader over a borrowed `NveSceneView`.
class SceneReader {
 public:
  SceneReader() noexcept = default;

  /// Validates `view` once; `error` (optional) receives the reason on failure.
  [[nodiscard]] static std::optional<SceneReader> open(
      const NveSceneView* view,
      Key64 expected_schema = Key64{NVE_RUNTIME_SCHEMA_HASH},
      SceneError* error = nullptr) noexcept {
    const SceneError e = validate(view, expected_schema);
    if (error != nullptr) *error = e;
    if (e != SceneError::None) return std::nullopt;
    return SceneReader(*view);
  }

  [[nodiscard]] uint64_t frame_id() const noexcept { return frame_id_; }
  /// Bumped by every runtime scene reset; ids and caches from another epoch are stale.
  [[nodiscard]] uint32_t scene_epoch() const noexcept { return scene_epoch_; }
  [[nodiscard]] uint32_t node_capacity() const noexcept { return static_cast<uint32_t>(rows_.size()); }
  [[nodiscard]] std::span<const NveNodeRow> rows() const noexcept { return rows_; }
  [[nodiscard]] std::span<const NveLayoutRect> rects() const noexcept { return rects_; }
  /// Live nodes whose content (row, tables, children) changed or that were created.
  [[nodiscard]] std::span<const uint32_t> dirty_indices() const noexcept { return dirty_; }
  /// Live nodes, disjoint from `dirty_indices`, whose box relative to their parent's rect (origin,
  /// width, height) changed. A moved node's descendants are not listed (their local boxes held).
  [[nodiscard]] std::span<const uint32_t> moved_indices() const noexcept { return moved_; }
  [[nodiscard]] std::span<const uint32_t> removed_ids() const noexcept { return removed_; }
  [[nodiscard]] std::span<const uint32_t> children_slab() const noexcept { return children_; }
  [[nodiscard]] std::string_view text_slab() const noexcept { return text_; }
  [[nodiscard]] uint32_t table_count() const noexcept { return static_cast<uint32_t>(tables_.size()); }
  /// Visual scroll shifts sorted by `node_id`; subtract from descendants' (pre-scroll) rects.
  [[nodiscard]] std::span<const NveScrollOffset> scroll_offsets() const noexcept { return scroll_; }
  /// Live resource ids (re)defined on this frame, strictly ascending: drop host caches keyed by them.
  [[nodiscard]] std::span<const uint32_t> dirty_resources() const noexcept { return dirty_resources_; }
  /// Whether resource `id` was (re)defined on this frame (O(log n); ascending order checked on open).
  [[nodiscard]] bool resource_dirty(uint32_t id) const noexcept {
    return std::binary_search(dirty_resources_.begin(), dirty_resources_.end(), id);
  }

  /// Visual shift of scroll container `node_id` (`{0, 0}` when not scrolled; O(log n)).
  [[nodiscard]] NveScrollOffset scroll_offset(uint32_t node_id) const noexcept {
    const auto it = std::lower_bound(scroll_.begin(), scroll_.end(), node_id,
                                     [](const NveScrollOffset& s, uint32_t id) { return s.node_id < id; });
    return it != scroll_.end() && it->node_id == node_id ? *it : NveScrollOffset{node_id, 0.0f, 0.0f};
  }

  [[nodiscard]] const NveNodeRow* row(uint32_t index) const noexcept {
    return index < rows_.size() ? &rows_[index] : nullptr;
  }

  [[nodiscard]] const NveLayoutRect* rect(uint32_t index) const noexcept {
    return index < rects_.size() ? &rects_[index] : nullptr;
  }

  [[nodiscard]] bool is_live(uint32_t index) const noexcept {
    return index < rows_.size() && (rows_[index].flags & NVE_NODE_ROW_FLAG_LIVE) != 0u;
  }

  [[nodiscard]] bool has_table(uint32_t index, uint8_t slot) const noexcept {
    if (index >= rows_.size() || slot >= 128u) return false;
    const size_t word = static_cast<size_t>(slot) >> 5u;
    const uint32_t bit = static_cast<uint32_t>(slot) & 31u;
    return (rows_[index].table_set[word] & (uint32_t{1} << bit)) != 0u;
  }

  /// `text_slab[offset .. offset + len)`; `nullopt` when out of range or not on UTF-8 boundaries.
  [[nodiscard]] std::optional<std::string_view> text(uint32_t offset, uint32_t len) const noexcept {
    if (offset > text_.size() || len > text_.size() - offset) return std::nullopt;
    if (!is_boundary(offset) || !is_boundary(static_cast<size_t>(offset) + len)) return std::nullopt;
    return text_.substr(offset, len);
  }

  /// `children_slab[offset .. offset + count)`; `nullopt` when out of range.
  [[nodiscard]] std::optional<std::span<const uint32_t>> children(uint32_t offset, uint32_t count) const noexcept {
    if (offset > children_.size() || count > children_.size() - offset) return std::nullopt;
    return children_.subspan(offset, count);
  }

  /// Convenience child-ID slice for node `index` (empty when out of range or invalid span).
  [[nodiscard]] std::optional<std::span<const uint32_t>> node_children(uint32_t index) const noexcept {
    if (index >= rows_.size()) return std::nullopt;
    return children(rows_[index].child_off, rows_[index].child_len);
  }

  /// Visits `(index, row, rect)` for every dirty node index in O(dirty) (in range: checked on open).
  template <class Fn>
  void for_each_dirty(Fn&& visit) const {
    for (const uint32_t index : dirty_) visit(index, rows_[index], rects_[index]);
  }

  [[nodiscard]] std::optional<TableReader> table(uint32_t index) const noexcept {
    if (index >= tables_.size()) return std::nullopt;
    return TableReader::from_view(tables_[index]);
  }

  /// Published table with `key`, if any (O(tables)).
  [[nodiscard]] std::optional<TableReader> find_table(Key64 key) const noexcept {
    for (const NveTableView& t : tables_) {
      if (Key64::from_halves(t.key_lo, t.key_hi) == key) return TableReader::from_view(t);
    }
    return std::nullopt;
  }

 private:
  explicit SceneReader(const NveSceneView& v) noexcept
      : frame_id_(v.frame_id),
        scene_epoch_(v.scene_epoch),
        rows_(span_of(v.rows, v.node_capacity)),
        rects_(span_of(v.rects, v.node_capacity)),
        text_(v.text_slab_len > 0u
                  ? std::string_view(reinterpret_cast<const char*>(v.text_slab), v.text_slab_len)
                  : std::string_view()),
        children_(span_of(v.children_slab, v.children_slab_len)),
        dirty_(span_of(v.dirty_indices, v.dirty_count)),
        moved_(span_of(v.moved_indices, v.moved_count)),
        removed_(span_of(v.removed_ids, v.removed_count)),
        tables_(span_of(v.tables, v.table_count)),
        scroll_(span_of(v.scroll_offsets, v.scroll_count)),
        dirty_resources_(span_of(v.dirty_resources, v.dirty_resource_count)) {}

  template <class T>
  static std::span<const T> span_of(const T* p, uint32_t count) noexcept {
    return count > 0u ? std::span<const T>(p, static_cast<size_t>(count)) : std::span<const T>();
  }

  template <class T>
  static SceneError check_slab(const T* p, uint32_t count) noexcept {
    if (count == 0u) return SceneError::None;
    if (p == nullptr || !count_fits_host(count, sizeof(T))) return SceneError::NullSlab;
    if ((reinterpret_cast<uintptr_t>(p) % alignof(T)) != 0u) return SceneError::MisalignedSlab;
    return SceneError::None;
  }

  static SceneError validate(const NveSceneView* v, Key64 expected_schema) noexcept {
    if (v == nullptr) return SceneError::NullView;
    if (v->schema_hash != expected_schema.value) return SceneError::SchemaMismatch;
    if (const SceneError e = check_slab(v->rows, v->node_capacity); e != SceneError::None) return e;
    if (const SceneError e = check_slab(v->rects, v->node_capacity); e != SceneError::None) return e;
    if (const SceneError e = check_slab(v->text_slab, v->text_slab_len); e != SceneError::None) return e;
    if (const SceneError e = check_slab(v->children_slab, v->children_slab_len); e != SceneError::None) return e;
    if (const SceneError e = check_slab(v->dirty_indices, v->dirty_count); e != SceneError::None) return e;
    if (const SceneError e = check_slab(v->moved_indices, v->moved_count); e != SceneError::None) return e;
    if (const SceneError e = check_slab(v->removed_ids, v->removed_count); e != SceneError::None) return e;
    if (const SceneError e = check_slab(v->tables, v->table_count); e != SceneError::None) return e;
    if (const SceneError e = check_slab(v->scroll_offsets, v->scroll_count); e != SceneError::None) return e;
    if (const SceneError e = check_slab(v->dirty_resources, v->dirty_resource_count); e != SceneError::None) return e;
    if (v->text_slab_len > 0u && nve_utf8_valid(v->text_slab, v->text_slab_len) == 0) {
      return SceneError::InvalidUtf8;
    }
    // Slots index the 128-bit `table_set` of every row: in range and published once.
    uint32_t slots[4]{};
    for (uint32_t i = 0u; i < v->table_count; ++i) {
      const uint32_t slot = v->tables[i].slot;
      if (slot >= 128u || (slots[slot >> 5u] & (uint32_t{1} << (slot & 31u))) != 0u) return SceneError::InvalidTable;
      slots[slot >> 5u] |= uint32_t{1} << (slot & 31u);
      if (!TableReader::from_view(v->tables[i])) return SceneError::InvalidTable;
    }
    const auto in_range = [cap = v->node_capacity](const uint32_t* p, uint32_t n) noexcept {
      return std::all_of(p, p + n, [cap](uint32_t index) { return index < cap; });
    };
    if ((v->dirty_count > 0u && !in_range(v->dirty_indices, v->dirty_count)) ||
        (v->moved_count > 0u && !in_range(v->moved_indices, v->moved_count))) {
      return SceneError::InvalidIndex;
    }
    // `scroll_offset()` and `resource_dirty()` binary-search: ids must be strictly ascending.
    for (uint32_t i = 1u; i < v->scroll_count; ++i) {
      if (v->scroll_offsets[i - 1u].node_id >= v->scroll_offsets[i].node_id) {
        return SceneError::UnsortedScrollOffsets;
      }
    }
    for (uint32_t i = 1u; i < v->dirty_resource_count; ++i) {
      if (v->dirty_resources[i - 1u] >= v->dirty_resources[i]) return SceneError::UnsortedDirtyResources;
    }
    return SceneError::None;
  }

  [[nodiscard]] bool is_boundary(size_t at) const noexcept {
    return at == text_.size() || (static_cast<uint8_t>(text_[at]) & 0xC0u) != 0x80u;
  }

  uint64_t frame_id_ = 0u;
  uint32_t scene_epoch_ = 0u;
  std::span<const NveNodeRow> rows_;
  std::span<const NveLayoutRect> rects_;
  std::string_view text_;
  std::span<const uint32_t> children_;
  std::span<const uint32_t> dirty_;
  std::span<const uint32_t> moved_;
  std::span<const uint32_t> removed_;
  std::span<const NveTableView> tables_;
  std::span<const NveScrollOffset> scroll_;
  std::span<const uint32_t> dirty_resources_;
};

}  // namespace nve::host
