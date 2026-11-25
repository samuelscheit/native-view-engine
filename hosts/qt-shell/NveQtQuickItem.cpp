#include "NveQtQuickItem.hpp"
#include "NveQtCommon.hpp"

#include <QColor>
#include <QFont>
#include <QFontMetricsF>
#include <QImage>
#include <QKeyEvent>
#include <QMatrix4x4>
#include <QMouseEvent>
#include <QPainter>
#include <QQmlEngine>
#include <QQuickWindow>
#include <QSGClipNode>
#include <QSGFlatColorMaterial>
#include <QSGGeometry>
#include <QSGGeometryNode>
#include <QSGNode>
#include <QSGOpacityNode>
#include <QSGSimpleRectNode>
#include <QSGSimpleTextureNode>
#include <QSGTexture>
#include <QSGTransformNode>
#include <QTextLayout>
#include <QTextOption>
#include <QWheelEvent>

#include <algorithm>
#include <chrono>
#include <cmath>
#include <limits>
#include <numbers>
#include <string>
#include <unordered_map>
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

/// Build perimeter points in clockwise order for a rounded rectangle `[x, x+w] x [y, y+h]`.
void appendRoundedRectPerimeter(
    std::vector<Point2D>& out,
    float x,
    float y,
    float w,
    float h,
    float radius) {
  out.clear();
  if (w <= 0.0f || h <= 0.0f) {
    return;
  }
  const float max_r = std::min(w, h) * 0.5f;
  const float r = std::clamp(radius, 0.0f, max_r);
  if (r <= 0.05f) {
    out.push_back({x, y});
    out.push_back({x + w, y});
    out.push_back({x + w, y + h});
    out.push_back({x, y + h});
    return;
  }

  out.reserve(static_cast<size_t>(4 * (kCornerSegments + 1)));
  constexpr float kHalfPi = static_cast<float>(std::numbers::pi * 0.5);
  struct Corner {
    float cx;
    float cy;
    float start_angle;
  };
  const Corner corners[4] = {
      {x + w - r, y + r, -kHalfPi},         // top-right:    -pi/2 -> 0
      {x + w - r, y + h - r, 0.0f},         // bottom-right: 0     -> pi/2
      {x + r, y + h - r, kHalfPi},          // bottom-left:  pi/2  -> pi
      {x + r, y + r, 2.0f * kHalfPi},       // top-left:     pi    -> 3*pi/2
  };
  for (const Corner& c : corners) {
    for (int i = 0; i <= kCornerSegments; ++i) {
      const float t = static_cast<float>(i) / static_cast<float>(kCornerSegments);
      const float angle = c.start_angle + t * kHalfPi;
      out.push_back({c.cx + r * std::cos(angle), c.cy + r * std::sin(angle)});
    }
  }
}

/// Build a portable `DrawTriangles` `QSGGeometry` for a filled rounded rectangle.
/// Uses explicit triangles (not `DrawTriangleFan`) for full Metal/Vulkan/D3D11/D3D12/OpenGL/Software
/// RHI portability in Qt 6.
QSGGeometry* createRoundedRectFillGeometry(float w, float h, float radius) {
  std::vector<Point2D> ring;
  appendRoundedRectPerimeter(ring, 0.0f, 0.0f, w, h, radius);
  const size_t n = ring.size();
  if (n < 3u) {
    auto* empty = new QSGGeometry(QSGGeometry::defaultAttributes_Point2D(), 0);
    empty->setDrawingMode(QSGGeometry::DrawTriangles);
    return empty;
  }

  auto* geom = new QSGGeometry(
      QSGGeometry::defaultAttributes_Point2D(), static_cast<int>(n * 3u));
  geom->setDrawingMode(QSGGeometry::DrawTriangles);
  auto* verts = geom->vertexDataAsPoint2D();

  const float cx = w * 0.5f;
  const float cy = h * 0.5f;
  size_t vi = 0;
  for (size_t i = 0; i < n; ++i) {
    const Point2D& p0 = ring[i];
    const Point2D& p1 = ring[(i + 1u) % n];
    verts[vi++].set(cx, cy);
    verts[vi++].set(p0.x, p0.y);
    verts[vi++].set(p1.x, p1.y);
  }
  return geom;
}

/// Build a portable `DrawTriangles` `QSGGeometry` for a square or rounded border ring.
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
  if (inner_w <= 0.01f || inner_h <= 0.01f) {
    return createRoundedRectFillGeometry(w, h, outer_r);
  }

  const float inner_r = std::max(0.0f, outer_r - bw);

  // Sample outer and inner rings with matching vertex counts so every segment forms a clean quad.
  const bool rounded = (outer_r > kMinRadiusForTessellation);
  std::vector<Point2D> outer;
  std::vector<Point2D> inner;

  if (!rounded) {
    outer = {{0.0f, 0.0f}, {w, 0.0f}, {w, h}, {0.0f, h}};
    inner = {{bw, bw}, {w - bw, bw}, {w - bw, h - bw}, {bw, h - bw}};
  } else {
    auto sample_ring = [](std::vector<Point2D>& dst, float x, float y, float rw, float rh, float r) {
      dst.clear();
      dst.reserve(static_cast<size_t>(4 * (kCornerSegments + 1)));
      constexpr float kHalfPi = static_cast<float>(std::numbers::pi * 0.5);
      const float cr = std::clamp(r, 0.0f, std::min(rw, rh) * 0.5f);
      struct Corner {
        float cx;
        float cy;
        float start_angle;
      };
      const Corner corners[4] = {
          {x + rw - cr, y + cr, -kHalfPi},
          {x + rw - cr, y + rh - cr, 0.0f},
          {x + cr, y + rh - cr, kHalfPi},
          {x + cr, y + cr, 2.0f * kHalfPi},
      };
      for (const Corner& c : corners) {
        for (int i = 0; i <= kCornerSegments; ++i) {
          const float t = static_cast<float>(i) / static_cast<float>(kCornerSegments);
          const float a = c.start_angle + t * kHalfPi;
          dst.push_back({c.cx + cr * std::cos(a), c.cy + cr * std::sin(a)});
        }
      }
    };
    sample_ring(outer, 0.0f, 0.0f, w, h, outer_r);
    sample_ring(inner, bw, bw, inner_w, inner_h, inner_r);
  }

  const size_t n = std::min(outer.size(), inner.size());
  auto* geom = new QSGGeometry(
      QSGGeometry::defaultAttributes_Point2D(), static_cast<int>(n * 6u));
  geom->setDrawingMode(QSGGeometry::DrawTriangles);
  auto* verts = geom->vertexDataAsPoint2D();

  size_t vi = 0;
  for (size_t i = 0; i < n; ++i) {
    const size_t next = (i + 1u) % n;
    const Point2D& o0 = outer[i];
    const Point2D& o1 = outer[next];
    const Point2D& i0 = inner[i];
    const Point2D& i1 = inner[next];

    verts[vi++].set(o0.x, o0.y);
    verts[vi++].set(o1.x, o1.y);
    verts[vi++].set(i0.x, i0.y);

    verts[vi++].set(i0.x, i0.y);
    verts[vi++].set(o1.x, o1.y);
    verts[vi++].set(i1.x, i1.y);
  }
  return geom;
}

