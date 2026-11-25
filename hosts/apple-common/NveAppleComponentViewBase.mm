#import "NveAppleComponentViewBase.h"

#import "NveAppleCanvasView.h"
#import "NveAppleComponentHost.h"
#import "NveAppleComponentHostInternal.h"
#import "NveAppleImageCache.h"
#import "NveAppleValueBridge.h"

#if defined(NVE_ENABLE_NATIVE_VIEWS) && NVE_ENABLE_NATIVE_VIEWS
#import "NveAppleNativeViews.h"
#endif

#include <memory>
#include <mutex>
#include <span>

namespace {

struct DefaultNativeViewHooks {
  std::mutex mu;
  nve::apple::NativeViewRegistrar registrar;
  nve::apple::NativeViewBinder binder;
};

DefaultNativeViewHooks &default_native_view_hooks() {
  static DefaultNativeViewHooks hooks;
  return hooks;
}

} // namespace

@implementation NveAppleImageView
#if TARGET_OS_OSX
- (NSView *)hitTest:(NSPoint)point {
  return self.gestureRecognizers.count > 0 ? [super hitTest:point] : nil;
}
#endif
@end

@interface NveAppleComponentViewBase () <NveAppleComponentViewAdapter>
@end

@implementation NveAppleComponentViewBase {
  NveAppleComponentHost *_nveHost;
  nve::NativeViewRegistry *_externalNativeViewRegistry;
  std::unique_ptr<nve::NativeViewRegistry> _ownedNativeViewRegistry;
  nve::apple::NativeViewBinder _nativeViewBinder;
}

- (instancetype)initWithFrame:(CGRect)frame {
  self = [super initWithFrame:frame];
  if (self) {
#if TARGET_OS_OSX
    self.wantsLayer = YES;
#endif
    _nveHost = [[NveAppleComponentHost alloc] initWithAdapter:self
                                                  errorDomain:[self nveErrorDomain]];
    [self _installDefaultNativeViewHooksIfConfigured];
    NveGetInterfaceFn defaultIface = [self defaultNveGetInterface];
    if (defaultIface != nullptr) {
      if ([_nveHost attachInterface:defaultIface surfaceId:_nveHost.surfaceId]) {
        [self bindNativeViewsToActiveFabricHost];
      }
    }
  }
  return self;
}

- (void)_installDefaultNativeViewHooksIfConfigured {
  nve::apple::NativeViewRegistrar registrar;
  nve::apple::NativeViewBinder binder;
  {
    auto &hooks = default_native_view_hooks();
    std::lock_guard<std::mutex> lock(hooks.mu);
    registrar = hooks.registrar;
    binder = hooks.binder;
  }
  if (registrar || binder) {
    [self installNativeViewHooksWithRegistrar:std::move(registrar) binder:std::move(binder)];
    return;
  }
#if defined(NVE_ENABLE_NATIVE_VIEWS) && NVE_ENABLE_NATIVE_VIEWS
  [self installNativeViewHooksWithRegistrar:&nve::apple::register_apple_native_views
                                     binder:&nve::apple::bind_apple_native_views_to_fabric_host];
#endif
}

- (nve::NativeViewRegistry * _Nullable)nativeViewRegistry {
  if (_externalNativeViewRegistry != nullptr) {
    return _externalNativeViewRegistry;
  }
  return _ownedNativeViewRegistry.get();
}

- (void)attachNativeViewRegistry:(nve::NativeViewRegistry * _Nullable)registry
                          binder:(nve::apple::NativeViewBinder)binder {
  _ownedNativeViewRegistry.reset();
  _externalNativeViewRegistry = registry;
  _nativeViewBinder = std::move(binder);
  [self bindNativeViewsToActiveFabricHost];
}

- (void)installNativeViewHooksWithRegistrar:(nve::apple::NativeViewRegistrar)registrar
                                     binder:(nve::apple::NativeViewBinder)binder {
  _externalNativeViewRegistry = nullptr;
  _ownedNativeViewRegistry = std::make_unique<nve::NativeViewRegistry>();
  if (registrar) {
    registrar(*_ownedNativeViewRegistry);
  }
  _nativeViewBinder = std::move(binder);
  [self bindNativeViewsToActiveFabricHost];
}

- (void)bindNativeViewsToActiveFabricHost {
  nve::NativeViewRegistry *registry = [self nativeViewRegistry];
  if (registry == nullptr || !_nativeViewBinder) {
    return;
  }
  NveFabricHostHandle host = [_nveHost _activeFabricHost];
  if (host == nullptr) {
    return;
  }
  _nativeViewBinder(host, *registry);
}

- (NveAppleComponentHost *)nveHost {
  return _nveHost;
}

- (NSString *)nveErrorDomain {
#if TARGET_OS_OSX
  return @"nve.macos.rncomponent";
#else
  return @"nve.ios.rncomponent";
#endif
}

