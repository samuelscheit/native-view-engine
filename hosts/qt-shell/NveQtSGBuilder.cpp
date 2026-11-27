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
// re-rasterized only when the record's paint generation or the device pixel ratio changed.
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
  std::unordered_map<uint32_t, NodeSG*> index;
};

/// A premultiplied HiDPI canvas of logical `size`, cleared, with a painter set up by `paint`.
template <class Paint>
QImage raster(QSizeF size, qreal dpr, Paint&& paint) {
  const QSize px(std::max(1, static_cast<int>(std::ceil(size.width() * dpr))),
                 std::max(1, static_cast<int>(std::ceil(size.height() * dpr))));
  QImage image(px, QImage::Format_ARGB32_Premultiplied);
  image.setDevicePixelRatio(dpr);
  image.fill(Qt::transparent);
  QPainter p(&image);
  p.setRenderHint(QPainter::Antialiasing, true);
  paint(p);
  return image;
}

/// Convex rounded-rect clip geometry as a triangle fan around its centre.
QSGGeometry* rounded_clip_geometry(const QRectF& r, qreal radius) {
  const QPolygonF outline = rounded_path(r, radius).toFillPolygon();
  auto* g = new QSGGeometry(QSGGeometry::defaultAttributes_Point2D(), static_cast<int>(outline.size()) + 1);
  g->setDrawingMode(QSGGeometry::DrawTriangleFan);
  QSGGeometry::Point2D* pts = g->vertexDataAsPoint2D();
  pts[0].set(static_cast<float>(r.center().x()), static_cast<float>(r.center().y()));
  for (qsizetype i = 0; i < outline.size(); ++i) {
    pts[i + 1].set(static_cast<float>(outline[i].x()), static_cast<float>(outline[i].y()));
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
  if (radius > 0.0) {
    clip->setGeometry(rounded_clip_geometry(r, radius));
    clip->setFlag(QSGNode::OwnsGeometry, true);
  }
  clip->markDirty(QSGNode::DirtyGeometry);
}

void update_visual(NodeSG& sg, const QuickNodeView& v, const NveQtQuickItem::Impl& impl, QQuickWindow& window,
                   qreal dpr, SceneGraphStats& stats) {
  if (sg.painted_generation == v.paint_generation && sg.painted_dpr == dpr) {
    ++stats.texturesReused;
    return;
  }
  sg.painted_generation = v.paint_generation;
  sg.painted_dpr = dpr;
  const QSizeF size = v.local.size();
  const bool empty = size.width() <= 0.0 || size.height() <= 0.0;
  const BoxVisual& visual = v.paint.visual;
  QSGNode* const after_shadow = sg.rect != nullptr ? static_cast<QSGNode*>(sg.rect)
                                : sg.painted.node != nullptr ? static_cast<QSGNode*>(sg.painted.node)
                                                             : sg.clip_parent;
  if (!empty && visual.has_outer_shadow()) {
    const QMarginsF m = outer_shadow_margins(visual.shadows);
    const QRectF area = QRectF(QPointF(), size).marginsAdded(m);
    const QImage img = raster(area.size(), dpr, [&](QPainter& p) {
      paint_outer_shadows(p, QRectF(QPointF(m.left(), m.top()), size), visual.radius, visual.shadows);
    });
    sg.set_quad(sg.shadow, window, img, area, after_shadow);
  } else {
    sg.set_quad(sg.shadow, window, QImage(), QRectF(), nullptr);
  }
  if (empty || (!v.is_input && v.paint.paints_only_background())) {
    sg.set_quad(sg.painted, window, QImage(), QRectF(), nullptr);
    if (!empty && visual.background.alpha() > 0) {
      if (sg.rect == nullptr) ++stats.solidRectNodes;
      sg.set_rect(QRectF(QPointF(), size), visual.background);
    } else {
      sg.drop_rect();
    }
    return;
  }
  sg.drop_rect();
  const bool focused = impl.focused_ == v.id;
  const QImage img = raster(size, dpr, [&](QPainter& p) {
    paint_node(p, QRectF(QPointF(), size), v.paint);
    if (v.is_input) paint_input(p, QRectF(QPointF(), size), v, focused);
  });
  sg.set_quad(sg.painted, window, img, QRectF(QPointF(), size), sg.clip_parent);
  switch (v.paint.content) {
    case NodeContent::Text: ++stats.textRasterizations; break;
    case NodeContent::Image: ++stats.imageRasterizations; break;
    case NodeContent::Canvas: ++stats.canvasRasterizations; break;
    case NodeContent::None: v.is_input ? ++stats.textRasterizations : ++stats.boxRasterizations; break;
  }
}

NodeSG& ensure(RootSG& root, QuickNodeView& v, SceneGraphStats& stats) {
  if (v.sg != nullptr) {
    ++stats.nodesReused;
    return *v.sg;
  }
  v.sg = new NodeSG(v.id);
  root.index[v.id] = v.sg;
  return *v.sg;
}

void update_node(RootSG& root, QuickNodeView& v, const NveQtQuickItem::Impl& impl, QQuickWindow& window, qreal dpr,
                 SceneGraphStats& stats) {
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
  update_visual(sg, v, impl, window, dpr, stats);
}

void relink(RootSG& root, QSGNode* container, QuickNodeView& parent, const NveQtQuickItem::Impl& impl,
            QQuickWindow& window, qreal dpr, SceneGraphStats& stats) {
  std::vector<QuickNodeView*> kids = parent.children;
  std::stable_sort(kids.begin(), kids.end(),
                   [](const QuickNodeView* a, const QuickNodeView* b) { return a->z < b->z; });
  container->removeAllChildNodes();
  for (QuickNodeView* k : kids) {
    if (k->sg == nullptr) update_node(root, *k, impl, window, dpr, stats);
    if (QSGNode* old = k->sg->parent(); old != nullptr) old->removeChildNode(k->sg);
    container->appendChildNode(k->sg);
  }
  parent.children_dirty = false;
}

/// Every record of the tree (the scene graph is rebuilt from scratch).
void mark_all(NveQtQuickItem::Impl& impl, QuickNodeView& v) {
  v.sg = nullptr;
  v.children_dirty = true;
  impl.sg_dirty_.push_back(&v);
  for (QuickNodeView* c : v.children) mark_all(impl, *c);
}

}  // namespace

