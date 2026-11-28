// `D2DPaintCache` (WIC-decoded bitmaps of `data:` URLs, local files and inline bytes; per-node
// DirectWrite layouts) and headless BMP screenshots of the visual tree.
#include "NveWinD2DInternal.hpp"

#include <nve/host/urls.hpp>

#include <shlwapi.h>
#include <wincodec.h>
#include <wincrypt.h>

#include <algorithm>
#include <cmath>
#include <filesystem>
#include <fstream>
#include <string>
#include <string_view>
#include <vector>

namespace nve::win {

namespace {

constexpr size_t kMaxBitmaps = 64u;

bool starts_with_ci(std::string_view s, std::string_view prefix) noexcept {
  return s.size() >= prefix.size() && host::ascii_iequals(s.substr(0u, prefix.size()), prefix);
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

/// A drive-rooted path `X:\...`: never UNC (`\\host`), NT / device (`\??\`, `\\?\`, `\\.\`), or
/// drive-relative (`X:foo`).
bool drive_path(const std::wstring& p) noexcept {
  return p.size() >= 3u && ::PathGetDriveNumberW(p.c_str()) >= 0 && p[1] == L':' && p[2] == L'\\';
}

/// Local filesystem path of an image URL (`host::local_image_path`; `file:` URLs decoded by
/// `PathCreateFromUrlW`), only ever a local drive path: a UNC path would make the OS
/// authenticate to that host (NTLM leak) and block the UI thread on the network. Checked after
/// decoding and again after `GetFullPathNameW` (`..`, `/` and device names normalized).
std::wstring local_file(std::string_view url) {
  const auto local = host::local_image_path(url);
  if (!local) return {};
  std::wstring path;
  if (starts_with_ci(url, "file:")) {
    const std::wstring wurl = wide(url);
    wchar_t buf[MAX_PATH * 4] = {};
    DWORD len = static_cast<DWORD>(std::size(buf));
    if (FAILED(::PathCreateFromUrlW(wurl.c_str(), buf, &len, 0))) return {};
    path.assign(buf, len);
  } else {
    path = wide(*local);
  }
  if (!host::local_decoded_path(narrow(path))) return {};
  std::replace(path.begin(), path.end(), L'/', L'\\');
  if (!drive_path(path)) return {};
  wchar_t full[MAX_PATH * 4] = {};
  const DWORD n = ::GetFullPathNameW(path.c_str(), static_cast<DWORD>(std::size(full)), full, nullptr);
  if (n == 0u || n >= std::size(full)) return {};
  const std::wstring resolved(full, n);
  return drive_path(resolved) ? resolved : std::wstring();
}

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

// --- D2DPaintCache ---------------------------------------------------------------------------

void D2DPaintCache::release() noexcept {
  for (auto& [key, b] : bitmaps_) b.bitmap->Release();
  bitmaps_.clear();
  lru_.clear();
  for (auto& [id, l] : layouts_) l.layout->Release();
  layouts_.clear();
  if (wic_ != nullptr) wic_->Release();
  wic_ = nullptr;
  if (com_) ::CoUninitialize();  // after the last COM object of this initialization
  com_ = false;
  target_ = nullptr;
}

ID2D1Bitmap* D2DPaintCache::bitmap(ID2D1RenderTarget* rt, const WinImageSource& src) {
  if (rt == nullptr || src.empty()) return nullptr;
  if (rt != target_) {  // bitmaps belong to one target
    for (auto& [key, b] : bitmaps_) b.bitmap->Release();
    bitmaps_.clear();
    lru_.clear();
    target_ = rt;
  }
  if (const auto it = bitmaps_.find(src.key); it != bitmaps_.end()) {
    lru_.splice(lru_.begin(), lru_, it->second.at);
    return it->second.bitmap;
  }
  ID2D1Bitmap* bmp = decode(rt, src);
  if (bmp == nullptr) return nullptr;
  if (bitmaps_.size() >= kMaxBitmaps) {
    const auto victim = bitmaps_.find(lru_.back());
    victim->second.bitmap->Release();
    bitmaps_.erase(victim);
    lru_.pop_back();
  }
  lru_.push_front(src.key);
  bitmaps_.emplace(src.key, Bitmap{bmp, lru_.begin()});
  return bmp;
}

ID2D1Bitmap* D2DPaintCache::decode(ID2D1RenderTarget* rt, const WinImageSource& src) {
  if (wic_ == nullptr) {
    // S_OK / S_FALSE count (balanced by `release`); RPC_E_CHANGED_MODE: COM is up in another
    // apartment model, not ours to uninitialize.
    if (!com_) com_ = SUCCEEDED(::CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED));
    if (FAILED(::CoCreateInstance(CLSID_WICImagingFactory, nullptr, CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&wic_)))) {
      wic_ = nullptr;
      return nullptr;
    }
  }
  std::vector<uint8_t> url_bytes;
  std::span<const uint8_t> memory = src.encoded;
  std::wstring path;
  if (!src.url.empty()) {
    if (starts_with_ci(src.url, "data:image/")) {
      url_bytes = data_url_bytes(src.url);
      memory = url_bytes;
    } else {
      path = local_file(src.url);  // remote URLs are not fetched by the shell
    }
    if (memory.empty() && path.empty()) return nullptr;
  }
  IWICBitmapDecoder* decoder = nullptr;
  IWICStream* stream = nullptr;
  HRESULT hr = E_FAIL;
  if (!path.empty()) {
    hr = wic_->CreateDecoderFromFilename(path.c_str(), nullptr, GENERIC_READ, WICDecodeMetadataCacheOnLoad, &decoder);
  } else if (!memory.empty() && memory.size() <= 0xFFFFFFFFu) {
    hr = wic_->CreateStream(&stream);
    if (SUCCEEDED(hr)) hr = stream->InitializeFromMemory(const_cast<BYTE*>(memory.data()), static_cast<DWORD>(memory.size()));
    if (SUCCEEDED(hr)) hr = wic_->CreateDecoderFromStream(stream, nullptr, WICDecodeMetadataCacheOnLoad, &decoder);
  }
  IWICBitmapFrameDecode* frame = nullptr;
  IWICFormatConverter* converter = nullptr;
  ID2D1Bitmap* bitmap = nullptr;
  if (SUCCEEDED(hr)) hr = decoder->GetFrame(0u, &frame);
  if (SUCCEEDED(hr)) hr = wic_->CreateFormatConverter(&converter);
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

IDWriteTextLayout* D2DPaintCache::layout(const DirectWriteTextMeasurer& m, uint32_t node, std::string_view text,
                                         const TextPaint& style, float width, float height) {
  auto it = layouts_.find(node);
  if (it != layouts_.end() && it->second.width == width && it->second.height == height && it->second.text == text &&
      it->second.style == style) {
    return it->second.layout;
  }
  IDWriteTextLayout* fresh = m.create_layout(text, style, width, height);
  if (it != layouts_.end()) {
    it->second.layout->Release();
    layouts_.erase(it);
  }
  if (fresh == nullptr) return nullptr;
  layouts_.emplace(node, Layout{fresh, std::string(text), style, width, height});
  return fresh;
}

void D2DPaintCache::prune(const NveWinVisualTree& tree) {
  for (auto it = layouts_.begin(); it != layouts_.end();) {
    if (tree.find(it->first) == nullptr) {
      it->second.layout->Release();
      it = layouts_.erase(it);
    } else {
      ++it;
    }
  }
}

// --- Headless screenshots --------------------------------------------------------------------

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
  D2DPaintCache cache;  // released with this target, below
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
      } else if (FAILED(paint_visual_tree_d2d(rt, factory, tree, text, cache))) {
        error = "paint_visual_tree_d2d failed";
      } else {
        ::GdiFlush();
        ok = write_bmp(out_bmp_path, width_px, height_px, static_cast<const uint8_t*>(bits), &error);
      }
    }
  }
  cache.release();
  if (rt != nullptr) rt->Release();
  if (previous != nullptr) ::SelectObject(dc, previous);
  if (dib != nullptr) ::DeleteObject(dib);
  if (dc != nullptr) ::DeleteDC(dc);
  factory->Release();
  if (!ok && out_error != nullptr) *out_error = error;
  return ok;
}

}  // namespace nve::win