- (CGSize)nveDefaultSurfaceSize {
#if TARGET_OS_OSX
  return CGSizeMake(480.0, 320.0);
#else
  return CGSizeMake(320.0, 200.0);
#endif
}

- (CGFloat)nveCurrentDisplayScale {
#if TARGET_OS_OSX
  if (self.window != nil && self.window.backingScaleFactor > 0.0) {
    return self.window.backingScaleFactor;
  }
  if ([NSScreen mainScreen] != nil && [NSScreen mainScreen].backingScaleFactor > 0.0) {
    return [NSScreen mainScreen].backingScaleFactor;
  }
  return 2.0;
#else
  const CGFloat scale = self.traitCollection.displayScale;
  return scale > 0.0 ? scale : 2.0;
#endif
}

- (void)propagateBoundsToNveHost {
  const CGRect b = self.bounds;
  [_nveHost updateSurfaceWidth:static_cast<float>(b.size.width)
                        height:static_cast<float>(b.size.height)
                   scaleFactor:static_cast<float>([self nveCurrentDisplayScale])];
}

- (uint32_t)nveSurfaceId {
  return _nveHost.surfaceId;
}

- (uint32_t)nveNodeCount {
  return _nveHost.nodeCount;
}

- (uint32_t)nveLastReconciledDirtyCount {
  return _nveHost.lastReconciledDirtyCount;
}

- (uint32_t)nveLastReconciledRemovedCount {
  return _nveHost.lastReconciledRemovedCount;
}

- (BOOL)isPaired {
  return _nveHost.isPaired;
}

- (BOOL)isPairedConnected {
  return _nveHost.isPairedConnected;
}

- (BOOL)hasReplayedFrame0 {
  return _nveHost.hasReplayedFrame0;
}

- (nullable NveGetInterfaceFn)defaultNveGetInterface {
  return nullptr;
}

- (BOOL)attachNveInterface:(NveGetInterfaceFn)getInterface surfaceId:(uint32_t)surfaceId {
  const BOOL ok = [_nveHost attachInterface:getInterface surfaceId:surfaceId];
  if (ok) {
    [self bindNativeViewsToActiveFabricHost];
  }
  return ok;
}

- (BOOL)loadNveDylibAtPath:(NSString *)dylibPath
                 surfaceId:(uint32_t)surfaceId
                     error:(NSError * _Nullable * _Nullable)error {
  const BOOL ok = [_nveHost loadDylibAtPath:dylibPath surfaceId:surfaceId error:error];
  if (ok) {
    [self bindNativeViewsToActiveFabricHost];
  }
  return ok;
}

- (BOOL)reloadNveDylibAtPath:(NSString *)dylibPath
                       error:(NSError * _Nullable * _Nullable)error {
  const BOOL ok = [_nveHost reloadDylibAtPath:dylibPath error:error];
  if (ok) {
    [self bindNativeViewsToActiveFabricHost];
  }
  return ok;
}

- (BOOL)connectPairedEndpoint:(NSString *)endpoint
                        token:(NSString *)token
                    transport:(nullable NSString *)transport
                    surfaceId:(uint32_t)surfaceId
                        error:(NSError * _Nullable * _Nullable)error {
  const BOOL ok = [_nveHost connectPairedEndpoint:endpoint
                                            token:token
                                        transport:transport
                                        surfaceId:surfaceId
                                            error:error];
  if (ok) {
    [self bindNativeViewsToActiveFabricHost];
  }
  return ok;
}

- (void)disconnectPaired {
  [_nveHost disconnectPaired];
}

- (BOOL)replayFrame0SnapshotAtPath:(NSString *)snapshotPath
             expectedSchemaVersion:(uint32_t)expectedSchemaVersion
                     restoreTimeUs:(uint64_t * _Nullable)outRestoreUs {
  const BOOL ok = [_nveHost replayFrame0SnapshotAtPath:snapshotPath
                                 expectedSchemaVersion:expectedSchemaVersion
                                         restoreTimeUs:outRestoreUs];
  if (ok) {
    [self bindNativeViewsToActiveFabricHost];
  }
  return ok;
}

- (BOOL)saveSnapshotAtPath:(NSString *)snapshotPath
             schemaVersion:(uint32_t)schemaVersion {
  return [_nveHost saveSnapshotAtPath:snapshotPath schemaVersion:schemaVersion];
}

- (BOOL)applyPropsJson:(NSString *)propsJson {
  return [_nveHost applyPropsJson:propsJson];
}

- (BOOL)applyPropsDictionary:(NSDictionary<NSString *, id> *)props {
  return [_nveHost applyPropsDictionary:props];
}

- (void)renderNveFrame {
  [_nveHost renderFrame];
}

- (void)drainNveEmittedEvents {
  [_nveHost drainEmittedEvents];
}

- (void)simulatePressOnNodeId:(uint32_t)nodeId atPoint:(CGPoint)point {
  [_nveHost dispatchPressOnNodeId:nodeId
                                x:static_cast<float>(point.x)
                                y:static_cast<float>(point.y)];
}