QSGNode* sync_scene_graph(QSGNode* old_root, NveQtQuickItem::Impl& impl, QQuickWindow& window) {
  auto* root = static_cast<RootSG*>(old_root);
  if (root == nullptr) {
    // First pass, or Qt dropped the previous tree (window change): nothing old exists anymore.
    root = new RootSG();
    impl.sg_dead_.clear();
    impl.sg_dirty_.clear();
    mark_all(impl, impl.root_);
  }
  SceneGraphStats& stats = impl.stats_;
  if (impl.sg_dirty_.empty() && impl.sg_dead_.empty()) return root;
  ++stats.syncCount;

  // 1. Wrappers of destroyed records: detach everything first (a dead wrapper may hold live
  //    children awaiting their new parent, or other dead wrappers), then delete each alone.
  for (NodeSG* dead : impl.sg_dead_) {
    if (QSGNode* p = dead->parent(); p != nullptr) p->removeChildNode(dead);
  }
  for (NodeSG* dead : impl.sg_dead_) {
    while (QSGNode* c = dead->scroll->firstChild()) dead->scroll->removeChildNode(c);
    if (auto it = root->index.find(dead->id); it != root->index.end() && it->second == dead) root->index.erase(it);
    delete dead;
  }
  impl.sg_dead_.clear();

  // 2. Dirty records' properties and visuals; 3. child lists whose membership / order changed.
  const qreal dpr = window.effectiveDevicePixelRatio();
  std::vector<QuickNodeView*> dirty;
  dirty.swap(impl.sg_dirty_);
  for (QuickNodeView* v : dirty) {
    if (v == &impl.root_) continue;
    update_node(*root, *v, impl, window, dpr, stats);
    ++stats.nodesUpdated;
  }
  const QSizeF item(impl.config().width, impl.config().height);
  root->background->setRect(QRectF(QPointF(), item));
  root->background->setColor(impl.root_.paint.visual.background);
  for (QuickNodeView* v : dirty) {
    if (!v->children_dirty) continue;
    QSGNode* container = v == &impl.root_ ? root->container : v->sg->scroll;
    relink(*root, container, *v, impl, window, dpr, stats);
  }
  stats.retainedNodes = static_cast<uint32_t>(root->index.size());
  return root;
}

const QSGNode* find_node_sg(const QSGNode* root, uint32_t node_id) noexcept {
  if (root == nullptr) return nullptr;
  const auto& index = static_cast<const RootSG*>(root)->index;
  const auto it = index.find(node_id);
  return it != index.end() ? it->second : nullptr;
}

void paint_input(QPainter& p, const QRectF& box, const QuickNodeView& v, bool focused) {
  const QuickInput& in = v.input;
  const QRectF content = v.paint.content_box.translated(box.topLeft());
  QtTextStyle style = v.paint.style;
  style.wrap = in.multiline;
  style.max_lines = in.multiline ? 0 : 1;
  const QString shown = in.secure ? QString(in.edit.size(), QChar(0x2022)) : in.edit;
  const int caret = std::clamp(in.caret, 0, static_cast<int>(shown.size()));
  QString text = shown;
  text.insert(caret, in.preedit);
  p.save();
  p.setClipRect(content);
  if (text.isEmpty() && !in.placeholder.isEmpty()) {
    QtTextStyle dim = style;
    dim.color.setAlphaF(style.color.alphaF() * 0.5f);
    (void)layout_text(in.placeholder, dim, content.width(), &p, content.topLeft());
  } else {
    (void)layout_text(text, style, content.width(), &p, content.topLeft());
  }
  const QFontMetricsF fm(style.font);
  const qreal line = fm.height();
  if (!in.preedit.isEmpty()) {  // composition underline
    const qreal x0 = content.left() + fm.horizontalAdvance(shown.left(caret));
    p.fillRect(QRectF(x0, content.top() + line - 1.0, fm.horizontalAdvance(in.preedit), 1.0), style.color);
  }
  if (focused && !in.read_only) {
    const qreal x = content.left() + fm.horizontalAdvance(text.left(caret + static_cast<int>(in.preedit.size())));
    p.fillRect(QRectF(std::min(x, content.right() - 1.0), content.top(), 1.0, line), style.color);
  }
  p.restore();
}

}  // namespace nve::qt
