#include "NveWinD2DInternal.hpp"

#if defined(_WIN32)

#include <algorithm>
#include <cctype>
#include <cmath>
#include <cstdint>
#include <fstream>
#include <list>
#include <string>
#include <string_view>
#include <unordered_map>
#include <vector>
#include <wincodec.h>

namespace nve::win {

namespace {

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

namespace d2d_internal {

ID2D1Bitmap* get_or_decode_d2d_bitmap(ID2D1RenderTarget* rt, std::string_view uri) {
  return WinD2DBitmapCache::shared().get_or_decode(rt, uri);
}

void clear_d2d_bitmap_cache_for_target(ID2D1RenderTarget* rt) {
  WinD2DBitmapCache::shared().clear_for_target(rt);
}

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

} // namespace d2d_internal

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

  d2d_internal::clear_d2d_bitmap_cache_for_target(dc_rt);
  ::SelectObject(mem_dc, prev_obj);
  ::DeleteObject(dib);
  ::DeleteDC(mem_dc);
  dc_rt->Release();
  factory->Release();
  return ok;
}

} // namespace nve::win

#endif // defined(_WIN32)
