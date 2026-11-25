#include "NveWinHostWindow.hpp"

#if defined(_WIN32)

#include <algorithm>
#include <cctype>
#include <cmath>
#include <cstring>
#include <fstream>
#include <list>
#include <string>
#include <unordered_map>
#include <vector>
#include <wincodec.h>
#include <windowsx.h>

namespace nve::win {

namespace {

constexpr wchar_t kWindowClassName[] = L"NveWinHostWindowClass";

D2D1_COLOR_F to_d2d_color(uint32_t rgba) noexcept {
  const nve::RgbaColor c = nve::unpack_rgba(rgba);
  return D2D1::ColorF(c.r, c.g, c.b, c.a);
}

std::string ascii_lower(std::string_view sv) {
  std::string out;
  out.reserve(sv.size());
  for (unsigned char ch : sv) {
    out.push_back(static_cast<char>(std::tolower(ch)));
  }
  return out;
}

bool decode_base64(std::string_view input, std::vector<uint8_t>& out) {
  out.clear();
  static constexpr int8_t kTable[256] = {
      -1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,
      -1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,62,-1,-1,-1,63,52,53,54,55,56,57,58,59,60,61,-1,-1,-1,-2,-1,-1,
      -1, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,-1,-1,-1,-1,-1,
      -1,26,27,28,29,30,31,32,33,34,35,36,37,38,39,40,41,42,43,44,45,46,47,48,49,50,51,-1,-1,-1,-1,-1,
      -1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,
      -1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,
      -1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,
      -1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,
  };
  out.reserve((input.size() * 3u) / 4u);
  uint32_t acc = 0u;
  int bits = 0;
  for (unsigned char ch : input) {
    if (std::isspace(ch)) continue;
    const int8_t val = kTable[ch];
    if (val == -2) break;
    if (val < 0) return false;
    acc = (acc << 6u) | static_cast<uint32_t>(val);
    bits += 6;
    if (bits >= 8) {
      bits -= 8;
      out.push_back(static_cast<uint8_t>((acc >> bits) & 0xFFu));
    }
  }
  return !out.empty();
}

/// Bounded-LRU WIC -> `ID2D1Bitmap` cache keyed by `(render_target, uri)`.
class WinD2DBitmapCache {
 public:
  explicit WinD2DBitmapCache(size_t max_entries = 64u)
      : max_entries_(std::max<size_t>(1u, max_entries)) {}

  ~WinD2DBitmapCache() {
    clear_bitmaps();
    if (wic_factory_ != nullptr) {
      wic_factory_->Release();
      wic_factory_ = nullptr;
    }
  }

  static WinD2DBitmapCache& shared() {
    static WinD2DBitmapCache instance(64u);
    return instance;
  }

  void clear_for_target(ID2D1RenderTarget* rt) {
    if (rt == nullptr) {
      return;
    }
    if (bound_rt_ == rt) {
      clear_bitmaps();
      bound_rt_ = nullptr;
    }
  }

  ID2D1Bitmap* get_or_decode(ID2D1RenderTarget* rt, std::string_view uri) {
    if (rt == nullptr || uri.empty()) {
      return nullptr;
    }
    if (bound_rt_ != rt) {
      clear_bitmaps();
      bound_rt_ = rt;
    }
    const std::string key(uri);
    auto it = entries_.find(key);
    if (it != entries_.end()) {
      lru_.splice(lru_.begin(), lru_, it->second.lru_it);
      it->second.lru_it = lru_.begin();
      return it->second.bitmap;
    }

    ID2D1Bitmap* bmp = decode_wic_bitmap(rt, uri);
    if (bmp == nullptr) {
      return nullptr;
    }

    while (entries_.size() >= max_entries_ && !lru_.empty()) {
      const std::string& victim = lru_.back();
      auto vit = entries_.find(victim);
      if (vit != entries_.end()) {
        if (vit->second.bitmap != nullptr) {
          vit->second.bitmap->Release();
        }
        entries_.erase(vit);
      }
      lru_.pop_back();
    }

    lru_.push_front(key);
    Entry entry{};
    entry.bitmap = bmp;
    entry.lru_it = lru_.begin();
    entries_.emplace(key, entry);
    return bmp;
  }

 private:
  struct Entry {
    ID2D1Bitmap* bitmap{nullptr};
    std::list<std::string>::iterator lru_it;
  };

