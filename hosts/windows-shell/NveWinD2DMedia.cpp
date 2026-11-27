// WIC image decoding (`data:` URLs, `file://` URLs, absolute paths, inline encoded bytes) into a
// bounded per-render-target LRU of `ID2D1Bitmap`s, and headless BMP screenshots of the visual tree.
#include "NveWinD2DInternal.hpp"

#include <shlwapi.h>
#include <wincodec.h>
#include <wincrypt.h>

#include <algorithm>
#include <cctype>
#include <cmath>
#include <filesystem>
#include <fstream>
#include <list>
#include <string>
#include <string_view>
#include <unordered_map>
#include <vector>

namespace nve::win {

namespace {

bool starts_with_ci(std::string_view s, std::string_view prefix) noexcept {
  if (s.size() < prefix.size()) return false;
  for (size_t i = 0; i < prefix.size(); ++i) {
    if (std::tolower(static_cast<unsigned char>(s[i])) != prefix[i]) return false;
  }
  return true;
}

/// Base64 payload of a `data:image/...;base64,` URL (native `CryptStringToBinaryA`).
std::vector<uint8_t> data_url_bytes(std::string_view url) {
  const size_t marker = url.find(";base64,");
  if (marker == std::string_view::npos) return {};
  const std::string_view b64 = url.substr(marker + 8u);
  if (b64.empty() || b64.size() > 0x7FFFFFFFu) return {};
  DWORD len = 0u;
  if (!::CryptStringToBinaryA(b64.data(), static_cast<DWORD>(b64.size()), CRYPT_STRING_BASE64, nullptr, &len, nullptr,
                              nullptr) ||
      len == 0u) {
    return {};
  }
  std::vector<uint8_t> out(len);
  if (!::CryptStringToBinaryA(b64.data(), static_cast<DWORD>(b64.size()), CRYPT_STRING_BASE64, out.data(), &len, nullptr,
                              nullptr)) {
    return {};
  }
  out.resize(len);
  return out;
}

/// Filesystem path of a `file://` URL (percent-decoded by `PathCreateFromUrlW`) or an absolute path.
std::wstring file_path(std::string_view url) {
  if (starts_with_ci(url, "file:")) {
    const std::wstring wurl = wide(url);
    wchar_t path[MAX_PATH * 4] = {};
    DWORD len = static_cast<DWORD>(std::size(path));
    return SUCCEEDED(::PathCreateFromUrlW(wurl.c_str(), path, &len, 0)) ? std::wstring(path, len) : std::wstring();
  }
  const bool drive = url.size() > 2u && std::isalpha(static_cast<unsigned char>(url[0])) && url[1] == ':';
  return drive || starts_with_ci(url, "\\\\") || starts_with_ci(url, "/") ? wide(url) : std::wstring();
}

class BitmapCache {
 public:
  static BitmapCache& shared() {
    static BitmapCache cache;
    return cache;
  }
  ~BitmapCache() {
    for (auto& [rt, lru] : targets_) clear(lru);
    if (wic_ != nullptr) wic_->Release();
  }

  ID2D1Bitmap* get(ID2D1RenderTarget* rt, const WinImageSource& src) {
    if (rt == nullptr || src.empty()) return nullptr;
    Lru& lru = targets_[rt];
    if (const auto it = lru.entries.find(src.key); it != lru.entries.end()) {
      lru.order.splice(lru.order.begin(), lru.order, it->second.at);
      return it->second.bitmap;
    }
    ID2D1Bitmap* bmp = decode(rt, src);
    if (bmp == nullptr) return nullptr;
    while (lru.entries.size() >= kMaxEntries) {
      const auto victim = lru.entries.find(lru.order.back());
      victim->second.bitmap->Release();
      lru.entries.erase(victim);
      lru.order.pop_back();
    }
    lru.order.push_front(src.key);
    lru.entries.emplace(src.key, Entry{bmp, lru.order.begin()});
    return bmp;
  }

  void release(ID2D1RenderTarget* rt) {
    if (const auto it = targets_.find(rt); it != targets_.end()) {
      clear(it->second);
      targets_.erase(it);
    }
  }

