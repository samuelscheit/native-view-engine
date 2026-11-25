#pragma once

#include "NveWinHostWindow.hpp"

#if defined(_WIN32)

#include <string_view>

namespace nve::win::d2d_internal {

/// Fetch or WIC-decode a cached `ID2D1Bitmap*` bound to `rt` (owned by the shared LRU cache).
ID2D1Bitmap* get_or_decode_d2d_bitmap(ID2D1RenderTarget* rt, std::string_view uri);

/// Release all cached `ID2D1Bitmap*` instances bound to `rt` before `rt->Release()`.
void clear_d2d_bitmap_cache_for_target(ID2D1RenderTarget* rt);

/// Compute destination and source crop rectangles for React Native `ImageResizeMode`s.
bool compute_d2d_image_rects(
    ImageResizeMode mode,
    const D2D1_RECT_F& dst_bounds,
    const D2D1_SIZE_F& bmp_size,
    D2D1_RECT_F& out_dst,
    D2D1_RECT_F& out_src) noexcept;

} // namespace nve::win::d2d_internal

#endif // defined(_WIN32)
