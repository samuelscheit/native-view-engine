#pragma once

#import "NveAppleSupport.h"

#include <nve/host/scene_nodes.hpp>

namespace nve::apple {

/// The UIKit / AppKit projection of one node decoded by host-cxx (`nve::host::read_node`, with
/// inherited typography resolved by `RetainedTree`): kind (`Link` folds into an interactive
/// container), absolute frame, paint style, semantics, and content.
NodeView node_view(const nve::host::NodeData &node);

/// The platform view class `node` realizes (what `node_view(node).kind` would be, without the copies).
NodeKind node_kind(const nve::host::NodeData &node) noexcept;

}  // namespace nve::apple
