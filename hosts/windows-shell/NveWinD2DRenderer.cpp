#include "NveWinD2DInternal.hpp"

#if defined(_WIN32)

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <span>
#include <vector>

namespace nve::win {

namespace {

D2D1_COLOR_F to_d2d_color(uint32_t rgba) noexcept {
  const nve::RgbaColor c = nve::unpack_rgba(rgba);
  return D2D1::ColorF(c.r, c.g, c.b, c.a);
}

void paint_node_recursive(
    ID2D1RenderTarget* rt,
    ID2D1SolidColorBrush* brush,
    const NveWinVisualTree& tree,
    const DirectWriteTextMeasurer& measurer,
    uint32_t node_id,
    float origin_x,
    float origin_y,
    uint32_t depth) {
  if (depth > 512u) {
    return;
  }
  const NveWinNodeVisual* node = tree.find(node_id);
  if (node == nullptr) {
    return;
  }

  const float clamped_opacity = std::clamp(node->opacity, 0.0f, 1.0f);
  if (clamped_opacity <= 0.0f) {
    return;
  }

  ID2D1Layer* opacity_layer = nullptr;
  const bool push_layer = (clamped_opacity < 0.999f);
  if (push_layer) {
    if (SUCCEEDED(rt->CreateLayer(nullptr, &opacity_layer)) && opacity_layer != nullptr) {
      D2D1_LAYER_PARAMETERS params = D2D1::LayerParameters(
          D2D1::InfiniteRect(),
          nullptr,
          D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
          D2D1::IdentityMatrix(),
          clamped_opacity,
          nullptr,
          D2D1_LAYER_OPTIONS_NONE);
      rt->PushLayer(params, opacity_layer);
    }
  }

  const float abs_x = origin_x + node->layout.x;
  const float abs_y = origin_y + node->layout.y;
  const float w = std::max(0.0f, node->layout.width);
  const float h = std::max(0.0f, node->layout.height);
  const D2D1_RECT_F rect = D2D1::RectF(abs_x, abs_y, abs_x + w, abs_y + h);

  if (w > 0.0f && h > 0.0f) {
    const float radius = std::clamp(node->border_radius, 0.0f, 0.5f * std::min(w, h));

    const nve::RgbaColor bg = nve::unpack_rgba(node->background_rgba);
    if (bg.a8 > 0) {
      brush->SetColor(D2D1::ColorF(bg.r, bg.g, bg.b, bg.a));
      if (radius > 0.0f) {
        rt->FillRoundedRectangle(D2D1::RoundedRect(rect, radius, radius), brush);
      } else {
        rt->FillRectangle(rect, brush);
      }
    }

    if (node->kind == NVE_NODE_KIND_IMAGE && !node->image_src.empty()) {
      ID2D1Bitmap* bmp = d2d_internal::get_or_decode_d2d_bitmap(rt, node->image_src);
      if (bmp != nullptr) {
        D2D1_RECT_F dst_rect{};
        D2D1_RECT_F src_rect{};
        if (d2d_internal::compute_d2d_image_rects(
                node->resize_mode, rect, bmp->GetSize(), dst_rect, src_rect)) {
          rt->PushAxisAlignedClip(rect, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
          rt->DrawBitmap(bmp, dst_rect, 1.0f, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, src_rect);
          rt->PopAxisAlignedClip();
        }
      }
    }

    if (node->kind == NVE_NODE_KIND_SKIA_CANVAS && !node->draw_list.empty()) {
      (void)render_draw_list_to_d2d(rt, brush, rect, node->draw_list);
    }

    if (node->is_text_leaf && !node->text.empty()) {
      const nve::RgbaColor fg = nve::unpack_rgba(node->text_color_rgba);
      if (fg.a8 > 0) {
        IDWriteTextLayout* text_layout = nullptr;
        if (measurer.create_text_layout(
                node->text, node->font_size, std::max(1.0f, w), std::max(1.0f, h), &text_layout)) {
          brush->SetColor(D2D1::ColorF(fg.r, fg.g, fg.b, fg.a));
          rt->DrawTextLayout(
              D2D1::Point2F(abs_x, abs_y), text_layout, brush, D2D1_DRAW_TEXT_OPTIONS_NONE);
          text_layout->Release();
        }
      }
    }
  }

  if (!node->children.empty()) {
    const bool clip_scroll =
        (node->kind == NVE_NODE_KIND_SCROLL_VIEW || node->clip_bounds) && w > 0.0f && h > 0.0f;
    if (clip_scroll) {
      rt->PushAxisAlignedClip(rect, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
    }

    const float child_origin_x =
        abs_x - (node->kind == NVE_NODE_KIND_SCROLL_VIEW ? node->scroll_offset_x : 0.0f);
    const float child_origin_y =
        abs_y - (node->kind == NVE_NODE_KIND_SCROLL_VIEW ? node->scroll_offset_y : 0.0f);

    std::vector<uint32_t> ordered = node->children;
    std::stable_sort(ordered.begin(), ordered.end(), [&tree](uint32_t a, uint32_t b) {
      const NveWinNodeVisual* va = tree.find(a);
      const NveWinNodeVisual* vb = tree.find(b);
      return (va ? va->z_index : 0) < (vb ? vb->z_index : 0);
    });
    for (uint32_t child_id : ordered) {
      paint_node_recursive(
          rt, brush, tree, measurer, child_id, child_origin_x, child_origin_y, depth + 1u);
    }

    if (clip_scroll) {
      rt->PopAxisAlignedClip();
    }
  }

  if (w > 0.0f && h > 0.0f) {
    const float radius = std::clamp(node->border_radius, 0.0f, 0.5f * std::min(w, h));
    const nve::RgbaColor border = nve::unpack_rgba(node->border_color_rgba);
    if (node->border_width > 0.0f && border.a8 > 0) {
      const float half = 0.5f * node->border_width;
      const D2D1_RECT_F inset =
          D2D1::RectF(rect.left + half, rect.top + half, rect.right - half, rect.bottom - half);
      if (inset.right > inset.left && inset.bottom > inset.top) {
        brush->SetColor(D2D1::ColorF(border.r, border.g, border.b, border.a));
        if (radius > 0.0f) {
          const float inner_r = std::max(0.0f, radius - half);
          rt->DrawRoundedRectangle(
              D2D1::RoundedRect(inset, inner_r, inner_r), brush, node->border_width);
        } else {
          rt->DrawRectangle(inset, brush, node->border_width);
        }
      }
    }
  }

  if (opacity_layer != nullptr) {
    rt->PopLayer();
    opacity_layer->Release();
  }
}

} // namespace

nve::DrawListDecodeResult render_draw_list_to_d2d(
    ID2D1RenderTarget* rt,
    ID2D1SolidColorBrush* brush,
    const D2D1_RECT_F& bounds,
    std::span<const uint8_t> bytes) noexcept {
  if (rt == nullptr || brush == nullptr || bytes.empty()) {
    return nve::DrawListDecodeResult{nve::DrawListError::Ok, 0u, 0u};
  }
  const nve::DrawListDecodeResult check = nve::validate_draw_list(bytes);
  if (!check.ok()) {
    return check;
  }

  const float bw = bounds.right - bounds.left;
  const float bh = bounds.bottom - bounds.top;
  if (bw <= 0.0f || bh <= 0.0f) {
    return nve::DrawListDecodeResult{nve::DrawListError::Ok, 0u, bytes.size()};
  }

  // Note: ID2D1Resource::GetFactory returns a non-owning pointer (does NOT AddRef).
  ID2D1Factory* factory = nullptr;
  rt->GetFactory(&factory);

  ID2D1StrokeStyle* round_stroke = nullptr;
  if (factory != nullptr) {
    const D2D1_STROKE_STYLE_PROPERTIES stroke_props = D2D1::StrokeStyleProperties(
        D2D1_CAP_STYLE_ROUND,
        D2D1_CAP_STYLE_ROUND,
        D2D1_CAP_STYLE_ROUND,
        D2D1_LINE_JOIN_ROUND);
    (void)factory->CreateStrokeStyle(&stroke_props, nullptr, 0u, &round_stroke);
  }

  D2D1::Matrix3x2F prev_transform = D2D1::Matrix3x2F::Identity();
  rt->GetTransform(&prev_transform);
  rt->PushAxisAlignedClip(bounds, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
  rt->SetTransform(D2D1::Matrix3x2F::Translation(bounds.left, bounds.top) * prev_transform);

  struct D2DDrawListVisitor {
    ID2D1RenderTarget* target;
    ID2D1SolidColorBrush* solid_brush;
    ID2D1Factory* d2d_factory;
    ID2D1StrokeStyle* stroke_style;
    float w;
    float h;
    uint32_t clip_depth{0u};

    static D2D1_COLOR_F to_color(nve::DrawColorRgba c) noexcept {
      return D2D1::ColorF(
          static_cast<float>(c.r) / 255.0f,
          static_cast<float>(c.g) / 255.0f,
          static_cast<float>(c.b) / 255.0f,
          static_cast<float>(c.a) / 255.0f);
    }

    void on_clear(const nve::DrawClearOp& op) noexcept {
      if (op.color.a == 0u) {
        return;
      }
      solid_brush->SetColor(to_color(op.color));
      target->FillRectangle(D2D1::RectF(0.0f, 0.0f, w, h), solid_brush);
    }

    void on_fill_rect(const nve::DrawFillRectOp& op) noexcept {
      if (op.w <= 0.0f || op.h <= 0.0f || op.color.a == 0u) {
        return;
      }
      solid_brush->SetColor(to_color(op.color));
      target->FillRectangle(D2D1::RectF(op.x, op.y, op.x + op.w, op.y + op.h), solid_brush);
    }

    void on_fill_rounded_rect(const nve::DrawFillRoundedRectOp& op) noexcept {
      if (op.w <= 0.0f || op.h <= 0.0f || op.color.a == 0u) {
        return;
      }
      const D2D1_RECT_F rc = D2D1::RectF(op.x, op.y, op.x + op.w, op.y + op.h);
      const float r = std::clamp(op.radius, 0.0f, 0.5f * std::min(op.w, op.h));
      solid_brush->SetColor(to_color(op.color));
      if (r > 0.0f) {
        target->FillRoundedRectangle(D2D1::RoundedRect(rc, r, r), solid_brush);
      } else {
        target->FillRectangle(rc, solid_brush);
      }
    }

    void on_fill_circle(const nve::DrawFillCircleOp& op) noexcept {
      if (op.radius <= 0.0f || op.color.a == 0u) {
        return;
      }
      solid_brush->SetColor(to_color(op.color));
      target->FillEllipse(
          D2D1::Ellipse(D2D1::Point2F(op.cx, op.cy), op.radius, op.radius), solid_brush);
    }

    void on_stroke_line(const nve::DrawStrokeLineOp& op) noexcept {
      if (op.width <= 0.0f || op.color.a == 0u) {
        return;
      }
      solid_brush->SetColor(to_color(op.color));
      target->DrawLine(
          D2D1::Point2F(op.x0, op.y0),
          D2D1::Point2F(op.x1, op.y1),
          solid_brush,
          op.width,
          stroke_style);
    }

    void on_stroke_polyline(const nve::DrawStrokePolylineOp& op) noexcept {
      if (op.points.size() < 2u || op.width <= 0.0f || op.color.a == 0u) {
        return;
      }
      solid_brush->SetColor(to_color(op.color));
      if (d2d_factory != nullptr) {
        ID2D1PathGeometry* path = nullptr;
        if (SUCCEEDED(d2d_factory->CreatePathGeometry(&path)) && path != nullptr) {
          ID2D1GeometrySink* sink = nullptr;
          if (SUCCEEDED(path->Open(&sink)) && sink != nullptr) {
            const nve::DrawPoint p0 = op.points[0u];
            sink->BeginFigure(D2D1::Point2F(p0.x, p0.y), D2D1_FIGURE_BEGIN_HOLLOW);
            for (size_t i = 1u; i < op.points.size(); ++i) {
              const nve::DrawPoint pt = op.points[i];
              sink->AddLine(D2D1::Point2F(pt.x, pt.y));
            }
            sink->EndFigure(D2D1_FIGURE_END_OPEN);
            const HRESULT close_hr = sink->Close();
            sink->Release();
            if (SUCCEEDED(close_hr)) {
              target->DrawGeometry(path, solid_brush, op.width, stroke_style);
            }
          }
          path->Release();
        }
        return;
      }
      for (size_t i = 1u; i < op.points.size(); ++i) {
        const nve::DrawPoint a = op.points[i - 1u];
        const nve::DrawPoint b = op.points[i];
        target->DrawLine(
            D2D1::Point2F(a.x, a.y),
            D2D1::Point2F(b.x, b.y),
            solid_brush,
            op.width,
            stroke_style);
      }
    }

    void on_clip_rect(const nve::DrawClipRectOp& op) noexcept {
      const float cw = (op.w > 0.0f) ? op.w : 0.0f;
      const float ch = (op.h > 0.0f) ? op.h : 0.0f;
      target->PushAxisAlignedClip(
          D2D1::RectF(op.x, op.y, op.x + cw, op.y + ch), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
      ++clip_depth;
    }

    void on_restore_clip(const nve::DrawRestoreClipOp&) noexcept {
      if (clip_depth > 0u) {
        target->PopAxisAlignedClip();
        --clip_depth;
      }
    }
  };

  D2DDrawListVisitor visitor{rt, brush, factory, round_stroke, bw, bh, 0u};
  const nve::DrawListDecodeResult res = nve::decode_draw_list(bytes, visitor);
  while (visitor.clip_depth > 0u) {
    rt->PopAxisAlignedClip();
    --visitor.clip_depth;
  }
  rt->SetTransform(prev_transform);
  rt->PopAxisAlignedClip();
  if (round_stroke != nullptr) {
    round_stroke->Release();
  }
  return res;
}

HRESULT paint_visual_tree_d2d(
    ID2D1RenderTarget* rt,
    const NveWinVisualTree& tree,
    const DirectWriteTextMeasurer& measurer) {
  if (rt == nullptr) {
    return E_INVALIDARG;
  }

  ID2D1SolidColorBrush* brush = nullptr;
  HRESULT hr = rt->CreateSolidColorBrush(D2D1::ColorF(D2D1::ColorF::White), &brush);
  if (FAILED(hr) || brush == nullptr) {
    return hr;
  }

  rt->BeginDraw();
  rt->SetTransform(D2D1::Matrix3x2F::Identity());

  const nve::RgbaColor root_bg = nve::unpack_rgba(tree.root_background_rgba());
  if (root_bg.a8 > 0) {
    rt->Clear(to_d2d_color(tree.root_background_rgba()));
  } else {
    rt->Clear(D2D1::ColorF(0.06f, 0.09f, 0.16f, 1.0f));
  }

  std::vector<uint32_t> ordered_roots = tree.root_children();
  std::stable_sort(ordered_roots.begin(), ordered_roots.end(), [&tree](uint32_t a, uint32_t b) {
    const NveWinNodeVisual* va = tree.find(a);
    const NveWinNodeVisual* vb = tree.find(b);
    return (va ? va->z_index : 0) < (vb ? vb->z_index : 0);
  });

  for (uint32_t child_id : ordered_roots) {
    paint_node_recursive(rt, brush, tree, measurer, child_id, 0.0f, 0.0f, 0u);
  }

  hr = rt->EndDraw();
  brush->Release();
  return hr;
}

} // namespace nve::win

#endif // defined(_WIN32)
