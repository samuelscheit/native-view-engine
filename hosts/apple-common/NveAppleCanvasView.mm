#import "NveAppleCanvasView.h"

#include <algorithm>
#include <cstring>
#include <vector>

namespace nve::apple {

namespace {

inline void set_cg_fill_color(CGContextRef ctx, DrawColorRgba c) noexcept {
  CGContextSetRGBFillColor(
      ctx,
      static_cast<CGFloat>(c.r) / 255.0,
      static_cast<CGFloat>(c.g) / 255.0,
      static_cast<CGFloat>(c.b) / 255.0,
      static_cast<CGFloat>(c.a) / 255.0);
}

inline void set_cg_stroke_color(CGContextRef ctx, DrawColorRgba c) noexcept {
  CGContextSetRGBStrokeColor(
      ctx,
      static_cast<CGFloat>(c.r) / 255.0,
      static_cast<CGFloat>(c.g) / 255.0,
      static_cast<CGFloat>(c.b) / 255.0,
      static_cast<CGFloat>(c.a) / 255.0);
}

struct CoreGraphicsDrawVisitor {
  CGContextRef ctx;
  CGRect bounds;
  uint32_t clip_depth{0u};

  void on_clear(const DrawClearOp &op) noexcept {
    CGContextClearRect(ctx, bounds);
    if (op.color.a > 0u) {
      set_cg_fill_color(ctx, op.color);
      CGContextFillRect(ctx, bounds);
    }
  }

  void on_fill_rect(const DrawFillRectOp &op) noexcept {
    if (op.w <= 0.0f || op.h <= 0.0f || op.color.a == 0u) {
      return;
    }
    set_cg_fill_color(ctx, op.color);
    CGContextFillRect(ctx, CGRectMake(op.x, op.y, op.w, op.h));
  }

  void on_fill_rounded_rect(const DrawFillRoundedRectOp &op) noexcept {
    if (op.w <= 0.0f || op.h <= 0.0f || op.color.a == 0u) {
      return;
    }
    const CGRect rect = CGRectMake(op.x, op.y, op.w, op.h);
    const CGFloat maxR = std::min(rect.size.width, rect.size.height) * 0.5;
    const CGFloat r = std::clamp(static_cast<CGFloat>(op.radius), static_cast<CGFloat>(0.0), maxR);
    set_cg_fill_color(ctx, op.color);
    if (r <= 0.0) {
      CGContextFillRect(ctx, rect);
      return;
    }
    CGPathRef path = CGPathCreateWithRoundedRect(rect, r, r, nullptr);
    if (path != nullptr) {
      CGContextAddPath(ctx, path);
      CGContextFillPath(ctx);
      CGPathRelease(path);
    }
  }

  void on_fill_circle(const DrawFillCircleOp &op) noexcept {
    if (op.radius <= 0.0f || op.color.a == 0u) {
      return;
    }
    const CGFloat r = static_cast<CGFloat>(op.radius);
    const CGRect ellipse =
        CGRectMake(static_cast<CGFloat>(op.cx) - r, static_cast<CGFloat>(op.cy) - r, r * 2.0, r * 2.0);
    set_cg_fill_color(ctx, op.color);
    CGContextFillEllipseInRect(ctx, ellipse);
  }

  void on_stroke_line(const DrawStrokeLineOp &op) noexcept {
    if (op.width <= 0.0f || op.color.a == 0u) {
      return;
    }
    set_cg_stroke_color(ctx, op.color);
    CGContextSetLineWidth(ctx, static_cast<CGFloat>(op.width));
    CGContextSetLineCap(ctx, kCGLineCapRound);
    CGContextBeginPath(ctx);
    CGContextMoveToPoint(ctx, static_cast<CGFloat>(op.x0), static_cast<CGFloat>(op.y0));
    CGContextAddLineToPoint(ctx, static_cast<CGFloat>(op.x1), static_cast<CGFloat>(op.y1));
    CGContextStrokePath(ctx);
  }

  void on_stroke_polyline(const DrawStrokePolylineOp &op) noexcept {
    if (op.points.size() < 2u || op.width <= 0.0f || op.color.a == 0u) {
      return;
    }
    set_cg_stroke_color(ctx, op.color);
    CGContextSetLineWidth(ctx, static_cast<CGFloat>(op.width));
    CGContextSetLineCap(ctx, kCGLineCapRound);
    CGContextSetLineJoin(ctx, kCGLineJoinRound);
    CGContextBeginPath(ctx);
    const DrawPoint p0 = op.points[0u];
    CGContextMoveToPoint(ctx, static_cast<CGFloat>(p0.x), static_cast<CGFloat>(p0.y));
    for (size_t i = 1u; i < op.points.size(); ++i) {
      const DrawPoint pt = op.points[i];
      CGContextAddLineToPoint(ctx, static_cast<CGFloat>(pt.x), static_cast<CGFloat>(pt.y));
    }
    CGContextStrokePath(ctx);
  }

