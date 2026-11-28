#pragma once

#import "NveAppleSupport.h"

#include <nve/host/runtime_session.hpp>

#include <optional>
#include <vector>

namespace nve::apple {

/// Published core tables of one borrowed runtime scene, read through the generated
/// `nve_tables.generated.h` specs. Dense rows count only when the node's `table_set` holds the
/// table; sparse rows resolve in O(1) through the runtime's published row index.
class SceneTables {
 public:
  explicit SceneTables(const nve::host::SceneReader &scene);

  const nve::host::SceneReader &scene() const noexcept { return scene_; }

  /// Live slot of raw `NodeId` (generation-checked).
  std::optional<uint32_t> index_of(uint32_t node_id) const noexcept;
  /// Raw `NodeId` of live slot `index` (0 when dead).
  uint32_t node_id_at(uint32_t index) const noexcept;

  /// Fully resolved node: kind, frame, style (incl. inherited typography), semantics, content.
  NodeView describe(uint32_t index) const;
  /// Whether `index` sets any inherited typography (its descendants' styles depend on it).
  bool sets_typography(uint32_t index) const noexcept { return row(typography_, index).has_value(); }

  /// Live children of `index` in paint order (stable by `z_index`).
  std::vector<uint32_t> paint_ordered_children(uint32_t index) const;

 private:
  std::optional<uint32_t> row(const std::optional<nve::host::TableReader> &table, uint32_t index) const noexcept;
  std::string_view string(const std::optional<nve::host::TableReader> &table, uint32_t index, NveFieldSpec f) const noexcept;
  int32_t z_index(uint32_t index) const noexcept;
  void resolve_typography(uint32_t index, NodeStyle &style) const noexcept;

  const nve::host::SceneReader &scene_;
  std::optional<nve::host::TableReader> visual_, typography_, text_, image_, scroll_, accessibility_, text_input_, link_,
      interaction_;
};

}  // namespace nve::apple