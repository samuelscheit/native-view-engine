#pragma once

#import <Foundation/Foundation.h>
#import <QuartzCore/QuartzCore.h>
#import <TargetConditionals.h>

#if TARGET_OS_OSX
#import <AppKit/AppKit.h>
typedef NSView NveBlurPlatformView;
typedef NSColor NveBlurPlatformColor;
#elif !TARGET_OS_WATCH
#import <UIKit/UIKit.h>
typedef UIView NveBlurPlatformView;
typedef UIColor NveBlurPlatformColor;
#endif

#include <cstdint>
#include <optional>

namespace nve::blur::apple {

enum class BlurMaterial : uint32_t {
  UltraThin = 0,
  Thin = 1,
  Regular = 2,
  Thick = 3,
  Chrome = 4,
  SystemMaterial = 5,
};

struct BlurSurfaceProps {
  BlurMaterial material{BlurMaterial::Regular};
  float intensity{1.0f};
  std::optional<uint32_t> tint_rgba;
};

#if !TARGET_OS_WATCH
/// Converts a packed `0xRRGGBBAA` color to `NSColor` / `UIColor`.
NveBlurPlatformColor * _Nonnull color_from_rrggbbaa(uint32_t rrggbbaa);
#endif

} // namespace nve::blur::apple

#if !TARGET_OS_WATCH
NS_ASSUME_NONNULL_BEGIN

#if TARGET_OS_OSX
/// AppKit native backdrop blur surface backed by `NSVisualEffectView` for `nve.blur_surface`.
@interface NveAppleBlurSurfaceView : NSVisualEffectView
#else
/// UIKit native backdrop blur surface backed by `UIVisualEffectView` for `nve.blur_surface`.
@interface NveAppleBlurSurfaceView : UIVisualEffectView
#endif
@property (nonatomic, readonly) nve::blur::apple::BlurMaterial nveBlurMaterial;
@property (nonatomic, readonly) float nveBlurIntensity;
@property (nonatomic, readonly) BOOL nveHasTint;
@property (nonatomic, readonly) uint32_t nveTintRgba;
@property (nonatomic, readonly, nullable) NveBlurPlatformView *nveTintOverlayView;

- (void)applyBlurProps:(const nve::blur::apple::BlurSurfaceProps &)props;
@end

NS_ASSUME_NONNULL_END
#endif