  void on_clip_rect(const DrawClipRectOp &op) noexcept {
    CGContextSaveGState(ctx);
    ++clip_depth;
    const CGFloat cw = op.w > 0.0f ? static_cast<CGFloat>(op.w) : 0.0;
    const CGFloat ch = op.h > 0.0f ? static_cast<CGFloat>(op.h) : 0.0;
    CGContextClipToRect(ctx, CGRectMake(static_cast<CGFloat>(op.x), static_cast<CGFloat>(op.y), cw, ch));
  }

  void on_restore_clip(const DrawRestoreClipOp &) noexcept {
    if (clip_depth > 0u) {
      CGContextRestoreGState(ctx);
      --clip_depth;
    }
  }
};

} // namespace

nve::DrawListDecodeResult render_draw_list_to_cg_context(
    CGContextRef _Nullable ctx,
    CGRect bounds,
    std::span<const uint8_t> bytes) noexcept {
  if (ctx == nullptr || bytes.empty()) {
    return nve::DrawListDecodeResult{nve::DrawListError::Ok, 0u, 0u};
  }
  const nve::DrawListDecodeResult check = nve::validate_draw_list(bytes);
  if (!check.ok()) {
    return check;
  }

  CGContextSaveGState(ctx);
  CGContextSetAllowsAntialiasing(ctx, true);
  CGContextSetShouldAntialias(ctx, true);

  CoreGraphicsDrawVisitor visitor{ctx, bounds, 0u};
  const nve::DrawListDecodeResult res = nve::decode_draw_list(bytes, visitor);
  while (visitor.clip_depth > 0u) {
    CGContextRestoreGState(ctx);
    --visitor.clip_depth;
  }
  CGContextRestoreGState(ctx);
  return res;
}

} // namespace nve::apple

@implementation NveAppleCanvasView {
  std::vector<uint8_t> _drawListBytes;
  uint64_t _redrawRequestCount;
}

- (instancetype)initWithFrame:(CGRect)frame {
  self = [super initWithFrame:frame];
  if (self) {
    _redrawRequestCount = 0u;
#if TARGET_OS_OSX
    self.wantsLayer = YES;
    self.layerContentsRedrawPolicy = NSViewLayerContentsRedrawOnSetNeedsDisplay;
#else
    self.opaque = NO;
    self.backgroundColor = [UIColor clearColor];
    self.contentMode = UIViewContentModeRedraw;
#endif
  }
  return self;
}

#if TARGET_OS_OSX
- (BOOL)isFlipped {
  return YES;
}

- (NSView *)hitTest:(NSPoint)point {
  return self.gestureRecognizers.count > 0 ? [super hitTest:point] : nil;
}

- (void)setFrameSize:(NSSize)newSize {
  const NSSize oldSize = self.frame.size;
  [super setFrameSize:newSize];
  if (!NSEqualSizes(oldSize, newSize) && !_drawListBytes.empty()) {
    [self setNeedsDisplay:YES];
  }
}
#endif

- (uint64_t)redrawRequestCount {
  return _redrawRequestCount;
}

- (BOOL)updateDrawListBytes:(std::span<const uint8_t>)bytes {
  if (_drawListBytes.size() == bytes.size() &&
      (bytes.empty() || std::memcmp(_drawListBytes.data(), bytes.data(), bytes.size()) == 0)) {
    return NO;
  }
  _drawListBytes.assign(bytes.begin(), bytes.end());
  ++_redrawRequestCount;
#if TARGET_OS_OSX
  [self setNeedsDisplay:YES];
#else
  [self setNeedsDisplay];
#endif
  return YES;
}

- (std::span<const uint8_t>)drawListBytes {
  return std::span<const uint8_t>(_drawListBytes.data(), _drawListBytes.size());
}

- (nve::DrawListDecodeResult)renderIntoContext:(CGContextRef)context bounds:(CGRect)bounds {
  return nve::apple::render_draw_list_to_cg_context(context, bounds, [self drawListBytes]);
}

- (void)drawRect:(CGRect)dirtyRect {
  (void)dirtyRect;
  if (_drawListBytes.empty()) {
    return;
  }
#if TARGET_OS_OSX
  CGContextRef ctx = [NSGraphicsContext currentContext].CGContext;
#else
  CGContextRef ctx = UIGraphicsGetCurrentContext();
#endif
  if (ctx == nullptr) {
    return;
  }
  (void)[self renderIntoContext:ctx bounds:self.bounds];
}

@end
