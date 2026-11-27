#import "NveAppleComponentHostInternal.h"

#include <cmath>

using nve::host::RuntimeSession;

static NSMapTable<NvePlatformView *, NveAppleComponentHost *> *nve_host_registry() {
  static NSMapTable *table = [NSMapTable weakToWeakObjectsMapTable];
  return table;
}

/// Weak host box handed to the runtime wake callback (fires on any thread; only dereferenced on
/// the main queue).
@interface NveAppleHostWakeBox : NSObject
@property (nonatomic, weak) NveAppleComponentHost *host;
@end
@implementation NveAppleHostWakeBox
@end

@implementation NveAppleComponentHost {
  /// Retained `NveAppleHostWakeBox` handed to the runtime; released only once the runtime has
  /// confirmed (by a successful clear) that no worker thread can still fire it.
  void *_wakeCtx;
  CFTimeInterval _lastTick;
}

- (instancetype)initWithAdapter:(id<NveAppleComponentViewAdapter>)adapter errorDomain:(NSString *)errorDomain {
  auto session = RuntimeSession::create(nve_runtime_get_interface(NVE_RUNTIME_ABI_VERSION));
  if (!session) return nil;
  if ((self = [super init])) {
    _adapter = adapter;
    _errorDomain = [errorDomain copy];
    _session = std::move(*session);
    __weak NveAppleComponentHost *weakSelf = self;
    _reconciler = [[NveAppleSceneReconciler alloc] initWithAdapter:adapter
                                                         resources:^NSData *(uint32_t resourceId) {
                                                           return [weakSelf _resourceBytes:resourceId];
                                                         }];
    (void)_session.set_text_measurer(&nve::apple::native_text_measure, nullptr);
    NveAppleHostWakeBox *box = [NveAppleHostWakeBox new];
    box.host = self;
    _wakeCtx = (void *)CFBridgingRetain(box);
    const NveRuntimeStatus wake = _session.set_wake_callback(
        [](void *ctx) {
          NveAppleHostWakeBox *b = (__bridge NveAppleHostWakeBox *)ctx;
          dispatch_async(dispatch_get_main_queue(), ^{
            [b.host _schedulePump];
          });
        },
        _wakeCtx);
    if (wake != NVE_RUNTIME_OK) {  // never installed: nothing can fire it
      CFBridgingRelease(_wakeCtx);
      _wakeCtx = nullptr;
    }
    [nve_host_registry() setObject:self forKey:[adapter nveHostRootView]];
  }
  return self;
}

- (void)dealloc {
  // A NULL `set_wake_callback` drains in-flight callbacks; only then may the box die. If clearing
  // fails (e.g. a non-owner thread), the box is leaked rather than freed under a firing worker.
  if (_wakeCtx != nullptr && _session.clear_wake_callback() == NVE_RUNTIME_OK) CFBridgingRelease(_wakeCtx);
  // Destroy the runtime (and the app it owns) before unloading the app's code.
  _session.reset();
  nve::apple::close_dylib(_dylibHandle);
}

- (BOOL)_deferIfReconciling:(dispatch_block_t)block {
  if (!_reconciling) return NO;
  dispatch_async(dispatch_get_main_queue(), block);
  return YES;
}

+ (NveAppleComponentHost *)hostForRootView:(NvePlatformView *)rootView {
  return [nve_host_registry() objectForKey:rootView];
}

- (NSError *)_errorWithCode:(NSInteger)code description:(NSString *)description {
  return [NSError errorWithDomain:_errorDomain code:code userInfo:@{NSLocalizedDescriptionKey : description}];
}

- (NSError *)_runtimeError:(NSString *)what {
  const std::string detail = _session.last_error();
  return [self _errorWithCode:static_cast<NSInteger>(nve::apple::DylibErrorCode::MissingSurfaceInterface)
                  description:[NSString stringWithFormat:@"%@: %s", what, detail.empty() ? "runtime error" : detail.c_str()]];
}

#pragma mark - Properties

- (uint32_t)surfaceId { return _surfaceId; }
- (BOOL)isAppAttached { return _appAttached; }
- (BOOL)isPaired { return _pairRequested; }
- (BOOL)isPairedConnected { return _session.pair_is_connected(); }
- (BOOL)hasReplayedFrame0 { return _hasReplayedFrame0; }
- (uint32_t)lastReconciledDirtyCount { return _reconciler.lastReconciledDirtyCount; }
- (uint32_t)lastReconciledRemovedCount { return _reconciler.lastReconciledRemovedCount; }

- (uint32_t)nodeCount {
  NveRuntimeMetrics m{};
  return _session.metrics(&m) == NVE_RUNTIME_OK ? m.live_nodes : 0u;
}

