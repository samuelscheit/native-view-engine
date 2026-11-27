// Helpers shared by the Direct2D renderer, the WIC image cache, and the host window.
#pragma once

#include "NveWinHostWindow.hpp"

namespace nve::win::d2d_internal {

[[nodiscard]] inline D2D1_RECT_F to_d2d(const Rect& r) noexcept { return D2D1::RectF(r.x, r.y, r.right(), r.bottom()); }

/// Packed RGBA8888 (`r | g<<8 | b<<16 | a<<24`) with its alpha scaled by `alpha`.
[[nodiscard]] inline D2D1_COLOR_F to_d2d(uint32_t rgba, float alpha = 1.0f) noexcept {
  const auto ch = [rgba](uint32_t shift) { return static_cast<float>((rgba >> shift) & 0xFFu) / 255.0f; };
  return D2D1::ColorF(ch(0u), ch(8u), ch(16u), ch(24u) * alpha);
}

/// Cached (bounded LRU) WIC-decoded bitmap of `src` for render target `rt`; `nullptr` when it
/// does not decode. Valid until `rt` is released or the entry is evicted (use it immediately).
ID2D1Bitmap* bitmap_for(ID2D1RenderTarget* rt, const WinImageSource& src);

/// Drops every cached bitmap of `rt` (before releasing it).
void release_bitmaps(ID2D1RenderTarget* rt);

}  // namespace nve::win::d2d_internal