- (nullable NveAppleBasePlatformView *)nativeViewForNodeId:(uint32_t)nodeId {
  return [_nveHost nativeViewForNodeId:nodeId];
}

- (void)handleNveEvent:(NSString *)eventName payload:(NSDictionary<NSString *, id> *)payload {
  if (self.onNveEvent) {
    self.onNveEvent(eventName, payload);
  }
}

#if defined(NVE_WITH_RN_FABRIC) && NVE_WITH_RN_FABRIC
- (void)updateLayoutMetrics:(const facebook::react::LayoutMetrics &)layoutMetrics
           oldLayoutMetrics:(const facebook::react::LayoutMetrics &)oldLayoutMetrics {
  [super updateLayoutMetrics:layoutMetrics oldLayoutMetrics:oldLayoutMetrics];
  const auto &size = layoutMetrics.frame.size;
  [_nveHost updateSurfaceWidth:static_cast<float>(size.width)
                        height:static_cast<float>(size.height)
                   scaleFactor:static_cast<float>(layoutMetrics.pointScaleFactor)];
}

- (void)prepareForRecycle {
  [super prepareForRecycle];
  [_nveHost clearNativeNodeViews];
}
#endif

#pragma mark - NveAppleComponentViewAdapter (Shared Defaults)

- (NvePlatformView *)nveHostRootView {
  return self;
}

- (NveSurfaceConfig)nveSurfaceConfigForSurfaceId:(uint32_t)surfaceId {
  const CGRect bounds = self.bounds;
  const CGSize fallback = [self nveDefaultSurfaceSize];
  NveSurfaceConfig cfg{};
  cfg.surface_id = surfaceId;
  cfg.width = static_cast<float>(bounds.size.width > 0.0 ? bounds.size.width : fallback.width);
  cfg.height = static_cast<float>(bounds.size.height > 0.0 ? bounds.size.height : fallback.height);
  cfg.scale_factor = static_cast<float>([self nveCurrentDisplayScale]);
  return cfg;
}

- (void)nveEmitEvent:(NSString *)eventName payload:(NSDictionary<NSString *, id> *)payload {
  [self handleNveEvent:eventName payload:payload];
}

- (void)nveApplyRootBackgroundRgba:(uint32_t)rgba {
#if TARGET_OS_OSX
  self.wantsLayer = YES;
  if (rgba != 0) {
    self.layer.backgroundColor = nve::apple::color_from_rgba(rgba).CGColor;
  }
#else
  if (rgba != 0) {
    self.backgroundColor = nve::apple::color_from_rgba(rgba);
  }
#endif
}

- (NvePlatformView *)nveCreateNativeViewForNode:(const NveRenderNode &)node
                                      semantics:(const nve::apple::NodeSemantics &)semantics {
  (void)node;
  (void)semantics;
  return [[NvePlatformView alloc] initWithFrame:CGRectZero];
}

- (void)nveUpdateNativeView:(NvePlatformView *)view
                    forNode:(const NveRenderNode &)node
                       text:(std::string_view)text
                  semantics:(const nve::apple::NodeSemantics &)semantics
                interactive:(BOOL)interactive {
  (void)view;
  (void)node;
  (void)text;
  (void)semantics;
  (void)interactive;
}

@end

namespace nve::apple {

void set_default_native_view_hooks(NativeViewRegistrar registrar, NativeViewBinder binder) {
  auto &hooks = default_native_view_hooks();
  std::lock_guard<std::mutex> lock(hooks.mu);
  hooks.registrar = std::move(registrar);
  hooks.binder = std::move(binder);
}

void clear_default_native_view_hooks() {
  auto &hooks = default_native_view_hooks();
  std::lock_guard<std::mutex> lock(hooks.mu);
  hooks.registrar = nullptr;
  hooks.binder = nullptr;
}

void update_image_view(
    NveAppleImageView * _Nonnull imgView,
    std::string_view src,
    ImageResizeMode resizeMode,
    float borderRadius) {
  NSString *nextSrc = nsstring_or_nil_from_sv(src);
  const BOOL sameSrc =
      (nextSrc == nil && imgView.nveImageSrc == nil) ||
      (nextSrc != nil && [nextSrc isEqualToString:imgView.nveImageSrc]);
  if (!sameSrc) {
    imgView.nveImageSrc = nextSrc;
    imgView.image = src.empty() ? nil : shared_image_cache().image_for_source(src);
  }
  apply_image_resize_mode(imgView, resizeMode, borderRadius);
}

void update_canvas_view(NvePlatformView * _Nonnull view, std::string_view drawBytes) {
  if (![view isKindOfClass:[NveAppleCanvasView class]]) {
    return;
  }
  NveAppleCanvasView *canvasView = (NveAppleCanvasView *)view;
  const std::span<const uint8_t> bytes(
      reinterpret_cast<const uint8_t *>(drawBytes.data()), drawBytes.size());
  [canvasView updateDrawListBytes:bytes];
}

} // namespace nve::apple
