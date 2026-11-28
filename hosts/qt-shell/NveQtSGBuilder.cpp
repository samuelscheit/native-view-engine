// The retained Qt Scene Graph of `NveQtQuickItem` (render thread, GUI thread blocked). Per node:
//
//   NodeSG (QSGTransformNode: position)
//   └ QSGOpacityNode (opacity; 0 hides the subtree)
//     ├ shadow     outer box shadows (texture reaching past the box), when present
//     ├ visual     plain background (QSGSimpleRectNode) or the painted node (texture), when present
//     └ clip parent  QSGClipNode for `overflow` / scroll containers (rounded when needed), else an
//       │            identity QSGTransformNode
//       └ QSGTransformNode (-scroll offset) -> child NodeSGs in paint order (z-index, scene order)
//
// Renderables are retained and updated in place. Every container whose children change after it
// is attached records render state (transform / opacity / clip node, never a plain `QSGNode`): the
// software adaptation seeds nodes added later from their parent's recorded state only
// (docs/UPSTREAM.md). Only records marked dirty on the GUI thread are visited; textures are
// re-uploaded only when the record's GUI-rasterized images (`QuickRaster`) changed: no text,
// font, or blur work happens here (NveQtQuickRaster.cpp).
//
// Ownership: the root's index owns every `NodeSG` (none is owned by its scene-graph parent), so
// a wrapper detached from the tree (its record orphaned, or its parent relinked) is never leaked
// and never deleted twice; Qt deleting the root deletes them all.
#include "NveQtQuickInternal.hpp"

#include <QQuickWindow>
#include <QSGClipNode>
#include <QSGFlatColorMaterial>
#include <QSGGeometry>
#include <QSGOpacityNode>
#include <QSGSimpleRectNode>
#include <QSGSimpleTextureNode>
#include <QSGTexture>
#include <QSGTransformNode>

#include <algorithm>
#include <cmath>
#include <memory>

namespace nve::qt {

namespace {

/// A retained texture quad owning its texture (replaced textures are deleted after the swap).
struct TextureQuad {
  QSGSimpleTextureNode* node = nullptr;
  std::unique_ptr<QSGTexture> texture;
};

}  // namespace

class NodeSG final : public QSGTransformNode {
 public:
  explicit NodeSG(uint32_t node_id) : id(node_id) {
    setFlag(QSGNode::OwnedByParent, false);  // owned by `RootSG::index`
    appendChildNode(opacity);
    opacity->appendChildNode(clip_parent);
    clip_parent->appendChildNode(scroll);
  }

  /// Swaps the clip parent between a `QSGClipNode` and an identity transform, keeping children.
  void set_clipping(bool on) {
    if (on == clipping) return;
    clipping = on;
    clip_parent->removeChildNode(scroll);
    opacity->removeChildNode(clip_parent);
    delete clip_parent;
    clip_parent = on ? static_cast<QSGNode*>(new QSGClipNode()) : new QSGTransformNode();
    opacity->appendChildNode(clip_parent);
    clip_parent->appendChildNode(scroll);
    clip_size = QSizeF();
    clip_radius = -1.0;
  }

  /// Shows `image` at `rect` in `quad` (created before `before`), or removes the quad when null.
  void set_quad(TextureQuad& quad, QQuickWindow& window, const QImage& image, const QRectF& rect, QSGNode* before) {
    if (image.isNull()) {
      if (quad.node != nullptr) {
        opacity->removeChildNode(quad.node);
        delete quad.node;
        quad.node = nullptr;
      }
      quad.texture.reset();
      return;
    }
    std::unique_ptr<QSGTexture> texture(window.createTextureFromImage(image));
    if (quad.node == nullptr) {
      quad.node = new QSGSimpleTextureNode();
      quad.node->setFiltering(QSGTexture::Linear);
      quad.node->setTexture(texture.get());
      opacity->insertChildNodeBefore(quad.node, before);
    } else {
      quad.node->setTexture(texture.get());
    }
    quad.node->setRect(rect);
    quad.texture = std::move(texture);  // the previous texture is no longer referenced
  }

  void set_rect(const QRectF& r, const QColor& color) {
    if (rect == nullptr) {
      rect = new QSGSimpleRectNode(r, color);
      opacity->insertChildNodeBefore(rect, clip_parent);
      return;
    }
    if (rect->rect() != r) rect->setRect(r);
    if (rect->color() != color) rect->setColor(color);
  }
  void drop_rect() {
    if (rect == nullptr) return;
    opacity->removeChildNode(rect);
    delete rect;
    rect = nullptr;
  }