/// Create a `QSGGeometryNode` filled with a solid color and optional rounded corners.
QSGGeometryNode* createFilledBoxNode(float w, float h, float radius, const QColor& color) {
  if (radius <= kMinRadiusForTessellation) {
    return new QSGSimpleRectNode(QRectF(0.0, 0.0, static_cast<qreal>(w), static_cast<qreal>(h)), color);
  }
  auto* node = new QSGGeometryNode();
  node->setGeometry(createRoundedRectFillGeometry(w, h, radius));
  node->setFlag(QSGNode::OwnsGeometry);
  auto* mat = new QSGFlatColorMaterial();
  mat->setColor(color);
  node->setMaterial(mat);
  node->setFlag(QSGNode::OwnsMaterial);
  return node;
}

/// Create a `QSGGeometryNode` for a border stroke around `[0, w] x [0, h]`.
QSGGeometryNode* createBorderNode(
    float w,
    float h,
    float radius,
    float border_width,
    const QColor& color) {
  auto* node = new QSGGeometryNode();
  node->setGeometry(createBorderRingGeometry(w, h, radius, border_width));
  node->setFlag(QSGNode::OwnsGeometry);
  auto* mat = new QSGFlatColorMaterial();
  mat->setColor(color);
  node->setMaterial(mat);
  node->setFlag(QSGNode::OwnsMaterial);
  return node;
}

/// Create a `QSGClipNode` for `[0, w] x [0, h]`, using GPU scissor clipping when rectangular
/// and stencil geometry clipping when rounded.
QSGClipNode* createClipNode(float w, float h, float radius) {
  auto* clip = new QSGClipNode();
  const QRectF rect(0.0, 0.0, static_cast<qreal>(w), static_cast<qreal>(h));
  if (radius <= kMinRadiusForTessellation) {
    clip->setIsRectangular(true);
    clip->setClipRect(rect);
    auto* geom = new QSGGeometry(QSGGeometry::defaultAttributes_Point2D(), 4);
    QSGGeometry::updateRectGeometry(geom, rect);
    clip->setGeometry(geom);
    clip->setFlag(QSGNode::OwnsGeometry);
  } else {
    clip->setIsRectangular(false);
    clip->setClipRect(rect);
    clip->setGeometry(createRoundedRectFillGeometry(w, h, radius));
    clip->setFlag(QSGNode::OwnsGeometry);
  }
  return clip;
}

/// Rasterize UTF-8 text into a HiDPI premultiplied RGBA `QImage` using `QTextLayout` + `QPainter`.
QImage rasterizeTextImage(
    const std::string& text,
    float font_size,
    uint32_t text_rgba,
    float layout_w,
    float layout_h,
    qreal dpr,
    bool is_text_input,
    bool focused,
    QSizeF& out_logical_size) {
  const qreal eff_dpr = (dpr > 0.0 && std::isfinite(dpr)) ? dpr : 1.0;
  const QFont font = makeNveFont(font_size);
  const QFontMetricsF fm(font);

  const qreal inset_x = is_text_input ? 4.0 : 0.0;
  const qreal logical_w = std::max<qreal>(1.0, std::ceil(static_cast<qreal>(layout_w)));
  const qreal logical_h = std::max<qreal>(
      std::ceil(fm.height()),
      std::ceil(static_cast<qreal>(layout_h)));
  out_logical_size = QSizeF(logical_w, logical_h);

  const int img_w = std::max(1, static_cast<int>(std::ceil(logical_w * eff_dpr)));
  const int img_h = std::max(1, static_cast<int>(std::ceil(logical_h * eff_dpr)));

  QImage img(img_w, img_h, QImage::Format_RGBA8888_Premultiplied);
  img.setDevicePixelRatio(eff_dpr);
  img.fill(Qt::transparent);

  const QString qtext = QString::fromUtf8(text.data(), static_cast<qsizetype>(text.size()));
  const qreal wrap_width = std::max<qreal>(1.0, logical_w - 2.0 * inset_x);

  // Lay out lines with QTextLayout so word-wrapping and line metrics are exact.
  QTextLayout layout(qtext, font);
  QTextOption opt;
  opt.setAlignment(Qt::AlignLeft);
  opt.setWrapMode(
      is_text_input ? QTextOption::NoWrap : QTextOption::WrapAtWordBoundaryOrAnywhere);
  layout.setTextOption(opt);

  qreal text_height = 0.0;
  qreal last_line_width = 0.0;
  qreal last_line_y = 0.0;
  qreal last_line_h = fm.height();

  layout.beginLayout();
  while (true) {
    QTextLine line = layout.createLine();
    if (!line.isValid()) {
      break;
    }
    line.setLineWidth(wrap_width);
    line.setPosition(QPointF(inset_x, text_height));
    last_line_width = line.naturalTextWidth();
    last_line_y = text_height;
    last_line_h = line.height();
    text_height += line.height();
  }
  layout.endLayout();

  // Vertically center single- or multi-line text inside the layout box (matching QLabel VCenter).
  const qreal y_offset = std::max<qreal>(0.0, (logical_h - text_height) * 0.5);

  QPainter painter(&img);
  painter.setRenderHint(QPainter::Antialiasing, true);
  painter.setRenderHint(QPainter::TextAntialiasing, true);
  painter.setFont(font);
  QColor fg = qcolorFromRgba(text_rgba);
  if (fg.alpha() == 0 && is_text_input) {
    fg = QColor(255, 255, 255, 255);
  }
  painter.setPen(fg);
  layout.draw(&painter, QPointF(0.0, y_offset));

  if (is_text_input && focused) {
    const qreal caret_x = std::min(logical_w - 2.0, inset_x + last_line_width + 1.0);
    const qreal caret_y = y_offset + last_line_y + 1.0;
    const qreal caret_h = std::max<qreal>(1.0, last_line_h - 2.0);
    painter.fillRect(QRectF(caret_x, caret_y, 1.5, caret_h), fg);
  }
  painter.end();
  return img;
}

/// Remove the last UTF-8 codepoint from a UTF-8 std::string.
void popUtf8Char(std::string& s) {
  if (s.empty()) {
    return;
  }
  while (!s.empty()) {
    const unsigned char c = static_cast<unsigned char>(s.back());
    s.pop_back();
    if ((c & 0xC0u) != 0x80u) {
      break;
    }
  }
}

/// Cached per-node GPU text texture owned by the Scene Graph root node on the render thread.
struct TextTextureCacheEntry {
  std::string text;
  float font_size{16.0f};
  uint32_t text_rgba{0};
  int quant_w{0};
  int quant_h{0};
  int quant_dpr{100};
  bool is_text_input{false};
  bool focused{false};

  std::unique_ptr<QSGTexture> texture;
  QSizeF logical_size;
  uint64_t last_used_generation{0};
  uint64_t texture_generation{0};

  bool matches(
      std::string_view next_text,
      float next_font_size,
      uint32_t next_text_rgba,
      int next_qw,
      int next_qh,
      int next_qdpr,
      bool next_input,
      bool next_focused) const noexcept {
    return text == next_text &&
           std::fabs(font_size - next_font_size) < 0.01f &&
           text_rgba == next_text_rgba &&
           quant_w == next_qw &&
           quant_h == next_qh &&
           quant_dpr == next_qdpr &&
           is_text_input == next_input &&
           focused == next_focused;
  }
};

