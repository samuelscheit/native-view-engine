#pragma once

#import "NveAppleSupport.h"

#include <nve/canvas/draw_list.hpp>

#include <cstdint>
#include <span>

NS_ASSUME_NONNULL_BEGIN

/// Flipped `NSView` (macOS) / `UIView` (iOS) painting flat 4-byte-aligned `nve.res.draw_list`
/// commands onto a CoreGraphics `CGContext`.
@interface NveAppleCanvasView : NvePlatformView

/// Number of redraws scheduled because a new draw-list byte buffer arrived.
@property (nonatomic, readonly) uint64_t redrawCount;
/// Copy of the latest flat `nve.res.draw_list` bytes held by this canvas view (nil when empty).
@property (nonatomic, copy, readonly, nullable) NSData *drawListData;

/// Updates the canvas draw list and schedules a redraw only when the bytes changed.
- (void)setDrawListBytes:(nullable NSData *)bytes;

@end

NS_ASSUME_NONNULL_END

#ifdef __cplusplus
namespace nve::apple {

/// Validates and paints all 8 flat `nve.res.draw_list` opcodes onto `ctx` within `bounds`
/// (honoring `nve::canvas::kMaxClipDepth` overflow balancing and RAII clip unwind).
/// Malformed payloads invoke zero CoreGraphics calls and return the exact `DrawListError`.
nve::canvas::DrawListDecodeResult render_draw_list_to_cgcontext(
    CGContextRef _Nullable ctx,
    CGRect bounds,
    std::span<const uint8_t> bytes) noexcept;

}  // namespace nve::apple
#endif
