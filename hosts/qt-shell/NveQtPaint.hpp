// Shared Qt painting for the Widgets and Quick hosts: colors, box visuals (background, border,
// radius, outer / inset box shadows), image fit and tint, and the `NVE_DRAW1` canvas painter.
#pragma once

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

/// `node`'s visuals; `shadows` is its decoded `nve.res.box_shadows` resource (empty when none).
[[nodiscard]] BoxVisual box_visual(const host::NodeData& node, std::vector<host::BoxShadow> shadows);

/// Paints the background (rounded) and inset shadows clipped to `box`; the border is separate so
/// content can sit between them.
void paint_box_background(QPainter& p, const QRectF& box, const BoxVisual& v);
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

/// Paints all 8 `NVE_DRAW1` opcodes into `bounds` (validated first; balanced save/restore even
/// on unbalanced clip stacks). `Clear` replaces pixels on image devices and blends elsewhere.
canvas::DrawListDecodeResult paint_draw_list(QPainter& p, const QRectF& bounds, std::span<const uint8_t> bytes);

/// Rasterizes a draw list into a HiDPI premultiplied image (null when empty, invalid, or zero-sized).
[[nodiscard]] QImage rasterize_draw_list(std::span<const uint8_t> bytes, QSizeF size, qreal dpr);

}  // namespace nve::qt