#pragma mark - App lifecycle

- (BOOL)_attachGetInterface:(NveGetInterfaceFn)getInterface error:(NSError **)error {
  if (_pairRequested) [self disconnectPaired];
  if (_session.attach_app_interface(getInterface) != NVE_RUNTIME_OK) {
    if (error) *error = [self _runtimeError:@"Failed to initialize NVE surface instance"];
    return NO;
  }
  _appAttached = getInterface != nullptr;  // NULL detaches
  if (!_pendingSnapshotState.empty()) {
    (void)_session.restore_state(_pendingSnapshotState);
    _pendingSnapshotState.clear();
  }
  [self renderFrame];
  return YES;
}

- (BOOL)attachInterface:(NveGetInterfaceFn)getInterface surfaceId:(uint32_t)surfaceId {
  _surfaceId = surfaceId;
  if (![self _attachGetInterface:getInterface error:nil]) return NO;
  nve::apple::close_dylib(_dylibHandle);  // a statically linked app replaces any loaded dylib
  return YES;
}

- (BOOL)loadDylibAtPath:(NSString *)dylibPath surfaceId:(uint32_t)surfaceId error:(NSError **)error {
  nve::apple::LoadedDylib lib;
  if (!nve::apple::open_nve_dylib(dylibPath, _errorDomain, &lib, error)) return NO;
  _surfaceId = surfaceId;
  if (![self _attachGetInterface:lib.get_interface error:error]) {
    nve::apple::close_dylib(lib.handle);
    return NO;
  }
  nve::apple::close_dylib(_dylibHandle);
  _dylibHandle = lib.handle;
  return YES;
}

#pragma mark - Props, input, frames

- (BOOL)applyPropsDictionary:(NSDictionary<NSString *, id> *)props {
  NSDictionary<NSString *, id> *copy = [props copy];
  if ([self _deferIfReconciling:^{ [self applyPropsDictionary:copy]; }]) return YES;
  _arena.reset();
  const std::vector<NveMapEntry> entries = nve::apple::map_entries_from_dictionary(props, _arena);
  bool changed = false;
  const NveRuntimeStatus st =
      _session.set_props(entries.empty() ? nullptr : entries.data(), static_cast<uint32_t>(entries.size()), &changed);
  _arena.reset();
  if (st != NVE_RUNTIME_OK) return NO;
  [self renderFrame];
  return YES;
}

- (BOOL)applyHotPatch:(NSData *)patchNvp1 {
  NSData *copy = [patchNvp1 copy];
  if ([self _deferIfReconciling:^{ [self applyHotPatch:copy]; }]) return YES;
  bool changed = false;
  const auto st = _session.apply_hot_patch(
      std::span<const uint8_t>(static_cast<const uint8_t *>(patchNvp1.bytes), patchNvp1.length), &changed);
  [self renderFrame];
  return st == NVE_RUNTIME_OK && changed;
}

- (void)_dispatch:(const NveDirectEvent &)ev {
  const NveEventBatch batch{&ev, 1u, 0u};
  (void)_session.dispatch_events(batch);
  _arena.reset();
  [self renderFrame];
}

- (void)dispatchPressOnNodeId:(uint32_t)nodeId x:(float)x y:(float)y {
  if ([self _deferIfReconciling:^{ [self dispatchPressOnNodeId:nodeId x:x y:y]; }]) return;
  [self _dispatch:nve::apple::make_point_event(_surfaceId, nodeId, NVE_EVENT_PRESS, x, y)];
}

- (void)dispatchTextChangeOnNodeId:(uint32_t)nodeId text:(NSString *)text {
  NSString *copy = [text copy];
  if ([self _deferIfReconciling:^{ [self dispatchTextChangeOnNodeId:nodeId text:copy]; }]) return;
  [self _dispatch:nve::apple::make_text_event(_surfaceId, nodeId, NVE_EVENT_TEXT_CHANGE, copy.UTF8String ?: "", _arena)];
}

- (void)dispatchSubmitOnNodeId:(uint32_t)nodeId text:(NSString *)text {
  NSString *copy = [text copy];
  if ([self _deferIfReconciling:^{ [self dispatchSubmitOnNodeId:nodeId text:copy]; }]) return;
  [self _dispatch:nve::apple::make_text_event(_surfaceId, nodeId, NVE_EVENT_SUBMIT, copy.UTF8String ?: "", _arena)];
}