/// Custom Scene Graph root node owning the retained content subtree and per-node GPU text texture
/// cache. Destroyed automatically on the Scene Graph render thread when the item or window closes.
class NveSGRootNode final : public QSGNode {
 public:
  NveSGRootNode() = default;

  ~NveSGRootNode() override {
    // Delete content subtree before releasing cached QSGTextures so no QSGSimpleTextureNode ever
    // outlives the texture it references.
    removeAllChildNodes();
    text_cache_.clear();
  }

  std::unordered_map<uint32_t, TextTextureCacheEntry> text_cache_;
  uint64_t last_built_generation{0};
  int last_built_dpr_centis{0};
  uint64_t total_rasterizations{0};
  uint64_t total_cache_hits{0};
};

} // namespace

class NveQtQuickItem::Impl {
 public:
  explicit Impl(NveQtQuickItem* owner) : owner_(owner) {
    session_.set_text_measurer(NveQtQuickItem::makeQtTextMeasurer());
  }

  ~Impl() {
    session_.reset();
    scene_.clear();
    id_to_index_.clear();
  }

  NveSurfaceConfig currentSurfaceConfig(uint32_t surface_id) const {
    NveSurfaceConfig cfg{};
    cfg.surface_id = surface_id;
    const qreal w = owner_->width();
    const qreal h = owner_->height();
    cfg.width = static_cast<float>(w > 0.0 ? w : 640.0);
    cfg.height = static_cast<float>(h > 0.0 ? h : 420.0);
    const qreal dpr = effectiveDpr();
    cfg.scale_factor = static_cast<float>(dpr);
    return cfg;
  }

  qreal effectiveDpr() const {
    if (QQuickWindow* win = owner_->window()) {
      const qreal dpr = win->effectiveDevicePixelRatio();
      if (dpr > 0.0 && std::isfinite(dpr)) {
        return dpr;
      }
    }
    return 1.0;
  }

  void clearSceneState() {
    scene_.clear();
    id_to_index_.clear();
    scroll_offsets_.clear();
    text_input_buffers_.clear();
    pressed_node_id_ = 0;
    focused_node_id_ = 0;
    bumpGenerationAndRequestUpdate();
  }

  void syncScene() {
    if (session_.active_fabric_host() == nullptr) {
      return;
    }
    const auto snapshots = session_.snapshot_nodes();
    const uint64_t fid = ++scene_frame_id_;
    scene_.sync(fid, snapshots);
    rebuildNodeIndex();
    pruneTransientState();
    bumpGenerationAndRequestUpdate();
  }

  void rebuildNodeIndex() {
    id_to_index_.clear();
    const auto& nodes = scene_.nodes();
    id_to_index_.reserve(nodes.size());
    for (size_t i = 0; i < nodes.size(); ++i) {
      id_to_index_.emplace(nodes[i].node_id, i);
    }
  }

  void pruneTransientState() {
    for (auto it = scroll_offsets_.begin(); it != scroll_offsets_.end();) {
      if (id_to_index_.find(it->first) == id_to_index_.end()) {
        it = scroll_offsets_.erase(it);
      } else {
        ++it;
      }
    }
    for (auto it = text_input_buffers_.begin(); it != text_input_buffers_.end();) {
      if (id_to_index_.find(it->first) == id_to_index_.end()) {
        it = text_input_buffers_.erase(it);
      } else {
        ++it;
      }
    }
    if (focused_node_id_ != 0 && id_to_index_.find(focused_node_id_) == id_to_index_.end()) {
      focused_node_id_ = 0;
    }
    if (pressed_node_id_ != 0 && id_to_index_.find(pressed_node_id_) == id_to_index_.end()) {
      pressed_node_id_ = 0;
    }
  }

  void bumpGenerationAndRequestUpdate() {
    ++scene_generation_;
    owner_->update();
    Q_EMIT owner_->sceneChanged();
  }

  const NveRenderNode* findNode(uint32_t node_id) const noexcept {
    const auto it = id_to_index_.find(node_id);
    if (it == id_to_index_.end() || it->second >= scene_.nodes().size()) {
      return nullptr;
    }
    return &scene_.nodes()[it->second];
  }

  std::vector<uint32_t> sortedChildren(const NveRenderNode& node) const {
    std::vector<uint32_t> result;
    if (node.child_count == 0u) {
      return result;
    }
    const auto& slab = scene_.children_slab();
    const size_t start = static_cast<size_t>(node.child_offset);
    const size_t count = static_cast<size_t>(node.child_count);
    if (start > slab.size() || count > slab.size() - start) {
      return result;
    }
    result.assign(
        slab.begin() + static_cast<std::ptrdiff_t>(start),
        slab.begin() + static_cast<std::ptrdiff_t>(start + count));

    // Stable sort by z_index so higher z_index draws last (on top) while preserving source order.
    bool needs_sort = false;
    int32_t prev_z = std::numeric_limits<int32_t>::min();
    for (uint32_t cid : result) {
      const NveRenderNode* child = findNode(cid);
      const int32_t z = (child != nullptr) ? child->z_index : 0;
      if (z < prev_z) {
        needs_sort = true;
        break;
      }
      prev_z = z;
    }
    if (needs_sort) {
      std::stable_sort(result.begin(), result.end(), [this](uint32_t a, uint32_t b) {
        const NveRenderNode* na = findNode(a);
        const NveRenderNode* nb = findNode(b);
        return (na ? na->z_index : 0) < (nb ? nb->z_index : 0);
      });
    }
    return result;
  }

  QPointF scrollOffsetFor(uint32_t node_id) const {
    const auto it = scroll_offsets_.find(node_id);
    return (it != scroll_offsets_.end()) ? it->second : QPointF(0.0, 0.0);
  }

  QSizeF computeScrollContentSize(const NveRenderNode& scroll_node) const {
    qreal max_w = static_cast<qreal>(scroll_node.width);
    qreal max_h = static_cast<qreal>(scroll_node.height);
    const auto kids = sortedChildren(scroll_node);
    for (uint32_t cid : kids) {
      if (const NveRenderNode* child = findNode(cid)) {
        max_w = std::max(max_w, static_cast<qreal>(child->x + child->width));
        max_h = std::max(max_h, static_cast<qreal>(child->y + child->height));
      }
    }
    return QSizeF(max_w, max_h);
  }

  static bool isInteractiveNode(const NveRenderNode& node) noexcept {
    return (node.flags & NVE_NODE_INTERACTIVE) != 0u ||
           node.kind == NVE_NODE_KIND_PRESSABLE ||
           node.kind == NVE_NODE_KIND_TEXT_INPUT;
  }