 private:
  static constexpr size_t kMaxEntries = 64u;
  struct Entry {
    ID2D1Bitmap* bitmap = nullptr;
    std::list<std::string>::iterator at;
  };
  struct Lru {
    std::list<std::string> order;  // most recent first
    std::unordered_map<std::string, Entry> entries;
  };

  static void clear(Lru& lru) {
    for (auto& [key, e] : lru.entries) e.bitmap->Release();
    lru.entries.clear();
    lru.order.clear();
  }

  IWICImagingFactory* wic() {
    if (wic_ == nullptr) {
      (void)::CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);  // S_FALSE / RPC_E_CHANGED_MODE: COM is up
      (void)::CoCreateInstance(CLSID_WICImagingFactory, nullptr, CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&wic_));
    }
    return wic_;
  }

  ID2D1Bitmap* decode(ID2D1RenderTarget* rt, const WinImageSource& src) {
    IWICImagingFactory* f = wic();
    if (f == nullptr) return nullptr;
    std::vector<uint8_t> url_bytes;
    std::span<const uint8_t> memory = src.encoded;
    std::wstring path;
    if (!src.url.empty()) {
      if (starts_with_ci(src.url, "data:image/")) {
        url_bytes = data_url_bytes(src.url);
        memory = url_bytes;
      } else {
        path = file_path(src.url);
      }
      if (memory.empty() && path.empty()) return nullptr;  // remote URLs: not fetched by the shell
    }
    IWICBitmapDecoder* decoder = nullptr;
    IWICStream* stream = nullptr;
    HRESULT hr = E_FAIL;
    if (!path.empty()) {
      hr = f->CreateDecoderFromFilename(path.c_str(), nullptr, GENERIC_READ, WICDecodeMetadataCacheOnLoad, &decoder);
    } else if (!memory.empty() && memory.size() <= 0xFFFFFFFFu) {
      hr = f->CreateStream(&stream);
      if (SUCCEEDED(hr)) {
        hr = stream->InitializeFromMemory(const_cast<BYTE*>(memory.data()), static_cast<DWORD>(memory.size()));
      }
      if (SUCCEEDED(hr)) hr = f->CreateDecoderFromStream(stream, nullptr, WICDecodeMetadataCacheOnLoad, &decoder);
    }
    IWICBitmapFrameDecode* frame = nullptr;
    IWICFormatConverter* converter = nullptr;
    ID2D1Bitmap* bitmap = nullptr;
    if (SUCCEEDED(hr)) hr = decoder->GetFrame(0u, &frame);
    if (SUCCEEDED(hr)) hr = f->CreateFormatConverter(&converter);
    if (SUCCEEDED(hr)) {
      hr = converter->Initialize(frame, GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, nullptr, 0.0,
                                 WICBitmapPaletteTypeCustom);
    }
    if (SUCCEEDED(hr)) hr = rt->CreateBitmapFromWicBitmap(converter, nullptr, &bitmap);
    for (IUnknown* u : {static_cast<IUnknown*>(converter), static_cast<IUnknown*>(frame), static_cast<IUnknown*>(decoder),
                        static_cast<IUnknown*>(stream)}) {
      if (u != nullptr) u->Release();
    }
    return SUCCEEDED(hr) ? bitmap : nullptr;
  }

  IWICImagingFactory* wic_ = nullptr;
  std::unordered_map<ID2D1RenderTarget*, Lru> targets_;
};

bool fail(std::string* out_error, const char* message) {
  if (out_error != nullptr) *out_error = message;
  return false;
}

/// Writes a top-down 32-bit BGRA buffer as a bottom-up `.bmp`.
bool write_bmp(std::string_view path, uint32_t w, uint32_t h, const uint8_t* bgra, std::string* out_error) {
  const uint32_t row = w * 4u;
  BITMAPFILEHEADER file{};
  file.bfType = 0x4D42;  // "BM"
  file.bfOffBits = sizeof(BITMAPFILEHEADER) + sizeof(BITMAPINFOHEADER);
  file.bfSize = file.bfOffBits + row * h;
  BITMAPINFOHEADER info{};
  info.biSize = sizeof(BITMAPINFOHEADER);
  info.biWidth = static_cast<LONG>(w);
  info.biHeight = static_cast<LONG>(h);
  info.biPlanes = 1;
  info.biBitCount = 32;
  info.biCompression = BI_RGB;
  info.biSizeImage = row * h;
  std::ofstream out(std::filesystem::path(wide(path)), std::ios::binary | std::ios::trunc);
  if (!out.is_open()) return fail(out_error, "cannot open the BMP output file");
  out.write(reinterpret_cast<const char*>(&file), sizeof(file));
  out.write(reinterpret_cast<const char*>(&info), sizeof(info));
  for (uint32_t y = h; y-- > 0u;) out.write(reinterpret_cast<const char*>(bgra + static_cast<size_t>(y) * row), row);
  return out.good() || fail(out_error, "writing the BMP output file failed");
}

}  // namespace

