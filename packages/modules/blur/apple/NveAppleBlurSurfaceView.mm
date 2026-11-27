#import "NveAppleBlurSurfaceView.h"

#include <algorithm>
#include <cmath>

namespace nve::blur::apple {

#if !TARGET_OS_WATCH
NveBlurPlatformColor * _Nonnull color_from_rrggbbaa(uint32_t rrggbbaa) {
  const CGFloat r = static_cast<CGFloat>((rrggbbaa >> 24u) & 0xFFu) / 255.0;
  const CGFloat g = static_cast<CGFloat>((rrggbbaa >> 16u) & 0xFFu) / 255.0;
  const CGFloat b = static_cast<CGFloat>((rrggbbaa >> 8u) & 0xFFu) / 255.0;
  const CGFloat a = static_cast<CGFloat>(rrggbbaa & 0xFFu) / 255.0;
#if TARGET_OS_OSX
  return [NSColor colorWithSRGBRed:r green:g blue:b alpha:a];
#else
  return [UIColor colorWithRed:r green:g blue:b alpha:a];
#endif
}

namespace {

#if TARGET_OS_OSX
NSVisualEffectMaterial map_macos_blur_material(BlurMaterial mat) noexcept {
  switch (mat) {
    case BlurMaterial::UltraThin:
      return NSVisualEffectMaterialUnderPageBackground;
    case BlurMaterial::Thin:
      return NSVisualEffectMaterialPopover;
    case BlurMaterial::Regular:
      return NSVisualEffectMaterialSidebar;
    case BlurMaterial::Thick:
      return NSVisualEffectMaterialHeaderView;
    case BlurMaterial::Chrome:
      return NSVisualEffectMaterialTitlebar;
    case BlurMaterial::SystemMaterial:
      return NSVisualEffectMaterialContentBackground;
  }
  return NSVisualEffectMaterialSidebar;
}
#else
UIBlurEffectStyle map_ios_blur_style(BlurMaterial mat) noexcept {
  switch (mat) {
    case BlurMaterial::UltraThin:
      return UIBlurEffectStyleSystemUltraThinMaterial;
    case BlurMaterial::Thin:
      return UIBlurEffectStyleSystemThinMaterial;
    case BlurMaterial::Regular:
      return UIBlurEffectStyleSystemMaterial;
    case BlurMaterial::Thick:
      return UIBlurEffectStyleSystemThickMaterial;
    case BlurMaterial::Chrome:
      return UIBlurEffectStyleSystemChromeMaterial;
    case BlurMaterial::SystemMaterial:
      return UIBlurEffectStyleSystemMaterial;
  }
  return UIBlurEffectStyleSystemMaterial;
}
#endif

} // namespace
#endif // !TARGET_OS_WATCH

} // namespace nve::blur::apple

#if !TARGET_OS_WATCH
NS_ASSUME_NONNULL_BEGIN

@interface NveAppleBlurSurfaceView ()
@property (nonatomic, readwrite) nve::blur::apple::BlurMaterial nveBlurMaterial;
@property (nonatomic, readwrite) float nveBlurIntensity;
@property (nonatomic, readwrite) BOOL nveHasTint;
@property (nonatomic, readwrite) uint32_t nveTintRgba;
@end

@implementation NveAppleBlurSurfaceView {
#if TARGET_OS_OSX
  NSView *_tintOverlay;
#endif
}

- (instancetype)initWithFrame:(CGRect)frame {
#if TARGET_OS_OSX
  self = [super initWithFrame:frame];
  if (self) {
    self.wantsLayer = YES;
    self.blendingMode = NSVisualEffectBlendingModeWithinWindow;
    self.state = NSVisualEffectStateActive;
    self.material = NSVisualEffectMaterialSidebar;
    _tintOverlay = [[NSView alloc] initWithFrame:self.bounds];
    _tintOverlay.wantsLayer = YES;
    _tintOverlay.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
    [self addSubview:_tintOverlay];
  }
#else
  self = [super initWithEffect:[UIBlurEffect effectWithStyle:UIBlurEffectStyleSystemMaterial]];
  if (self) {
    self.frame = frame;
    self.clipsToBounds = YES;
  }
#endif
  if (self) {
    _nveBlurMaterial = nve::blur::apple::BlurMaterial::Regular;
    _nveBlurIntensity = 1.0f;
    _nveHasTint = NO;
    _nveTintRgba = 0u;
  }
  return self;
}

#if TARGET_OS_OSX
- (BOOL)isFlipped {
  return YES;
}
#endif

- (nullable NveBlurPlatformView *)nveTintOverlayView {
#if TARGET_OS_OSX
  return _tintOverlay;
#else
  return self.contentView;
#endif
}

- (void)applyBlurProps:(const nve::blur::apple::BlurSurfaceProps &)props {
  const float clamped =
      std::isfinite(props.intensity) ? std::clamp(props.intensity, 0.0f, 1.0f) : 1.0f;
  _nveBlurMaterial = props.material;
  _nveBlurIntensity = clamped;
  _nveHasTint = props.tint_rgba.has_value() ? YES : NO;
  _nveTintRgba = props.tint_rgba.value_or(0u);

#if TARGET_OS_OSX
  self.material = nve::blur::apple::map_macos_blur_material(props.material);
  self.alphaValue = static_cast<CGFloat>(clamped);
  if (_tintOverlay != nil) {
    _tintOverlay.frame = self.bounds;
    _tintOverlay.layer.backgroundColor =
        props.tint_rgba.has_value()
            ? nve::blur::apple::color_from_rrggbbaa(*props.tint_rgba).CGColor
            : [NSColor clearColor].CGColor;
  }
#else
  self.effect =
      [UIBlurEffect effectWithStyle:nve::blur::apple::map_ios_blur_style(props.material)];
  self.alpha = static_cast<CGFloat>(clamped);
  self.contentView.backgroundColor =
      props.tint_rgba.has_value()
          ? nve::blur::apple::color_from_rrggbbaa(*props.tint_rgba)
          : [UIColor clearColor];
#endif
}

@end

NS_ASSUME_NONNULL_END
#endif
