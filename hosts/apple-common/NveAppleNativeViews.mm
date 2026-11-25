#import "NveAppleNativeViews.h"
#import "NveAppleCamera.h"

#include <algorithm>
#include <cctype>
#include <cmath>
#include <string>

namespace nve::apple {

bool is_safe_apple_webview_url(std::string_view url) noexcept {
  std::string norm;
  norm.reserve(url.size());
  for (char ch : url) {
    const unsigned char u = static_cast<unsigned char>(ch);
    if (u <= 0x20u || u == 0x7Fu) continue;
    norm.push_back(static_cast<char>(std::tolower(u)));
  }
  if (norm.empty()) return false;
  if (norm.starts_with("https://") || norm.starts_with("http://") || norm == "about:blank") {
    return true;
  }
  return false;
}

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

namespace {

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

} // namespace

} // namespace nve::apple

@implementation NveAppleBlurSurfaceView {
  nve::BlurMaterial _nveBlurMaterial;
  float _nveBlurIntensity;
  BOOL _nveHasTint;
  uint32_t _nveTintRgba;
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

- (nve::BlurMaterial)nveBlurMaterial {
  return _nveBlurMaterial;
}

- (float)nveBlurIntensity {
  return _nveBlurIntensity;
}

- (BOOL)nveHasTint {
  return _nveHasTint;
}

- (uint32_t)nveTintRgba {
  return _nveTintRgba;
}

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

@implementation NveAppleWebView {
  NSString *_nveLastRequestedUrl;
  BOOL _nveAllowsBackForward;
  NSUInteger _nveNavigationCount;
  NSUInteger _nveReloadCount;
  NSUInteger _nveGoBackCount;
  NSUInteger _nveGoForwardCount;
  NSString *_nveLastEvaluatedScript;
}

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

- (nullable NSString *)nveLastRequestedUrl {
  return _nveLastRequestedUrl;
}

- (BOOL)nveAllowsBackForward {
  return _nveAllowsBackForward;
}

- (NSUInteger)nveNavigationCount {
  return _nveNavigationCount;
}

- (NSUInteger)nveReloadCount {
  return _nveReloadCount;
}

- (NSUInteger)nveGoBackCount {
  return _nveGoBackCount;
}

- (NSUInteger)nveGoForwardCount {
  return _nveGoForwardCount;
}

- (nullable NSString *)nveLastEvaluatedScript {
  return _nveLastEvaluatedScript;
}

- (BOOL)_loadSafeUrl:(std::string_view)url {
  if (!nve::apple::is_safe_apple_webview_url(url)) {
    return NO;
  }
  NSString *nsStr = nve::apple::nsstring_from_sv(url);
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
  NSString *nextUrl = nve::apple::nsstring_from_sv(props.url);
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
      NSString *js = nve::apple::nsstring_from_sv(cmd.text);
      _nveLastEvaluatedScript = [js copy];
      [self evaluateJavaScript:js completionHandler:nil];
      return YES;
    }
  }
  return NO;
}

@end

namespace nve::apple {

namespace {

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

} // namespace

void register_apple_native_views(nve::NativeViewRegistry &registry) {
  registry.register_adapter(nve::kNativeKindBlurSurface, make_blur_adapter());
  registry.register_adapter(nve::kNativeKindBlurAlias, make_blur_adapter());
  registry.register_adapter(nve::kNativeKindWebView, make_webview_adapter());
  registry.register_adapter(nve::kNativeKindWebViewAlias, make_webview_adapter());
  register_apple_camera_native_view(registry);
}

void bind_apple_native_views_to_fabric_host(
    NveFabricHostHandle _Nullable host,
    nve::NativeViewRegistry &registry) {
  if (host == nullptr) return;
  registry.bind_to_fabric_host(host, nve::kNativeKindBlurSurface);
  registry.bind_to_fabric_host(host, nve::kNativeKindBlurAlias);
  registry.bind_to_fabric_host(host, nve::kNativeKindWebView);
  registry.bind_to_fabric_host(host, nve::kNativeKindWebViewAlias);
  bind_apple_camera_native_view_to_fabric_host(host, registry);
}

NvePlatformView * _Nullable native_view_for_node(
    const nve::NativeViewRegistry &registry,
    uint32_t node_id) {
  void *raw = registry.instance_for(node_id);
  return raw != nullptr ? (__bridge NvePlatformView *)raw : nil;
}

} // namespace nve::apple