namespace d2d_internal {

ID2D1Bitmap* bitmap_for(ID2D1RenderTarget* rt, const WinImageSource& src) { return BitmapCache::shared().get(rt, src); }

void release_bitmaps(ID2D1RenderTarget* rt) { BitmapCache::shared().release(rt); }

}  // namespace d2d_internal

bool render_visual_tree_to_bmp(const NveWinVisualTree& tree, const DirectWriteTextMeasurer& text, uint32_t width_px,
                               uint32_t height_px, float scale_factor, std::string_view out_bmp_path,
                               std::string* out_error) {
  if (width_px == 0u || height_px == 0u || width_px > 16384u || height_px > 16384u) {
    return fail(out_error, "screenshot size must be 1..16384 px");
  }
  ID2D1Factory* factory = nullptr;
  if (FAILED(::D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, &factory))) {
    return fail(out_error, "D2D1CreateFactory failed");
  }
  const float dpi = 96.0f * (std::isfinite(scale_factor) && scale_factor > 0.0f ? scale_factor : 1.0f);
  const D2D1_RENDER_TARGET_PROPERTIES props = D2D1::RenderTargetProperties(
      D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1::PixelFormat(DXGI_FORMAT_B8G8R8A8_UNORM, D2D1_ALPHA_MODE_PREMULTIPLIED), dpi,
      dpi);
  ID2D1DCRenderTarget* rt = nullptr;
  HDC dc = nullptr;
  HBITMAP dib = nullptr;
  HGDIOBJ previous = nullptr;
  void* bits = nullptr;
  bool ok = false;
  std::string error;
  if (FAILED(factory->CreateDCRenderTarget(&props, &rt))) {
    error = "CreateDCRenderTarget failed";
  } else if ((dc = ::CreateCompatibleDC(nullptr)) == nullptr) {
    error = "CreateCompatibleDC failed";
  } else {
    BITMAPINFO bmi{};
    bmi.bmiHeader.biSize = sizeof(BITMAPINFOHEADER);
    bmi.bmiHeader.biWidth = static_cast<LONG>(width_px);
    bmi.bmiHeader.biHeight = -static_cast<LONG>(height_px);  // top-down
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = BI_RGB;
    dib = ::CreateDIBSection(dc, &bmi, DIB_RGB_COLORS, &bits, nullptr, 0);
    if (dib == nullptr || bits == nullptr) {
      error = "CreateDIBSection failed";
    } else {
      previous = ::SelectObject(dc, dib);
      const RECT bind{0, 0, static_cast<LONG>(width_px), static_cast<LONG>(height_px)};
      if (FAILED(rt->BindDC(dc, &bind))) {
        error = "ID2D1DCRenderTarget::BindDC failed";
      } else if (FAILED(paint_visual_tree_d2d(rt, factory, tree, text))) {
        error = "paint_visual_tree_d2d failed";
      } else {
        ::GdiFlush();
        ok = write_bmp(out_bmp_path, width_px, height_px, static_cast<const uint8_t*>(bits), &error);
      }
    }
  }
  if (rt != nullptr) {
    d2d_internal::release_bitmaps(rt);
    rt->Release();
  }
  if (previous != nullptr) ::SelectObject(dc, previous);
  if (dib != nullptr) ::DeleteObject(dib);
  if (dc != nullptr) ::DeleteDC(dc);
  factory->Release();
  if (!ok && out_error != nullptr) *out_error = error;
  return ok;
}

}  // namespace nve::win
