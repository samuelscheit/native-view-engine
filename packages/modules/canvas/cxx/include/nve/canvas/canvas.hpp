// `nve.canvas` for C++ view hosts: the generated table binding and the `nve.res.draw_list` kind.
// A canvas node's `draw_list` word is a `ResourceId` whose bytes (`RuntimeSceneGuard::resource`,
// kind `kDrawListKind`) are a 4-byte-aligned flat `u32`-word draw list (`FlatResource`);
// `generation` bumps on every redraw request.
#pragma once

#include <cstdint>

#include <nve/host/bound_table.hpp>
#include <nve/host/xxh64.hpp>

#include "draw_list.hpp"
#include "nve_canvas_tables.generated.h"

namespace nve::canvas {

/// `<DrawList as FlatResource>::KIND`.
inline constexpr host::Key64 kDrawListKind = host::Key64::of("nve.res.draw_list");

struct CanvasData {
  uint32_t draw_list = 0u;  // `ResourceId`, 0 = nothing drawn
  uint32_t generation = 0u;
};

/// `nve.canvas` bound to one acquired scene (absent until an app publishes a canvas).
class CanvasTable {
 public:
  void bind(const host::SceneReader& scene) { table_.bind(scene, NVE_TABLE_CANVAS_DESC); }
  [[nodiscard]] bool has(uint32_t index) const noexcept { return table_.has(index); }
  [[nodiscard]] CanvasData at(uint32_t index) const noexcept {
    const host::RowRef r = table_.at(index);
    return CanvasData{r.bits(NVE_TABLE_CANVAS_DRAW_LIST), r.bits(NVE_TABLE_CANVAS_GENERATION)};
  }
  [[nodiscard]] const host::BoundTable& table() const noexcept { return table_; }

 private:
  host::BoundTable table_;
};

}  // namespace nve::canvas
