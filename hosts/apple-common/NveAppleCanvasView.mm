#import "NveAppleCanvasView.h"

#include <algorithm>
#include <cmath>
#include <cstring>
#include <vector>

namespace nve::apple {

namespace {

using namespace nve::canvas;

CGColorSpaceRef srgb_color_space() noexcept {
  static CGColorSpaceRef space = []() {
    CGColorSpaceRef cs = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
    return cs != nullptr ? cs : CGColorSpaceCreateDeviceRGB();
  }();
  return space;
}

inline void set_fill_rgba(CGContextRef ctx, DrawColorRgba c) noexcept {
  const CGFloat comps[4] = {
      static_cast<CGFloat>(c.r) / 255.0,
      static_cast<CGFloat>(c.g) / 255.0,
      static_cast<CGFloat>(c.b) / 255.0,
      static_cast<CGFloat>(c.a) / 255.0,
  };
  CGContextSetFillColor(ctx, comps);
}

inline void set_stroke_rgba(CGContextRef ctx, DrawColorRgba c) noexcept {
  const CGFloat comps[4] = {
      static_cast<CGFloat>(c.r) / 255.0,
      static_cast<CGFloat>(c.g) / 255.0,
      static_cast<CGFloat>(c.b) / 255.0,
      static_cast<CGFloat>(c.a) / 255.0,
  };
  CGContextSetStrokeColor(ctx, comps);
}

struct CGStateGuard {
  CGContextRef ctx;
  explicit CGStateGuard(CGContextRef c) noexcept : ctx(c) { CGContextSaveGState(ctx); }
  ~CGStateGuard() noexcept { CGContextRestoreGState(ctx); }
  CGStateGuard(const CGStateGuard&) = delete;
  CGStateGuard& operator=(const CGStateGuard&) = delete;
};

struct CGDrawListVisitor {
  CGContextRef ctx;
  CGRect local_bounds;

  void on_clear(const DrawClearOp& op) noexcept {
    CGStateGuard guard(ctx);
    CGContextSetBlendMode(ctx, kCGBlendModeCopy);
    set_fill_rgba(ctx, op.color);
    CGContextFillRect(ctx, local_bounds);
  }

  void on_fill_rect(const DrawFillRectOp& op) noexcept {
    if (op.w <= 0.0f || op.h <= 0.0f || op.color.a == 0u) return;
    set_fill_rgba(ctx, op.color);
    CGContextFillRect(ctx, CGRectMake(op.x, op.y, op.w, op.h));
  }

  void on_fill_rounded_rect(const DrawFillRoundedRectOp& op) noexcept {
    if (op.w <= 0.0f || op.h <= 0.0f || op.color.a == 0u) return;
    set_fill_rgba(ctx, op.color);
    const CGRect rect = CGRectMake(op.x, op.y, op.w, op.h);
    const CGFloat r = std::min<CGFloat>(
        {std::max<CGFloat>(0.0, op.radius), rect.size.width * 0.5, rect.size.height * 0.5});
    if (r <= 0.0) {
      CGContextFillRect(ctx, rect);
      return;
    }
    CGPathRef path = CGPathCreateWithRoundedRect(rect, r, r, nullptr);
    if (path == nullptr) return;
    CGContextAddPath(ctx, path);
    CGContextFillPath(ctx);
    CGPathRelease(path);
  }

  void on_fill_circle(const DrawFillCircleOp& op) noexcept {
    if (op.radius <= 0.0f || op.color.a == 0u) return;
    set_fill_rgba(ctx, op.color);
    const CGFloat d = static_cast<CGFloat>(op.radius) * 2.0;
    CGContextFillEllipseInRect(ctx, CGRectMake(op.cx - op.radius, op.cy - op.radius, d, d));
  }

  void on_stroke_line(const DrawStrokeLineOp& op) noexcept {
    if (op.width <= 0.0f || op.color.a == 0u) return;
    set_stroke_rgba(ctx, op.color);
    CGContextSetLineWidth(ctx, op.width);
    CGContextSetLineCap(ctx, kCGLineCapRound);
    CGContextSetLineJoin(ctx, kCGLineJoinRound);
    CGContextBeginPath(ctx);
    CGContextMoveToPoint(ctx, op.x0, op.y0);
    CGContextAddLineToPoint(ctx, op.x1, op.y1);
    CGContextStrokePath(ctx);
  }

