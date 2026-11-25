#pragma once

#import <Foundation/Foundation.h>
#import <QuartzCore/QuartzCore.h>
#import <TargetConditionals.h>

#ifndef NVE_NV_ENABLE_BLUR
#define NVE_NV_ENABLE_BLUR 1
#endif

#ifndef NVE_NV_ENABLE_WEBVIEW
#define NVE_NV_ENABLE_WEBVIEW 1
#endif

#ifndef NVE_NV_ENABLE_VIDEO
#define NVE_NV_ENABLE_VIDEO 1
#endif

#if TARGET_OS_OSX
#import <AppKit/AppKit.h>
typedef NSView NvePlatformView;
typedef NSColor NvePlatformColor;
#elif !TARGET_OS_WATCH
#import <UIKit/UIKit.h>
typedef UIView NvePlatformView;
typedef UIColor NvePlatformColor;
#endif

#if NVE_NV_ENABLE_BLUR && !TARGET_OS_WATCH
#define NVE_APPLE_HAS_BLUR 1
#else
#define NVE_APPLE_HAS_BLUR 0
#endif

// WebKit (`WKWebView`) is unavailable on watchOS and tvOS.
#if NVE_NV_ENABLE_WEBVIEW && !TARGET_OS_TV && !TARGET_OS_WATCH
#define NVE_APPLE_HAS_WEBVIEW 1
#import <WebKit/WebKit.h>
#else
#define NVE_APPLE_HAS_WEBVIEW 0
#endif

// `AVPlayerLayer` is unavailable on watchOS.
#if NVE_NV_ENABLE_VIDEO && !TARGET_OS_WATCH
#define NVE_APPLE_HAS_VIDEO 1
#import <AVFoundation/AVFoundation.h>
#else
#define NVE_APPLE_HAS_VIDEO 0
#endif

#include "nve_native_view.hpp"

#include <cstdint>
#include <string_view>

NS_ASSUME_NONNULL_BEGIN

#if NVE_APPLE_HAS_BLUR
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
#endif // NVE_APPLE_HAS_BLUR

#if NVE_APPLE_HAS_WEBVIEW
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
#endif // NVE_APPLE_HAS_WEBVIEW

#if NVE_APPLE_HAS_VIDEO
/// Shared Apple (AppKit & UIKit) native `AVPlayerLayer` adapter for `"nve.video_player"`.
@interface NveAppleVideoPlayerView : NvePlatformView
@property (nonatomic, strong, readonly) AVPlayer *nvePlayer;
@property (nonatomic, strong, readonly) AVPlayerLayer *nvePlayerLayer;
@property (nonatomic, readonly, copy, nullable) NSString *nveSourceUri;
@property (nonatomic, readonly) BOOL nveAutoplay;
@property (nonatomic, readonly) BOOL nveMuted;
@property (nonatomic, readonly) BOOL nveLooping;
@property (nonatomic, readonly) BOOL nveControls;
@property (nonatomic, readonly) BOOL nveIsPlaying;
@property (nonatomic, readonly) uint64_t nvePositionMs;
@property (nonatomic, readonly) float nveVolume;
@property (nonatomic, readonly) NSUInteger nvePlayCount;
@property (nonatomic, readonly) NSUInteger nvePauseCount;
@property (nonatomic, readonly) NSUInteger nveSeekCount;
@property (nonatomic, readonly) NSUInteger nveVolumeChangeCount;

- (void)applyVideoProps:(const nve::VideoPlayerProps &)props;
- (BOOL)dispatchVideoCommand:(const nve::VideoPlayerCommand &)cmd;
- (void)stopAndCleanup;
@end
#endif // NVE_APPLE_HAS_VIDEO

NS_ASSUME_NONNULL_END

namespace nve::apple {

#if !TARGET_OS_WATCH
/// Converts a `0xRRGGBBAA` packed color (used by `BlurSurfaceProps::tint`) to `NSColor`/`UIColor`.
NvePlatformColor * _Nonnull color_from_rrggbbaa(uint32_t rrggbbaa);
#endif

#if NVE_APPLE_HAS_WEBVIEW
/// Returns `true` if `url` uses a safe scheme (`https://`, `http://`, or `about:blank`).
bool is_safe_apple_webview_url(std::string_view url) noexcept;
#endif

#if NVE_APPLE_HAS_VIDEO
/// Returns `true` if `uri` uses a safe media scheme (`https://`, `http://`, or `file://`).
bool is_safe_apple_media_uri(std::string_view uri) noexcept;
#endif

/// Registers enabled Apple native view adapters (`BlurSurface`, `WebView`, `VideoPlayer`) in `registry`.
void register_apple_native_views(nve::NativeViewRegistry &registry);

/// Binds enabled Apple native view kinds registered in `registry` to `host` so `NVNP` raw props
/// and `NVNC` `DispatchCommand` frames flow directly to mounted platform views.
void bind_apple_native_views_to_fabric_host(
    NveFabricHostHandle _Nullable host,
    nve::NativeViewRegistry &registry);

#if !TARGET_OS_WATCH
/// Returns the mounted `NSView` / `UIView` instance tracked by `registry` for `node_id`, or `nil`.
NvePlatformView * _Nullable native_view_for_node(
    const nve::NativeViewRegistry &registry,
    uint32_t node_id);
#endif

#if NVE_APPLE_HAS_BLUR
/// Returns the mounted `NveAppleBlurSurfaceView` for `node_id` if its kind is blur, or `nil`.
NveAppleBlurSurfaceView * _Nullable blur_surface_for_node(
    const nve::NativeViewRegistry &registry,
    uint32_t node_id);
#endif

#if NVE_APPLE_HAS_WEBVIEW
/// Returns the mounted `NveAppleWebView` for `node_id` if its kind is webview, or `nil`.
NveAppleWebView * _Nullable web_view_for_node(
    const nve::NativeViewRegistry &registry,
    uint32_t node_id);
#endif

#if NVE_APPLE_HAS_VIDEO
/// Returns the mounted `NveAppleVideoPlayerView` for `node_id` if its kind is video, or `nil`.
NveAppleVideoPlayerView * _Nullable video_player_for_node(
    const nve::NativeViewRegistry &registry,
    uint32_t node_id);
#endif

} // namespace nve::apple
