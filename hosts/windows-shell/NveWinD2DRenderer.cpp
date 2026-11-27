// Direct2D painting of the retained `NveWinVisualTree` (shadows, backgrounds, images, canvas draw
// lists, DirectWrite text, rounded clips, borders, opacity groups) and the `NVE_DRAW1` renderer.
#include "NveWinD2DInternal.hpp"

#include <algorithm>
#include <cmath>
#include <vector>

namespace nve::win {

using d2d_internal::to_d2d;

namespace {

float corner_radius(const WinNodeVisual& n, const Rect& r) noexcept {
  const float v = n.visual.border_radius;
  return std::isfinite(v) ? std::clamp(v, 0.0f, 0.5f * std::min(r.width, r.height)) : 0.0f;
}

/// One pushed `ID2D1Layer` (opacity group and/or geometric clip) with the geometry it owns.
struct Layer {
  ID2D1Layer* layer = nullptr;
  ID2D1Geometry* mask = nullptr;
};

/// Paints the visual tree in `NveWinVisualTree::walk` order.
class Painter {
 public:
  Painter(ID2D1RenderTarget* rt, ID2D1Factory* factory, ID2D1SolidColorBrush* brush,
          const DirectWriteTextMeasurer& text) noexcept
      : rt_(rt), factory_(factory), brush_(brush), text_(text) {}