  void on_stroke_polyline(const DrawStrokePolylineOp& op) noexcept {
    if (op.points.empty() || op.width <= 0.0f || op.color.a == 0u) return;
    if (op.points.size() == 1u) {
      set_fill_rgba(ctx, op.color);
      const DrawPoint p0 = op.points[0u];
      const CGFloat r = static_cast<CGFloat>(op.width) * 0.5;
      CGContextFillEllipseInRect(ctx, CGRectMake(p0.x - r, p0.y - r, op.width, op.width));
      return;
    }
    set_stroke_rgba(ctx, op.color);
    CGContextSetLineWidth(ctx, op.width);
    CGContextSetLineCap(ctx, kCGLineCapRound);
    CGContextSetLineJoin(ctx, kCGLineJoinRound);
    CGContextBeginPath(ctx);
    const DrawPoint p0 = op.points[0u];
    CGContextMoveToPoint(ctx, p0.x, p0.y);
    for (size_t i = 1u; i < op.points.size(); ++i) {
      const DrawPoint pt = op.points[i];
      CGContextAddLineToPoint(ctx, pt.x, pt.y);
    }
    CGContextStrokePath(ctx);
  }

  void on_clip_rect(const DrawClipRectOp& op) noexcept {
    CGContextSaveGState(ctx);
    CGContextClipToRect(
        ctx, CGRectMake(op.x, op.y, std::max(0.0f, op.w), std::max(0.0f, op.h)));
  }

  void on_restore_clip(const DrawRestoreClipOp&) noexcept {
    CGContextRestoreGState(ctx);
  }
};

}  // namespace

DrawListDecodeResult render_draw_list_to_cgcontext(
    CGContextRef ctx,
    CGRect bounds,
    std::span<const uint8_t> bytes) noexcept {
  if (bytes.empty()) return DrawListDecodeResult{DrawListError::Ok, 0u, 0u};
  const DrawListDecodeResult check = validate_draw_list(bytes);
  if (!check.ok()) return check;
  if (ctx == nullptr || !std::isfinite(bounds.origin.x) || !std::isfinite(bounds.origin.y) ||
      !std::isfinite(bounds.size.width) || !std::isfinite(bounds.size.height) ||
      !(bounds.size.width > 0.0) || !(bounds.size.height > 0.0)) {
    return check;
  }

  CGStateGuard guard(ctx);
  CGContextSetAllowsAntialiasing(ctx, true);
  CGContextSetShouldAntialias(ctx, true);
  CGContextClipToRect(ctx, bounds);
  CGContextTranslateCTM(ctx, bounds.origin.x, bounds.origin.y);
  CGColorSpaceRef space = srgb_color_space();
  CGContextSetFillColorSpace(ctx, space);
  CGContextSetStrokeColorSpace(ctx, space);

  CGDrawListVisitor visitor{ctx, CGRectMake(0.0, 0.0, bounds.size.width, bounds.size.height)};
  (void)stream_validated_draw_list(bytes, visitor);
  return check;
}

}  // namespace nve::apple

@implementation NveAppleCanvasView {
  std::vector<uint8_t> _drawList;
  uint64_t _redrawCount;
}

- (instancetype)initWithFrame:(CGRect)frame {
  if ((self = [super initWithFrame:frame])) {
#if TARGET_OS_OSX
    self.wantsLayer = YES;
#else
    self.opaque = NO;
    self.backgroundColor = UIColor.clearColor;
    self.contentMode = UIViewContentModeRedraw;
#endif
  }
  return self;
}

- (uint64_t)redrawCount {
  return _redrawCount;
}

- (BOOL)hasDrawList {
  return !_drawList.empty();
}

- (nullable NSData *)drawListData {
  if (_drawList.empty()) return nil;
  return [NSData dataWithBytes:_drawList.data() length:_drawList.size()];
}

- (void)setDrawListBytes:(nullable NSData *)bytes {
  const uint8_t *src = bytes != nil && bytes.length > 0u ? static_cast<const uint8_t *>(bytes.bytes) : nullptr;
  const size_t len = bytes != nil ? bytes.length : 0u;
  if (len == _drawList.size() && (len == 0u || std::memcmp(src, _drawList.data(), len) == 0)) {
    return;
  }
  if (len == 0u) {
    _drawList.clear();
  } else {
    _drawList.assign(src, src + len);
  }
  ++_redrawCount;
#if TARGET_OS_OSX
  [self setNeedsDisplay:YES];
#else
  [self setNeedsDisplay];
#endif
}

#if TARGET_OS_OSX
- (BOOL)isFlipped {
  return YES;
}

- (NSView *)hitTest:(NSPoint)point {
  return self.gestureRecognizers.count > 0 ? [super hitTest:point] : nil;
}

- (void)drawRect:(NSRect)dirtyRect {
  (void)dirtyRect;
  if (_drawList.empty()) return;
  CGContextRef ctx = NSGraphicsContext.currentContext.CGContext;
  (void)nve::apple::render_draw_list_to_cgcontext(ctx, self.bounds, _drawList);
}
#else
- (void)drawRect:(CGRect)rect {
  (void)rect;
  if (_drawList.empty()) return;
  CGContextRef ctx = UIGraphicsGetCurrentContext();
  (void)nve::apple::render_draw_list_to_cgcontext(ctx, self.bounds, _drawList);
}
#endif

@end