  uint32_t hitTestRecursive(
      uint32_t node_id,
      const QPointF& parent_origin,
      const QRectF& clip_rect,
      const QPointF& query_pos,
      bool interactive_only,
      bool scrollable_only) const {
    const NveRenderNode* node = findNode(node_id);
    if (node == nullptr || node->opacity_u8 == 0u) {
      return 0u;
    }

    const bool is_root = (node_id == NVE_ROOT_NODE_ID);
    const QPointF origin = is_root
                               ? QPointF(0.0, 0.0)
                               : QPointF(
                                     parent_origin.x() + static_cast<qreal>(node->x),
                                     parent_origin.y() + static_cast<qreal>(node->y));
    const qreal w = is_root && node->width <= 0.0f ? owner_->width() : static_cast<qreal>(node->width);
    const qreal h = is_root && node->height <= 0.0f ? owner_->height() : static_cast<qreal>(node->height);
    const QRectF bounds(origin.x(), origin.y(), w, h);

    const bool clips_children =
        (node->kind == NVE_NODE_KIND_SCROLL_VIEW) ||
        (node->kind == NVE_NODE_KIND_TEXT_INPUT) ||
        ((node->flags & NVE_NODE_CLIP_BOUNDS) != 0u && node->child_count > 0u);

    const QRectF next_clip = clips_children ? clip_rect.intersected(bounds) : clip_rect;
    if (clips_children && !next_clip.contains(query_pos)) {
      return 0u;
    }

    const QPointF scroll_off =
        (node->kind == NVE_NODE_KIND_SCROLL_VIEW) ? scrollOffsetFor(node_id) : QPointF(0.0, 0.0);
    const QPointF child_origin(origin.x() - scroll_off.x(), origin.y() - scroll_off.y());

    // Traverse children in reverse paint order (topmost z_index first).
    const auto kids = sortedChildren(*node);
    for (auto it = kids.rbegin(); it != kids.rend(); ++it) {
      const uint32_t hit = hitTestRecursive(
          *it, child_origin, next_clip, query_pos, interactive_only, scrollable_only);
      if (hit != 0u) {
        return hit;
      }
    }

    if (is_root) {
      return 0u;
    }

    if (!bounds.contains(query_pos) || !clip_rect.contains(query_pos)) {
      return 0u;
    }
    if (scrollable_only) {
      return (node->kind == NVE_NODE_KIND_SCROLL_VIEW) ? node_id : 0u;
    }
    if (interactive_only) {
      return isInteractiveNode(*node) ? node_id : 0u;
    }
    return node_id;
  }

  bool findGlobalBoundsRecursive(
      uint32_t current_id,
      uint32_t target_id,
      const QPointF& parent_origin,
      QRectF& out_rect) const {
    const NveRenderNode* node = findNode(current_id);
    if (node == nullptr) {
      return false;
    }
    const bool is_root = (current_id == NVE_ROOT_NODE_ID);
    const QPointF origin = is_root
                               ? QPointF(0.0, 0.0)
                               : QPointF(
                                     parent_origin.x() + static_cast<qreal>(node->x),
                                     parent_origin.y() + static_cast<qreal>(node->y));
    if (current_id == target_id) {
      out_rect = QRectF(
          origin.x(),
          origin.y(),
          static_cast<qreal>(node->width),
          static_cast<qreal>(node->height));
      return true;
    }

    const QPointF scroll_off =
        (node->kind == NVE_NODE_KIND_SCROLL_VIEW) ? scrollOffsetFor(current_id) : QPointF(0.0, 0.0);
    const QPointF child_origin(origin.x() - scroll_off.x(), origin.y() - scroll_off.y());
    const auto kids = sortedChildren(*node);
    for (uint32_t cid : kids) {
      if (findGlobalBoundsRecursive(cid, target_id, child_origin, out_rect)) {
        return true;
      }
    }
    return false;
  }

  uint32_t dispatchDirectBatch(NveEventBatch batch) {
    if (!session_.is_valid() || batch.ptr == nullptr || batch.len == 0u) {
      return 0u;
    }
    const uint32_t dirty = session_.dispatch_direct_events(batch, /*auto_render=*/true);
    if (dirty != 0u) {
      syncScene();
    }
    (void)session_.drain_events();
    return dirty;
  }

  uint32_t dispatchSingleDirectEvent(const NveDirectEvent& ev) {
    return dispatchDirectBatch(NveEventBatch{&ev, 1u});
  }

  NveDirectEvent makeBaseEvent(uint32_t node_id, nve::EventKind kind, uint8_t payload_tag) const {
    NveDirectEvent ev{};
    ev.surface_id = surface_id_;
    ev.node_id = node_id;
    ev.timestamp_us = nve::current_timestamp_us();
    ev.kind = static_cast<uint8_t>(kind);
    ev.payload_tag = payload_tag;
    ev.value = nve::make_null_value();
    return ev;
  }

  void setFocusedNode(uint32_t next_focus) {
    if (focused_node_id_ == next_focus) {
      return;
    }
    const uint32_t prev_focus = focused_node_id_;
    focused_node_id_ = next_focus;

    NveDirectEvent batch_events[2]{};
    uint32_t count = 0;
    if (prev_focus != 0u) {
      batch_events[count++] = makeBaseEvent(prev_focus, nve::EventKind::Blur, 0u);
    }
    if (next_focus != 0u) {
      batch_events[count++] = makeBaseEvent(next_focus, nve::EventKind::Focus, 0u);
    }
    if (count > 0u) {
      (void)dispatchDirectBatch(NveEventBatch{batch_events, count});
    }
    bumpGenerationAndRequestUpdate();
  }

  std::string effectiveTextForNode(const NveRenderNode& node) const {
    if (node.kind == NVE_NODE_KIND_TEXT_INPUT) {
      const auto it = text_input_buffers_.find(node.node_id);
      if (it != text_input_buffers_.end()) {
        return it->second;
      }
    }
    const std::string_view sv = scene_.text_for(node);
    return std::string(sv.data(), sv.size());
  }