  const uint32_t id;
  QSGOpacityNode* opacity = new QSGOpacityNode();
  TextureQuad shadow;
  TextureQuad painted;
  QSGSimpleRectNode* rect = nullptr;
  QSGNode* clip_parent = new QSGTransformNode();
  QSGTransformNode* scroll = new QSGTransformNode();  // children container
  bool clipping = false;
  uint64_t painted_generation = 0u;
  qreal painted_dpr = 0.0;
  QSizeF clip_size;
  qreal clip_radius = -1.0;
};

namespace {

/// Root of the item's tree: the ROOT background, then the top-level nodes; indexes every wrapper.
class RootSG final : public QSGNode {
 public:
  RootSG() {
    background->setColor(Qt::transparent);
    appendChildNode(background);
    appendChildNode(container);
  }
  QSGSimpleRectNode* background = new QSGSimpleRectNode();
  QSGTransformNode* container = new QSGTransformNode();  // records state (see the file comment)
  /// Owns every wrapper, attached or not (a deleted wrapper detaches itself and its children).
  std::unordered_map<uint32_t, std::unique_ptr<NodeSG>> index;
};

/// Convex rounded-rect clip geometry as a triangle list around its centre (one triangle per
/// outline edge). Never a triangle fan: Metal and Direct3D have no fan topology, so the RHI
/// renderer cannot draw one.
QSGGeometry* rounded_clip_geometry(const QRectF& r, qreal radius) {
  const QPolygonF outline = rounded_path(r, radius).toFillPolygon();
  const qsizetype edges = outline.size();
  auto* g = new QSGGeometry(QSGGeometry::defaultAttributes_Point2D(), static_cast<int>(edges * 3));
  g->setDrawingMode(QSGGeometry::DrawTriangles);
  QSGGeometry::Point2D* pts = g->vertexDataAsPoint2D();
  const auto x = [](qreal v) { return static_cast<float>(v); };
  for (qsizetype i = 0; i < edges; ++i) {
    const QPointF& a = outline[i];
    const QPointF& b = outline[(i + 1) % edges];
    pts[i * 3].set(x(r.center().x()), x(r.center().y()));
    pts[i * 3 + 1].set(x(a.x()), x(a.y()));
    pts[i * 3 + 2].set(x(b.x()), x(b.y()));
  }
  return g;
}

void update_clip(NodeSG& sg, const QuickNodeView& v) {
  sg.set_clipping(v.clips);
  if (!v.clips) return;
  const qreal radius = v.paint.clip_rounded ? v.paint.visual.radius : 0.0;
  if (sg.clip_size == v.local.size() && sg.clip_radius == radius) return;
  sg.clip_size = v.local.size();
  sg.clip_radius = radius;
  auto* clip = static_cast<QSGClipNode*>(sg.clip_parent);
  const QRectF r(QPointF(), v.local.size());
  clip->setClipRect(r);  // the rectangular / software path
  clip->setIsRectangular(radius <= 0.0);
  // Always the current shape: a rounded geometry left over from an earlier radius would keep
  // clipping the corners of a now-rectangular box on renderers that clip by geometry.
  QSGGeometry* g = nullptr;
  if (radius > 0.0) {
    g = rounded_clip_geometry(r, radius);
  } else {
    g = new QSGGeometry(QSGGeometry::defaultAttributes_Point2D(), 4);
    QSGGeometry::updateRectGeometry(g, r);
    g->setDrawingMode(QSGGeometry::DrawTriangleStrip);
  }
  clip->setGeometry(g);  // deletes the previous one (owned)
  clip->setFlag(QSGNode::OwnsGeometry, true);
  clip->markDirty(QSGNode::DirtyGeometry);
}

/// Uploads `v`'s GUI-rasterized images when they changed since this wrapper last showed them.
void update_visual(NodeSG& sg, const QuickNodeView& v, QQuickWindow& window, SceneGraphStats& stats) {
  const QuickRaster& r = v.raster;
  if (sg.painted_generation == r.generation && sg.painted_dpr == r.dpr) {
    ++stats.texturesReused;
    return;
  }
  sg.painted_generation = r.generation;
  sg.painted_dpr = r.dpr;
  QSGNode* const after_shadow = sg.rect != nullptr ? static_cast<QSGNode*>(sg.rect)
                                : sg.painted.node != nullptr ? static_cast<QSGNode*>(sg.painted.node)
                                                             : sg.clip_parent;
  sg.set_quad(sg.shadow, window, r.shadow, r.shadow_rect, after_shadow);
  const QRectF box(QPointF(), v.local.size());
  sg.set_quad(sg.painted, window, r.painted, box, sg.clip_parent);
  if (!r.solid) {
    sg.drop_rect();
  } else {
    if (sg.rect == nullptr) ++stats.solidRectNodes;
    sg.set_rect(box, v.paint.visual.background);
  }
}

NodeSG& ensure(RootSG& root, QuickNodeView& v, SceneGraphStats& stats) {
  if (v.sg != nullptr) {
    ++stats.nodesReused;
    return *v.sg;
  }
  auto sg = std::make_unique<NodeSG>(v.id);
  v.sg = sg.get();
  root.index[v.id] = std::move(sg);
  return *v.sg;
}

void update_node(RootSG& root, QuickNodeView& v, QQuickWindow& window, SceneGraphStats& stats) {
  NodeSG& sg = ensure(root, v, stats);
  QMatrix4x4 m;
  m.translate(static_cast<float>(v.local.x()), static_cast<float>(v.local.y()));
  if (sg.matrix() != m) sg.setMatrix(m);
  const qreal opacity = v.visible ? v.opacity : 0.0;
  if (sg.opacity->opacity() != opacity) sg.opacity->setOpacity(opacity);
  update_clip(sg, v);
  QMatrix4x4 s;
  s.translate(static_cast<float>(-v.scroll.x()), static_cast<float>(-v.scroll.y()));
  if (sg.scroll->matrix() != s) sg.scroll->setMatrix(s);
  update_visual(sg, v, window, stats);
}

void relink(RootSG& root, QSGNode* container, QuickNodeView& parent, QQuickWindow& window, SceneGraphStats& stats) {
  std::vector<QuickNodeView*> kids = parent.children;
  std::sort(kids.begin(), kids.end(), paints_before);
  // Wrappers are not owned by their parent: removing them only detaches.
  while (QSGNode* c = container->firstChild()) container->removeChildNode(c);
  for (QuickNodeView* k : kids) {
    if (k->sg == nullptr) update_node(root, *k, window, stats);
    if (QSGNode* old = k->sg->parent(); old != nullptr) old->removeChildNode(k->sg);
    container->appendChildNode(k->sg);
  }
  parent.children_dirty = false;
}

}  // namespace

QSGNode* sync_scene_graph(QSGNode* old_root, NveQtQuickItem::Impl& impl, QQuickWindow& window) {
  auto* root = static_cast<RootSG*>(old_root);
  if (root == nullptr) {
    // First pass, or Qt dropped the previous tree (window change): nothing old exists anymore.
    root = new RootSG();
    impl.reset_scene_graph();
  }
  SceneGraphStats& stats = impl.stats_;
  if (impl.sg_dirty_.empty() && impl.sg_dead_.empty()) return root;
  ++stats.syncCount;

  // 1. Wrappers of destroyed records (deleting one detaches it and its children, which may be
  //    live wrappers awaiting their new parent).
  for (NodeSG* dead : impl.sg_dead_) {
    if (auto it = root->index.find(dead->id); it != root->index.end() && it->second.get() == dead) {
      root->index.erase(it);
    }
  }
  impl.sg_dead_.clear();

  // 2. Dirty records' properties and visuals; 3. child lists whose membership / order changed.
  std::vector<QuickNodeView*> dirty;
  dirty.swap(impl.sg_dirty_);
  std::erase(dirty, nullptr);  // destroyed since queued
  for (QuickNodeView* v : dirty) v->sg_dirty_slot = -1;
  for (QuickNodeView* v : dirty) {
    if (v == &impl.root_) continue;
    update_node(*root, *v, window, stats);
    ++stats.nodesUpdated;
  }
  const QSizeF item(impl.config().width, impl.config().height);
  root->background->setRect(QRectF(QPointF(), item));
  root->background->setColor(impl.root_.paint.visual.background);
  for (QuickNodeView* v : dirty) {
    if (!v->children_dirty) continue;
    QSGNode* container = v == &impl.root_ ? root->container : v->sg->scroll;
    relink(*root, container, *v, window, stats);
  }
  stats.retainedNodes = static_cast<uint32_t>(root->index.size());
  return root;
}

const QSGNode* find_node_sg(const QSGNode* root, uint32_t node_id) noexcept {
  if (root == nullptr) return nullptr;
  const auto& index = static_cast<const RootSG*>(root)->index;
  const auto it = index.find(node_id);
  return it != index.end() ? it->second.get() : nullptr;
}

}  // namespace nve::qt
