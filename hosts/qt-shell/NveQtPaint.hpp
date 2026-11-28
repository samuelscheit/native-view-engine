// Shared Qt painting for the Widgets and Quick hosts: colors, box visuals (background, border,
// radius, outer / inset box shadows), image fit and tint, and the flat draw-list canvas painter.
#pragma once

#include "NveQtText.hpp"

#include <nve/canvas/canvas.hpp>
#include <nve/host/core_resources.hpp>
#include <nve/host/scene_nodes.hpp>

#include <QColor>
#include <QImage>
#include <QMarginsF>
#include <QPainter>
#include <QPainterPath>
#include <QRectF>
#include <QSizeF>

#include <cstdint>
#include <span>
#include <vector>

namespace nve::qt {

/// Packed RGBA8888 (`r` in the low byte) as a `QColor`.
[[nodiscard]] QColor qcolor(uint32_t rgba) noexcept;

/// Owned box visuals of one node (outer shadows paint below it, inset shadows inside it).
struct BoxVisual {
  QColor background{Qt::transparent};
  QColor border_color{Qt::transparent};
  qreal border_width = 0.0;
  qreal radius = 0.0;
  std::vector<host::BoxShadow> shadows;

  [[nodiscard]] bool has_border() const noexcept { return border_width > 0.0 && border_color.alpha() > 0; }
  [[nodiscard]] bool has_outer_shadow() const noexcept;
  [[nodiscard]] bool has_inset_shadow() const noexcept;
  friend bool operator==(const BoxVisual&, const BoxVisual&) noexcept = default;
};

/// A cleared premultiplied canvas for a shadow of logical `size` wanted at `dpr` device pixels per
/// unit: at most 4096 device pixels per axis (a larger area gets a lower `scale`). Paint into it
/// with `QPainter::scale(scale, scale)` (never through a device pixel ratio: QPainter treats one
/// below 1 as 1) and draw it into a logical target rect.
struct ShadowCanvas {
  QImage image;       // null when `size` is empty or the allocation fails
  qreal scale = 0.0;  // device pixels per logical unit, at most `dpr`

  /// Exact 1:1 logical rect covered by `image` when anchored at `top_left` (never stretches
  /// `ceil`-padded edge pixels into a smaller logical box).
  [[nodiscard]] QRectF logical_rect(QPointF top_left) const noexcept {
    return scale > 0.0 ? QRectF(top_left, QSizeF(image.width() / scale, image.height() / scale)) : QRectF();
  }
};
[[nodiscard]] ShadowCanvas shadow_canvas(QSizeF size, qreal dpr);

/// `node`'s visuals; `shadows` is its decoded `nve.res.box_shadows` resource (empty when none).

/// `node`'s visuals; `shadows` is its decoded `nve.res.box_shadows` resource (empty when none).
[[nodiscard]] BoxVisual box_visual(const host::NodeData& node, std::vector<host::BoxShadow> shadows);

/// Box shadows of one box rasterized once (blur included) and redrawn as an image until the box
/// size, radius, shadows, or device pixel ratio change. Copies start empty.
class ShadowCache {
 public:
  ShadowCache() = default;
  ShadowCache(const ShadowCache&) noexcept {}
  ShadowCache& operator=(const ShadowCache&) noexcept {
    image_ = QImage();
    return *this;
  }

  /// Paints the outer (or `inset`) shadows of `box` as `paint_outer_shadows` (or the inset
  /// shadows `paint_box_background` casts into a padding box) would.
  void paint(QPainter& p, const QRectF& box, qreal radius, std::span<const host::BoxShadow> shadows, bool inset);

 private:
  QImage image_;
  QRectF area_;  // logical rect the image covers, relative to the box's top-left
  QSizeF size_;
  qreal radius_ = 0.0;
  qreal dpr_ = 0.0;
  bool inset_ = false;
  std::vector<host::BoxShadow> shadows_;
};

/// Paints the background (rounded) and inset shadows clipped to `box` (through `inset_cache`
/// when given); the border is separate so content can sit between them.
void paint_box_background(QPainter& p, const QRectF& box, const BoxVisual& v, ShadowCache* inset_cache = nullptr);
void paint_box_border(QPainter& p, const QRectF& box, const BoxVisual& v);

/// How far the outer shadows reach beyond the box on each side.
[[nodiscard]] QMarginsF outer_shadow_margins(std::span<const host::BoxShadow> shadows) noexcept;

/// Paints the outer shadows of `box` (CSS: offset, spread, Gaussian blur of `blur / 2`, never
/// under the box itself).
void paint_outer_shadows(QPainter& p, const QRectF& box, qreal radius, std::span<const host::BoxShadow> shadows);

/// Rounded-rect path with the radius clamped to half the shorter side.
[[nodiscard]] QPainterPath rounded_path(const QRectF& r, qreal radius);

/// Destination and source rects of an image of `image` size drawn into `box` with
/// `NVE_IMAGE_FIT_*` `fit`; `false` when nothing is visible.
[[nodiscard]] bool image_fit_rects(uint8_t fit, const QRectF& box, const QSizeF& image, QRectF& out_dst,
                                   QRectF& out_src) noexcept;

/// `image` recolored with `tint` (alpha kept), as for `nve.image.tint`.
[[nodiscard]] QImage tinted_image(const QImage& image, const QColor& tint);

/// Paints `image` into `box` with `fit`, clipped to the rounded box.
void paint_image(QPainter& p, const QRectF& box, qreal radius, const QImage& image, uint8_t fit);

/// Paints all 8 flat `nve.res.draw_list` opcodes into `bounds` (validated first; balanced
/// save/restore even on unbalanced clip stacks). `Clear` replaces pixels on image devices and
/// blends elsewhere.
canvas::DrawListDecodeResult paint_draw_list(QPainter& p, const QRectF& bounds, std::span<const uint8_t> bytes);

/// Rasterizes a draw list into a HiDPI premultiplied image (null when empty, invalid, or zero-sized).
[[nodiscard]] QImage rasterize_draw_list(std::span<const uint8_t> bytes, QSizeF size, qreal dpr);

/// What a node paints between its background and its border.
enum class NodeContent : uint8_t { None, Text, Image, Canvas };

/// Everything one node paints inside its own box (outer shadows paint separately, below it).
struct NodePaint {
  BoxVisual visual;
  QRectF content_box;  // relative to the node's box origin
  bool clip_rounded = false;  // `overflow` clipping with rounded corners
  NodeContent content = NodeContent::None;
  QString text;
  QtTextStyle style;
  QImage image;
  uint8_t fit = NVE_IMAGE_FIT_COVER;
  std::vector<uint8_t> draw_list;  // owned: painted after the scene is released
  // Repaint cache (a widget repaints its unchanged node often: hover, expose, scrolling).
  mutable TextLayoutCache text_layout;

  [[nodiscard]] bool paints_only_background() const noexcept {
    return content == NodeContent::None && !visual.has_border() && !visual.has_inset_shadow() && visual.radius <= 0.0;
  }
};

/// Paints `n` into `box` (the node's rect, origin = the node's top-left): background, inset
/// shadows (through `inset_cache` when it repaints often, e.g. a widget), content (text / image /
/// canvas, clipped to the rounded box), border.
void paint_node(QPainter& p, const QRectF& box, const NodePaint& n, ShadowCache* inset_cache);

}  // namespace nve::qt
