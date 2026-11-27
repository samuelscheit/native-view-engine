// NVE host runtime: validated zero-copy reader over a published `NveSceneView`.
//
// `SceneReader<View>` wraps any `NveSceneView`-shaped struct from the generated ABI header, so this
// static runtime never hard-codes the node record layout: `nodes` keeps its generated element type.
// `open()` validates the schema hash, every pointer/count pair, and the text slab's UTF-8 once;
// afterwards reads are O(1) bounds checks that never re-validate.
#pragma once

#include <cstddef>
#include <cstdint>
#include <optional>
#include <span>
#include <string_view>
#include <type_traits>
#include <utility>

#include "table_reader.hpp"
#include "utf8.h"
#include "xxh64.hpp"

namespace nve::host {

enum class SceneError : uint8_t {
  None = 0,
  NullView,        ///< view pointer is null
  SchemaMismatch,  ///< `schema_hash` differs from the generated `NVE_SCENE_SCHEMA_HASH`
  NullSlab,        ///< a slab pointer is null while its count is non-zero
  MisalignedSlab,  ///< a non-empty slab pointer violates its element type alignment
  InvalidUtf8,     ///< `text_slab` is not well-formed UTF-8
  InvalidTable,    ///< a published `NveTableView` is malformed
  UnsortedScrollOffsets,  ///< `scroll_offsets` node ids are not strictly ascending
  UnsortedDirtyResources,  ///< `dirty_resources` ids are not strictly ascending
};

template <class View>
class SceneReader {
 public:
  using Node = std::remove_cvref_t<decltype(*std::declval<const View&>().nodes)>;
  using Table = std::remove_cvref_t<decltype(*std::declval<const View&>().tables)>;

  /// Validates `view` once; `error` (optional) receives the reason on failure.
  [[nodiscard]] static std::optional<SceneReader> open(const View* view, Key64 expected_schema,
                                                       SceneError* error = nullptr) noexcept {
    const SceneError e = validate(view, expected_schema);
    if (error != nullptr) *error = e;
    if (e != SceneError::None) return std::nullopt;
    return SceneReader(*view);
  }

  [[nodiscard]] uint64_t frame_id() const noexcept { return frame_id_; }
  [[nodiscard]] uint32_t node_count() const noexcept { return static_cast<uint32_t>(nodes_.size()); }
  [[nodiscard]] std::span<const Node> nodes() const noexcept { return nodes_; }
  [[nodiscard]] std::span<const uint32_t> dirty_indices() const noexcept { return dirty_; }
  [[nodiscard]] std::span<const uint32_t> removed_ids() const noexcept { return removed_; }
  [[nodiscard]] std::span<const uint32_t> children_slab() const noexcept { return children_; }
  [[nodiscard]] std::string_view text_slab() const noexcept { return text_; }
  [[nodiscard]] uint32_t table_count() const noexcept { return static_cast<uint32_t>(tables_.size()); }

  /// Node `index`, or null when out of range.
  [[nodiscard]] const Node* node(uint32_t index) const noexcept {
    return index < nodes_.size() ? &nodes_[index] : nullptr;
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

  /// Visits `(index, node)` for every in-range dirty index in O(dirty).
  template <class Fn>
  void for_each_dirty(Fn&& visit) const {
    for (const uint32_t index : dirty_) {
      if (index < nodes_.size()) visit(index, nodes_[index]);
    }
  }

  [[nodiscard]] std::optional<TableReader> table(uint32_t index) const noexcept {
    if (index >= tables_.size()) return std::nullopt;
    return TableReader::from_view(tables_[index]);
  }

  /// Published table with `key`, if any (O(tables)).
  [[nodiscard]] std::optional<TableReader> find_table(Key64 key) const noexcept {
    for (const Table& t : tables_) {
      if (Key64::from_halves(t.key_lo, t.key_hi) == key) return TableReader::from_view(t);
    }
    return std::nullopt;
  }

 private:
  explicit SceneReader(const View& v) noexcept
      : frame_id_(v.frame_id),
        nodes_(span_of(v.nodes, v.node_count)),
        text_(v.text_slab_len > 0u ? std::string_view(reinterpret_cast<const char*>(v.text_slab), v.text_slab_len)
                                   : std::string_view()),
        children_(span_of(v.children_slab, v.children_slab_len)),
        dirty_(span_of(v.dirty_indices, v.dirty_count)),
        removed_(span_of(v.removed_ids, v.removed_count)),
        tables_(span_of(v.tables, v.table_count)) {}

  template <class T>
  static std::span<const T> span_of(const T* p, uint32_t count) noexcept {
    return count > 0u ? std::span<const T>(p, count) : std::span<const T>();
  }

  /// Non-null when counted, and small enough to span on this host (32-bit `PTRDIFF_MAX`).
  template <class T>
  static bool slab_ok(const T* p, uint32_t count) noexcept {
    return count == 0u || (p != nullptr && count_fits_host(count, sizeof(T)));
  }

  static SceneError validate(const View* v, Key64 expected_schema) noexcept {
    if (v == nullptr) return SceneError::NullView;
    if (v->schema_hash != expected_schema.value) return SceneError::SchemaMismatch;
    if (!slab_ok(v->nodes, v->node_count) || !slab_ok(v->text_slab, v->text_slab_len) ||
        !slab_ok(v->children_slab, v->children_slab_len) || !slab_ok(v->dirty_indices, v->dirty_count) ||
        !slab_ok(v->removed_ids, v->removed_count) || !slab_ok(v->tables, v->table_count)) {
      return SceneError::NullSlab;
    }
    if (v->text_slab_len > 0u && nve_utf8_valid(v->text_slab, v->text_slab_len) == 0) return SceneError::InvalidUtf8;
    for (uint32_t i = 0; i < v->table_count; ++i) {
      if (!TableReader::from_view(v->tables[i])) return SceneError::InvalidTable;
    }
    return SceneError::None;
  }

  /// `true` when byte `at` starts a code point (or is the slab end).
  [[nodiscard]] bool is_boundary(size_t at) const noexcept {
    return at == text_.size() || (static_cast<uint8_t>(text_[at]) & 0xC0u) != 0x80u;
  }

  uint64_t frame_id_;
  std::span<const Node> nodes_;
  std::string_view text_;
  std::span<const uint32_t> children_;
  std::span<const uint32_t> dirty_;
  std::span<const uint32_t> removed_;
  std::span<const Table> tables_;
};

}  // namespace nve::host