  void clear_bitmaps() {
    for (auto& [_, entry] : entries_) {
      if (entry.bitmap != nullptr) {
        entry.bitmap->Release();
        entry.bitmap = nullptr;
      }
    }
    entries_.clear();
    lru_.clear();
  }

  bool ensure_wic_factory() {
    if (wic_factory_ != nullptr) {
      return true;
    }
    (void)::CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
    const HRESULT hr = ::CoCreateInstance(
        CLSID_WICImagingFactory,
        nullptr,
        CLSCTX_INPROC_SERVER,
        IID_PPV_ARGS(&wic_factory_));
    return SUCCEEDED(hr) && wic_factory_ != nullptr;
  }

  ID2D1Bitmap* decode_wic_bitmap(ID2D1RenderTarget* rt, std::string_view uri) {
    if (!ensure_wic_factory()) {
      return nullptr;
    }

    IWICBitmapDecoder* decoder = nullptr;
    IWICStream* stream = nullptr;
    std::vector<uint8_t> decoded_bytes;
    HRESULT hr = E_FAIL;

    if (uri.size() > 11u && ascii_lower(uri.substr(0, 11)) == "data:image/") {
      const size_t marker = uri.find(";base64,");
      if (marker == std::string_view::npos) {
        return nullptr;
      }
      if (!decode_base64(uri.substr(marker + 8u), decoded_bytes)) {
        return nullptr;
      }
      hr = wic_factory_->CreateStream(&stream);
      if (SUCCEEDED(hr) && stream != nullptr) {
        hr = stream->InitializeFromMemory(
            decoded_bytes.data(), static_cast<DWORD>(decoded_bytes.size()));
      }
      if (SUCCEEDED(hr)) {
        hr = wic_factory_->CreateDecoderFromStream(
            stream, nullptr, WICDecodeMetadataCacheOnLoad, &decoder);
      }
    } else if (uri.size() > 7u && ascii_lower(uri.substr(0, 7)) == "file://") {
      std::string_view path = uri.substr(7u);
      if (path.size() >= 3u && path[0] == '/' && std::isalpha(static_cast<unsigned char>(path[1])) &&
          path[2] == ':') {
        path.remove_prefix(1u);
      }
      const std::wstring wide_path = DirectWriteTextMeasurer::utf8_to_wide(path);
      hr = wic_factory_->CreateDecoderFromFilename(
          wide_path.c_str(), nullptr, GENERIC_READ, WICDecodeMetadataCacheOnLoad, &decoder);
    }

    IWICBitmapFrameDecode* frame = nullptr;
    IWICFormatConverter* converter = nullptr;
    ID2D1Bitmap* bitmap = nullptr;

    if (SUCCEEDED(hr) && decoder != nullptr) {
      hr = decoder->GetFrame(0, &frame);
    }
    if (SUCCEEDED(hr) && frame != nullptr) {
      hr = wic_factory_->CreateFormatConverter(&converter);
    }
    if (SUCCEEDED(hr) && converter != nullptr) {
      hr = converter->Initialize(
          frame,
          GUID_WICPixelFormat32bppPBGRA,
          WICBitmapDitherTypeNone,
          nullptr,
          0.0,
          WICBitmapPaletteTypeCustom);
    }
    if (SUCCEEDED(hr) && converter != nullptr) {
      hr = rt->CreateBitmapFromWicBitmap(converter, nullptr, &bitmap);
    }

    if (converter != nullptr) converter->Release();
    if (frame != nullptr) frame->Release();
    if (decoder != nullptr) decoder->Release();
    if (stream != nullptr) stream->Release();
    return SUCCEEDED(hr) ? bitmap : nullptr;
  }

