#pragma once

#import "NveAppleSupport.h"

#include <nve/canvas/canvas.hpp>
#include <nve/host/scene_nodes.hpp>

namespace nve::apple {

/// The UIKit / AppKit projection of one node decoded by host-cxx (`nve::host::read_node`, with
/// inherited typography resolved by `RetainedTree`) and optional `nve.canvas` table data.
NodeView node_view(const nve::host::NodeData &node, const nve::canvas::CanvasData *_Nullable canvas = nullptr);

/// The platform view class `node` realizes (what `node_view(node, ...).kind` would be, without copies).
NodeKind node_kind(const nve::host::NodeData &node, bool has_canvas = false) noexcept;

}  // namespace nve::apple
