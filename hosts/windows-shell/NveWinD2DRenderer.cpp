// Direct2D painting of the retained `NveWinVisualTree` (shadows, backgrounds, images, canvas draw
// lists, DirectWrite text, rounded clips, borders, opacity groups) and the flat draw-list renderer.
// Values arrive sealed (`read_node`, `decode_box_shadows`): nothing here re-validates them.
#include "NveWinD2DInternal.hpp"

#include <algorithm>
#include <cmath>
#include <vector>

namespace nve::win {

using d2d_internal::to_d2d;

namespace {

float corner_radius(const WinNodeVisual& n, const Rect& r) noexcept {
  return std::min(n.visual.border_radius, 0.5f * std::min(r.width, r.height));
}

/// One pushed `ID2D1Layer` (opacity group and/or geometric clip) with the geometry it owns.
struct Layer {
  ID2D1Layer* layer = nullptr;
  ID2D1Geometry* mask = nullptr;
  [[nodiscard]] bool pushed() const noexcept { return layer != nullptr; }
};

/// What `enter` pushed, handed back to `leave` (the walk keeps it on its own stack).
struct Pushed {
  Layer group;  // opacity
  Layer clip;   // rounded overflow clip
  bool axis_clip = false;
  explicit operator bool() const noexcept { return true; }  // always descend
};

/// Paints the visual tree in `NveWinVisualTree::walk` order; with a dirty rect, only nodes whose
/// painted area meets it draw (all are still traversed: children may overflow their parents).
class Painter {
 public:
  Painter(ID2D1RenderTarget* rt, ID2D1Factory* factory, ID2D1SolidColorBrush* brush, const DirectWriteTextMeasurer& text,
          D2DPaintCache& cache, const Rect* dirty) noexcept
      : rt_(rt), factory_(factory), brush_(brush), text_(text), cache_(cache), dirty_(dirty) {}

