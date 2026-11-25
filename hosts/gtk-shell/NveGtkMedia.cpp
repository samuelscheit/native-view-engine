#include "NveGtkInternal.hpp"

#include <algorithm>
#include <cmath>
#include <string>
#include <utility>

namespace nve {

namespace {

#if GTK_CHECK_VERSION(4, 8, 0)
GtkContentFit parse_gtk_content_fit(std::string_view mode) noexcept {
  if (mode == "contain") {
    return GTK_CONTENT_FIT_CONTAIN;
  }
  if (mode == "stretch" || mode == "fill") {
    return GTK_CONTENT_FIT_FILL;
  }
  if (mode == "center") {
    return GTK_CONTENT_FIT_SCALE_DOWN;
  }
  return GTK_CONTENT_FIT_COVER;
}
#endif

GdkTexture* decode_uri_to_gdk_texture(std::string_view uri) {
  if (uri.empty()) {
    return nullptr;
  }

  GError* err = nullptr;
  GdkTexture* tex = nullptr;

  if (uri.size() > 11u && gtk_internal::ascii_lower(uri.substr(0, 11)) == "data:image/") {
    const size_t marker = uri.find(";base64,");
    if (marker == std::string_view::npos) {
      return nullptr;
    }
    const std::string b64(uri.substr(marker + 8u));
    if (b64.empty()) {
      return nullptr;
    }
    gsize decoded_len = 0;
    guchar* decoded = g_base64_decode(b64.c_str(), &decoded_len);
    if (decoded == nullptr || decoded_len == 0) {
      g_free(decoded);
      return nullptr;
    }
    GBytes* bytes = g_bytes_new_take(decoded, decoded_len);
    tex = gdk_texture_new_from_bytes(bytes, &err);
    g_bytes_unref(bytes);
  } else if (uri.size() > 7u && gtk_internal::ascii_lower(uri.substr(0, 7)) == "file://") {
    const std::string uri_str(uri);
    gchar* filename = g_filename_from_uri(uri_str.c_str(), nullptr, &err);
    if (err != nullptr) {
      g_clear_error(&err);
    }
    if (filename != nullptr) {
      tex = gdk_texture_new_from_filename(filename, &err);
      g_free(filename);
    } else {
      const std::string fallback_path(uri.substr(7u));
      tex = gdk_texture_new_from_filename(fallback_path.c_str(), &err);
    }
  }

  if (err != nullptr) {
    g_clear_error(&err);
  }
  return tex;
}

} // namespace

// -----------------------------------------------------------------------------
// NVE_DRAW1 Cairo renderer
// -----------------------------------------------------------------------------

DrawListDecodeResult render_draw_list_to_cairo(
    cairo_t* cr,
    double width,
    double height,
    std::span<const uint8_t> bytes) noexcept {
  if (cr == nullptr || bytes.empty()) {
    return DrawListDecodeResult{DrawListError::Ok, 0u, 0u};
  }
  const DrawListDecodeResult check = validate_draw_list(bytes);
  if (!check.ok()) {
    return check;
  }

  const double bw = std::max(0.0, std::isfinite(width) ? width : 0.0);
  const double bh = std::max(0.0, std::isfinite(height) ? height : 0.0);
  if (bw <= 0.0 || bh <= 0.0) {
    return DrawListDecodeResult{DrawListError::Ok, 0u, bytes.size()};
  }

  cairo_save(cr);
  cairo_set_antialias(cr, CAIRO_ANTIALIAS_DEFAULT);
  cairo_rectangle(cr, 0.0, 0.0, bw, bh);
  cairo_clip(cr);

  struct CairoDrawVisitor {
    cairo_t* ctx;
    double w;
    double h;
    uint32_t clip_depth{0u};

    static void set_color(cairo_t* c, DrawColorRgba col) noexcept {
      cairo_set_source_rgba(
          c,
          static_cast<double>(col.r) / 255.0,
          static_cast<double>(col.g) / 255.0,
          static_cast<double>(col.b) / 255.0,
          static_cast<double>(col.a) / 255.0);
    }

    void on_clear(const DrawClearOp& op) noexcept {
      cairo_save(ctx);
      cairo_set_operator(ctx, CAIRO_OPERATOR_SOURCE);
      set_color(ctx, op.color);
      cairo_rectangle(ctx, 0.0, 0.0, w, h);
      cairo_fill(ctx);
      cairo_restore(ctx);
    }

    void on_fill_rect(const DrawFillRectOp& op) noexcept {
      if (op.w <= 0.0f || op.h <= 0.0f || op.color.a == 0u) {
        return;
      }
      set_color(ctx, op.color);
      cairo_rectangle(
          ctx,
          static_cast<double>(op.x),
          static_cast<double>(op.y),
          static_cast<double>(op.w),
          static_cast<double>(op.h));
      cairo_fill(ctx);
    }

    void on_fill_rounded_rect(const DrawFillRoundedRectOp& op) noexcept {
      if (op.w <= 0.0f || op.h <= 0.0f || op.color.a == 0u) {
        return;
      }
      const double rx = static_cast<double>(op.x);
      const double ry = static_cast<double>(op.y);
      const double rw = static_cast<double>(op.w);
      const double rh = static_cast<double>(op.h);
      const double r = std::clamp(static_cast<double>(op.radius), 0.0, std::min(rw, rh) * 0.5);
      set_color(ctx, op.color);
      if (r <= 0.0) {
        cairo_rectangle(ctx, rx, ry, rw, rh);
        cairo_fill(ctx);
        return;
      }
      constexpr double kHalfPi = 1.57079632679489661923;
      constexpr double kPi = 3.14159265358979323846;
      cairo_new_sub_path(ctx);
      cairo_arc(ctx, rx + rw - r, ry + r, r, -kHalfPi, 0.0);
      cairo_arc(ctx, rx + rw - r, ry + rh - r, r, 0.0, kHalfPi);
      cairo_arc(ctx, rx + r, ry + rh - r, r, kHalfPi, kPi);
      cairo_arc(ctx, rx + r, ry + r, r, kPi, kPi + kHalfPi);
      cairo_close_path(ctx);
      cairo_fill(ctx);
    }

    void on_fill_circle(const DrawFillCircleOp& op) noexcept {
      if (op.radius <= 0.0f || op.color.a == 0u) {
        return;
      }
      constexpr double kTwoPi = 6.28318530717958647692;
      set_color(ctx, op.color);
      cairo_new_sub_path(ctx);
      cairo_arc(
          ctx,
          static_cast<double>(op.cx),
          static_cast<double>(op.cy),
          static_cast<double>(op.radius),
          0.0,
          kTwoPi);
      cairo_close_path(ctx);
      cairo_fill(ctx);
    }

    void on_stroke_line(const DrawStrokeLineOp& op) noexcept {
      if (op.width <= 0.0f || op.color.a == 0u) {
        return;
      }
      set_color(ctx, op.color);
      cairo_set_line_width(ctx, static_cast<double>(op.width));
      cairo_set_line_cap(ctx, CAIRO_LINE_CAP_ROUND);
      cairo_set_line_join(ctx, CAIRO_LINE_JOIN_ROUND);
      cairo_new_path(ctx);
      cairo_move_to(ctx, static_cast<double>(op.x0), static_cast<double>(op.y0));
      cairo_line_to(ctx, static_cast<double>(op.x1), static_cast<double>(op.y1));
      cairo_stroke(ctx);
    }

    void on_stroke_polyline(const DrawStrokePolylineOp& op) noexcept {
      if (op.points.size() < 2u || op.width <= 0.0f || op.color.a == 0u) {
        return;
      }
      set_color(ctx, op.color);
      cairo_set_line_width(ctx, static_cast<double>(op.width));
      cairo_set_line_cap(ctx, CAIRO_LINE_CAP_ROUND);
      cairo_set_line_join(ctx, CAIRO_LINE_JOIN_ROUND);
      cairo_new_path(ctx);
      const DrawPoint p0 = op.points[0u];
      cairo_move_to(ctx, static_cast<double>(p0.x), static_cast<double>(p0.y));
      for (size_t i = 1u; i < op.points.size(); ++i) {
        const DrawPoint pt = op.points[i];
        cairo_line_to(ctx, static_cast<double>(pt.x), static_cast<double>(pt.y));
      }
      cairo_stroke(ctx);
    }

    void on_clip_rect(const DrawClipRectOp& op) noexcept {
      cairo_save(ctx);
      ++clip_depth;
      const double cw = (op.w > 0.0f) ? static_cast<double>(op.w) : 0.0;
      const double ch = (op.h > 0.0f) ? static_cast<double>(op.h) : 0.0;
      cairo_rectangle(ctx, static_cast<double>(op.x), static_cast<double>(op.y), cw, ch);
      cairo_clip(ctx);
    }

    void on_restore_clip(const DrawRestoreClipOp&) noexcept {
      if (clip_depth > 0u) {
        cairo_restore(ctx);
        --clip_depth;
      }
    }
  };

  CairoDrawVisitor visitor{cr, bw, bh, 0u};
  const DrawListDecodeResult res = decode_draw_list(bytes, visitor);
  while (visitor.clip_depth > 0u) {
    cairo_restore(cr);
    --visitor.clip_depth;
  }
  cairo_restore(cr);
  return res;
}

// -----------------------------------------------------------------------------
// GtkTextureLruCache
// -----------------------------------------------------------------------------

GtkTextureLruCache::GtkTextureLruCache(size_t max_entries)
    : max_entries_(std::max<size_t>(1u, max_entries)) {}

GtkTextureLruCache::~GtkTextureLruCache() {
  clear();
}

GdkTexture* GtkTextureLruCache::get(std::string_view uri) {
  if (uri.empty()) {
    return nullptr;
  }
  const std::string key(uri);
  auto it = entries_.find(key);
  if (it != entries_.end()) {
    lru_.splice(lru_.begin(), lru_, it->second.lru_it);
    it->second.lru_it = lru_.begin();
    ++hits_;
    return it->second.texture;
  }

  ++misses_;
  GdkTexture* decoded = decode_uri_to_gdk_texture(uri);
  if (decoded == nullptr) {
    return nullptr;
  }

  while (entries_.size() >= max_entries_ && !lru_.empty()) {
    const std::string& victim = lru_.back();
    auto vit = entries_.find(victim);
    if (vit != entries_.end()) {
      if (vit->second.texture != nullptr) {
        g_object_unref(vit->second.texture);
      }
      entries_.erase(vit);
    }
    lru_.pop_back();
  }

  lru_.push_front(key);
  Entry entry{};
  entry.texture = decoded;
  entry.lru_it = lru_.begin();
  entries_.emplace(key, std::move(entry));
  return decoded;
}

void GtkTextureLruCache::clear() {
  for (auto& [_, entry] : entries_) {
    if (entry.texture != nullptr) {
      g_object_unref(entry.texture);
      entry.texture = nullptr;
    }
  }
  entries_.clear();
  lru_.clear();
}

// -----------------------------------------------------------------------------
// NveGtkHost GtkPicture & GtkDrawingArea helpers
// -----------------------------------------------------------------------------

std::span<const uint8_t> NveGtkHost::canvas_draw_list_for_node_id(uint32_t node_id) const noexcept {
  const auto it = canvas_bindings_.find(node_id);
  if (it == canvas_bindings_.end() || it->second == nullptr) {
    return {};
  }
  return it->second->draw_list_bytes;
}

uint64_t NveGtkHost::canvas_redraw_count_for_node_id(uint32_t node_id) const noexcept {
  const auto it = canvas_bindings_.find(node_id);
  if (it == canvas_bindings_.end() || it->second == nullptr) {
    return 0u;
  }
  return it->second->redraw_count;
}

void NveGtkHost::apply_picture_fit(GtkWidget* picture, const std::string& resize_mode) {
  if (picture == nullptr || !GTK_IS_PICTURE(picture)) {
    return;
  }
#if GTK_CHECK_VERSION(4, 8, 0)
  gtk_picture_set_content_fit(GTK_PICTURE(picture), parse_gtk_content_fit(resize_mode));
#else
  (void)resize_mode;
#endif
}

NveGtkHost::CanvasBinding* NveGtkHost::ensure_canvas_binding(uint32_t node_id, GtkWidget* widget) {
  if (widget == nullptr || !GTK_IS_DRAWING_AREA(widget)) {
    return nullptr;
  }
  const auto it = canvas_bindings_.find(node_id);
  if (it != canvas_bindings_.end() && it->second != nullptr && it->second->drawing_area == widget) {
    return it->second.get();
  }
  remove_canvas_binding(node_id, widget);

  auto binding = std::make_unique<CanvasBinding>();
  binding->host = this;
  binding->node_id = node_id;
  binding->drawing_area = widget;
  gtk_drawing_area_set_draw_func(
      GTK_DRAWING_AREA(widget), &NveGtkHost::on_canvas_draw, binding.get(), nullptr);

  CanvasBinding* raw = binding.get();
  canvas_bindings_[node_id] = std::move(binding);
  return raw;
}

void NveGtkHost::remove_canvas_binding(uint32_t node_id, GtkWidget* widget) {
  const auto it = canvas_bindings_.find(node_id);
  if (it == canvas_bindings_.end()) {
    return;
  }
  if (it->second != nullptr) {
    GtkWidget* target = (widget != nullptr) ? widget : it->second->drawing_area;
    if (target != nullptr && GTK_IS_DRAWING_AREA(target)) {
      gtk_drawing_area_set_draw_func(GTK_DRAWING_AREA(target), nullptr, nullptr, nullptr);
    }
  }
  canvas_bindings_.erase(it);
}

void NveGtkHost::on_canvas_draw(
    GtkDrawingArea* area,
    cairo_t* cr,
    int width,
    int height,
    gpointer user_data) {
  (void)area;
  auto* binding = static_cast<CanvasBinding*>(user_data);
  if (binding == nullptr || cr == nullptr || binding->draw_list_bytes.empty()) {
    return;
  }
  (void)render_draw_list_to_cairo(
      cr,
      static_cast<double>(width),
      static_cast<double>(height),
      binding->draw_list_bytes);
}

} // namespace nve