  QSGNode* buildSGSubtree(
      uint32_t node_id,
      NveSGRootNode& root_node,
      QQuickWindow* win,
      qreal dpr,
      int dpr_centis,
      SceneGraphStats& stats) const {
    const NveRenderNode* node = findNode(node_id);
    if (node == nullptr || node->opacity_u8 == 0u) {
      return nullptr;
    }

    const bool is_root = (node_id == NVE_ROOT_NODE_ID);
    const float w = (is_root && node->width <= 0.0f)
                        ? static_cast<float>(owner_->width())
                        : node->width;
    const float h = (is_root && node->height <= 0.0f)
                        ? static_cast<float>(owner_->height())
                        : node->height;

    QSGNode* subtree_root = nullptr;
    QSGNode* current_parent = nullptr;

    auto attach_wrapper = [&](QSGNode* wrapper) {
      ++stats.totalSGNodes;
      if (subtree_root == nullptr) {
        subtree_root = wrapper;
      } else {
        current_parent->appendChildNode(wrapper);
      }
      current_parent = wrapper;
    };

    // 1. Node transform to (x, y) in parent coordinates.
    if (!is_root && (node->x != 0.0f || node->y != 0.0f)) {
      auto* xform = new QSGTransformNode();
      QMatrix4x4 mat;
      mat.translate(node->x, node->y);
      xform->setMatrix(mat);
      attach_wrapper(xform);
    } else {
      attach_wrapper(new QSGNode());
    }

    // 2. Hierarchical GPU opacity node when opacity < 1.0.
    if (node->opacity_u8 < 255u) {
      auto* op = new QSGOpacityNode();
      op->setOpacity(static_cast<qreal>(node->opacity_u8) / 255.0);
      ++stats.opacityNodes;
      attach_wrapper(op);
    }

    const QColor bg_color = qcolorFromRgba(node->bg_rgba);
    const QColor border_color = qcolorFromRgba(node->border_rgba);
    const bool has_bg = (bg_color.alpha() > 0) && (w > 0.0f) && (h > 0.0f);
    const bool is_focused_input =
        (node->kind == NVE_NODE_KIND_TEXT_INPUT && focused_node_id_ == node->node_id);
    const float eff_border_w =
        (node->border_width > 0.0f) ? node->border_width : (is_focused_input ? 1.5f : 0.0f);
    const QColor eff_border_color =
        (node->border_width > 0.0f && border_color.alpha() > 0)
            ? border_color
            : (is_focused_input ? QColor(59, 130, 246, 255) : border_color);
    const bool has_border =
        (eff_border_w > 0.0f) && (eff_border_color.alpha() > 0) && (w > 0.0f) && (h > 0.0f);

    // 3. Background geometry node (fast quad or rounded-rect triangle mesh).
    if (has_bg) {
      auto* bg_node = createFilledBoxNode(w, h, node->border_radius, bg_color);
      current_parent->appendChildNode(bg_node);
      ++stats.totalSGNodes;
      ++stats.geometryNodes;
    }

    // 4. Optional clip node for ScrollView / TextInput / overflow-clipping containers.
    const bool needs_clip =
        (w > 0.0f && h > 0.0f) &&
        (node->kind == NVE_NODE_KIND_SCROLL_VIEW ||
         node->kind == NVE_NODE_KIND_TEXT_INPUT ||
         ((node->flags & NVE_NODE_CLIP_BOUNDS) != 0u && node->child_count > 0u));

    QSGNode* children_parent = current_parent;
    if (needs_clip) {
      auto* clip_node = createClipNode(w, h, node->border_radius);
      current_parent->appendChildNode(clip_node);
      ++stats.totalSGNodes;
      ++stats.clipNodes;
      children_parent = clip_node;
    }

    // 5. ScrollView scroll offset transform inside the clip node.
    if (node->kind == NVE_NODE_KIND_SCROLL_VIEW) {
      const QPointF off = scrollOffsetFor(node->node_id);
      if (off.x() != 0.0 || off.y() != 0.0) {
        auto* scroll_xform = new QSGTransformNode();
        QMatrix4x4 mat;
        mat.translate(static_cast<float>(-off.x()), static_cast<float>(-off.y()));
        scroll_xform->setMatrix(mat);
        children_parent->appendChildNode(scroll_xform);
        ++stats.totalSGNodes;
        children_parent = scroll_xform;
      }
    }

    // 6. Crisp HiDPI text texture node with generation-tracked per-node caching.
    const bool is_leaf_text = (node->flags & NVE_NODE_LEAF_TEXT) != 0u;
    const bool is_text_input = (node->kind == NVE_NODE_KIND_TEXT_INPUT);
    if ((is_leaf_text || is_text_input) && w > 0.0f && h > 0.0f) {
      const std::string text_str = effectiveTextForNode(*node);
      if (!text_str.empty() || is_focused_input) {
        const int qw = std::max(1, static_cast<int>(std::lround(w * 2.0f)));
        const int qh = std::max(1, static_cast<int>(std::lround(h * 2.0f)));

        auto& entry = root_node.text_cache_[node->node_id];
        const bool hit = entry.texture_generation != 0u &&
                         (win == nullptr || entry.texture != nullptr) &&
                         entry.matches(
                             text_str,
                             node->font_size,
                             node->text_rgba,
                             qw,
                             qh,
                             dpr_centis,
                             is_text_input,
                             is_focused_input);
        if (hit) {
          ++root_node.total_cache_hits;
          entry.last_used_generation = scene_generation_;
        } else {
          QSizeF logical_size;
          const QImage img = rasterizeTextImage(
              text_str,
              node->font_size,
              node->text_rgba,
              w,
              h,
              dpr,
              is_text_input,
              is_focused_input,
              logical_size);

          entry.text = text_str;
          entry.font_size = node->font_size;
          entry.text_rgba = node->text_rgba;
          entry.quant_w = qw;
          entry.quant_h = qh;
          entry.quant_dpr = dpr_centis;
          entry.is_text_input = is_text_input;
          entry.focused = is_focused_input;
          entry.logical_size = logical_size;
          entry.last_used_generation = scene_generation_;
          ++entry.texture_generation;
          ++root_node.total_rasterizations;

          if (win != nullptr) {
            entry.texture.reset(
                win->createTextureFromImage(img, QQuickWindow::TextureHasAlphaChannel));
          } else {
            entry.texture.reset();
          }
        }

        if (entry.texture != nullptr) {
          auto* tex_node = new QSGSimpleTextureNode();
          tex_node->setTexture(entry.texture.get());
          tex_node->setOwnsTexture(false);
          tex_node->setFiltering(QSGTexture::Linear);
          tex_node->setRect(QRectF(0.0, 0.0, entry.logical_size.width(), entry.logical_size.height()));
          children_parent->appendChildNode(tex_node);
          ++stats.totalSGNodes;
          ++stats.textTextureNodes;
        }
      }
    }

    // 7. Recurse into sorted children.
    const auto kids = sortedChildren(*node);
    for (uint32_t cid : kids) {
      if (QSGNode* child_sg = buildSGSubtree(cid, root_node, win, dpr, dpr_centis, stats)) {
        children_parent->appendChildNode(child_sg);
      }
    }

    // 8. Border stroke rendered cleanly above background & clipped children.
    if (has_border) {
      auto* border_node =
          createBorderNode(w, h, node->border_radius, eff_border_w, eff_border_color);
      current_parent->appendChildNode(border_node);
      ++stats.totalSGNodes;
      ++stats.geometryNodes;
    }

    return subtree_root;
  }

  QSGNode* syncSceneGraph(QSGNode* old_node) {
    auto* root = static_cast<NveSGRootNode*>(old_node);
    if (root == nullptr) {
      root = new NveSGRootNode();
    }

    const qreal dpr = effectiveDpr();
    const int dpr_centis = std::max(1, static_cast<int>(std::lround(dpr * 100.0)));

    // Fast path: nothing in the scene or DPR changed since the last Scene Graph sync!
    if (root->last_built_generation == scene_generation_ &&
        root->last_built_dpr_centis == dpr_centis &&
        root->last_built_generation != 0u) {
      ++sg_stats_.syncCount;
      return root;
    }

    // Destroy the previous display list BEFORE pruning unused textures so no deleted
    // QSGSimpleTextureNode can outlive its cached QSGTexture.
    root->removeAllChildNodes();

    SceneGraphStats next_stats{};
    next_stats.syncCount = sg_stats_.syncCount + 1u;
    next_stats.sceneGeneration = scene_generation_;
    next_stats.totalSGNodes = 1u; // root node itself

    QQuickWindow* win = owner_->window();
    if (!scene_.nodes().empty()) {
      if (QSGNode* tree =
              buildSGSubtree(NVE_ROOT_NODE_ID, *root, win, dpr, dpr_centis, next_stats)) {
        root->appendChildNode(tree);
      }
    }

    // Evict textures belonging to removed or cleared text nodes so GPU memory stays bounded.
    for (auto it = root->text_cache_.begin(); it != root->text_cache_.end();) {
      if (it->second.last_used_generation != scene_generation_) {
        it = root->text_cache_.erase(it);
      } else {
        ++it;
      }
    }

    root->last_built_generation = scene_generation_;
    root->last_built_dpr_centis = dpr_centis;

    next_stats.textRasterizations = root->total_rasterizations;
    next_stats.textCacheHits = root->total_cache_hits;
    next_stats.cachedTextTextures = static_cast<uint32_t>(root->text_cache_.size());
    sg_stats_ = next_stats;

    return root;
  }

