#pragma once

#import "NveAppleSupport.h"
#import <CoreGraphics/CoreGraphics.h>

#include "nve_draw_list.hpp"

#include <cstdint>
#include <span>

NS_ASSUME_NONNULL_BEGIN

namespace nve::apple {

/// Renders a binary `NVE_DRAW1` draw-list stream directly onto `ctx` (assuming top-left origin
/// coordinates: `+x` right, `+y` down) with zero per-op allocations and guaranteed balanced
/// CoreGraphics GState save/restore even on corrupt or truncated inputs.
nve::DrawListDecodeResult render_draw_list_to_cg_context(
    CGContextRef _Nullable ctx,
    CGRect bounds,
    std::span<const uint8_t> bytes) noexcept;

} // namespace nve::apple

/// Shared CoreGraphics vector canvas view for `NVE_NODE_KIND_SKIA_CANVAS` on both iOS (`UIView`)
/// and macOS (`NSView`, flipped top-left origin).
///
/// Caches the raw `NVE_DRAW1` byte buffer and only requests a redraw when the payload changes.
@interface NveAppleCanvasView : NvePlatformView

/// Monotonic count of `setNeedsDisplay` calls triggered by draw-list byte updates.
@property (nonatomic, readonly) uint64_t redrawRequestCount;

/// Updates the cached `NVE_DRAW1` bytes and triggers `setNeedsDisplay` ONLY if bytes changed.
/// Returns `YES` if the bytes changed and a redraw was requested.
- (BOOL)updateDrawListBytes:(std::span<const uint8_t>)bytes;

/// Read-only view of the currently cached `NVE_DRAW1` bytes.
- (std::span<const uint8_t>)drawListBytes;

/// Renders the cached draw-list bytes directly into `context` (top-left origin).
- (nve::DrawListDecodeResult)renderIntoContext:(CGContextRef)context bounds:(CGRect)bounds;

@end

NS_ASSUME_NONNULL_END
