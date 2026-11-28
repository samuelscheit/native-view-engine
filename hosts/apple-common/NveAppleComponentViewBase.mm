#import "NveAppleComponentViewBase.h"
#import "NveAppleComponentHost.h"

@implementation NveAppleImageView
#if TARGET_OS_OSX
- (NSView *)hitTest:(NSPoint)point {
  return self.gestureRecognizers.count > 0 ? [super hitTest:point] : nil;
}
#endif
@end

namespace nve::apple {

void update_image_view(NveAppleImageView *view, NvePlatformImage *image, ImageResizeMode fit, float border_radius) {
  if (view.image != image) view.image = image;
  apply_image_resize_mode(view, fit, border_radius);
}

}  // namespace nve::apple

@implementation NveAppleComponentViewBase {
  NveAppleComponentHost *_nveHost;
}

- (instancetype)initWithFrame:(CGRect)frame {
  if ((self = [super initWithFrame:frame])) {
#if TARGET_OS_OSX
    self.wantsLayer = YES;
#endif
    _nveHost = [[NveAppleComponentHost alloc] initWithAdapter:self errorDomain:[self nveErrorDomain]];
    if (NveGetInterfaceFn iface = [self defaultNveGetInterface]) {
      (void)[_nveHost attachInterface:iface surfaceId:_nveHost.surfaceId];
    }
  }
  return self;
}

- (NveAppleComponentHost *)nveHost { return _nveHost; }
- (NSString *)nveErrorDomain { return @"dev.nve.apple"; }
- (CGSize)nveDefaultSurfaceSize { return CGSizeMake(320.0, 240.0); }
- (nullable NveGetInterfaceFn)defaultNveGetInterface { return nullptr; }

- (CGFloat)nveCurrentDisplayScale {
#if TARGET_OS_OSX
  if (self.window != nil && self.window.backingScaleFactor > 0.0) return self.window.backingScaleFactor;
  if (NSScreen.mainScreen.backingScaleFactor > 0.0) return NSScreen.mainScreen.backingScaleFactor;
  return 2.0;
#else
  const CGFloat scale = self.traitCollection.displayScale;
  return scale > 0.0 ? scale : 2.0;
#endif
}

- (void)propagateBoundsToNveHost {
  CGSize size = self.bounds.size;
  if (size.width <= 0.0 || size.height <= 0.0) size = [self nveDefaultSurfaceSize];
  [_nveHost updateSurfaceWidth:static_cast<float>(size.width)
                        height:static_cast<float>(size.height)
                   scaleFactor:static_cast<float>([self nveCurrentDisplayScale])];
}

#if TARGET_OS_OSX
- (void)setFrameSize:(NSSize)newSize {
  [super setFrameSize:newSize];
  [self propagateBoundsToNveHost];
}
#else
- (void)layoutSubviews {
  [super layoutSubviews];
  [self propagateBoundsToNveHost];
}
#endif

- (uint32_t)nveSurfaceId { return _nveHost.surfaceId; }
- (uint32_t)nveNodeCount { return _nveHost.nodeCount; }
- (uint32_t)nveLastReconciledDirtyCount { return _nveHost.lastReconciledDirtyCount; }
- (uint32_t)nveLastReconciledRemovedCount { return _nveHost.lastReconciledRemovedCount; }
- (BOOL)isPaired { return _nveHost.isPaired; }
- (BOOL)isPairedConnected { return _nveHost.isPairedConnected; }
- (BOOL)hasReplayedFrame0 { return _nveHost.hasReplayedFrame0; }

- (BOOL)attachNveInterface:(NveGetInterfaceFn)getInterface surfaceId:(uint32_t)surfaceId {
  [self propagateBoundsToNveHost];
  return [_nveHost attachInterface:getInterface surfaceId:surfaceId];
}

- (BOOL)loadNveDylibAtPath:(NSString *)dylibPath surfaceId:(uint32_t)surfaceId error:(NSError **)error {
  if (_nveHost == nil) {
    if (error) *error = [NSError errorWithDomain:[self nveErrorDomain] code:3 userInfo:@{NSLocalizedDescriptionKey : @"libnve_runtime is not linked"}];
    return NO;
  }
  [self propagateBoundsToNveHost];
  return [_nveHost loadDylibAtPath:dylibPath surfaceId:surfaceId error:error];
}