  bool enter(const WinNodeVisual& n, const PaintFrame& f) {
    Pushed p;
    const float own = std::isfinite(n.visual.opacity) ? std::clamp(n.visual.opacity, 0.0f, 1.0f) : 1.0f;
    if (own < 0.999f) p.group = push_layer(nullptr, own);
    const Rect& r = f.rect;
    const float radius = corner_radius(n, r);
    if (f.visible && !r.empty()) {
      for (const host::BoxShadow& s : n.shadows) {
        if (!s.inset) outer_shadow(s, r, radius);
      }
      if ((n.visual.background >> 24u) != 0u) fill(r, radius, n.visual.background, 1.0f);
      for (const host::BoxShadow& s : n.shadows) {
        if (s.inset) inset_shadow(s, r, radius);
      }
      if (!n.image.empty()) image(n, r);
      if (n.canvas && !n.draw_list.empty()) (void)render_draw_list_to_d2d(rt_, brush_, factory_, to_d2d(r), n.draw_list);
      text(n, r);
    }
    if (n.clips() && !r.empty()) {
      if (radius > 0.0f) {
        p.clip = push_layer(rounded(r, radius), 1.0f);
      } else {
        rt_->PushAxisAlignedClip(to_d2d(r), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        p.axis_clip = true;
      }
    }
    stack_.push_back(p);
    return true;
  }

  void leave(const WinNodeVisual& n, const PaintFrame& f) {
    Pushed p = stack_.back();
    stack_.pop_back();
    if (p.axis_clip) rt_->PopAxisAlignedClip();
    pop_layer(p.clip);
    const Rect& r = f.rect;
    const float bw = n.visual.border_width;
    if (f.visible && !r.empty() && std::isfinite(bw) && bw > 0.0f && (n.visual.border_color >> 24u) != 0u) {
      const float half = 0.5f * bw;
      const Rect inset{r.x + half, r.y + half, r.width - bw, r.height - bw};
      if (!inset.empty()) {
        brush_->SetColor(to_d2d(n.visual.border_color));
        const float rr = std::max(0.0f, corner_radius(n, r) - half);
        if (rr > 0.0f) {
          rt_->DrawRoundedRectangle(D2D1::RoundedRect(to_d2d(inset), rr, rr), brush_, bw);
        } else {
          rt_->DrawRectangle(to_d2d(inset), brush_, bw);
        }
      }
    }
    pop_layer(p.group);
  }

 private:
  struct Pushed {
    Layer group;  // opacity
    Layer clip;   // rounded overflow clip
    bool axis_clip = false;
  };

  ID2D1Geometry* rounded(const Rect& r, float radius) const {
    ID2D1RoundedRectangleGeometry* g = nullptr;
    if (factory_ == nullptr || FAILED(factory_->CreateRoundedRectangleGeometry(D2D1::RoundedRect(to_d2d(r), radius, radius), &g))) {
      return nullptr;
    }
    return g;
  }

  /// Pushes a layer (takes `mask`); a failed push releases it and pushes nothing.
  Layer push_layer(ID2D1Geometry* mask, float opacity) {
    Layer l;
    if (FAILED(rt_->CreateLayer(nullptr, &l.layer)) || l.layer == nullptr) {
      if (mask != nullptr) mask->Release();
      return Layer{};
    }
    l.mask = mask;
    rt_->PushLayer(D2D1::LayerParameters(D2D1::InfiniteRect(), mask, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                                         D2D1::IdentityMatrix(), opacity, nullptr, D2D1_LAYER_OPTIONS_NONE),
                   l.layer);
    return l;
  }

  void pop_layer(Layer& l) {
    if (l.layer == nullptr) return;
    rt_->PopLayer();
    l.layer->Release();
    if (l.mask != nullptr) l.mask->Release();
    l = Layer{};
  }

  void fill(const Rect& r, float radius, uint32_t rgba, float alpha) {
    brush_->SetColor(to_d2d(rgba, alpha));
    if (radius > 0.0f) {
      rt_->FillRoundedRectangle(D2D1::RoundedRect(to_d2d(r), radius, radius), brush_);
    } else {
      rt_->FillRectangle(to_d2d(r), brush_);
    }
  }

  void outer_shadow(const host::BoxShadow& s, const Rect& r, float radius) {
    for (const ShadowPass& pass : outer_shadow_passes(s, r, radius)) fill(pass.rect, pass.radius, s.color, pass.alpha);
  }

  /// Inside the (rounded) box, everything outside the offset / shrunk "hole" in the shadow
  /// colour; the blur reuses the outer passes' coverage ramp with the regions complemented.
  void inset_shadow(const host::BoxShadow& s, const Rect& r, float radius) {
    if ((s.color >> 24u) == 0u || factory_ == nullptr) return;
    host::BoxShadow hole = s;
    hole.inset = false;
    hole.spread_radius = -(std::isfinite(s.spread_radius) ? s.spread_radius : 0.0f);
    const std::vector<ShadowPass> passes = outer_shadow_passes(hole, r, radius);
    if (passes.empty()) {  // the spread swallowed the hole: the whole box is shadow
      fill(r, radius, s.color, 1.0f);
      return;
    }
    Layer clip = push_layer(rounded(r, radius), 1.0f);
    const Rect outside = r.inflate(std::fabs(s.offset_x) + std::fabs(s.offset_y) + std::fabs(s.spread_radius) +
                                   std::fabs(s.blur_radius) + 1.0f);
    for (size_t k = 0; k < passes.size(); ++k) {
      const ShadowPass& h = passes[passes.size() - 1u - k];  // smallest hole covers the most
      ID2D1RectangleGeometry* outer = nullptr;
      ID2D1Geometry* inner = h.rect.empty() ? nullptr : rounded(h.rect, h.radius);
      ID2D1PathGeometry* ring = nullptr;
      ID2D1GeometrySink* sink = nullptr;
      if (SUCCEEDED(factory_->CreateRectangleGeometry(to_d2d(outside), &outer)) &&
          SUCCEEDED(factory_->CreatePathGeometry(&ring)) && SUCCEEDED(ring->Open(&sink))) {
        const HRESULT combined = inner != nullptr
                                     ? outer->CombineWithGeometry(inner, D2D1_COMBINE_MODE_EXCLUDE, nullptr, sink)
                                     : outer->Simplify(D2D1_GEOMETRY_SIMPLIFICATION_OPTION_LINES, nullptr, sink);
        if (SUCCEEDED(sink->Close()) && SUCCEEDED(combined)) {
          brush_->SetColor(to_d2d(s.color, passes[k].alpha));
          rt_->FillGeometry(ring, brush_);
        }
      }
      for (IUnknown* u : {static_cast<IUnknown*>(sink), static_cast<IUnknown*>(ring), static_cast<IUnknown*>(inner),
                          static_cast<IUnknown*>(outer)}) {
        if (u != nullptr) u->Release();
      }
    }
    pop_layer(clip);
  }

  void image(const WinNodeVisual& n, const Rect& r) {
    ID2D1Bitmap* bmp = d2d_internal::bitmap_for(rt_, n.image);
    if (bmp == nullptr) return;
    const D2D1_SIZE_F size = bmp->GetSize();
    const auto placed = place_image(n.image_fit, r, size.width, size.height);
    if (!placed) return;
    rt_->PushAxisAlignedClip(to_d2d(r), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
    const D2D1_RECT_F src = to_d2d(placed->src);
    rt_->DrawBitmap(bmp, to_d2d(placed->dst), 1.0f, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, &src);
    rt_->PopAxisAlignedClip();
  }

  void text(const WinNodeVisual& n, const Rect& r) {
    std::string shown;
    float alpha = 1.0f;
    TextPaint style = n.text_style;
    if (n.kind == host::NodeKind::TextInput) {  // the live window overlays an EDIT; screenshots see this
      if (n.input.value.empty()) {
        shown = n.input.placeholder;
        alpha = 0.5f;
      } else if (n.input.secure) {
        for (const char c : n.input.value) {
          if ((static_cast<unsigned char>(c) & 0xC0u) != 0x80u) shown += "\xE2\x80\xA2";  // one bullet per code point
        }
      } else {
        shown = n.input.value;
      }
      if (!n.input.multiline) style.white_space = NVE_WHITE_SPACE_NO_WRAP;
    } else {
      shown = n.text;
    }
    if (shown.empty() || (style.color >> 24u) == 0u) return;
    IDWriteTextLayout* layout = text_.create_layout(shown, style, r.width, r.height);
    if (layout == nullptr) return;
    brush_->SetColor(to_d2d(style.color, alpha));
    rt_->DrawTextLayout(D2D1::Point2F(r.x, r.y), layout, brush_, D2D1_DRAW_TEXT_OPTIONS_NONE);
    layout->Release();
  }

  ID2D1RenderTarget* rt_;
  ID2D1Factory* factory_;
  ID2D1SolidColorBrush* brush_;
  const DirectWriteTextMeasurer& text_;
  std::vector<Pushed> stack_;
};

}  // namespace

canvas::DrawListDecodeResult render_draw_list_to_d2d(ID2D1RenderTarget* rt, ID2D1SolidColorBrush* brush,
                                                     ID2D1Factory* factory, const D2D1_RECT_F& bounds,
                                                     std::span<const uint8_t> bytes) noexcept {
  using namespace nve::canvas;
  if (rt == nullptr || brush == nullptr || bytes.empty()) return DrawListDecodeResult{DrawListError::Ok, 0u, 0u};
  const DrawListDecodeResult check = validate_draw_list(bytes);
  if (!check.ok()) return check;
  const float bw = bounds.right - bounds.left;
  const float bh = bounds.bottom - bounds.top;
  if (!(bw > 0.0f && bh > 0.0f)) return DrawListDecodeResult{DrawListError::Ok, 0u, bytes.size()};

  ID2D1StrokeStyle* round = nullptr;
  if (factory != nullptr) {
    const D2D1_STROKE_STYLE_PROPERTIES props =
        D2D1::StrokeStyleProperties(D2D1_CAP_STYLE_ROUND, D2D1_CAP_STYLE_ROUND, D2D1_CAP_STYLE_ROUND, D2D1_LINE_JOIN_ROUND);
    (void)factory->CreateStrokeStyle(&props, nullptr, 0u, &round);
  }
  D2D1::Matrix3x2F prev = D2D1::Matrix3x2F::Identity();
  rt->GetTransform(&prev);
  rt->PushAxisAlignedClip(bounds, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
  rt->SetTransform(D2D1::Matrix3x2F::Translation(bounds.left, bounds.top) * prev);

  struct Visitor {
    ID2D1RenderTarget* rt;
    ID2D1SolidColorBrush* brush;
    ID2D1Factory* factory;
    ID2D1StrokeStyle* stroke;
    float w, h;
    uint32_t clip_depth = 0u;

    void color(DrawColorRgba c) const {
      brush->SetColor(D2D1::ColorF(c.r / 255.0f, c.g / 255.0f, c.b / 255.0f, c.a / 255.0f));
    }
    void on_clear(const DrawClearOp& op) {
      // Clear replaces the canvas pixels: draw the colour over the canvas box (no blending under it).
      color(op.color);
      rt->FillRectangle(D2D1::RectF(0.0f, 0.0f, w, h), brush);
    }
    void on_fill_rect(const DrawFillRectOp& op) {
      if (op.w <= 0.0f || op.h <= 0.0f || op.color.a == 0u) return;
      color(op.color);
      rt->FillRectangle(D2D1::RectF(op.x, op.y, op.x + op.w, op.y + op.h), brush);
    }
    void on_fill_rounded_rect(const DrawFillRoundedRectOp& op) {
      if (op.w <= 0.0f || op.h <= 0.0f || op.color.a == 0u) return;
      const float r = std::clamp(op.radius, 0.0f, 0.5f * std::min(op.w, op.h));
      color(op.color);
      rt->FillRoundedRectangle(D2D1::RoundedRect(D2D1::RectF(op.x, op.y, op.x + op.w, op.y + op.h), r, r), brush);
    }
    void on_fill_circle(const DrawFillCircleOp& op) {
      if (op.radius <= 0.0f || op.color.a == 0u) return;
      color(op.color);
      rt->FillEllipse(D2D1::Ellipse(D2D1::Point2F(op.cx, op.cy), op.radius, op.radius), brush);
    }
    void on_stroke_line(const DrawStrokeLineOp& op) {
      if (op.width <= 0.0f || op.color.a == 0u) return;
      color(op.color);
      rt->DrawLine(D2D1::Point2F(op.x0, op.y0), D2D1::Point2F(op.x1, op.y1), brush, op.width, stroke);
    }
    void on_stroke_polyline(const DrawStrokePolylineOp& op) {
      if (op.points.size() < 2u || op.width <= 0.0f || op.color.a == 0u) return;
      color(op.color);
      ID2D1PathGeometry* path = nullptr;
      ID2D1GeometrySink* sink = nullptr;
      if (factory != nullptr && SUCCEEDED(factory->CreatePathGeometry(&path)) && SUCCEEDED(path->Open(&sink))) {
        sink->BeginFigure(D2D1::Point2F(op.points[0].x, op.points[0].y), D2D1_FIGURE_BEGIN_HOLLOW);
        for (size_t i = 1u; i < op.points.size(); ++i) sink->AddLine(D2D1::Point2F(op.points[i].x, op.points[i].y));
        sink->EndFigure(D2D1_FIGURE_END_OPEN);
        if (SUCCEEDED(sink->Close())) rt->DrawGeometry(path, brush, op.width, stroke);
      } else {
        for (size_t i = 1u; i < op.points.size(); ++i) {
          rt->DrawLine(D2D1::Point2F(op.points[i - 1u].x, op.points[i - 1u].y),
                       D2D1::Point2F(op.points[i].x, op.points[i].y), brush, op.width, stroke);
        }
      }
      if (sink != nullptr) sink->Release();
      if (path != nullptr) path->Release();
    }
    void on_clip_rect(const DrawClipRectOp& op) {
      rt->PushAxisAlignedClip(D2D1::RectF(op.x, op.y, op.x + std::max(0.0f, op.w), op.y + std::max(0.0f, op.h)),
                              D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
      ++clip_depth;
    }
    void on_restore_clip(const DrawRestoreClipOp&) {
      if (clip_depth == 0u) return;
      rt->PopAxisAlignedClip();
      --clip_depth;
    }
  } visitor{rt, brush, factory, round, bw, bh};

  const DrawListDecodeResult res = decode_draw_list(bytes, visitor);
  for (; visitor.clip_depth > 0u; --visitor.clip_depth) rt->PopAxisAlignedClip();  // unbalanced streams
  rt->SetTransform(prev);
  rt->PopAxisAlignedClip();
  if (round != nullptr) round->Release();
  return res;
}

HRESULT paint_visual_tree_d2d(ID2D1RenderTarget* rt, ID2D1Factory* factory, const NveWinVisualTree& tree,
                              const DirectWriteTextMeasurer& text) {
  if (rt == nullptr) return E_INVALIDARG;
  ID2D1SolidColorBrush* brush = nullptr;
  HRESULT hr = rt->CreateSolidColorBrush(D2D1::ColorF(D2D1::ColorF::Black), &brush);
  if (FAILED(hr)) return hr;
  rt->BeginDraw();
  rt->SetTransform(D2D1::Matrix3x2F::Identity());
  const uint32_t bg = tree.root_background();
  if ((bg >> 24u) != 0u) {
    rt->Clear(to_d2d(bg));
  } else {  // a transparent root shows the system window colour
    const COLORREF c = ::GetSysColor(COLOR_WINDOW);
    rt->Clear(D2D1::ColorF(GetRValue(c) / 255.0f, GetGValue(c) / 255.0f, GetBValue(c) / 255.0f, 1.0f));
  }
  Painter painter(rt, factory, brush, text);
  tree.walk(painter);
  hr = rt->EndDraw();
  brush->Release();
  return hr;
}

}  // namespace nve::win