  size_t max_entries_{64u};
  IWICImagingFactory* wic_factory_{nullptr};
  ID2D1RenderTarget* bound_rt_{nullptr};
  std::list<std::string> lru_;
  std::unordered_map<std::string, Entry> entries_;
};

bool compute_d2d_image_rects(
    ImageResizeMode mode,
    const D2D1_RECT_F& dst_bounds,
    const D2D1_SIZE_F& bmp_size,
    D2D1_RECT_F& out_dst,
    D2D1_RECT_F& out_src) noexcept {
  const float dw = dst_bounds.right - dst_bounds.left;
  const float dh = dst_bounds.bottom - dst_bounds.top;
  const float iw = bmp_size.width;
  const float ih = bmp_size.height;
  if (dw <= 0.0f || dh <= 0.0f || iw <= 0.0f || ih <= 0.0f) {
    return false;
  }

  switch (mode) {
    case ImageResizeMode::Stretch:
      out_dst = dst_bounds;
      out_src = D2D1::RectF(0.0f, 0.0f, iw, ih);
      return true;
    case ImageResizeMode::Contain: {
      const float scale = std::min(dw / iw, dh / ih);
      const float tw = iw * scale;
      const float th = ih * scale;
      const float ox = dst_bounds.left + (dw - tw) * 0.5f;
      const float oy = dst_bounds.top + (dh - th) * 0.5f;
      out_dst = D2D1::RectF(ox, oy, ox + tw, oy + th);
      out_src = D2D1::RectF(0.0f, 0.0f, iw, ih);
      return true;
    }
    case ImageResizeMode::Center: {
      const float draw_w = std::min(dw, iw);
      const float draw_h = std::min(dh, ih);
      const float ox = dst_bounds.left + (dw - draw_w) * 0.5f;
      const float oy = dst_bounds.top + (dh - draw_h) * 0.5f;
      const float sx = (iw - draw_w) * 0.5f;
      const float sy = (ih - draw_h) * 0.5f;
      out_dst = D2D1::RectF(ox, oy, ox + draw_w, oy + draw_h);
      out_src = D2D1::RectF(sx, sy, sx + draw_w, sy + draw_h);
      return true;
    }
    case ImageResizeMode::Cover:
    default: {
      const float scale = std::max(dw / iw, dh / ih);
      const float crop_w = std::min(iw, dw / scale);
      const float crop_h = std::min(ih, dh / scale);
      const float sx = (iw - crop_w) * 0.5f;
      const float sy = (ih - crop_h) * 0.5f;
      out_dst = dst_bounds;
      out_src = D2D1::RectF(sx, sy, sx + crop_w, sy + crop_h);
      return true;
    }
  }
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
      ID2D1Bitmap* bmp = WinD2DBitmapCache::shared().get_or_decode(rt, node->image_src);
      if (bmp != nullptr) {
        D2D1_RECT_F dst_rect{};
        D2D1_RECT_F src_rect{};
        if (compute_d2d_image_rects(node->resize_mode, rect, bmp->GetSize(), dst_rect, src_rect)) {
          rt->PushAxisAlignedClip(rect, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
          rt->DrawBitmap(bmp, dst_rect, 1.0f, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, src_rect);
          rt->PopAxisAlignedClip();
        }
      }
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

bool write_bgra_bmp_file(
    std::string_view out_path,
    uint32_t width_px,
    uint32_t height_px,
    const uint8_t* top_down_bgra,
    std::string* out_error) {
  if (out_path.empty() || width_px == 0 || height_px == 0 || top_down_bgra == nullptr) {
    if (out_error != nullptr) {
      *out_error = "Invalid BMP export arguments";
    }
    return false;
  }

  const uint32_t row_bytes = width_px * 4u;
  const uint32_t pixel_bytes = row_bytes * height_px;

  BITMAPFILEHEADER file_hdr{};
  file_hdr.bfType = 0x4D42;
  file_hdr.bfOffBits = sizeof(BITMAPFILEHEADER) + sizeof(BITMAPINFOHEADER);
  file_hdr.bfSize = file_hdr.bfOffBits + pixel_bytes;

  BITMAPINFOHEADER info_hdr{};
  info_hdr.biSize = sizeof(BITMAPINFOHEADER);
  info_hdr.biWidth = static_cast<LONG>(width_px);
  info_hdr.biHeight = static_cast<LONG>(height_px);
  info_hdr.biPlanes = 1;
  info_hdr.biBitCount = 32;
  info_hdr.biCompression = BI_RGB;
  info_hdr.biSizeImage = pixel_bytes;

  std::ofstream out(DirectWriteTextMeasurer::utf8_to_wide(out_path), std::ios::binary | std::ios::trunc);
  if (!out.is_open()) {
    if (out_error != nullptr) {
      *out_error = "Failed to open BMP output file";
    }
    return false;
  }

  out.write(reinterpret_cast<const char*>(&file_hdr), sizeof(file_hdr));
  out.write(reinterpret_cast<const char*>(&info_hdr), sizeof(info_hdr));
  for (uint32_t y = 0; y < height_px; ++y) {
    const uint32_t src_row = (height_px - 1u) - y;
    out.write(
        reinterpret_cast<const char*>(top_down_bgra + static_cast<size_t>(src_row) * row_bytes),
        row_bytes);
  }
  if (!out.good()) {
    if (out_error != nullptr) {
      *out_error = "Failed while writing BMP output file";
    }
    return false;
  }
  return true;
}

} // namespace

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

bool render_visual_tree_to_bmp(
    const NveWinVisualTree& tree,
    const DirectWriteTextMeasurer& measurer,
    uint32_t width_px,
    uint32_t height_px,
    float scale_factor,
    std::string_view out_bmp_path,
    std::string* out_error) {
  if (width_px == 0 || height_px == 0) {
    if (out_error != nullptr) {
      *out_error = "Screenshot dimensions must be non-zero";
    }
    return false;
  }

  ID2D1Factory* factory = nullptr;
  HRESULT hr = ::D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, &factory);
  if (FAILED(hr) || factory == nullptr) {
    if (out_error != nullptr) {
      *out_error = "D2D1CreateFactory failed";
    }
    return false;
  }

  const float effective_scale = (scale_factor > 0.0f && std::isfinite(scale_factor)) ? scale_factor : 1.0f;
  const float dpi = effective_scale * 96.0f;

  const D2D1_RENDER_TARGET_PROPERTIES props = D2D1::RenderTargetProperties(
      D2D1_RENDER_TARGET_TYPE_DEFAULT,
      D2D1::PixelFormat(DXGI_FORMAT_B8G8R8A8_UNORM, D2D1_ALPHA_MODE_PREMULTIPLIED),
      dpi,
      dpi,
      D2D1_RENDER_TARGET_USAGE_NONE,
      D2D1_FEATURE_LEVEL_DEFAULT);

  ID2D1DCRenderTarget* dc_rt = nullptr;
  hr = factory->CreateDCRenderTarget(&props, &dc_rt);
  if (FAILED(hr) || dc_rt == nullptr) {
    factory->Release();
    if (out_error != nullptr) {
      *out_error = "CreateDCRenderTarget failed";
    }
    return false;
  }

  HDC mem_dc = ::CreateCompatibleDC(nullptr);
  if (mem_dc == nullptr) {
    dc_rt->Release();
    factory->Release();
    if (out_error != nullptr) {
      *out_error = "CreateCompatibleDC failed";
    }
    return false;
  }

  BITMAPINFO bmi{};
  bmi.bmiHeader.biSize = sizeof(BITMAPINFOHEADER);
  bmi.bmiHeader.biWidth = static_cast<LONG>(width_px);
  bmi.bmiHeader.biHeight = -static_cast<LONG>(height_px);
  bmi.bmiHeader.biPlanes = 1;
  bmi.bmiHeader.biBitCount = 32;
  bmi.bmiHeader.biCompression = BI_RGB;

  void* dib_bits = nullptr;
  HBITMAP dib = ::CreateDIBSection(mem_dc, &bmi, DIB_RGB_COLORS, &dib_bits, nullptr, 0);
  if (dib == nullptr || dib_bits == nullptr) {
    ::DeleteDC(mem_dc);
    dc_rt->Release();
    factory->Release();
    if (out_error != nullptr) {
      *out_error = "CreateDIBSection failed";
    }
    return false;
  }

  HGDIOBJ prev_obj = ::SelectObject(mem_dc, dib);
  const RECT bind_rect{0, 0, static_cast<LONG>(width_px), static_cast<LONG>(height_px)};
  hr = dc_rt->BindDC(mem_dc, &bind_rect);

  bool ok = false;
  if (SUCCEEDED(hr)) {
    hr = paint_visual_tree_d2d(dc_rt, tree, measurer);
    if (SUCCEEDED(hr)) {
      ::GdiFlush();
      ok = write_bgra_bmp_file(
          out_bmp_path, width_px, height_px, static_cast<const uint8_t*>(dib_bits), out_error);
    } else if (out_error != nullptr) {
      *out_error = "paint_visual_tree_d2d failed";
    }
  } else if (out_error != nullptr) {
    *out_error = "ID2D1DCRenderTarget::BindDC failed";
  }

  WinD2DBitmapCache::shared().clear_for_target(dc_rt);
  ::SelectObject(mem_dc, prev_obj);
  ::DeleteObject(dib);
  ::DeleteDC(mem_dc);
  dc_rt->Release();
  factory->Release();
  return ok;
}

// -----------------------------------------------------------------------------
// NveWinHostWindow
// -----------------------------------------------------------------------------

NveWinHostWindow::NveWinHostWindow() {
  host_.set_on_visual_tree_changed([this]() {
    if (hwnd_ != nullptr) {
      ::InvalidateRect(hwnd_, nullptr, FALSE);
    }
  });
}

NveWinHostWindow::~NveWinHostWindow() {
  host_.set_on_visual_tree_changed({});
  discard_device_resources();
  if (d2d_factory_ != nullptr) {
    d2d_factory_->Release();
    d2d_factory_ = nullptr;
  }
  if (hwnd_ != nullptr && ::IsWindow(hwnd_)) {
    ::DestroyWindow(hwnd_);
    hwnd_ = nullptr;
  }
}

bool NveWinHostWindow::create(HINSTANCE instance, const WinHostWindowOptions& options) {
  options_ = options;
  if (!ensure_d2d_factory()) {
    return false;
  }

  WNDCLASSEXW wc{};
  wc.cbSize = sizeof(wc);
  wc.style = CS_HREDRAW | CS_VREDRAW;
  wc.lpfnWndProc = &NveWinHostWindow::static_wnd_proc;
  wc.hInstance = instance;
  wc.hCursor = ::LoadCursorW(nullptr, reinterpret_cast<LPCWSTR>(IDC_ARROW));
  wc.lpszClassName = kWindowClassName;
  ::RegisterClassExW(&wc);

  const std::wstring title_w = DirectWriteTextMeasurer::utf8_to_wide(
      options_.title.empty() ? "NVE Windows Host Shell" : options_.title);

  RECT desired{0, 0, options_.width, options_.height};
  const DWORD style = WS_OVERLAPPEDWINDOW;
  ::AdjustWindowRect(&desired, style, FALSE);

  hwnd_ = ::CreateWindowExW(
      0,
      kWindowClassName,
      title_w.c_str(),
      style,
      CW_USEDEFAULT,
      CW_USEDEFAULT,
      desired.right - desired.left,
      desired.bottom - desired.top,
      nullptr,
      nullptr,
      instance,
      this);

  return hwnd_ != nullptr;
}

void NveWinHostWindow::show(int cmd_show) {
  if (hwnd_ != nullptr) {
    ::ShowWindow(hwnd_, cmd_show);
    ::UpdateWindow(hwnd_);
  }
}

int NveWinHostWindow::run_message_loop() {
  MSG msg{};
  while (::GetMessageW(&msg, nullptr, 0, 0) > 0) {
    ::TranslateMessage(&msg);
    ::DispatchMessageW(&msg);
  }
  return static_cast<int>(msg.wParam);
}

float NveWinHostWindow::dpi_scale() const noexcept {
  if (hwnd_ == nullptr) {
    return 1.0f;
  }
  const UINT dpi = ::GetDpiForWindow(hwnd_);
  return (dpi > 0) ? (static_cast<float>(dpi) / 96.0f) : 1.0f;
}

NveSurfaceConfig NveWinHostWindow::current_surface_config() const noexcept {
  const float scale = dpi_scale();
  float dip_w = static_cast<float>(options_.width > 0 ? options_.width : 640);
  float dip_h = static_cast<float>(options_.height > 0 ? options_.height : 420);
  if (hwnd_ != nullptr) {
    RECT rc{};
    if (::GetClientRect(hwnd_, &rc)) {
      const float px_w = static_cast<float>(std::max<LONG>(0, rc.right - rc.left));
      const float px_h = static_cast<float>(std::max<LONG>(0, rc.bottom - rc.top));
      if (px_w > 0.0f && px_h > 0.0f) {
        dip_w = px_w / scale;
        dip_h = px_h / scale;
      }
    }
  }
  return NveSurfaceConfig{
      .surface_id = options_.surface_id,
      .width = dip_w,
      .height = dip_h,
      .scale_factor = scale,
  };
}

bool NveWinHostWindow::attach_interface(NveGetInterfaceFn get_interface) {
  return host_.attach_interface(get_interface, current_surface_config());
}

bool NveWinHostWindow::load_dylib(
    std::string_view path,
    int* out_err_code,
    std::string* out_err_msg) {
  return host_.load_dylib(path, current_surface_config(), out_err_code, out_err_msg);
}

bool NveWinHostWindow::reload_dylib(
    std::string_view path,
    int* out_err_code,
    std::string* out_err_msg) {
  return host_.reload_dylib(path, out_err_code, out_err_msg);
}

bool NveWinHostWindow::save_screenshot_bmp(
    std::string_view out_bmp_path,
    std::string* out_error) const {
  const NveSurfaceConfig cfg = host_.is_valid() ? host_.surface_config() : current_surface_config();
  const float scale = (cfg.scale_factor > 0.0f) ? cfg.scale_factor : 1.0f;
  const uint32_t px_w = static_cast<uint32_t>(std::max(1.0f, std::round(cfg.width * scale)));
  const uint32_t px_h = static_cast<uint32_t>(std::max(1.0f, std::round(cfg.height * scale)));
  return render_visual_tree_to_bmp(
      host_.visual_tree(), *host_.text_measurer(), px_w, px_h, scale, out_bmp_path, out_error);
}

bool NveWinHostWindow::ensure_d2d_factory() {
  if (d2d_factory_ != nullptr) {
    return true;
  }
  const HRESULT hr = ::D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, &d2d_factory_);
  return SUCCEEDED(hr) && d2d_factory_ != nullptr;
}

bool NveWinHostWindow::ensure_render_target() {
  if (render_target_ != nullptr) {
    return true;
  }
  if (hwnd_ == nullptr || !ensure_d2d_factory()) {
    return false;
  }

  RECT rc{};
  ::GetClientRect(hwnd_, &rc);
  const D2D1_SIZE_U size = D2D1::SizeU(
      static_cast<UINT32>(std::max<LONG>(1, rc.right - rc.left)),
      static_cast<UINT32>(std::max<LONG>(1, rc.bottom - rc.top)));

  const HRESULT hr = d2d_factory_->CreateHwndRenderTarget(
      D2D1::RenderTargetProperties(),
      D2D1::HwndRenderTargetProperties(hwnd_, size),
      &render_target_);
  if (FAILED(hr) || render_target_ == nullptr) {
    return false;
  }

  const float dpi = dpi_scale() * 96.0f;
  render_target_->SetDpi(dpi, dpi);
  return true;
}

void NveWinHostWindow::discard_device_resources() noexcept {
  if (render_target_ != nullptr) {
    WinD2DBitmapCache::shared().clear_for_target(render_target_);
    render_target_->Release();
    render_target_ = nullptr;
  }
}

void NveWinHostWindow::on_paint() {
  PAINTSTRUCT ps{};
  ::BeginPaint(hwnd_, &ps);
  if (ensure_render_target()) {
    const HRESULT hr =
        paint_visual_tree_d2d(render_target_, host_.visual_tree(), *host_.text_measurer());
    if (hr == D2DERR_RECREATE_TARGET) {
      discard_device_resources();
      ::InvalidateRect(hwnd_, nullptr, FALSE);
    }
  }
  ::EndPaint(hwnd_, &ps);
}

void NveWinHostWindow::on_resize(UINT width_px, UINT height_px) {
  if (width_px == 0 || height_px == 0) {
    return;
  }
  if (render_target_ != nullptr) {
    (void)render_target_->Resize(D2D1::SizeU(width_px, height_px));
  }
  const float scale = dpi_scale();
  host_.set_surface_size(
      static_cast<float>(width_px) / scale, static_cast<float>(height_px) / scale, scale);
  ::InvalidateRect(hwnd_, nullptr, FALSE);
}

void NveWinHostWindow::on_left_button_up(int x_px, int y_px) {
  const float scale = dpi_scale();
  const float dip_x = static_cast<float>(x_px) / scale;
  const float dip_y = static_cast<float>(y_px) / scale;
  if (host_.dispatch_click_at(dip_x, dip_y) > 0) {
    ::InvalidateRect(hwnd_, nullptr, FALSE);
  }
}

void NveWinHostWindow::on_mouse_wheel(
    int screen_x,
    int screen_y,
    short wheel_delta,
    bool horizontal) {
  if (hwnd_ == nullptr) {
    return;
  }
  POINT pt{screen_x, screen_y};
  ::ScreenToClient(hwnd_, &pt);
  const float scale = dpi_scale();
  const float dip_x = static_cast<float>(pt.x) / scale;
  const float dip_y = static_cast<float>(pt.y) / scale;
  const float step = (static_cast<float>(wheel_delta) / static_cast<float>(WHEEL_DELTA)) * 45.0f;
  const float dx = horizontal ? step : 0.0f;
  const float dy = horizontal ? 0.0f : -step;
  (void)host_.dispatch_wheel_at(dip_x, dip_y, dx, dy);
  ::InvalidateRect(hwnd_, nullptr, FALSE);
}

LRESULT CALLBACK NveWinHostWindow::static_wnd_proc(
    HWND hwnd,
    UINT msg,
    WPARAM wparam,
    LPARAM lparam) {
  NveWinHostWindow* self = nullptr;
  if (msg == WM_NCCREATE) {
    const auto* cs = reinterpret_cast<const CREATESTRUCTW*>(lparam);
    self = static_cast<NveWinHostWindow*>(cs->lpCreateParams);
    ::SetWindowLongPtrW(hwnd, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(self));
    if (self != nullptr) {
      self->hwnd_ = hwnd;
    }
  } else {
    self = reinterpret_cast<NveWinHostWindow*>(::GetWindowLongPtrW(hwnd, GWLP_USERDATA));
  }

  if (self != nullptr) {
    return self->handle_message(msg, wparam, lparam);
  }
  return ::DefWindowProcW(hwnd, msg, wparam, lparam);
}

LRESULT NveWinHostWindow::handle_message(UINT msg, WPARAM wparam, LPARAM lparam) {
  switch (msg) {
    case WM_ERASEBKGND:
      return 1;
    case WM_PAINT:
      on_paint();
      return 0;
    case WM_SIZE:
      on_resize(LOWORD(lparam), HIWORD(lparam));
      return 0;
    case WM_DPICHANGED: {
      const float new_dpi = static_cast<float>(HIWORD(wparam));
      const float new_scale = (new_dpi > 0.0f) ? (new_dpi / 96.0f) : 1.0f;
      if (render_target_ != nullptr) {
        render_target_->SetDpi(new_dpi, new_dpi);
      }
      if (const auto* suggested = reinterpret_cast<const RECT*>(lparam)) {
        ::SetWindowPos(
            hwnd_,
            nullptr,
            suggested->left,
            suggested->top,
            suggested->right - suggested->left,
            suggested->bottom - suggested->top,
            SWP_NOZORDER | SWP_NOACTIVATE);
      }
      RECT client{};
      if (::GetClientRect(hwnd_, &client)) {
        const float dip_w = static_cast<float>(std::max<LONG>(1, client.right - client.left)) / new_scale;
        const float dip_h = static_cast<float>(std::max<LONG>(1, client.bottom - client.top)) / new_scale;
        host_.set_surface_size(dip_w, dip_h, new_scale);
      }
      ::InvalidateRect(hwnd_, nullptr, FALSE);
      return 0;
    }
    case WM_LBUTTONUP:
      on_left_button_up(GET_X_LPARAM(lparam), GET_Y_LPARAM(lparam));
      return 0;
    case WM_MOUSEWHEEL:
      on_mouse_wheel(
          GET_X_LPARAM(lparam), GET_Y_LPARAM(lparam), GET_WHEEL_DELTA_WPARAM(wparam), false);
      return 0;
    case WM_MOUSEHWHEEL:
      on_mouse_wheel(
          GET_X_LPARAM(lparam), GET_Y_LPARAM(lparam), GET_WHEEL_DELTA_WPARAM(wparam), true);
      return 0;
    case WM_DESTROY:
      discard_device_resources();
      hwnd_ = nullptr;
      if (options_.quit_on_destroy) {
        ::PostQuitMessage(0);
      }
      return 0;
    default:
      return ::DefWindowProcW(hwnd_, msg, wparam, lparam);
  }
}

} // namespace nve::win

#endif // defined(_WIN32)
