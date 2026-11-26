#import "NveAppleComponentHostInternal.h"
#import "NveAppleValueBridge.h"

#import <objc/runtime.h>

#include <cmath>
#include <memory>
#include <span>
#include <string>
#include <vector>

@interface NveWeakAppleHostBox : NSObject
@property (nonatomic, weak) NveAppleComponentHost *host;
@end

@implementation NveWeakAppleHostBox
@end

namespace {

const void *const kNveAppleHostAssocKey = &kNveAppleHostAssocKey;

} // namespace

@implementation NveAppleComponentHost

+ (nullable NveAppleComponentHost *)hostForRootView:(NvePlatformView *)rootView {
  if (!rootView) return nil;
  NveWeakAppleHostBox *box = objc_getAssociatedObject(rootView, kNveAppleHostAssocKey);
  return box.host;
}

- (instancetype)initWithAdapter:(id<NveAppleComponentViewAdapter>)adapter
                    errorDomain:(NSString *)errorDomain {
  self = [super init];
  if (self) {
    _adapter = adapter;
    _errorDomain = [errorDomain copy];
    _reconciler = [[NveAppleSceneReconciler alloc] initWithAdapter:adapter];
    _pairedHost = nullptr;
    _pairedFrameCount = 0u;
    _frame0Host = nullptr;
    _dylibHandle = nullptr;
    _surfaceId = 1;
    NvePlatformView *rootView = [adapter nveHostRootView];
    if (rootView != nil) {
      NveWeakAppleHostBox *box = [[NveWeakAppleHostBox alloc] init];
      box.host = self;
      objc_setAssociatedObject(
          rootView, kNveAppleHostAssocKey, box, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    }
  }
  return self;
}

- (void)dealloc {
  [self disconnectPaired];
  // Destroy the bridge (which calls `surface->destroy`) before closing the dylib handle.
  _bridge.reset();
  if (_frame0Host != nullptr) {
    nve_fabric_host_destroy(_frame0Host);
    _frame0Host = nullptr;
  }
  nve::apple::close_dylib(_dylibHandle);
}

- (uint32_t)surfaceId {
  return _surfaceId;
}

- (uint32_t)lastReconciledDirtyCount {
  return _reconciler.lastReconciledDirtyCount;
}

- (uint32_t)lastReconciledRemovedCount {
  return _reconciler.lastReconciledRemovedCount;
}

- (NveFabricHostHandle)_activeFabricHost {
  if (_bridge) {
    return _bridge->fabric_host();
  }
  if (_pairedHost != nullptr) {
    return _pairedHost;
  }
  return _frame0Host;
}

- (BOOL)isPaired {
  return (_pairClient != nullptr && _pairedHost != nullptr) ? YES : NO;
}

- (BOOL)isPairedConnected {
  return (_pairClient != nullptr && _pairClient->is_connected()) ? YES : NO;
}

- (uint32_t)nodeCount {
  NveFabricHostHandle host = [self _activeFabricHost];
  return host ? nve_fabric_host_node_count(host) : 0;
}

- (BOOL)hasReplayedFrame0 {
  return _frame0Host != nullptr ? YES : NO;
}

- (BOOL)_installModule:(NveModule)mod
           dylibHandle:(void *)dylibHandle
             surfaceId:(uint32_t)surfaceId
         restoredState:(const std::vector<uint8_t> &)savedState
                 error:(NSError * _Nullable * _Nullable)error {
  id<NveAppleComponentViewAdapter> adapter = _adapter;
  if (!adapter || !mod.surface || !mod.surface->create) {
    nve::apple::close_dylib(dylibHandle);
    if (error) {
      *error = nve::apple::make_host_init_error(
          _errorDomain, @"Missing NVE_IFACE_SURFACE interface");
    }
    return NO;
  }

  const std::vector<uint8_t> &effectiveState =
      !savedState.empty() ? savedState : _pendingSnapshotState;

  // Transactional hot-swap: construct, restore state, and render the replacement bridge BEFORE
  // tearing down the currently running bridge, dylib, or mounted native views.
  __weak NveAppleComponentHost *weakSelf = self;
  auto nextBridge = std::make_unique<nve::NveRnComponentBridge>(
      mod,
      [adapter nveSurfaceConfigForSurfaceId:surfaceId],
      [weakSelf](const nve::DecodedEvent &ev) {
        __strong NveAppleComponentHost *strongSelf = weakSelf;
        if (!strongSelf) return;
        id<NveAppleComponentViewAdapter> strongAdapter = strongSelf->_adapter;
        if (!strongAdapter) return;
        NSString *eventName = [NSString stringWithUTF8String:ev.name.c_str()] ?: @"onCustom";
        NSDictionary<NSString *, id> *payload = nve::apple::payload_dictionary_from_event(ev);
        [strongAdapter nveEmitEvent:eventName payload:payload];
      });
  if (!nextBridge->is_valid()) {
    nextBridge.reset();
    nve::apple::close_dylib(dylibHandle);
    if (error) {
      *error = nve::apple::make_host_init_error(
          _errorDomain, @"Failed to initialize NVE surface instance");
    }
    return NO;
  }

  nve::apple::install_native_text_measurer(nextBridge->fabric_host());
  if (!effectiveState.empty()) {
    nextBridge->restore_state(
        NveBufferSlice{effectiveState.data(), effectiveState.size()}, /*auto_render=*/false);
  }

  NveFrameStats initialStats{};
  const int32_t rc = nextBridge->render(&initialStats);
  if (rc != 0) {
    nextBridge.reset();
    nve::apple::close_dylib(dylibHandle);
    if (error) {
      *error = nve::apple::make_host_init_error(
          _errorDomain,
          [NSString stringWithFormat:@"Initial frame render failed (rc=%d)", rc]);
    }
    return NO;
  }

  // Replacement is live and verified — now atomically tear down old state and commit.
  [self disconnectPaired];
  _bridge.reset();
  if (_frame0Host != nullptr) {
    nve_fabric_host_destroy(_frame0Host);
    _frame0Host = nullptr;
  }
  _pendingSnapshotState.clear();
  nve::apple::close_dylib(_dylibHandle);
  [_reconciler clearNativeNodeViews];

  _dylibHandle = dylibHandle;
  _surfaceId = surfaceId;
  _bridge = std::move(nextBridge);
  [_reconciler syncFromFabricHost:[self _activeFabricHost] hasMutations:YES];
  [self drainEmittedEvents];
  return YES;
}

- (BOOL)attachInterface:(NveGetInterfaceFn)getInterface surfaceId:(uint32_t)surfaceId {
  const NveModule mod = nve_resolve_module(getInterface);
  return [self _installModule:mod
                  dylibHandle:nullptr
                    surfaceId:surfaceId
                restoredState:{}
                        error:nil];
}

- (BOOL)loadDylibAtPath:(NSString *)dylibPath
              surfaceId:(uint32_t)surfaceId
                  error:(NSError * _Nullable * _Nullable)error {
  nve::apple::LoadedDylib loaded{};
  if (!nve::apple::open_nve_dylib(dylibPath, _errorDomain, &loaded, error)) {
    return NO;
  }
  return [self _installModule:loaded.module
                  dylibHandle:loaded.handle
                    surfaceId:surfaceId
                restoredState:{}
                        error:error];
}

- (BOOL)reloadDylibAtPath:(NSString *)dylibPath
                    error:(NSError * _Nullable * _Nullable)error {
  // Snapshot current state, validate + open + initialize replacement BEFORE tearing down old app.
  nve::apple::LoadedDylib loaded{};
  if (!nve::apple::open_nve_dylib(dylibPath, _errorDomain, &loaded, error)) {
    return NO;
  }
  std::vector<uint8_t> saved;
  if (_bridge) {
    saved = _bridge->snapshot_state();
  }
  return [self _installModule:loaded.module
                  dylibHandle:loaded.handle
                    surfaceId:_surfaceId
                restoredState:saved
                        error:error];
}

- (BOOL)connectPairedEndpoint:(NSString *)endpoint
                        token:(NSString *)token
                    transport:(nullable NSString *)transport
                    surfaceId:(uint32_t)surfaceId
                        error:(NSError * _Nullable * _Nullable)error {
  return nve::apple::host_connect_paired_endpoint(
      self, endpoint, token, transport, surfaceId, error);
}

- (BOOL)connectPairedConfig:(const nve::PairClientConfig &)config
                  surfaceId:(uint32_t)surfaceId
                      error:(NSError * _Nullable * _Nullable)error {
  return nve::apple::host_connect_paired_config(self, config, surfaceId, error);
}

- (void)disconnectPaired {
  nve::apple::host_disconnect_paired(self);
}

- (NSUInteger)pollPairedFrames {
  return nve::apple::host_poll_paired_frames(self);
}

- (BOOL)replayFrame0SnapshotAtPath:(NSString *)snapshotPath
             expectedSchemaVersion:(uint32_t)expectedSchemaVersion
                     restoreTimeUs:(uint64_t * _Nullable)outRestoreUs {
  return nve::apple::host_replay_frame0_snapshot(
      self, snapshotPath, expectedSchemaVersion, outRestoreUs);
}

- (BOOL)saveSnapshotAtPath:(NSString *)snapshotPath
             schemaVersion:(uint32_t)schemaVersion {
  return nve::apple::host_save_snapshot(self, snapshotPath, schemaVersion);
}

- (BOOL)applyPropsDictionary:(NSDictionary<NSString *, id> *)props {
  if (!_bridge || !props) return NO;
  _propsArena.reset();
  const NveValue mapVal = nve::apple::nve_value_from_objc_object(props, _propsArena, 0u);
  const NveMapEntry *entries = (mapVal.tag == NVE_VAL_MAP) ? mapVal.data.map_val.ptr : nullptr;
  const uint32_t count =
      (mapVal.tag == NVE_VAL_MAP) ? static_cast<uint32_t>(mapVal.data.map_val.len) : 0u;
  const bool dirty = _bridge->update_props(entries, count, /*auto_render=*/false);
  if (dirty) {
    [self renderFrame];
  } else {
    [self drainEmittedEvents];
  }
  return dirty ? YES : NO;
}

- (void)renderFrame {
  if (!_bridge) return;
  NveFrameStats stats{};
  const int32_t rc = _bridge->render(&stats);
  if (rc == 0) {
    [_reconciler syncFromFabricHost:[self _activeFabricHost]
                       hasMutations:(stats.mutation_count > 0u ? YES : NO)];
  }
  [self drainEmittedEvents];
}

- (void)drainEmittedEvents {
  if (!_bridge) return;
  (void)_bridge->drain_emitted_events();
}

- (void)_dispatchDirectEvent:(const NveDirectEvent &)ev {
  if (_bridge) {
    const uint32_t dirty =
        _bridge->dispatch_events(NveEventBatch{&ev, 1u, 0u}, /*auto_render=*/false);
    if (dirty != 0) {
      [self renderFrame];
    } else {
      [self drainEmittedEvents];
    }
  } else if (_pairClient && _pairedHost != nullptr) {
    std::vector<uint8_t> ring;
    if (nve::encode_direct_event_batch(NveEventBatch{&ev, 1u, 0u}, ring)) {
      (void)_pairClient->send_events(ring);
    }
  }
}

- (void)dispatchPressOnNodeId:(uint32_t)nodeId x:(float)x y:(float)y {
  if (!_bridge && !(_pairClient && _pairedHost != nullptr)) return;
  NveDirectEvent ev{};
  ev.surface_id = _surfaceId;
  ev.node_id = nodeId;
  ev.timestamp_us = nve::apple::current_timestamp_us();
  ev.kind = NVE_EVENT_PRESS;
  ev.payload_tag = NVE_EVENT_PAYLOAD_POINT;
  ev.x = x;
  ev.y = y;
  ev.value = nve::make_null_value();
  [self _dispatchDirectEvent:ev];
}

- (void)dispatchTextChangeOnNodeId:(uint32_t)nodeId text:(NSString *)text {
  if (!_bridge && !(_pairClient && _pairedHost != nullptr)) return;
  NSData *utf8 = [(text ?: @"") dataUsingEncoding:NSUTF8StringEncoding];
  const std::string_view sv =
      utf8 ? std::string_view(reinterpret_cast<const char *>(utf8.bytes), utf8.length)
           : std::string_view{};
  NveDirectEvent ev{};
  ev.surface_id = _surfaceId;
  ev.node_id = nodeId;
  ev.timestamp_us = nve::apple::current_timestamp_us();
  ev.kind = NVE_EVENT_TEXT_CHANGE;
  ev.payload_tag = NVE_EVENT_PAYLOAD_TEXT;
  ev.text = nve::make_str_slice(sv);
  ev.value = nve::make_null_value();
  [self _dispatchDirectEvent:ev];
}

- (void)dispatchSubmitOnNodeId:(uint32_t)nodeId text:(NSString *)text {
  if (!_bridge && !(_pairClient && _pairedHost != nullptr)) return;
  NSData *utf8 = [(text ?: @"") dataUsingEncoding:NSUTF8StringEncoding];
  const std::string_view sv =
      utf8 ? std::string_view(reinterpret_cast<const char *>(utf8.bytes), utf8.length)
           : std::string_view{};
  NveDirectEvent ev{};
  ev.surface_id = _surfaceId;
  ev.node_id = nodeId;
  ev.timestamp_us = nve::apple::current_timestamp_us();
  ev.kind = NVE_EVENT_SUBMIT;
  ev.payload_tag = NVE_EVENT_PAYLOAD_TEXT;
  ev.text = nve::make_str_slice(sv);
  ev.value = nve::make_null_value();
  [self _dispatchDirectEvent:ev];
}

- (void)dispatchFocusOnNodeId:(uint32_t)nodeId focused:(BOOL)focused {
  if (!_bridge && !(_pairClient && _pairedHost != nullptr)) return;
  NveDirectEvent ev{};
  ev.surface_id = _surfaceId;
  ev.node_id = nodeId;
  ev.timestamp_us = nve::apple::current_timestamp_us();
  ev.kind = static_cast<uint8_t>(focused ? nve::EventKind::Focus : nve::EventKind::Blur);
  ev.payload_tag = NVE_EVENT_PAYLOAD_EMPTY;
  ev.value = nve::make_null_value();
  [self _dispatchDirectEvent:ev];
}

- (void)dispatchScrollOnNodeId:(uint32_t)nodeId
                             x:(float)x
                             y:(float)y
                  contentWidth:(float)contentWidth
                 contentHeight:(float)contentHeight {
  if ((!_bridge && !(_pairClient && _pairedHost != nullptr)) || nodeId == 0u) return;
  NveDirectEvent ev{};
  ev.surface_id = _surfaceId;
  ev.node_id = nodeId;
  ev.timestamp_us = nve::apple::current_timestamp_us();
  ev.kind = static_cast<uint8_t>(nve::EventKind::Scroll);
  ev.payload_tag = NVE_EVENT_PAYLOAD_SCROLL;
  ev.x = std::isfinite(x) ? x : 0.0f;
  ev.y = std::isfinite(y) ? y : 0.0f;
  ev.width = (std::isfinite(contentWidth) && contentWidth >= 0.0f) ? contentWidth : 0.0f;
  ev.height = (std::isfinite(contentHeight) && contentHeight >= 0.0f) ? contentHeight : 0.0f;
  ev.value = nve::make_null_value();
  [self _dispatchDirectEvent:ev];
}

- (void)updateSurfaceWidth:(float)width height:(float)height scaleFactor:(float)scaleFactor {
  NveFabricHostHandle host = [self _activeFabricHost];
  if (!host) return;
  if (width > 0.0f && height > 0.0f) {
    if (_bridge) {
      _bridge->set_surface_size(width, height, scaleFactor);
    } else {
      nve::fabric_host_set_surface_size(host, width, height, scaleFactor);
    }
    [_reconciler syncFromFabricHost:host hasMutations:NO];
  }
}

- (BOOL)getRootLayoutSize:(CGSize *)outSize {
  NveFabricHostHandle host = [self _activeFabricHost];
  if (!host || !outSize) return NO;
  nve::LayoutRect rootRect{};
  if (nve::fabric_host_get_node_layout(host, NVE_ROOT_NODE_ID, &rootRect) &&
      rootRect.width > 0.0f && rootRect.height > 0.0f) {
    *outSize = CGSizeMake(rootRect.width, rootRect.height);
    return YES;
  }
  return NO;
}

- (nullable NvePlatformView *)nativeViewForNodeId:(uint32_t)nodeId {
  return [_reconciler nativeViewForNodeId:nodeId];
}

- (uint32_t)nodeIdForNativeView:(NvePlatformView *)view {
  return [_reconciler nodeIdForNativeView:view];
}

- (void)clearNativeNodeViews {
  [_reconciler clearNativeNodeViews];
}

- (void)syncNativeViewsFromHost {
  [_reconciler syncFromFabricHost:[self _activeFabricHost] hasMutations:NO];
}

@end