  Pushed enter(const WinNodeVisual& n, const PaintFrame& f) {
    Pushed p;
    const Rect& r = f.rect;
    if (n.visual.opacity < 0.999f) p.group = push_layer(nullptr, n.visual.opacity, nullptr);
    const float radius = corner_radius(n, r);
    if (f.visible && !r.empty() && (dirty_ == nullptr || dirty_->intersects(r.inflate(n.shadow_reach)))) {
      for (const host::BoxShadow& s : n.shadows) {
        if (!s.inset) outer_shadow(s, r, radius);
      }
      if ((n.visual.background >> 24u) != 0u) fill(r, radius, n.visual.background, 1.0f);
      for (const host::BoxShadow& s : n.shadows) {
        if (s.inset) inset_shadow(s, r, radius);
      }
      // Content draws in the content box (inside border and padding), clipped to the box.
      const Rect content{r.x + n.content.x, r.y + n.content.y, n.content.width, n.content.height};
      if (!n.image.empty() || (n.canvas && !n.draw_list.empty())) media(n, r, content, radius);
      if (n.kind == host::NodeKind::Text || n.kind == host::NodeKind::TextInput) text(n, content);
    }
    if (n.clips() && !r.empty()) {
      if (radius > 0.0f) {
        p.clip = push_layer(rounded(r, radius), 1.0f, &r);
      } else {
        rt_->PushAxisAlignedClip(to_d2d(r), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        p.axis_clip = true;
      }
    }
    return p;
  }

  void leave(const WinNodeVisual& n, const PaintFrame& f, Pushed& p) {
    if (p.axis_clip) rt_->PopAxisAlignedClip();
    pop_layer(p.clip);
    const Rect& r = f.rect;
    const float bw = n.visual.border_width;
    if (f.visible && bw > 0.0f && (n.visual.border_color >> 24u) != 0u && (dirty_ == nullptr || dirty_->intersects(r))) {
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
  ID2D1Geometry* rounded(const Rect& r, float radius) const {
    ID2D1RoundedRectangleGeometry* g = nullptr;
    if (factory_ == nullptr || FAILED(factory_->CreateRoundedRectangleGeometry(D2D1::RoundedRect(to_d2d(r), radius, radius), &g))) {
      return nullptr;
    }
    return g;
  }

  /// Pushes a layer (takes `mask`; `bounds` limits its surface, null = whole target). A failed
  /// layer creation releases `mask` and pushes nothing.
  Layer push_layer(ID2D1Geometry* mask, float opacity, const Rect* bounds) {
    Layer l;
    if (FAILED(rt_->CreateLayer(nullptr, &l.layer)) || l.layer == nullptr) {
      if (mask != nullptr) mask->Release();
      return Layer{};
    }
    l.mask = mask;
    rt_->PushLayer(D2D1::LayerParameters(bounds != nullptr ? to_d2d(*bounds) : D2D1::InfiniteRect(), mask,
                                         D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1::IdentityMatrix(), opacity, nullptr,
                                         D2D1_LAYER_OPTIONS_NONE),
                   l.layer);
    return l;
  }

  void pop_layer(Layer& l) {
    if (!l.pushed()) return;
    rt_->PopLayer();
    l.layer->Release();
    if (l.mask != nullptr) l.mask->Release();
    l = Layer{};
  }

  /// Clips to the node's rounded rect (or its plain rect); `false` when a rounded clip could not
  /// be built (nothing pushed: the caller must not draw unclipped).
  bool push_clip(const Rect& r, float radius, Layer& layer, bool& axis) {
    if (radius <= 0.0f) {
      rt_->PushAxisAlignedClip(to_d2d(r), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
      axis = true;
      return true;
    }
    ID2D1Geometry* mask = rounded(r, radius);
    if (mask == nullptr) return false;
    layer = push_layer(mask, 1.0f, &r);
    return layer.pushed();
  }

  void pop_clip(Layer& layer, bool axis) {
    if (axis) rt_->PopAxisAlignedClip();
    pop_layer(layer);
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
    hole.spread_radius = -s.spread_radius;
    const std::vector<ShadowPass> passes = outer_shadow_passes(hole, r, radius);
    if (passes.empty()) {  // the spread swallowed the hole: the whole box is shadow
      fill(r, radius, s.color, 1.0f);
      return;
    }
    Layer clip;
    bool axis = false;
    if (!push_clip(r, radius, clip, axis)) return;  // never paint the ring unclipped
    const Rect outside = r.inflate(std::fabs(s.offset_x) + std::fabs(s.offset_y) + std::fabs(s.spread_radius) + s.blur_radius + 1.0f);
    for (size_t k = 0; k < passes.size(); ++k) {
      const ShadowPass& h = passes[passes.size() - 1u - k];  // smallest hole covers the most
      ID2D1RectangleGeometry* outer = nullptr;
      ID2D1Geometry* inner = rounded(h.rect, h.radius);
      ID2D1PathGeometry* ring = nullptr;
      ID2D1GeometrySink* sink = nullptr;
      if (inner != nullptr && SUCCEEDED(factory_->CreateRectangleGeometry(to_d2d(outside), &outer)) &&
          SUCCEEDED(factory_->CreatePathGeometry(&ring)) && SUCCEEDED(ring->Open(&sink))) {
        const HRESULT combined = outer->CombineWithGeometry(inner, D2D1_COMBINE_MODE_EXCLUDE, nullptr, sink);
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
    pop_clip(clip, axis);
  }

  /// Image and canvas content placed in `content`, clipped to the node's (rounded) box `r`.
  void media(const WinNodeVisual& n, const Rect& r, const Rect& content, float radius) {
    Layer clip;
    bool axis = false;
    if (!push_clip(r, radius, clip, axis)) return;
    if (ID2D1Bitmap* bmp = n.image.empty() ? nullptr : cache_.bitmap(rt_, n.image)) {
      const D2D1_SIZE_F size = bmp->GetSize();
      if (const auto placed = place_image(n.image_fit, content, size.width, size.height)) {
        const D2D1_RECT_F src = to_d2d(placed->src);
        rt_->DrawBitmap(bmp, to_d2d(placed->dst), 1.0f, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, &src);
      }
    }
    if (n.canvas && !n.draw_list.empty()) {
      (void)render_draw_list_to_d2d(rt_, brush_, factory_, to_d2d(content), n.draw_list);
    }
    pop_clip(clip, axis);
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
      if (!n.input.multiline) {
        style.white_space = NVE_WHITE_SPACE_NO_WRAP;
      } else {
        style.overflow_wrap = NVE_OVERFLOW_WRAP_ANYWHERE;  // a text area breaks long words (`break-word`)
      }
    } else {
      shown = n.text;
    }
    if (shown.empty() || (style.color >> 24u) == 0u) return;
    IDWriteTextLayout* layout = cache_.layout(text_, n.id, shown, style, r.width, r.height);
    if (layout == nullptr) return;
    brush_->SetColor(to_d2d(style.color, alpha));
    // A line clamp or a clipping box ends at the layout box: lines DirectWrite laid out past
    // its max height must not paint (`CLIP` clips to the layout box).
    const D2D1_DRAW_TEXT_OPTIONS options =
        style.max_lines > 0u || n.clips() ? D2D1_DRAW_TEXT_OPTIONS_CLIP : D2D1_DRAW_TEXT_OPTIONS_NONE;
    rt_->DrawTextLayout(D2D1::Point2F(r.x, r.y), layout, brush_, options);
  }

  ID2D1RenderTarget* rt_;
  ID2D1Factory* factory_;
  ID2D1SolidColorBrush* brush_;
  const DirectWriteTextMeasurer& text_;
  D2DPaintCache& cache_;
  const Rect* dirty_;
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

    void color(DrawColorRgba c) const {
      brush->SetColor(D2D1::ColorF(c.r / 255.0f, c.g / 255.0f, c.b / 255.0f, c.a / 255.0f));
    }
    void on_clear(const DrawClearOp& op) {
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
    }
    void on_restore_clip(const DrawRestoreClipOp&) {
      rt->PopAxisAlignedClip();
    }
  } visitor{rt, brush, factory, round, bw, bh};

  (void)stream_validated_draw_list(bytes, visitor);
  rt->SetTransform(prev);
  rt->PopAxisAlignedClip();
  if (round != nullptr) round->Release();
  return check;
}

HRESULT paint_visual_tree_d2d(ID2D1RenderTarget* rt, ID2D1Factory* factory, const NveWinVisualTree& tree,
                              const DirectWriteTextMeasurer& text, D2DPaintCache& cache, const Rect* dirty) {
  if (rt == nullptr) return E_INVALIDARG;
  ID2D1SolidColorBrush* brush = nullptr;
  HRESULT hr = rt->CreateSolidColorBrush(D2D1::ColorF(D2D1::ColorF::Black), &brush);
  if (FAILED(hr)) return hr;
  rt->BeginDraw();
  rt->SetTransform(D2D1::Matrix3x2F::Identity());
  if (dirty != nullptr) rt->PushAxisAlignedClip(to_d2d(*dirty), D2D1_ANTIALIAS_MODE_ALIASED);
  const uint32_t bg = tree.root_background();
  if ((bg >> 24u) != 0u) {
    rt->Clear(to_d2d(bg));  // (Clear honours the axis-aligned clip)
  } else {  // a transparent root shows the system window colour
    const COLORREF c = ::GetSysColor(COLOR_WINDOW);
    rt->Clear(D2D1::ColorF(GetRValue(c) / 255.0f, GetGValue(c) / 255.0f, GetBValue(c) / 255.0f, 1.0f));
  }
  Painter painter(rt, factory, brush, text, cache, dirty);
  tree.walk(painter);
  if (dirty != nullptr) rt->PopAxisAlignedClip();
  hr = rt->EndDraw();
  brush->Release();
  cache.prune(tree);
  return hr;
}

}  // namespace nve::win
