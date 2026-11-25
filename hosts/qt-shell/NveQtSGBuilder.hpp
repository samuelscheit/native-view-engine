#pragma once

#include "NveQtCommon.hpp"
#include "NveQtTextCache.hpp"

#include <QColor>
#include <QPointF>
#include <QQuickWindow>
#include <QSGClipNode>
#include <QSGGeometry>
#include <QSGGeometryNode>
#include <QSGNode>
#include <QSGOpacityNode>
#include <QSGSimpleTextureNode>
#include <QSGTransformNode>
#include <QString>

#include <cstdint>
#include <string>
#include <unordered_map>
#include <unordered_set>
#include <vector>

namespace nve::qt {

struct SceneGraphStats;

/// Persistent retained Scene Graph subtree owned per `node_id` inside `NveSGRootNode`.
/// Unchanged sibling nodes keep their exact `QSGNode*` pointers across dirty frames.
struct RetainedSGNodeSlot {
  uint32_t node_id{0};
  uint32_t parent_id{0};
  QSGNode* wrapper{nullptr};
  bool wrapper_is_transform{false};
  QSGOpacityNode* opacity_node{nullptr};
  QSGGeometryNode* bg_node{nullptr};
  QSGClipNode* clip_node{nullptr};
  QSGTransformNode* scroll_xform{nullptr};
  QSGSimpleTextureNode* content_tex_node{nullptr};
  bool has_text_texture{false};
  bool has_image_texture{false};
  bool has_canvas_texture{false};
  std::string active_image_uri;
  QSGGeometryNode* border_node{nullptr};
  std::vector<uint32_t> mounted_children;
  int32_t z_index{0};

  QSGNode* contentParent() const noexcept {
    return (opacity_node != nullptr) ? static_cast<QSGNode*>(opacity_node) : wrapper;
  }

  QSGNode* childrenParent() const noexcept {
    if (scroll_xform != nullptr) {
      return scroll_xform;
    }
    if (clip_node != nullptr) {
      return clip_node;
    }
    return contentParent();
  }
};

/// Custom Scene Graph root node owning retained per-node `QSGNode` subtrees and render-thread
/// GPU text/image texture caches.
class NveSGRootNode final : public QSGNode {
 public:
  NveSGRootNode() = default;

  ~NveSGRootNode() override {
    // Destroy all SG nodes before releasing cached QSGTextures so no QSGSimpleTextureNode ever
    // outlives the texture it references.
    removeAllChildNodes();
    slots_.clear();
    cache_.clear();
  }

  const QSGNode* findRetainedWrapper(uint32_t node_id) const noexcept {
    const auto it = slots_.find(node_id);
    return (it != slots_.end()) ? it->second.wrapper : nullptr;
  }

  std::unordered_map<uint32_t, RetainedSGNodeSlot> slots_;
  NveQtTextCache cache_;
  uint64_t last_built_generation{0};
  int last_built_dpr_centis{0};
};

/// Immutable view of host state consumed during a Scene Graph synchronization pass.
struct SGBuildContext {
  NveSceneView view{};
  const std::unordered_map<uint32_t, size_t>* id_to_index{nullptr};
  const std::unordered_map<uint32_t, NodeSemantics>* node_semantics{nullptr};
  const std::unordered_map<uint32_t, ImageResizeMode>* image_resize_modes{nullptr};
  const std::unordered_map<uint32_t, QPointF>* scroll_offsets{nullptr};
  const std::unordered_map<uint32_t, std::string>* text_input_buffers{nullptr};
  const std::unordered_map<uint32_t, QString>* ime_preedit_buffers{nullptr};
  QQuickWindow* window{nullptr};
  qreal item_width{0.0};
  qreal item_height{0.0};
  qreal dpr{1.0};
  int dpr_centis{100};
  uint64_t scene_generation{0};
  uint32_t focused_node_id{0};
  bool full_resync{true};
  const std::unordered_set<uint32_t>* dirty_ids{nullptr};
  const std::unordered_set<uint32_t>* removed_ids{nullptr};
};

/// Perform an incremental `O(dirty + removed)` retained Scene Graph sync on `old_node`.
QSGNode* syncRetainedSceneGraph(
    QSGNode* old_node,
    const SGBuildContext& ctx,
    SceneGraphStats& io_stats);

/// Lookup a node in `view` using `id_to_index` (falling back to linear scan if stale).
const NveRenderNode* lookupSceneNode(
    const NveSceneView& view,
    const std::unordered_map<uint32_t, size_t>& id_to_index,
    uint32_t node_id) noexcept;

/// Compute total content extent of a `ScrollView` node in `view`.
QSizeF computeScrollContentSize(
    const NveSceneView& view,
    const std::unordered_map<uint32_t, size_t>& id_to_index,
    const NveRenderNode& scroll_node);

/// Front-to-back hit-test against `view`, honoring `z_index`, clip bounds, and scroll offsets.
uint32_t hitTestSceneView(
    const NveSceneView& view,
    const std::unordered_map<uint32_t, size_t>& id_to_index,
    const std::unordered_map<uint32_t, NodeSemantics>& node_semantics,
    const std::unordered_map<uint32_t, QPointF>& scroll_offsets,
    qreal item_width,
    qreal item_height,
    const QPointF& query,
    bool interactive_only,
    bool scrollable_only);

/// Compute a node's resolved bounding rectangle in item-local coordinates.
QRectF findSceneNodeGlobalBounds(
    const NveSceneView& view,
    const std::unordered_map<uint32_t, size_t>& id_to_index,
    const std::unordered_map<uint32_t, QPointF>& scroll_offsets,
    uint32_t target_id);

} // namespace nve::qt