- (BOOL)reloadNveDylibAtPath:(NSString *)dylibPath error:(NSError **)error {
  return [_nveHost reloadDylibAtPath:dylibPath error:error];
}

- (BOOL)connectPairedEndpoint:(NSString *)uri token:(NSString *)token surfaceId:(uint32_t)surfaceId error:(NSError **)error {
  [self propagateBoundsToNveHost];
  return [_nveHost connectPairedEndpoint:uri token:token surfaceId:surfaceId error:error];
}

- (void)disconnectPaired { [_nveHost disconnectPaired]; }

- (NSError *)_nveMissingRuntimeError {
  return [NSError errorWithDomain:[self nveErrorDomain]
                             code:static_cast<NSInteger>(NVE_RUNTIME_UNSUPPORTED)
                         userInfo:@{
                           NSLocalizedDescriptionKey : @"libnve_runtime is not linked",
                           NveAppleRuntimeStatusErrorKey : @(static_cast<NSInteger>(NVE_RUNTIME_UNSUPPORTED)),
                         }];
}

- (BOOL)saveFrame0ToPath:(NSString *)path appBuildId:(uint64_t)appBuildId envHash:(uint64_t)envHash error:(NSError **)error {
  if (_nveHost == nil) {
    if (error) *error = [self _nveMissingRuntimeError];
    return NO;
  }
  return [_nveHost saveFrame0ToPath:path appBuildId:appBuildId envHash:envHash error:error];
}

- (BOOL)bootFrame0FromPath:(NSString *)path appBuildId:(uint64_t)appBuildId envHash:(uint64_t)envHash error:(NSError **)error {
  if (_nveHost == nil) {
    if (error) *error = [self _nveMissingRuntimeError];
    return NO;
  }
  [self propagateBoundsToNveHost];
  return [_nveHost bootFrame0FromPath:path appBuildId:appBuildId envHash:envHash error:error];
}

- (BOOL)bootFrame0FromBytes:(NSData *)envelope appBuildId:(uint64_t)appBuildId envHash:(uint64_t)envHash error:(NSError **)error {
  if (_nveHost == nil) {
    if (error) *error = [self _nveMissingRuntimeError];
    return NO;
  }
  [self propagateBoundsToNveHost];
  return [_nveHost bootFrame0FromBytes:envelope appBuildId:appBuildId envHash:envHash error:error];
}

- (BOOL)applyPropsDictionary:(NSDictionary<NSString *, id> *)props { return [_nveHost applyPropsDictionary:props]; }
- (void)renderNveFrame { [_nveHost renderFrame]; }
- (void)drainNveEmittedEvents { [_nveHost drainEmittedEvents]; }

- (void)simulatePressOnNodeId:(uint32_t)nodeId atPoint:(CGPoint)point {
  [_nveHost dispatchPressOnNodeId:nodeId x:static_cast<float>(point.x) y:static_cast<float>(point.y)];
}

- (nullable NveAppleBasePlatformView *)nativeViewForNodeId:(uint32_t)nodeId { return [_nveHost nativeViewForNodeId:nodeId]; }

- (void)handleNveEvent:(NSString *)eventName payload:(NSDictionary<NSString *, id> *)payload {
  if (self.onNveEvent) self.onNveEvent(eventName, payload);
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

#pragma mark - NveAppleComponentViewAdapter (shared)

- (NvePlatformView *)nveHostRootView { return self; }

- (void)nveEmitEvent:(NSString *)eventName payload:(NSDictionary<NSString *, id> *)payload {
  [self handleNveEvent:eventName payload:payload];
}

- (NvePlatformView *)nveCreateNativeViewForNode:(const nve::apple::NodeView &)node {
  (void)node;
  return [[NvePlatformView alloc] initWithFrame:CGRectZero];
}

- (void)nveUpdateNativeView:(NvePlatformView *)view forNode:(const nve::apple::NodeView &)node image:(id)image {
  (void)image;
#if TARGET_OS_OSX
  view.wantsLayer = YES;
#endif
  nve::apple::apply_common_layer_style(view.layer, node.style);
  nve::apple::apply_node_accessibility(view, node.kind, node.semantics);
}

@end