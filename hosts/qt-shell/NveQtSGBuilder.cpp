#include "NveQtSGBuilder.hpp"
#include "NveQtQuickItem.hpp"

#include <QMatrix4x4>
#include <QRectF>
#include <QSGFlatColorMaterial>
#include <QSGSimpleRectNode>
#include <QSizeF>

#include <algorithm>
#include <cmath>
#include <limits>
#include <numbers>
#include <unordered_set>
#include <utility>
#include <vector>

namespace nve::qt {

namespace {

constexpr int kCornerSegments = 10;
constexpr float kMinRadiusForTessellation = 0.5f;

struct Point2D {
  float x;
  float y;
};

void sampleRoundedRectRing(
    std::vector<Point2D>& out, float x, float y, float w, float h, float radius,
    bool force_arc_count = false) {
  out.clear();
  if (w <= 0.0f || h <= 0.0f) return;
  const float r = std::clamp(radius, 0.0f, std::min(w, h) * 0.5f);
  if (!force_arc_count && r <= 0.05f) {
    out = {{x, y}, {x + w, y}, {x + w, y + h}, {x, y + h}};
    return;
  }
  out.reserve(static_cast<size_t>(4 * (kCornerSegments + 1)));
  constexpr float kHalfPi = static_cast<float>(std::numbers::pi * 0.5);
  const struct { float cx, cy, start; } corners[4] = {
      {x + w - r, y + r, -kHalfPi},
      {x + w - r, y + h - r, 0.0f},
      {x + r, y + h - r, kHalfPi},
      {x + r, y + r, 2.0f * kHalfPi},
  };
  for (const auto& c : corners) {
    for (int i = 0; i <= kCornerSegments; ++i) {
      const float a = c.start + (static_cast<float>(i) / kCornerSegments) * kHalfPi;
      out.push_back({c.cx + r * std::cos(a), c.cy + r * std::sin(a)});
    }
  }
}

QSGGeometry* createRoundedRectFillGeometry(float w, float h, float radius) {
  std::vector<Point2D> ring;
  sampleRoundedRectRing(ring, 0.0f, 0.0f, w, h, radius);
  const size_t n = ring.size();
  auto* geom = new QSGGeometry(
      QSGGeometry::defaultAttributes_Point2D(), n < 3u ? 0 : static_cast<int>(n * 3u));
  geom->setDrawingMode(QSGGeometry::DrawTriangles);
  if (n < 3u) return geom;
  auto* verts = geom->vertexDataAsPoint2D();
  const float cx = w * 0.5f, cy = h * 0.5f;
  size_t vi = 0;
  for (size_t i = 0; i < n; ++i) {
    const Point2D &p0 = ring[i], &p1 = ring[(i + 1u) % n];
    verts[vi++].set(cx, cy);
    verts[vi++].set(p0.x, p0.y);
    verts[vi++].set(p1.x, p1.y);
  }
  return geom;
}

QSGGeometry* createBorderRingGeometry(float w, float h, float radius, float border_width) {
  const float bw = std::clamp(border_width, 0.0f, std::min(w, h) * 0.5f);
  if (bw <= 0.0f || w <= 0.0f || h <= 0.0f) {
    auto* empty = new QSGGeometry(QSGGeometry::defaultAttributes_Point2D(), 0);
    empty->setDrawingMode(QSGGeometry::DrawTriangles);
    return empty;
  }
  const float outer_r = std::clamp(radius, 0.0f, std::min(w, h) * 0.5f);
  const float inner_w = std::max(0.0f, w - 2.0f * bw);
  const float inner_h = std::max(0.0f, h - 2.0f * bw);
  if (inner_w <= 0.01f || inner_h <= 0.01f) return createRoundedRectFillGeometry(w, h, outer_r);

  const bool rounded = (outer_r > kMinRadiusForTessellation);
  std::vector<Point2D> outer, inner;
  sampleRoundedRectRing(outer, 0.0f, 0.0f, w, h, rounded ? outer_r : 0.0f, rounded);
  sampleRoundedRectRing(
      inner, bw, bw, inner_w, inner_h, rounded ? std::max(0.0f, outer_r - bw) : 0.0f, rounded);

  const size_t n = std::min(outer.size(), inner.size());
  auto* geom = new QSGGeometry(QSGGeometry::defaultAttributes_Point2D(), static_cast<int>(n * 6u));
  geom->setDrawingMode(QSGGeometry::DrawTriangles);
  auto* verts = geom->vertexDataAsPoint2D();
  size_t vi = 0;
  for (size_t i = 0; i < n; ++i) {
    const Point2D &o0 = outer[i], &o1 = outer[(i + 1u) % n], &i0 = inner[i], &i1 = inner[(i + 1u) % n];
    verts[vi++].set(o0.x, o0.y); verts[vi++].set(o1.x, o1.y); verts[vi++].set(i0.x, i0.y);
    verts[vi++].set(i0.x, i0.y); verts[vi++].set(o1.x, o1.y); verts[vi++].set(i1.x, i1.y);
  }
  return geom;
}

QSGGeometryNode* createFlatGeomNode(QSGGeometry* geom, const QColor& color) {
  auto* node = new QSGGeometryNode();
  node->setGeometry(geom);
  node->setFlag(QSGNode::OwnsGeometry);
  auto* mat = new QSGFlatColorMaterial();
  mat->setColor(color);
  node->setMaterial(mat);
  node->setFlag(QSGNode::OwnsMaterial);
  return node;
}

QSGGeometryNode* createFilledBoxNode(float w, float h, float radius, const QColor& color) {
  if (radius <= kMinRadiusForTessellation) {
    return new QSGSimpleRectNode(QRectF(0.0, 0.0, w, h), color);
  }
  return createFlatGeomNode(createRoundedRectFillGeometry(w, h, radius), color);
}

QSGGeometryNode* createBorderNode(
    float w, float h, float radius, float border_width, const QColor& color) {
  return createFlatGeomNode(createBorderRingGeometry(w, h, radius, border_width), color);
}

void configureClipNode(QSGClipNode* clip, float w, float h, float radius) {
  const QRectF rect(0.0, 0.0, w, h);
  clip->setClipRect(rect);
  if (radius <= kMinRadiusForTessellation) {
    clip->setIsRectangular(true);
    auto* geom = new QSGGeometry(QSGGeometry::defaultAttributes_Point2D(), 4);
    QSGGeometry::updateRectGeometry(geom, rect);
    clip->setGeometry(geom);
  } else {
    clip->setIsRectangular(false);
    clip->setGeometry(createRoundedRectFillGeometry(w, h, radius));
  }
  clip->setFlag(QSGNode::OwnsGeometry);
  clip->markDirty(QSGNode::DirtyGeometry);
}

const NveRenderNode* findSceneNode(const SGBuildContext& ctx, uint32_t node_id) noexcept {
  return ctx.id_to_index ? lookupSceneNode(ctx.view, *ctx.id_to_index, node_id) : nullptr;
}

std::vector<uint32_t> sortedChildrenByZ(
    const NveSceneView& view, const std::unordered_map<uint32_t, size_t>& id_to_index,
    const NveRenderNode& node, bool skip_zero_opacity) {
  const auto raw_kids = nve::scene_node_children(view, node);
  std::vector<uint32_t> result;
  result.reserve(raw_kids.size());
  bool needs_sort = false;
  int32_t prev_z = std::numeric_limits<int32_t>::min();
  for (uint32_t cid : raw_kids) {
    const NveRenderNode* child = lookupSceneNode(view, id_to_index, cid);
    if (skip_zero_opacity && (child == nullptr || child->opacity_u8 == 0u)) continue;
    result.push_back(cid);
    const int32_t z = child ? child->z_index : 0;
    if (z < prev_z) needs_sort = true;
    prev_z = z;
  }
  if (needs_sort) {
    std::stable_sort(result.begin(), result.end(), [&](uint32_t a, uint32_t b) {
      const NveRenderNode* na = lookupSceneNode(view, id_to_index, a);
      const NveRenderNode* nb = lookupSceneNode(view, id_to_index, b);
      return (na ? na->z_index : 0) < (nb ? nb->z_index : 0);
    });
  }
  return result;
}

void detachMountedChildren(NveSGRootNode& root, RetainedSGNodeSlot& slot) {
  QSGNode* cp = slot.childrenParent();
  if (!cp) return;
  for (uint32_t cid : slot.mounted_children) {
    if (auto it = root.slots_.find(cid);
        it != root.slots_.end() && it->second.wrapper && it->second.wrapper->parent() == cp) {
      cp->removeChildNode(it->second.wrapper);
    }
  }
}

void destroySlot(NveSGRootNode& root, uint32_t node_id) {
  auto it = root.slots_.find(node_id);
  if (it == root.slots_.end()) { root.cache_.eraseText(node_id); return; }
  RetainedSGNodeSlot& slot = it->second;
  detachMountedChildren(root, slot);
  if (slot.parent_id != 0u) {
    if (auto pit = root.slots_.find(slot.parent_id); pit != root.slots_.end()) {
      auto& kids = pit->second.mounted_children;
      kids.erase(std::remove(kids.begin(), kids.end(), node_id), kids.end());
    }
  }
  if (slot.wrapper) {
    if (QSGNode* parent = slot.wrapper->parent()) parent->removeChildNode(slot.wrapper);
    delete slot.wrapper;
    slot.wrapper = nullptr;
  }
  root.cache_.eraseText(node_id);
  root.slots_.erase(it);
}

void replaceNodeInParent(QSGNode* old_child, QSGNode* new_child) {
  if (!old_child || !new_child) return;
  QSGNode* parent = old_child->parent();
  if (!parent) return;
  QSGNode* prev = old_child->previousSibling();
  parent->removeChildNode(old_child);
  if (prev) parent->insertChildNodeAfter(new_child, prev);
  else parent->prependChildNode(new_child);
}

QSGNode* childrenInsertAnchor(const RetainedSGNodeSlot& slot, QSGNode* cp) noexcept {
  if (slot.content_tex_node && slot.content_tex_node->parent() == cp) return slot.content_tex_node;
  if (cp == slot.contentParent() && slot.bg_node && slot.bg_node->parent() == cp) return slot.bg_node;
  return nullptr;
}

void attachOrderedChildren(
    NveSGRootNode& root, RetainedSGNodeSlot& slot, QSGNode* cp, uint32_t parent_id,
    const std::vector<uint32_t>& child_ids, bool update_list) {
  QSGNode* anchor = childrenInsertAnchor(slot, cp);
  if (update_list) {
    slot.mounted_children.clear();
    slot.mounted_children.reserve(child_ids.size());
  }
  for (uint32_t cid : child_ids) {
    auto it = root.slots_.find(cid);
    if (it == root.slots_.end() || !it->second.wrapper) continue;
    QSGNode* child_w = it->second.wrapper;
    if (QSGNode* old_p = child_w->parent()) old_p->removeChildNode(child_w);
    if (anchor) cp->insertChildNodeAfter(child_w, anchor);
    else cp->prependChildNode(child_w);
    anchor = child_w;
    if (update_list) { it->second.parent_id = parent_id; slot.mounted_children.push_back(cid); }
  }
}

void reconcileSlotChildrenOrder(
    NveSGRootNode& root, const SGBuildContext& ctx, uint32_t parent_id,
    const NveRenderNode& parent_node) {
  auto pit = root.slots_.find(parent_id);
  if (pit == root.slots_.end()) return;
  RetainedSGNodeSlot& slot = pit->second;
  QSGNode* cp = slot.childrenParent();
  if (!cp || !ctx.id_to_index) return;

  std::vector<uint32_t> desired = sortedChildrenByZ(ctx.view, *ctx.id_to_index, parent_node, true);
  bool same = (desired == slot.mounted_children);
  for (size_t i = 0u; same && i < desired.size(); ++i) {
    auto cit = root.slots_.find(desired[i]);
    same = (cit != root.slots_.end() && cit->second.wrapper && cit->second.wrapper->parent() == cp);
  }
  if (same) return;
  detachMountedChildren(root, slot);
  attachOrderedChildren(root, slot, cp, parent_id, desired, true);
}

void removeAndDeleteChild(QSGNode*& node_ptr) {
  if (!node_ptr) return;
  if (QSGNode* p = node_ptr->parent()) p->removeChildNode(node_ptr);
  delete node_ptr;
  node_ptr = nullptr;
}

void updateSlotContentTexture(
    NveSGRootNode& root, const SGBuildContext& ctx, RetainedSGNodeSlot& slot,
    const NveRenderNode& node, float w, float h, bool is_focused_input,
    uint32_t& out_rasterized_count) {
  ResolvedContentTexture res = resolveNodeContentTexture(
      root.cache_, ctx.view, node, w, h, ctx.dpr, ctx.dpr_centis, ctx.scene_generation,
      ctx.window, is_focused_input, ctx.node_semantics, ctx.image_resize_modes,
      ctx.text_input_buffers, ctx.ime_preedit_buffers, out_rasterized_count);

  QSGNode* cp = slot.childrenParent();
  if (res.texture) {
    if (!slot.content_tex_node) {
      slot.content_tex_node = new QSGSimpleTextureNode();
      slot.content_tex_node->setOwnsTexture(false);
      slot.content_tex_node->setFiltering(QSGTexture::Linear);
      if (cp == slot.contentParent() && slot.bg_node) cp->insertChildNodeAfter(slot.content_tex_node, slot.bg_node);
      else cp->prependChildNode(slot.content_tex_node);
    }
    slot.content_tex_node->setTexture(res.texture);
    slot.content_tex_node->setRect(res.dst_rect);
    const QSize ts = res.texture->textureSize();
    slot.content_tex_node->setSourceRect(res.use_src_rect ? res.src_rect : QRectF(0.0, 0.0, ts.width(), ts.height()));
    slot.has_text_texture = res.is_text;
    slot.has_image_texture = res.is_image;
    slot.active_image_uri = std::move(res.image_uri);
  } else {
    QSGNode* tex_base = slot.content_tex_node;
    removeAndDeleteChild(tex_base);
    slot.content_tex_node = nullptr;
    slot.has_text_texture = false;
    slot.has_image_texture = false;
    slot.active_image_uri.clear();
  }
}

void upsertSlot(
    NveSGRootNode& root, const SGBuildContext& ctx, const NveRenderNode& node,
    uint32_t& out_rasterized_count, std::unordered_set<uint32_t>& parents_needing_reorder) {
  if (node.opacity_u8 == 0u) {
    if (auto it = root.slots_.find(node.node_id); it != root.slots_.end()) {
      if (it->second.parent_id != 0u) parents_needing_reorder.insert(it->second.parent_id);
      destroySlot(root, node.node_id);
    }
    return;
  }

  const bool is_root = (node.node_id == NVE_ROOT_NODE_ID);
  const float w = (is_root && node.width <= 0.0f) ? static_cast<float>(ctx.item_width) : node.width;
  const float h = (is_root && node.height <= 0.0f) ? static_cast<float>(ctx.item_height) : node.height;
  const bool need_transform = (!is_root && (node.x != 0.0f || node.y != 0.0f));
  const bool need_opacity = (node.opacity_u8 < 255u);
  const QColor bg_color = qcolorFromRgba(node.bg_rgba), border_color = qcolorFromRgba(node.border_rgba);
  const bool has_bg = (bg_color.alpha() > 0) && (w > 0.0f) && (h > 0.0f);
  const bool is_focused_input = (node.kind == NVE_NODE_KIND_TEXT_INPUT && ctx.focused_node_id == node.node_id);
  const float eff_border_w = (node.border_width > 0.0f) ? node.border_width : (is_focused_input ? 1.5f : 0.0f);
  const QColor eff_border_color =
      (node.border_width > 0.0f && border_color.alpha() > 0)
          ? border_color : (is_focused_input ? QColor(59, 130, 246, 255) : border_color);
  const bool has_border = (eff_border_w > 0.0f) && (eff_border_color.alpha() > 0) && (w > 0.0f) && (h > 0.0f);
  const bool needs_clip =
      (w > 0.0f && h > 0.0f) &&
      (node.kind == NVE_NODE_KIND_SCROLL_VIEW || node.kind == NVE_NODE_KIND_TEXT_INPUT ||
       (node.kind == NVE_NODE_KIND_IMAGE && node.border_radius > kMinRadiusForTessellation) ||
       ((node.flags & NVE_NODE_CLIP_BOUNDS) != 0u && node.child_count > 0u));

  QPointF scroll_off;
  if (node.kind == NVE_NODE_KIND_SCROLL_VIEW && ctx.scroll_offsets) {
    if (auto sit = ctx.scroll_offsets->find(node.node_id); sit != ctx.scroll_offsets->end()) scroll_off = sit->second;
  }
  const bool need_scroll_xform =
      (node.kind == NVE_NODE_KIND_SCROLL_VIEW) && (scroll_off.x() != 0.0 || scroll_off.y() != 0.0);

  auto [slot_it, created] = root.slots_.try_emplace(node.node_id);
  RetainedSGNodeSlot& slot = slot_it->second;
  slot.node_id = node.node_id;

  if (created && !is_root && ctx.view.nodes) {
    for (uint32_t i = 0u; i < ctx.view.node_count; ++i) {
      const auto kids = nve::scene_node_children(ctx.view, ctx.view.nodes[i]);
      if (std::find(kids.begin(), kids.end(), node.node_id) != kids.end()) {
        parents_needing_reorder.insert(ctx.view.nodes[i].node_id);
        break;
      }
    }
  }
  if (!created && slot.z_index != node.z_index && slot.parent_id != 0u) {
    parents_needing_reorder.insert(slot.parent_id);
  }
  slot.z_index = node.z_index;

  const bool structure_changed =
      created || (slot.wrapper_is_transform != need_transform) ||
      ((slot.opacity_node != nullptr) != need_opacity) ||
      ((slot.clip_node != nullptr) != needs_clip) ||
      ((slot.scroll_xform != nullptr) != need_scroll_xform);

  if (structure_changed) {
    detachMountedChildren(root, slot);
    QSGNode* old_wrapper = slot.wrapper;
    QSGNode* new_wrapper = need_transform ? static_cast<QSGNode*>(new QSGTransformNode()) : new QSGNode();
    if (old_wrapper) { replaceNodeInParent(old_wrapper, new_wrapper); delete old_wrapper; }
    else if (is_root) root.appendChildNode(new_wrapper);

    slot.wrapper = new_wrapper;
    slot.wrapper_is_transform = need_transform;
    slot.opacity_node = nullptr; slot.bg_node = nullptr; slot.clip_node = nullptr;
    slot.scroll_xform = nullptr; slot.content_tex_node = nullptr; slot.border_node = nullptr;

    QSGNode* cur = slot.wrapper;
    if (need_opacity) { slot.opacity_node = new QSGOpacityNode(); cur->appendChildNode(slot.opacity_node); cur = slot.opacity_node; }
    if (needs_clip) { slot.clip_node = new QSGClipNode(); cur->appendChildNode(slot.clip_node); cur = slot.clip_node; }
    if (need_scroll_xform) { slot.scroll_xform = new QSGTransformNode(); cur->appendChildNode(slot.scroll_xform); }
    if (QSGNode* cp = slot.childrenParent()) attachOrderedChildren(root, slot, cp, node.node_id, slot.mounted_children, false);
  }

  if (need_transform) { QMatrix4x4 mat; mat.translate(node.x, node.y); static_cast<QSGTransformNode*>(slot.wrapper)->setMatrix(mat); }
  if (slot.opacity_node) slot.opacity_node->setOpacity(static_cast<qreal>(node.opacity_u8) / 255.0);
  if (slot.clip_node != nullptr) {
    configureClipNode(slot.clip_node, w, h, node.border_radius);
  }
  if (slot.scroll_xform != nullptr) {
    QMatrix4x4 mat;
    mat.translate(static_cast<float>(-scroll_off.x()), static_cast<float>(-scroll_off.y()));
    slot.scroll_xform->setMatrix(mat);
  }

  QSGNode* content_parent = slot.contentParent();
  QSGNode* bg_base = slot.bg_node;
  removeAndDeleteChild(bg_base);
  slot.bg_node = nullptr;
  if (has_bg) {
    slot.bg_node = createFilledBoxNode(w, h, node.border_radius, bg_color);
    content_parent->prependChildNode(slot.bg_node);
  }

  updateSlotContentTexture(root, ctx, slot, node, w, h, is_focused_input, out_rasterized_count);

  QSGNode* border_base = slot.border_node;
  removeAndDeleteChild(border_base);
  slot.border_node = nullptr;
  if (has_border) {
    slot.border_node = createBorderNode(w, h, node.border_radius, eff_border_w, eff_border_color);
    content_parent->appendChildNode(slot.border_node);
  }
}

void computeStats(const NveSGRootNode& root, SceneGraphStats& out) {
  out.totalSGNodes = 1u;
  for (const auto& [_, slot] : root.slots_) {
    if (slot.wrapper) ++out.totalSGNodes;
    if (slot.opacity_node) { ++out.totalSGNodes; ++out.opacityNodes; }
    if (slot.bg_node) { ++out.totalSGNodes; ++out.geometryNodes; }
    if (slot.clip_node) { ++out.totalSGNodes; ++out.clipNodes; }
    if (slot.scroll_xform) ++out.totalSGNodes;
    if (slot.content_tex_node) {
      ++out.totalSGNodes;
      if (slot.has_text_texture) ++out.textTextureNodes;
      else if (slot.has_image_texture) ++out.imageTextureNodes;
    }
    if (slot.border_node) { ++out.totalSGNodes; ++out.geometryNodes; }
  }
}

} // namespace

QSGNode* syncRetainedSceneGraph(
    QSGNode* old_node,
    const SGBuildContext& ctx,
    SceneGraphStats& io_stats) {
  auto* root = static_cast<NveSGRootNode*>(old_node);
  if (root == nullptr) root = new NveSGRootNode();

  if (root->last_built_generation == ctx.scene_generation &&
      root->last_built_dpr_centis == ctx.dpr_centis &&
      root->last_built_generation != 0u) {
    ++io_stats.syncCount;
    return root;
  }

  if (ctx.view.node_count == 0u || ctx.view.nodes == nullptr) {
    root->removeAllChildNodes();
    root->slots_.clear();
    root->cache_.clear();
    root->last_built_generation = ctx.scene_generation;
    root->last_built_dpr_centis = ctx.dpr_centis;
    SceneGraphStats empty{};
    empty.syncCount = io_stats.syncCount + 1u;
    empty.sceneGeneration = ctx.scene_generation;
    empty.totalSGNodes = 1u;
    empty.textRasterizations = root->cache_.totalRasterizations();
    empty.textCacheHits = root->cache_.totalCacheHits();
    io_stats = empty;
    return root;
  }

  const bool dpr_changed =
      (root->last_built_dpr_centis != 0 && root->last_built_dpr_centis != ctx.dpr_centis);
  const bool do_full = ctx.full_resync || root->slots_.empty() || dpr_changed;

  uint32_t rasterized_this_sync = 0u;
  uint64_t dirty_updated = 0u;
  std::unordered_set<uint32_t> parents_needing_reorder;

  if (do_full) {
    std::vector<uint32_t> stale_ids;
    for (const auto& [nid, _] : root->slots_) {
      if (findSceneNode(ctx, nid) == nullptr) stale_ids.push_back(nid);
    }
    for (uint32_t sid : stale_ids) destroySlot(*root, sid);
    for (uint32_t i = 0u; i < ctx.view.node_count; ++i) {
      upsertSlot(*root, ctx, ctx.view.nodes[i], rasterized_this_sync, parents_needing_reorder);
      ++dirty_updated;
      parents_needing_reorder.insert(ctx.view.nodes[i].node_id);
    }
  } else {
    if (ctx.removed_ids != nullptr) {
      for (uint32_t rid : *ctx.removed_ids) destroySlot(*root, rid);
    }
    if (ctx.dirty_ids != nullptr) {
      for (uint32_t nid : *ctx.dirty_ids) {
        const NveRenderNode* node = findSceneNode(ctx, nid);
        if (node == nullptr) { destroySlot(*root, nid); continue; }
        upsertSlot(*root, ctx, *node, rasterized_this_sync, parents_needing_reorder);
        ++dirty_updated;
        parents_needing_reorder.insert(nid);
      }
    }
  }
  if (auto it = root->slots_.find(NVE_ROOT_NODE_ID);
      it != root->slots_.end() && it->second.wrapper && it->second.wrapper->parent() != root) {
    root->appendChildNode(it->second.wrapper);
  }
  for (uint32_t pid : parents_needing_reorder) {
    if (const NveRenderNode* pnode = findSceneNode(ctx, pid)) {
      reconcileSlotChildrenOrder(*root, ctx, pid, *pnode);
    }
  }

  root->cache_.pruneTextIf([&](uint32_t nid) {
    auto it = root->slots_.find(nid);
    return it == root->slots_.end() || !it->second.has_text_texture;
  });
  std::unordered_set<std::string> live_uris;
  for (const auto& [_, slot] : root->slots_) {
    if (slot.has_image_texture && !slot.active_image_uri.empty()) live_uris.insert(slot.active_image_uri);
  }
  root->cache_.pruneImagesIf([&](const std::string& uri) { return live_uris.find(uri) == live_uris.end(); });

  SceneGraphStats next_stats{};
  next_stats.syncCount = io_stats.syncCount + 1u;
  next_stats.sceneGeneration = ctx.scene_generation;
  computeStats(*root, next_stats);
  if (next_stats.textTextureNodes >= rasterized_this_sync) {
    root->cache_.addRetainedCacheHits(next_stats.textTextureNodes - rasterized_this_sync);
  }

  root->last_built_generation = ctx.scene_generation;
  root->last_built_dpr_centis = ctx.dpr_centis;
  next_stats.textRasterizations = root->cache_.totalRasterizations();
  next_stats.textCacheHits = root->cache_.totalCacheHits();
  next_stats.cachedTextTextures = root->cache_.textCount();
  next_stats.cachedImageTextures = root->cache_.imageCount();
  next_stats.dirtyNodesUpdated = dirty_updated;
  next_stats.retainedNodesReused =
      (root->slots_.size() >= dirty_updated) ? static_cast<uint64_t>(root->slots_.size() - dirty_updated) : 0u;
  io_stats = next_stats;
  return root;
}

const NveRenderNode* lookupSceneNode(
    const NveSceneView& view, const std::unordered_map<uint32_t, size_t>& id_to_index,
    uint32_t node_id) noexcept {
  if (view.nodes == nullptr) return nullptr;
  if (auto it = id_to_index.find(node_id);
      it != id_to_index.end() && it->second < view.node_count && view.nodes[it->second].node_id == node_id) {
    return &view.nodes[it->second];
  }
  for (uint32_t i = 0u; i < view.node_count; ++i) {
    if (view.nodes[i].node_id == node_id) return &view.nodes[i];
  }
  return nullptr;
}

QSizeF computeScrollContentSize(
    const NveSceneView& view, const std::unordered_map<uint32_t, size_t>& id_to_index,
    const NveRenderNode& scroll_node) {
  qreal max_w = scroll_node.width, max_h = scroll_node.height;
  for (uint32_t cid : nve::scene_node_children(view, scroll_node)) {
    if (const NveRenderNode* child = lookupSceneNode(view, id_to_index, cid)) {
      max_w = std::max(max_w, static_cast<qreal>(child->x + child->width));
      max_h = std::max(max_h, static_cast<qreal>(child->y + child->height));
    }
  }
  return QSizeF(max_w, max_h);
}

namespace {

QPointF scrollOffsetOf(
    const NveRenderNode& node, const std::unordered_map<uint32_t, QPointF>& scroll_offsets) {
  if (node.kind != NVE_NODE_KIND_SCROLL_VIEW) return QPointF();
  auto sit = scroll_offsets.find(node.node_id);
  return (sit != scroll_offsets.end()) ? sit->second : QPointF();
}

uint32_t hitTestRecursiveImpl(
    const NveSceneView& view, const std::unordered_map<uint32_t, size_t>& id_to_index,
    const std::unordered_map<uint32_t, NodeSemantics>& node_semantics,
    const std::unordered_map<uint32_t, QPointF>& scroll_offsets, qreal item_w, qreal item_h,
    uint32_t id, const QPointF& parent_origin, const QRectF& clip_rect, const QPointF& query,
    bool interactive_only, bool scrollable_only) {
  const NveRenderNode* node = lookupSceneNode(view, id_to_index, id);
  if (node == nullptr || node->opacity_u8 == 0u) return 0u;
  const bool is_root = (id == NVE_ROOT_NODE_ID);
  const QPointF origin = is_root ? QPointF() : QPointF(parent_origin.x() + node->x, parent_origin.y() + node->y);
  const QRectF bounds(
      origin.x(), origin.y(), (is_root && node->width <= 0.0f) ? item_w : node->width,
      (is_root && node->height <= 0.0f) ? item_h : node->height);
  const bool clips = (node->kind == NVE_NODE_KIND_SCROLL_VIEW) || (node->kind == NVE_NODE_KIND_TEXT_INPUT) ||
                     ((node->flags & NVE_NODE_CLIP_BOUNDS) != 0u && node->child_count > 0u);
  const QRectF next_clip = clips ? clip_rect.intersected(bounds) : clip_rect;
  if (clips && !next_clip.contains(query)) return 0u;

  const QPointF soff = scrollOffsetOf(*node, scroll_offsets);
  const QPointF child_origin(origin.x() - soff.x(), origin.y() - soff.y());
  const auto kids = sortedChildrenByZ(view, id_to_index, *node, false);
  for (auto it = kids.rbegin(); it != kids.rend(); ++it) {
    if (const uint32_t hit = hitTestRecursiveImpl(
            view, id_to_index, node_semantics, scroll_offsets, item_w, item_h,
            *it, child_origin, next_clip, query, interactive_only, scrollable_only);
        hit != 0u) {
      return hit;
    }
  }
  if (is_root || !bounds.contains(query) || !clip_rect.contains(query)) return 0u;
  if (scrollable_only) return (node->kind == NVE_NODE_KIND_SCROLL_VIEW) ? id : 0u;
  if (interactive_only) {
    const auto sit = node_semantics.find(id);
    const bool disabled = (sit != node_semantics.end() && sit->second.disabled);
    const bool interactive = (node->flags & NVE_NODE_INTERACTIVE) != 0u ||
                             node->kind == NVE_NODE_KIND_PRESSABLE || node->kind == NVE_NODE_KIND_TEXT_INPUT;
    return (interactive && !disabled) ? id : 0u;
  }
  return id;
}

bool findBoundsRecursiveImpl(
    const NveSceneView& view, const std::unordered_map<uint32_t, size_t>& id_to_index,
    const std::unordered_map<uint32_t, QPointF>& scroll_offsets, uint32_t cur_id,
    uint32_t target_id, const QPointF& parent_origin, QRectF& out_rect) {
  const NveRenderNode* node = lookupSceneNode(view, id_to_index, cur_id);
  if (node == nullptr) return false;
  const QPointF origin = (cur_id == NVE_ROOT_NODE_ID)
                             ? QPointF()
                             : QPointF(parent_origin.x() + node->x, parent_origin.y() + node->y);
  if (cur_id == target_id) { out_rect = QRectF(origin.x(), origin.y(), node->width, node->height); return true; }
  const QPointF soff = scrollOffsetOf(*node, scroll_offsets);
  const QPointF child_origin(origin.x() - soff.x(), origin.y() - soff.y());
  for (uint32_t cid : nve::scene_node_children(view, *node)) {
    if (findBoundsRecursiveImpl(view, id_to_index, scroll_offsets, cid, target_id, child_origin, out_rect)) return true;
  }
  return false;
}

} // namespace

uint32_t hitTestSceneView(
    const NveSceneView& view, const std::unordered_map<uint32_t, size_t>& id_to_index,
    const std::unordered_map<uint32_t, NodeSemantics>& node_semantics,
    const std::unordered_map<uint32_t, QPointF>& scroll_offsets, qreal item_width, qreal item_height,
    const QPointF& query, bool interactive_only, bool scrollable_only) {
  if (view.node_count == 0u || view.nodes == nullptr) return 0u;
  return hitTestRecursiveImpl(
      view, id_to_index, node_semantics, scroll_offsets, item_width, item_height, NVE_ROOT_NODE_ID,
      QPointF(), QRectF(0.0, 0.0, std::max<qreal>(item_width, 1.0e6), std::max<qreal>(item_height, 1.0e6)),
      query, interactive_only, scrollable_only);
}

QRectF findSceneNodeGlobalBounds(
    const NveSceneView& view, const std::unordered_map<uint32_t, size_t>& id_to_index,
    const std::unordered_map<uint32_t, QPointF>& scroll_offsets, uint32_t target_id) {
  QRectF rect;
  return findBoundsRecursiveImpl(view, id_to_index, scroll_offsets, NVE_ROOT_NODE_ID, target_id, QPointF(), rect)
             ? rect : QRectF();
}

} // namespace nve::qt