  NveQtQuickItem* owner_{nullptr};
  nve::NativeHostSession session_;
  nve::SceneTableBuilder scene_;
  std::unordered_map<uint32_t, size_t> id_to_index_;
  std::unordered_map<uint32_t, QPointF> scroll_offsets_;
  std::unordered_map<uint32_t, std::string> text_input_buffers_;

  QString dylib_path_;
  uint32_t surface_id_{1};
  uint64_t scene_frame_id_{0};
  uint64_t scene_generation_{0};

  uint32_t pressed_node_id_{0};
  QPointF press_origin_{};
  uint32_t focused_node_id_{0};

  SceneGraphStats sg_stats_{};
};

NveQtQuickItem::NveQtQuickItem(QQuickItem* parent)
    : QQuickItem(parent), impl_(std::make_unique<Impl>(this)) {
  setFlag(ItemHasContents, true);
  setAcceptedMouseButtons(Qt::LeftButton | Qt::RightButton);
  setAcceptHoverEvents(true);
  setImplicitWidth(640.0);
  setImplicitHeight(420.0);
}

NveQtQuickItem::~NveQtQuickItem() = default;

int NveQtQuickItem::registerQmlType() {
  return qmlRegisterType<nve::qt::NveQtQuickItem>("Nve", 1, 0, "NveSurface");
}

nve::TextMeasurerCallback NveQtQuickItem::makeQtTextMeasurer() {
  return makeSharedQtTextMeasurer();
}

bool NveQtQuickItem::attachInterface(NveGetInterfaceFn getInterface, uint32_t surfaceId) {
  const bool was_loaded = impl_->session_.is_valid();
  const NveSurfaceConfig cfg = impl_->currentSurfaceConfig(surfaceId);
  if (!impl_->session_.attach_interface(getInterface, cfg)) {
    return false;
  }
  const bool sid_changed = (impl_->surface_id_ != surfaceId);
  impl_->surface_id_ = surfaceId;
  impl_->clearSceneState();
  impl_->syncScene();
  (void)impl_->session_.drain_events();
  if (sid_changed) {
    Q_EMIT surfaceIdChanged();
  }
  if (!was_loaded) {
    Q_EMIT appLoadedChanged();
  }
  return true;
}

bool NveQtQuickItem::loadDylib(
    const QString& path,
    uint32_t surfaceId,
    int* outErrorCode,
    QString* outErrorMessage) {
  const bool was_loaded = impl_->session_.is_valid();
  const QByteArray utf8 = path.toUtf8();
  const NveSurfaceConfig cfg = impl_->currentSurfaceConfig(surfaceId);
  int err_code = 0;
  std::string err_msg;
  const bool ok = impl_->session_.load_dylib(
      std::string_view(utf8.constData(), static_cast<size_t>(utf8.size())),
      cfg,
      &err_code,
      &err_msg);
  if (outErrorCode != nullptr) {
    *outErrorCode = err_code;
  }
  if (outErrorMessage != nullptr) {
    *outErrorMessage = QString::fromStdString(err_msg);
  }
  if (!ok) {
    return false;
  }

  const bool path_changed = (impl_->dylib_path_ != path);
  const bool sid_changed = (impl_->surface_id_ != surfaceId);
  impl_->dylib_path_ = path;
  impl_->surface_id_ = surfaceId;
  impl_->clearSceneState();
  impl_->syncScene();
  (void)impl_->session_.drain_events();

  if (path_changed) {
    Q_EMIT dylibPathChanged();
  }
  if (sid_changed) {
    Q_EMIT surfaceIdChanged();
  }
  if (!was_loaded) {
    Q_EMIT appLoadedChanged();
  }
  return true;
}

bool NveQtQuickItem::hotReload(
    const QString& path,
    int* outErrorCode,
    QString* outErrorMessage) {
  const QByteArray utf8 = path.toUtf8();
  if (!impl_->session_.is_valid()) {
    const NveSurfaceConfig cfg = impl_->currentSurfaceConfig(impl_->surface_id_);
    impl_->session_.set_surface_size(cfg.width, cfg.height, cfg.scale_factor);
  }
  int err_code = 0;
  std::string err_msg;
  const bool ok = impl_->session_.reload_dylib(
      std::string_view(utf8.constData(), static_cast<size_t>(utf8.size())),
      &err_code,
      &err_msg);
  if (outErrorCode != nullptr) {
    *outErrorCode = err_code;
  }
  if (outErrorMessage != nullptr) {
    *outErrorMessage = QString::fromStdString(err_msg);
  }
  if (!ok) {
    return false;
  }
  const bool path_changed = (impl_->dylib_path_ != path);
  impl_->dylib_path_ = path;
  impl_->clearSceneState();
  impl_->syncScene();
  (void)impl_->session_.drain_events();
  if (path_changed) {
    Q_EMIT dylibPathChanged();
  }
  return true;
}

bool NveQtQuickItem::replaySnapshot(
    const QString& snapshotPath,
    uint32_t expectedSchemaVersion,
    uint32_t surfaceId,
    uint64_t* outRestoreUs) {
  const QByteArray utf8 = snapshotPath.toUtf8();
  const NveSurfaceConfig cfg = impl_->currentSurfaceConfig(surfaceId);
  impl_->session_.set_surface_config(cfg);

  using Clock = std::chrono::steady_clock;
  const auto t0 = Clock::now();
  const bool ok = impl_->session_.replay_frame0_snapshot(
      std::string_view(utf8.constData(), static_cast<size_t>(utf8.size())),
      expectedSchemaVersion);
  if (!ok) {
    return false;
  }
  const bool sid_changed = (impl_->surface_id_ != surfaceId);
  impl_->surface_id_ = surfaceId;
  impl_->clearSceneState();
  impl_->syncScene();
  const auto t1 = Clock::now();
  if (outRestoreUs != nullptr) {
    *outRestoreUs = static_cast<uint64_t>(
        std::chrono::duration_cast<std::chrono::microseconds>(t1 - t0).count());
  }
  if (sid_changed) {
    Q_EMIT surfaceIdChanged();
  }
  return true;
}

bool NveQtQuickItem::saveSnapshot(const QString& snapshotPath, uint32_t schemaVersion) const {
  const QByteArray utf8 = snapshotPath.toUtf8();
  return impl_->session_.save_snapshot(
      std::string_view(utf8.constData(), static_cast<size_t>(utf8.size())), schemaVersion);
}

bool NveQtQuickItem::applyPropsJson(std::string_view propsJson) {
  if (!impl_->session_.is_valid()) {
    return false;
  }
  const bool dirty = impl_->session_.update_props_json(propsJson, /*auto_render=*/true);
  if (dirty) {
    impl_->syncScene();
  }
  (void)impl_->session_.drain_events();
  return dirty;
}

bool NveQtQuickItem::applyPropsDirect(const NveMapEntry* entries, uint32_t count) {
  if (!impl_->session_.is_valid()) {
    return false;
  }
  const bool dirty = impl_->session_.update_props_direct(entries, count, /*auto_render=*/true);
  if (dirty) {
    impl_->syncScene();
  }
  (void)impl_->session_.drain_events();
  return dirty;
}

