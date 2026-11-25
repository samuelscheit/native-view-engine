#import "NveAppleNativeViews.h"

#if NVE_APPLE_HAS_VIDEO
#import <CoreMedia/CoreMedia.h>
#endif

#include <algorithm>
#include <cctype>
#include <cmath>
#include <limits>
#include <string>

namespace nve::apple {

#if !TARGET_OS_WATCH
NvePlatformColor * _Nonnull color_from_rrggbbaa(uint32_t rrggbbaa) {
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
#endif

namespace {

#if NVE_APPLE_HAS_WEBVIEW || NVE_APPLE_HAS_VIDEO
std::string normalize_url_for_scheme_check(std::string_view url) {
  std::string norm;
  norm.reserve(url.size());
  for (char ch : url) {
    const unsigned char u = static_cast<unsigned char>(ch);
    if (u <= 0x20u || u == 0x7Fu) continue;
    norm.push_back(static_cast<char>(std::tolower(u)));
  }
  return norm;
}

NSString * _Nonnull nsstring_from_sv_local(std::string_view sv) {
  if (sv.empty()) {
    return @"";
  }
  NSString *str = [[NSString alloc] initWithBytes:sv.data()
                                           length:sv.size()
                                         encoding:NSUTF8StringEncoding];
  return str ?: @"";
}
#endif // NVE_APPLE_HAS_WEBVIEW || NVE_APPLE_HAS_VIDEO

#if NVE_APPLE_HAS_BLUR
bool is_blur_kind(std::string_view k) noexcept {
  return k == nve::kNativeKindBlurSurface || k == nve::kNativeKindBlurAlias;
}

#if TARGET_OS_OSX
NSVisualEffectMaterial map_macos_blur_material(nve::BlurMaterial mat) noexcept {
  switch (mat) {
    case nve::BlurMaterial::UltraThin:
      return NSVisualEffectMaterialUnderPageBackground;
    case nve::BlurMaterial::Thin:
      return NSVisualEffectMaterialPopover;
    case nve::BlurMaterial::Regular:
      return NSVisualEffectMaterialSidebar;
    case nve::BlurMaterial::Thick:
      return NSVisualEffectMaterialHeaderView;
    case nve::BlurMaterial::Chrome:
      return NSVisualEffectMaterialTitlebar;
    case nve::BlurMaterial::SystemMaterial:
      return NSVisualEffectMaterialContentBackground;
  }
  return NSVisualEffectMaterialSidebar;
}
#else
UIBlurEffectStyle map_ios_blur_style(nve::BlurMaterial mat) noexcept {
  switch (mat) {
    case nve::BlurMaterial::UltraThin:
      return UIBlurEffectStyleSystemUltraThinMaterial;
    case nve::BlurMaterial::Thin:
      return UIBlurEffectStyleSystemThinMaterial;
    case nve::BlurMaterial::Regular:
      return UIBlurEffectStyleSystemMaterial;
    case nve::BlurMaterial::Thick:
      return UIBlurEffectStyleSystemThickMaterial;
    case nve::BlurMaterial::Chrome:
      return UIBlurEffectStyleSystemChromeMaterial;
    case nve::BlurMaterial::SystemMaterial:
      return UIBlurEffectStyleSystemMaterial;
  }
  return UIBlurEffectStyleSystemMaterial;
}
#endif
#endif // NVE_APPLE_HAS_BLUR

#if NVE_APPLE_HAS_WEBVIEW
bool is_webview_kind(std::string_view k) noexcept {
  return k == nve::kNativeKindWebView || k == nve::kNativeKindWebViewAlias;
}
#endif

#if NVE_APPLE_HAS_VIDEO
bool is_video_kind(std::string_view k) noexcept {
  return k == nve::kNativeKindVideoPlayer || k == nve::kNativeKindVideoAlias;
}
#endif

} // namespace

#if NVE_APPLE_HAS_WEBVIEW
bool is_safe_apple_webview_url(std::string_view url) noexcept {
  const std::string norm = normalize_url_for_scheme_check(url);
  if (norm.empty()) return false;
  return norm.starts_with("https://") || norm.starts_with("http://") || norm == "about:blank";
}
#endif

#if NVE_APPLE_HAS_VIDEO
bool is_safe_apple_media_uri(std::string_view uri) noexcept {
  const std::string norm = normalize_url_for_scheme_check(uri);
  if (norm.empty()) return false;
  return norm.starts_with("https://") || norm.starts_with("http://") || norm.starts_with("file://");
}
#endif

} // namespace nve::apple

NS_ASSUME_NONNULL_BEGIN

// =============================================================================
// BlurSurface (NSVisualEffectView / UIVisualEffectView)
// =============================================================================

#if NVE_APPLE_HAS_BLUR
@interface NveAppleBlurSurfaceView ()
@property (nonatomic, readwrite) nve::BlurMaterial nveBlurMaterial;
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
    _nveBlurMaterial = nve::BlurMaterial::Regular;
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

- (nullable NvePlatformView *)nveTintOverlayView {
#if TARGET_OS_OSX
  return _tintOverlay;
#else
  return self.contentView;
#endif
}

- (void)applyBlurProps:(const nve::BlurSurfaceProps &)props {
  const float clamped =
      std::isfinite(props.intensity) ? std::clamp(props.intensity, 0.0f, 1.0f) : 1.0f;
  _nveBlurMaterial = props.material;
  _nveBlurIntensity = clamped;
  _nveHasTint = props.tint.has_value() ? YES : NO;
  _nveTintRgba = props.tint.value_or(0u);

#if TARGET_OS_OSX
  self.material = nve::apple::map_macos_blur_material(props.material);
  self.alphaValue = static_cast<CGFloat>(clamped);
  if (_tintOverlay != nil) {
    _tintOverlay.frame = self.bounds;
    _tintOverlay.layer.backgroundColor = props.tint.has_value()
                                             ? nve::apple::color_from_rrggbbaa(*props.tint).CGColor
                                             : [NSColor clearColor].CGColor;
  }
#else
  self.effect = [UIBlurEffect effectWithStyle:nve::apple::map_ios_blur_style(props.material)];
  self.alpha = static_cast<CGFloat>(clamped);
  self.contentView.backgroundColor =
      props.tint.has_value() ? nve::apple::color_from_rrggbbaa(*props.tint) : [UIColor clearColor];
#endif
}

@end
#endif // NVE_APPLE_HAS_BLUR

// =============================================================================
// WebView (WKWebView)
// =============================================================================

#if NVE_APPLE_HAS_WEBVIEW
@interface NveAppleWebView ()
@property (nonatomic, readwrite, copy, nullable) NSString *nveLastRequestedUrl;
@property (nonatomic, readwrite) BOOL nveAllowsBackForward;
@property (nonatomic, readwrite) NSUInteger nveNavigationCount;
@property (nonatomic, readwrite) NSUInteger nveReloadCount;
@property (nonatomic, readwrite) NSUInteger nveGoBackCount;
@property (nonatomic, readwrite) NSUInteger nveGoForwardCount;
@property (nonatomic, readwrite, copy, nullable) NSString *nveLastEvaluatedScript;
@end

@implementation NveAppleWebView

+ (instancetype)makeDefaultWithFrame:(CGRect)frame {
  WKWebViewConfiguration *cfg = [[WKWebViewConfiguration alloc] init];
  return [[self alloc] initWithFrame:frame configuration:cfg];
}

- (instancetype)initWithFrame:(CGRect)frame configuration:(WKWebViewConfiguration *)configuration {
  self = [super initWithFrame:frame configuration:configuration];
  if (self) {
    _nveAllowsBackForward = YES;
    self.allowsBackForwardNavigationGestures = YES;
  }
  return self;
}

- (BOOL)_loadSafeUrl:(std::string_view)url {
  if (!nve::apple::is_safe_apple_webview_url(url)) {
    return NO;
  }
  NSString *nsStr = nve::apple::nsstring_from_sv_local(url);
  NSURL *nsUrl = [NSURL URLWithString:nsStr];
  if (nsUrl == nil) {
    return NO;
  }
  _nveLastRequestedUrl = [nsStr copy];
  ++_nveNavigationCount;
  [self loadRequest:[NSURLRequest requestWithURL:nsUrl]];
  return YES;
}

- (void)applyWebViewProps:(const nve::WebViewProps &)props {
  _nveAllowsBackForward = props.allows_back_forward ? YES : NO;
  self.allowsBackForwardNavigationGestures = _nveAllowsBackForward;
  NSString *nextUrl = nve::apple::nsstring_from_sv_local(props.url);
  if (_nveLastRequestedUrl == nil || ![_nveLastRequestedUrl isEqualToString:nextUrl]) {
    (void)[self _loadSafeUrl:props.url];
  }
}

- (BOOL)dispatchWebViewCommand:(const nve::WebViewCommand &)cmd {
  switch (cmd.kind) {
    case nve::WebViewCommandKind::Navigate:
      return [self _loadSafeUrl:cmd.text];
    case nve::WebViewCommandKind::Reload:
      ++_nveReloadCount;
      [self reload];
      return YES;
    case nve::WebViewCommandKind::GoBack:
      if (!_nveAllowsBackForward) return NO;
      ++_nveGoBackCount;
      if (self.canGoBack) {
        [self goBack];
      }
      return YES;
    case nve::WebViewCommandKind::GoForward:
      if (!_nveAllowsBackForward) return NO;
      ++_nveGoForwardCount;
      if (self.canGoForward) {
        [self goForward];
      }
      return YES;
    case nve::WebViewCommandKind::EvaluateScript: {
      NSString *js = nve::apple::nsstring_from_sv_local(cmd.text);
      _nveLastEvaluatedScript = [js copy];
      [self evaluateJavaScript:js completionHandler:nil];
      return YES;
    }
  }
  return NO;
}

@end
#endif // NVE_APPLE_HAS_WEBVIEW

// =============================================================================
// VideoPlayer (AVPlayer + AVPlayerLayer)
// =============================================================================

#if NVE_APPLE_HAS_VIDEO
@interface NveAppleVideoPlayerView ()
@property (nonatomic, strong, readwrite) AVPlayer *nvePlayer;
@property (nonatomic, strong, readwrite) AVPlayerLayer *nvePlayerLayer;
@property (nonatomic, readwrite, copy, nullable) NSString *nveSourceUri;
@property (nonatomic, readwrite) BOOL nveAutoplay;
@property (nonatomic, readwrite) BOOL nveMuted;
@property (nonatomic, readwrite) BOOL nveLooping;
@property (nonatomic, readwrite) BOOL nveControls;
@property (nonatomic, readwrite) BOOL nveIsPlaying;
@property (nonatomic, readwrite) uint64_t nvePositionMs;
@property (nonatomic, readwrite) float nveVolume;
@property (nonatomic, readwrite) NSUInteger nvePlayCount;
@property (nonatomic, readwrite) NSUInteger nvePauseCount;
@property (nonatomic, readwrite) NSUInteger nveSeekCount;
@property (nonatomic, readwrite) NSUInteger nveVolumeChangeCount;
@end

@implementation NveAppleVideoPlayerView

#if !TARGET_OS_OSX
+ (Class)layerClass {
  return [AVPlayerLayer class];
}
#endif

- (instancetype)initWithFrame:(CGRect)frame {
  self = [super initWithFrame:frame];
  if (self) {
    _nvePlayer = [[AVPlayer alloc] init];
    _nvePlayer.actionAtItemEnd = AVPlayerActionAtItemEndPause;
#if TARGET_OS_OSX
    _nvePlayerLayer = [AVPlayerLayer playerLayerWithPlayer:_nvePlayer];
    _nvePlayerLayer.videoGravity = AVLayerVideoGravityResizeAspect;
    self.layer = _nvePlayerLayer;
    self.wantsLayer = YES;
    _nvePlayerLayer.frame = self.bounds;
#else
    _nvePlayerLayer = (AVPlayerLayer *)self.layer;
    _nvePlayerLayer.player = _nvePlayer;
    _nvePlayerLayer.videoGravity = AVLayerVideoGravityResizeAspect;
    self.clipsToBounds = YES;
#endif
    _nveControls = YES;
    _nveVolume = 1.0f;
  }
  return self;
}

- (void)dealloc {
  [[NSNotificationCenter defaultCenter] removeObserver:self];
}

#if TARGET_OS_OSX
- (BOOL)isFlipped {
  return YES;
}

- (void)layout {
  [super layout];
  _nvePlayerLayer.frame = self.bounds;
}
#endif

- (void)_handleItemDidReachEnd:(NSNotification *)note {
  if (!_nveLooping || note.object != _nvePlayer.currentItem) {
    return;
  }
  [_nvePlayer seekToTime:kCMTimeZero];
  [_nvePlayer play];
}

- (void)applyVideoProps:(const nve::VideoPlayerProps &)props {
  NSString *nextUri = nve::apple::nsstring_from_sv_local(props.source_uri);
  const BOOL uriChanged = (_nveSourceUri == nil || ![_nveSourceUri isEqualToString:nextUri]);
  if (uriChanged && nve::apple::is_safe_apple_media_uri(props.source_uri)) {
    NSURL *url = [NSURL URLWithString:nextUri];
    if (url != nil) {
      [[NSNotificationCenter defaultCenter] removeObserver:self
                                                      name:AVPlayerItemDidPlayToEndTimeNotification
                                                    object:_nvePlayer.currentItem];
      _nveSourceUri = [nextUri copy];
      AVPlayerItem *item = [AVPlayerItem playerItemWithURL:url];
      [_nvePlayer replaceCurrentItemWithPlayerItem:item];
      [[NSNotificationCenter defaultCenter] addObserver:self
                                               selector:@selector(_handleItemDidReachEnd:)
                                                   name:AVPlayerItemDidPlayToEndTimeNotification
                                                 object:item];
    }
  }
  _nveAutoplay = props.autoplay ? YES : NO;
  _nveMuted = props.muted ? YES : NO;
  _nveLooping = props.looping ? YES : NO;
  _nveControls = props.controls ? YES : NO;
  _nvePlayer.muted = _nveMuted;
  _nvePlayer.actionAtItemEnd =
      _nveLooping ? AVPlayerActionAtItemEndNone : AVPlayerActionAtItemEndPause;
  if (_nveAutoplay && _nveSourceUri.length > 0 && !_nveIsPlaying) {
    _nveIsPlaying = YES;
    ++_nvePlayCount;
    [_nvePlayer play];
  }
}

- (BOOL)dispatchVideoCommand:(const nve::VideoPlayerCommand &)cmd {
  switch (cmd.kind) {
    case nve::VideoPlayerCommandKind::Play:
      _nveIsPlaying = YES;
      ++_nvePlayCount;
      [_nvePlayer play];
      return YES;
    case nve::VideoPlayerCommandKind::Pause:
      _nveIsPlaying = NO;
      ++_nvePauseCount;
      [_nvePlayer pause];
      return YES;
    case nve::VideoPlayerCommandKind::SeekMs: {
      _nvePositionMs = cmd.seek_ms;
      ++_nveSeekCount;
      const uint64_t capped = std::min<uint64_t>(
          cmd.seek_ms, static_cast<uint64_t>(std::numeric_limits<int64_t>::max()));
      [_nvePlayer seekToTime:CMTimeMake(static_cast<int64_t>(capped), 1000)];
      return YES;
    }
    case nve::VideoPlayerCommandKind::SetVolume: {
      if (!std::isfinite(cmd.volume)) return NO;
      _nveVolume = std::clamp(cmd.volume, 0.0f, 1.0f);
      ++_nveVolumeChangeCount;
      _nvePlayer.volume = _nveVolume;
      return YES;
    }
  }
  return NO;
}

- (void)stopAndCleanup {
  [[NSNotificationCenter defaultCenter] removeObserver:self];
  _nveIsPlaying = NO;
  [_nvePlayer pause];
  [_nvePlayer replaceCurrentItemWithPlayerItem:nil];
}

@end
#endif // NVE_APPLE_HAS_VIDEO

NS_ASSUME_NONNULL_END

// =============================================================================
// Registry Adapters & Lookup Helpers
// =============================================================================

namespace nve::apple {

namespace {

#if NVE_APPLE_HAS_BLUR
nve::NativeViewAdapter make_blur_adapter() {
  nve::NativeViewAdapter adapter{};
  adapter.create = [](uint32_t, std::string_view, const nve::NvnpDecoder *) -> void * {
    NveAppleBlurSurfaceView *view = [[NveAppleBlurSurfaceView alloc] initWithFrame:CGRectZero];
    return (__bridge_retained void *)view;
  };
  adapter.update_props = [](void *inst, uint32_t, const nve::NvnpDecoder &dec) {
    if (inst == nullptr) return;
    nve::BlurSurfaceProps props{};
    if (nve::decode_blur_surface_props(dec, props) == nve::NativeViewCodecError::Ok) {
      NveAppleBlurSurfaceView *view = (__bridge NveAppleBlurSurfaceView *)inst;
      [view applyBlurProps:props];
    }
  };
  adapter.unmount = [](void *inst, uint32_t) {
    if (inst == nullptr) return;
    NveAppleBlurSurfaceView *view = (__bridge_transfer NveAppleBlurSurfaceView *)inst;
    [view removeFromSuperview];
  };
  return adapter;
}
#endif

#if NVE_APPLE_HAS_WEBVIEW
nve::NativeViewAdapter make_webview_adapter() {
  nve::NativeViewAdapter adapter{};
  adapter.create = [](uint32_t, std::string_view, const nve::NvnpDecoder *) -> void * {
    NveAppleWebView *view = [NveAppleWebView makeDefaultWithFrame:CGRectZero];
    return (__bridge_retained void *)view;
  };
  adapter.update_props = [](void *inst, uint32_t, const nve::NvnpDecoder &dec) {
    if (inst == nullptr) return;
    nve::WebViewProps props{};
    if (nve::decode_web_view_props(dec, props) == nve::NativeViewCodecError::Ok) {
      NveAppleWebView *view = (__bridge NveAppleWebView *)inst;
      [view applyWebViewProps:props];
    }
  };
  adapter.dispatch_command =
      [](void *inst, uint32_t, std::string_view, const nve::NvncDecoder &dec) -> bool {
    if (inst == nullptr) return false;
    nve::WebViewCommand cmd{};
    if (nve::decode_web_view_command(dec, cmd) != nve::NativeViewCodecError::Ok) {
      return false;
    }
    NveAppleWebView *view = (__bridge NveAppleWebView *)inst;
    return [view dispatchWebViewCommand:cmd] ? true : false;
  };
  adapter.unmount = [](void *inst, uint32_t) {
    if (inst == nullptr) return;
    NveAppleWebView *view = (__bridge_transfer NveAppleWebView *)inst;
    [view stopLoading];
    [view removeFromSuperview];
  };
  return adapter;
}
#endif

#if NVE_APPLE_HAS_VIDEO
nve::NativeViewAdapter make_video_adapter() {
  nve::NativeViewAdapter adapter{};
  adapter.create = [](uint32_t, std::string_view, const nve::NvnpDecoder *) -> void * {
    NveAppleVideoPlayerView *view = [[NveAppleVideoPlayerView alloc] initWithFrame:CGRectZero];
    return (__bridge_retained void *)view;
  };
  adapter.update_props = [](void *inst, uint32_t, const nve::NvnpDecoder &dec) {
    if (inst == nullptr) return;
    nve::VideoPlayerProps props{};
    if (nve::decode_video_player_props(dec, props) == nve::NativeViewCodecError::Ok) {
      NveAppleVideoPlayerView *view = (__bridge NveAppleVideoPlayerView *)inst;
      [view applyVideoProps:props];
    }
  };
  adapter.dispatch_command =
      [](void *inst, uint32_t, std::string_view, const nve::NvncDecoder &dec) -> bool {
    if (inst == nullptr) return false;
    nve::VideoPlayerCommand cmd{};
    if (nve::decode_video_player_command(dec, cmd) != nve::NativeViewCodecError::Ok) {
      return false;
    }
    NveAppleVideoPlayerView *view = (__bridge NveAppleVideoPlayerView *)inst;
    return [view dispatchVideoCommand:cmd] ? true : false;
  };
  adapter.unmount = [](void *inst, uint32_t) {
    if (inst == nullptr) return;
    NveAppleVideoPlayerView *view = (__bridge_transfer NveAppleVideoPlayerView *)inst;
    [view stopAndCleanup];
    [view removeFromSuperview];
  };
  return adapter;
}
#endif

} // namespace

void register_apple_native_views(nve::NativeViewRegistry &registry) {
  (void)registry;
#if NVE_APPLE_HAS_BLUR
  registry.register_adapter(nve::kNativeKindBlurSurface, make_blur_adapter());
  registry.register_adapter(nve::kNativeKindBlurAlias, make_blur_adapter());
#endif
#if NVE_APPLE_HAS_WEBVIEW
  registry.register_adapter(nve::kNativeKindWebView, make_webview_adapter());
  registry.register_adapter(nve::kNativeKindWebViewAlias, make_webview_adapter());
#endif
#if NVE_APPLE_HAS_VIDEO
  registry.register_adapter(nve::kNativeKindVideoPlayer, make_video_adapter());
  registry.register_adapter(nve::kNativeKindVideoAlias, make_video_adapter());
#endif
}

void bind_apple_native_views_to_fabric_host(
    NveFabricHostHandle _Nullable host,
    nve::NativeViewRegistry &registry) {
  (void)registry;
  if (host == nullptr) return;
#if NVE_APPLE_HAS_BLUR
  registry.bind_to_fabric_host(host, nve::kNativeKindBlurSurface);
  registry.bind_to_fabric_host(host, nve::kNativeKindBlurAlias);
#endif
#if NVE_APPLE_HAS_WEBVIEW
  registry.bind_to_fabric_host(host, nve::kNativeKindWebView);
  registry.bind_to_fabric_host(host, nve::kNativeKindWebViewAlias);
#endif
#if NVE_APPLE_HAS_VIDEO
  registry.bind_to_fabric_host(host, nve::kNativeKindVideoPlayer);
  registry.bind_to_fabric_host(host, nve::kNativeKindVideoAlias);
#endif
}

#if !TARGET_OS_WATCH
NvePlatformView * _Nullable native_view_for_node(
    const nve::NativeViewRegistry &registry,
    uint32_t node_id) {
  void *raw = registry.instance_for(node_id);
  return raw != nullptr ? (__bridge NvePlatformView *)raw : nil;
}
#endif

#if NVE_APPLE_HAS_BLUR
NveAppleBlurSurfaceView * _Nullable blur_surface_for_node(
    const nve::NativeViewRegistry &registry,
    uint32_t node_id) {
  if (!is_blur_kind(registry.kind_for(node_id))) return nil;
  void *raw = registry.instance_for(node_id);
  return raw != nullptr ? (__bridge NveAppleBlurSurfaceView *)raw : nil;
}
#endif

#if NVE_APPLE_HAS_WEBVIEW
NveAppleWebView * _Nullable web_view_for_node(
    const nve::NativeViewRegistry &registry,
    uint32_t node_id) {
  if (!is_webview_kind(registry.kind_for(node_id))) return nil;
  void *raw = registry.instance_for(node_id);
  return raw != nullptr ? (__bridge NveAppleWebView *)raw : nil;
}
#endif

#if NVE_APPLE_HAS_VIDEO
NveAppleVideoPlayerView * _Nullable video_player_for_node(
    const nve::NativeViewRegistry &registry,
    uint32_t node_id) {
  if (!is_video_kind(registry.kind_for(node_id))) return nil;
  void *raw = registry.instance_for(node_id);
  return raw != nullptr ? (__bridge NveAppleVideoPlayerView *)raw : nil;
}
#endif

} // namespace nve::apple