- (void)dispatchFocusOnNodeId:(uint32_t)nodeId focused:(BOOL)focused {
  if ([self _deferIfReconciling:^{ [self dispatchFocusOnNodeId:nodeId focused:focused]; }]) return;
  NveDirectEvent ev = nve::apple::make_point_event(_surfaceId, nodeId, focused ? NVE_EVENT_FOCUS : NVE_EVENT_BLUR, 0.0f, 0.0f);
  ev.payload_tag = NVE_EVENT_PAYLOAD_EMPTY;
  [self _dispatch:ev];
}

- (void)dispatchScrollOnNodeId:(uint32_t)nodeId x:(float)x y:(float)y contentWidth:(float)cw contentHeight:(float)ch {
  if ([self _deferIfReconciling:^{ [self dispatchScrollOnNodeId:nodeId x:x y:y contentWidth:cw contentHeight:ch]; }]) return;
  [self _dispatch:nve::apple::make_scroll_event(_surfaceId, nodeId, x, y, cw, ch)];
}

- (void)_schedulePump {
  if (_pumpScheduled) return;
  _pumpScheduled = YES;
  __weak NveAppleComponentHost *weakSelf = self;
  dispatch_async(dispatch_get_main_queue(), ^{
    NveAppleComponentHost *strongSelf = weakSelf;
    if (strongSelf == nil) return;
    strongSelf->_pumpScheduled = NO;
    [strongSelf renderFrame];
  });
}

- (void)renderFrame {
  if (_reconciling) {  // view updates re-entered us: tick after this reconcile finishes
    [self _schedulePump];
    return;
  }
  const CFTimeInterval now = CACurrentMediaTime();
  const float dt = _lastTick == 0 ? 0.0f : static_cast<float>(std::min(now - _lastTick, 0.25));
  _lastTick = now;
  bool changed = false;
  if (_session.tick(dt, &changed) == NVE_RUNTIME_OK && changed) [self syncNativeViewsFromHost];
  [self drainEmittedEvents];
}

- (void)drainEmittedEvents {
  id<NveAppleComponentViewAdapter> adapter = _adapter;
  NveEventBatch batch{};
  if (adapter == nil || _session.drain_emitted_events(&batch) != NVE_RUNTIME_OK || batch.ptr == nullptr) return;
  // Copy out before emitting: handlers may call back into the runtime, invalidating the batch.
  NSMutableArray<NSArray *> *events = [NSMutableArray arrayWithCapacity:batch.len];
  for (uint32_t i = 0; i < batch.len; ++i) {
    [events addObject:@[ nve::apple::event_name(batch.ptr[i]), nve::apple::payload_dictionary_from_event(batch.ptr[i]) ]];
  }
  for (NSArray *e in events) [adapter nveEmitEvent:e[0] payload:e[1]];
}

#pragma mark - Layout

- (void)updateSurfaceWidth:(float)width height:(float)height scaleFactor:(float)scaleFactor {
  if (!(width > 0.0f) || !(height > 0.0f)) return;
  // A resize from a layout pass inside reconcile would fail with BUSY and be lost: apply it next turn.
  if ([self _deferIfReconciling:^{ [self updateSurfaceWidth:width height:height scaleFactor:scaleFactor]; }]) return;
  if (_session.set_viewport(width, height, scaleFactor > 0.0f ? scaleFactor : 1.0f) == NVE_RUNTIME_OK) [self renderFrame];
}

- (BOOL)getRootLayoutSize:(CGSize *)outSize {
  auto guard = _session.acquire_scene();
  if (!guard || (*guard)->rect(1u) == nullptr) return NO;
  const NveLayoutRect *r = (*guard)->rect(1u);
  *outSize = CGSizeMake(r->width, r->height);
  return r->width > 0.0f || r->height > 0.0f;
}

#pragma mark - Views

- (NvePlatformView *)nativeViewForNodeId:(uint32_t)nodeId { return [_reconciler nativeViewForNodeId:nodeId]; }
- (uint32_t)nodeIdForNativeView:(NvePlatformView *)view { return [_reconciler nodeIdForNativeView:view]; }
- (void)clearNativeNodeViews { [_reconciler clearNativeNodeViews]; }

- (void)syncNativeViewsFromHost {
  if (_reconciling) return;
  auto guard = _session.acquire_scene();
  if (!guard) return;
  _reconciling = YES;
  _activeScene = &*guard;
  [_reconciler syncFromScene:guard->reader()];
  _activeScene = nullptr;
  _reconciling = NO;
}

/// Payload of resource `id` from the scene being reconciled (copied: it borrows the guard).
- (NSData *)_resourceBytes:(uint32_t)resourceId {
  if (_activeScene == nullptr) return nil;
  const auto res = _activeScene->resource(resourceId);
  if (!res || res->bytes.empty()) return nil;
  return [NSData dataWithBytes:res->bytes.data() length:res->bytes.size()];
}

@end