bool NveQtQuickItem::applyHotPatchJson(std::string_view patchJson) {
  if (!impl_->session_.is_valid() || impl_->session_.bridge() == nullptr) {
    return false;
  }
  const bool dirty =
      impl_->session_.bridge()->apply_hot_patch_json(patchJson, /*auto_render=*/true);
  if (dirty) {
    impl_->syncScene();
  }
  (void)impl_->session_.drain_events();
  return dirty;
}

int32_t NveQtQuickItem::renderFrame() {
  const int32_t rc = impl_->session_.render(nullptr);
  if (rc == 0) {
    impl_->syncScene();
  }
  (void)impl_->session_.drain_events();
  return rc;
}

uint32_t NveQtQuickItem::dispatchDirectEvents(NveEventBatch batch) {
  return impl_->dispatchDirectBatch(batch);
}

uint32_t NveQtQuickItem::dispatchPress(uint32_t nodeId, float x, float y) {
  NveDirectEvent ev = impl_->makeBaseEvent(nodeId, nve::EventKind::Press, 1u);
  ev.x = x;
  ev.y = y;
  return impl_->dispatchSingleDirectEvent(ev);
}

uint32_t NveQtQuickItem::dispatchPress(uint32_t nodeId, const QPointF& localPoint) {
  return dispatchPress(
      nodeId, static_cast<float>(localPoint.x()), static_cast<float>(localPoint.y()));
}

uint32_t NveQtQuickItem::dispatchScroll(
    uint32_t nodeId,
    float offsetX,
    float offsetY,
    float contentWidth,
    float contentHeight) {
  NveDirectEvent ev = impl_->makeBaseEvent(nodeId, nve::EventKind::Scroll, 2u);
  ev.x = offsetX;
  ev.y = offsetY;
  ev.width = contentWidth;
  ev.height = contentHeight;
  return impl_->dispatchSingleDirectEvent(ev);
}

uint32_t NveQtQuickItem::dispatchTextChange(uint32_t nodeId, std::string_view text) {
  NveDirectEvent ev = impl_->makeBaseEvent(nodeId, nve::EventKind::TextChange, 3u);
  ev.text = nve::make_str_slice(text);
  return impl_->dispatchSingleDirectEvent(ev);
}

uint32_t NveQtQuickItem::dispatchFocus(uint32_t nodeId, bool focused) {
  NveDirectEvent ev =
      impl_->makeBaseEvent(nodeId, focused ? nve::EventKind::Focus : nve::EventKind::Blur, 0u);
  return impl_->dispatchSingleDirectEvent(ev);
}

uint32_t NveQtQuickItem::hitTestNode(const QPointF& itemPos, bool interactiveOnly) const {
  if (impl_->scene_.nodes().empty()) {
    return 0u;
  }
  const QRectF root_clip(
      0.0,
      0.0,
      std::max<qreal>(width(), 1.0e6),
      std::max<qreal>(height(), 1.0e6));
  return impl_->hitTestRecursive(
      NVE_ROOT_NODE_ID, QPointF(0.0, 0.0), root_clip, itemPos, interactiveOnly, false);
}

uint32_t NveQtQuickItem::hitTestScrollableNode(const QPointF& itemPos) const {
  if (impl_->scene_.nodes().empty()) {
    return 0u;
  }
  const QRectF root_clip(
      0.0,
      0.0,
      std::max<qreal>(width(), 1.0e6),
      std::max<qreal>(height(), 1.0e6));
  return impl_->hitTestRecursive(
      NVE_ROOT_NODE_ID, QPointF(0.0, 0.0), root_clip, itemPos, false, true);
}

QRectF NveQtQuickItem::sceneNodeGlobalBounds(uint32_t nodeId) const {
  QRectF rect;
  if (impl_->findGlobalBoundsRecursive(NVE_ROOT_NODE_ID, nodeId, QPointF(0.0, 0.0), rect)) {
    return rect;
  }
  return QRectF();
}

NveSceneView NveQtQuickItem::sceneView() const noexcept {
  return impl_->scene_.view();
}

const nve::SceneTableBuilder& NveQtQuickItem::sceneBuilder() const noexcept {
  return impl_->scene_;
}

std::optional<NveRenderNode> NveQtQuickItem::sceneNode(uint32_t nodeId) const {
  if (const NveRenderNode* node = impl_->findNode(nodeId)) {
    return *node;
  }
  return std::nullopt;
}

std::string_view NveQtQuickItem::sceneNodeText(uint32_t nodeId) const {
  if (const NveRenderNode* node = impl_->findNode(nodeId)) {
    if (node->kind == NVE_NODE_KIND_TEXT_INPUT) {
      const auto it = impl_->text_input_buffers_.find(nodeId);
      if (it != impl_->text_input_buffers_.end()) {
        return it->second;
      }
    }
    return impl_->scene_.text_for(*node);
  }
  return {};
}

QPointF NveQtQuickItem::scrollOffsetForNode(uint32_t nodeId) const {
  return impl_->scrollOffsetFor(nodeId);
}

void NveQtQuickItem::setScrollOffsetForNode(uint32_t nodeId, const QPointF& offset) {
  const NveRenderNode* node = impl_->findNode(nodeId);
  if (node == nullptr) {
    return;
  }
  const QSizeF content = impl_->computeScrollContentSize(*node);
  const qreal max_x = std::max<qreal>(0.0, content.width() - static_cast<qreal>(node->width));
  const qreal max_y = std::max<qreal>(0.0, content.height() - static_cast<qreal>(node->height));
  const QPointF clamped(std::clamp(offset.x(), 0.0, max_x), std::clamp(offset.y(), 0.0, max_y));
  if (impl_->scrollOffsetFor(nodeId) != clamped) {
    impl_->scroll_offsets_[nodeId] = clamped;
    impl_->bumpGenerationAndRequestUpdate();
  }
}

uint32_t NveQtQuickItem::focusedNodeId() const noexcept {
  return impl_->focused_node_id_;
}

SceneGraphStats NveQtQuickItem::sceneGraphStats() const noexcept {
  return impl_->sg_stats_;
}

QSGNode* NveQtQuickItem::syncSceneGraphNode(QSGNode* oldNode) {
  return impl_->syncSceneGraph(oldNode);
}

QString NveQtQuickItem::dylibPath() const {
  return impl_->dylib_path_;
}

void NveQtQuickItem::setDylibPath(const QString& path) {
  if (path == impl_->dylib_path_) {
    return;
  }
  if (path.isEmpty()) {
    impl_->dylib_path_.clear();
    impl_->session_.reset();
    impl_->clearSceneState();
    Q_EMIT dylibPathChanged();
    Q_EMIT appLoadedChanged();
    return;
  }
  (void)loadDylib(path, impl_->surface_id_);
}

uint32_t NveQtQuickItem::surfaceId() const noexcept {
  return impl_->surface_id_;
}

void NveQtQuickItem::setSurfaceId(uint32_t id) {
  if (impl_->surface_id_ == id) {
    return;
  }
  impl_->surface_id_ = id;
  NveSurfaceConfig cfg = impl_->currentSurfaceConfig(id);
  impl_->session_.set_surface_config(cfg);
  Q_EMIT surfaceIdChanged();
}

uint32_t NveQtQuickItem::nodeCount() const {
  return impl_->session_.node_count();
}

