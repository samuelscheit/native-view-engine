#pragma once

#import "NveAppleSupport.h"
#import <WebKit/WebKit.h>

#include "nve_native_view.hpp"

NS_ASSUME_NONNULL_BEGIN

#if TARGET_OS_OSX
/// AppKit native backdrop blur surface backed by `NSVisualEffectView` for `"nve.blur_surface"`.
@interface NveAppleBlurSurfaceView : NSVisualEffectView
#else
/// UIKit native backdrop blur surface backed by `UIVisualEffectView` for `"nve.blur_surface"`.
@interface NveAppleBlurSurfaceView : UIVisualEffectView
#endif
@property (nonatomic, readonly) nve::BlurMaterial nveBlurMaterial;
@property (nonatomic, readonly) float nveBlurIntensity;
@property (nonatomic, readonly) BOOL nveHasTint;
@property (nonatomic, readonly) uint32_t nveTintRgba;
@property (nonatomic, readonly, nullable) NvePlatformView *nveTintOverlayView;

- (void)applyBlurProps:(const nve::BlurSurfaceProps &)props;
@end

/// Shared Apple (AppKit & UIKit) native `WKWebView` adapter for `"nve.web_view"`.
@interface NveAppleWebView : WKWebView
@property (nonatomic, readonly, copy, nullable) NSString *nveLastRequestedUrl;
@property (nonatomic, readonly) BOOL nveAllowsBackForward;
@property (nonatomic, readonly) NSUInteger nveNavigationCount;
@property (nonatomic, readonly) NSUInteger nveReloadCount;
@property (nonatomic, readonly) NSUInteger nveGoBackCount;
@property (nonatomic, readonly) NSUInteger nveGoForwardCount;
@property (nonatomic, readonly, copy, nullable) NSString *nveLastEvaluatedScript;

+ (instancetype)makeDefaultWithFrame:(CGRect)frame;
- (void)applyWebViewProps:(const nve::WebViewProps &)props;
- (BOOL)dispatchWebViewCommand:(const nve::WebViewCommand &)cmd;
@end

NS_ASSUME_NONNULL_END

namespace nve::apple {

/// Returns `true` if `url` uses a safe scheme (`https://`, `http://`, or `about:blank`).
bool is_safe_apple_webview_url(std::string_view url) noexcept;

/// Converts a `0xRRGGBBAA` packed color (used by `BlurSurfaceProps::tint`) to `NSColor`/`UIColor`.
NvePlatformColor * _Nonnull color_from_rrggbbaa(uint32_t rrggbbaa);

/// Registers real AppKit/UIKit `BlurSurface` (`NSVisualEffectView`/`UIVisualEffectView`) and
/// `WebView` (`WKWebView`) mount adapters in `registry`.
void register_apple_native_views(nve::NativeViewRegistry &registry);

/// Binds Apple native view kinds registered in `registry` to `host` so `NVNP` raw props and
/// `NVNC` `DispatchCommand` frames flow directly to mounted platform views.
void bind_apple_native_views_to_fabric_host(
    NveFabricHostHandle _Nullable host,
    nve::NativeViewRegistry &registry);

/// Returns the mounted `NSView` / `UIView` instance tracked by `registry` for `node_id`, or `nil`.
NvePlatformView * _Nullable native_view_for_node(
    const nve::NativeViewRegistry &registry,
    uint32_t node_id);

} // namespace nve::apple