bool NveQtQuickItem::isAppLoaded() const noexcept {
  return impl_->session_.is_valid();
}

uint64_t NveQtQuickItem::sceneGeneration() const noexcept {
  return impl_->scene_generation_;
}

void NveQtQuickItem::setOnNveEvent(nve::EventCallback callback) {
  impl_->session_.set_event_callback(std::move(callback));
}

void NveQtQuickItem::setTextMeasurer(nve::TextMeasurerCallback measurer) {
  impl_->session_.set_text_measurer(std::move(measurer));
  if (impl_->session_.active_fabric_host() != nullptr) {
    const NveSurfaceConfig cfg = impl_->currentSurfaceConfig(impl_->surface_id_);
    impl_->session_.set_surface_size(cfg.width, cfg.height, cfg.scale_factor);
    impl_->syncScene();
  }
}

nve::NativeHostSession& NveQtQuickItem::session() noexcept {
  return impl_->session_;
}

const nve::NativeHostSession& NveQtQuickItem::session() const noexcept {
  return impl_->session_;
}

QSGNode* NveQtQuickItem::updatePaintNode(
    QSGNode* oldNode,
    UpdatePaintNodeData* /*updatePaintNodeData*/) {
  return impl_->syncSceneGraph(oldNode);
}

void NveQtQuickItem::geometryChange(const QRectF& newGeometry, const QRectF& oldGeometry) {
  QQuickItem::geometryChange(newGeometry, oldGeometry);
  if (newGeometry.size() == oldGeometry.size()) {
    return;
  }
  if (newGeometry.width() > 0.0 && newGeometry.height() > 0.0) {
    const qreal dpr = impl_->effectiveDpr();
    impl_->session_.set_surface_size(
        static_cast<float>(newGeometry.width()),
        static_cast<float>(newGeometry.height()),
        static_cast<float>(dpr));
    if (impl_->session_.active_fabric_host() != nullptr) {
      impl_->syncScene();
    }
  }
}

void NveQtQuickItem::mousePressEvent(QMouseEvent* event) {
  if (event == nullptr || event->button() != Qt::LeftButton) {
    QQuickItem::mousePressEvent(event);
    return;
  }

  const QPointF pos = event->position();
  const uint32_t hit = hitTestNode(pos, /*interactiveOnly=*/true);
  if (hit != 0u) {
    if (const NveRenderNode* node = impl_->findNode(hit)) {
      if (node->kind == NVE_NODE_KIND_TEXT_INPUT) {
        forceActiveFocus(Qt::MouseFocusReason);
        impl_->setFocusedNode(hit);
      } else {
        impl_->setFocusedNode(0u);
      }
    }
    impl_->pressed_node_id_ = hit;
    impl_->press_origin_ = pos;
    event->accept();
    return;
  }

  // Clicking outside a focused text input blurs it.
  if (impl_->focused_node_id_ != 0u) {
    impl_->setFocusedNode(0u);
  }
  impl_->pressed_node_id_ = 0u;
  event->ignore();
}

void NveQtQuickItem::mouseMoveEvent(QMouseEvent* event) {
  if (event == nullptr) {
    return;
  }
  if (impl_->pressed_node_id_ != 0u && (event->buttons() & Qt::LeftButton)) {
    const QRectF bounds = sceneNodeGlobalBounds(impl_->pressed_node_id_);
    if (!bounds.isValid() || !bounds.contains(event->position())) {
      // Pointer dragged outside the pressed target; cancel pending press activation on release.
      impl_->pressed_node_id_ = 0u;
    }
    event->accept();
    return;
  }
  event->ignore();
}

void NveQtQuickItem::mouseReleaseEvent(QMouseEvent* event) {
  if (event == nullptr || event->button() != Qt::LeftButton) {
    QQuickItem::mouseReleaseEvent(event);
    return;
  }

  const uint32_t target = impl_->pressed_node_id_;
  impl_->pressed_node_id_ = 0u;
  if (target == 0u) {
    event->ignore();
    return;
  }

  const QPointF pos = event->position();
  const QRectF bounds = sceneNodeGlobalBounds(target);
  event->accept();
  if (bounds.isValid() && bounds.contains(pos)) {
    const float local_x = static_cast<float>(pos.x() - bounds.x());
    const float local_y = static_cast<float>(pos.y() - bounds.y());
    (void)dispatchPress(target, local_x, local_y);
  }
}

void NveQtQuickItem::wheelEvent(QWheelEvent* event) {
  if (event == nullptr) {
    return;
  }
  const uint32_t scroll_id = hitTestScrollableNode(event->position());
  if (scroll_id == 0u) {
    event->ignore();
    return;
  }
  const NveRenderNode* node = impl_->findNode(scroll_id);
  if (node == nullptr) {
    event->ignore();
    return;
  }

  QPointF delta = QPointF(event->pixelDelta());
  if (delta.isNull()) {
    // Convert standard 120-unit wheel ticks into smooth logical pixels (45px per notch).
    delta = QPointF(event->angleDelta()) * (45.0 / 120.0);
  }

  const QSizeF content = impl_->computeScrollContentSize(*node);
  const qreal max_x = std::max<qreal>(0.0, content.width() - static_cast<qreal>(node->width));
  const qreal max_y = std::max<qreal>(0.0, content.height() - static_cast<qreal>(node->height));

  const QPointF prev = impl_->scrollOffsetFor(scroll_id);
  const QPointF next(
      std::clamp(prev.x() - delta.x(), 0.0, max_x),
      std::clamp(prev.y() - delta.y(), 0.0, max_y));

  impl_->scroll_offsets_[scroll_id] = next;
  impl_->bumpGenerationAndRequestUpdate();
  (void)dispatchScroll(
      scroll_id,
      static_cast<float>(next.x()),
      static_cast<float>(next.y()),
      static_cast<float>(content.width()),
      static_cast<float>(content.height()));
  event->accept();
}

void NveQtQuickItem::keyPressEvent(QKeyEvent* event) {
  if (event == nullptr || impl_->focused_node_id_ == 0u) {
    QQuickItem::keyPressEvent(event);
    return;
  }

  const uint32_t target_id = impl_->focused_node_id_;
  const NveRenderNode* node = impl_->findNode(target_id);
  if (node == nullptr || node->kind != NVE_NODE_KIND_TEXT_INPUT) {
    impl_->setFocusedNode(0u);
    event->ignore();
    return;
  }

  if (event->key() == Qt::Key_Escape) {
    impl_->setFocusedNode(0u);
    event->accept();
    return;
  }

  std::string current = impl_->effectiveTextForNode(*node);
  bool changed = false;

  if (event->key() == Qt::Key_Backspace) {
    if (!current.empty()) {
      popUtf8Char(current);
      changed = true;
    }
  } else {
    const QString text = event->text();
    if (!text.isEmpty() && !text.at(0).isNull() && text.at(0).isPrint()) {
      const QByteArray utf8 = text.toUtf8();
      current.append(utf8.constData(), static_cast<size_t>(utf8.size()));
      changed = true;
    }
  }

  if (changed) {
    impl_->text_input_buffers_[target_id] = current;
    impl_->bumpGenerationAndRequestUpdate();
    (void)dispatchTextChange(target_id, current);
    event->accept();
    return;
  }

  event->ignore();
}

} // namespace nve::qt
