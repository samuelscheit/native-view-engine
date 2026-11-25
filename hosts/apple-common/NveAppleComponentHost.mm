#import "NveAppleComponentHost.h"

#import <CoreFoundation/CoreFoundation.h>

#include "nve_snapshot.hpp"
#include "nve_value.hpp"

#include <chrono>
#include <cmath>
#include <memory>
#include <vector>

namespace {

NSError *make_init_error(NSString *domain, NSString *description) {
  return [NSError errorWithDomain:domain
                             code:static_cast<NSInteger>(nve::apple::DylibErrorCode::MissingSurfaceInterface)
                         userInfo:@{NSLocalizedDescriptionKey : description}];
}

NveValue nve_value_from_objc_object(id _Nullable obj, nve::BumpArena &arena, uint32_t depth) {
  if (obj == nil || obj == [NSNull null] || depth > NVE_BIN_VALUE_MAX_DEPTH) {
    return nve::make_null_value();
  }
  if ([obj isKindOfClass:[NSNumber class]]) {
    NSNumber *num = (NSNumber *)obj;
    if (CFGetTypeID((__bridge CFTypeRef)num) == CFBooleanGetTypeID()) {
      return nve::make_bool_value(num.boolValue ? true : false);
    }
    if (CFNumberIsFloatType((__bridge CFNumberRef)num)) {
      const double d = num.doubleValue;
      return std::isfinite(d) ? nve::make_f64_value(d) : nve::make_null_value();
    }
    return nve::make_i64_value(static_cast<int64_t>(num.longLongValue));
  }
  if ([obj isKindOfClass:[NSString class]]) {
    NSString *str = (NSString *)obj;
    const char *utf8 = str.UTF8String;
    if (!utf8) {
      return nve::make_str_value(NveStrSlice{nullptr, 0});
    }
    return nve::make_str_value(arena.copy_str(std::string_view(utf8)));
  }
  if ([obj isKindOfClass:[NSData class]]) {
    NSData *data = (NSData *)obj;
    return nve::make_bytes_value(
        arena.copy_bytes(static_cast<const uint8_t *>(data.bytes), data.length));
  }
  if ([obj isKindOfClass:[NSArray class]]) {
    NSArray *arr = (NSArray *)obj;
    const size_t n = arr.count;
    NveValue *elems = n > 0 ? arena.alloc_array<NveValue>(n) : nullptr;
    for (size_t i = 0; i < n; ++i) {
      elems[i] = nve_value_from_objc_object(arr[i], arena, depth + 1u);
    }
    return nve::make_array_value(elems, n);
  }
  if ([obj isKindOfClass:[NSDictionary class]]) {
    NSDictionary *dict = (NSDictionary *)obj;
    const size_t n = dict.count;
    NveMapEntry *entries = n > 0 ? arena.alloc_array<NveMapEntry>(n) : nullptr;
    __block size_t idx = 0;
    [dict enumerateKeysAndObjectsUsingBlock:^(id key, id val, BOOL *stop) {
      (void)stop;
      if (idx >= n) return;
      NSString *keyStr = [key isKindOfClass:[NSString class]] ? (NSString *)key : [key description];
      const char *kutf8 = keyStr.UTF8String;
      entries[idx++] = NveMapEntry{
          kutf8 ? arena.copy_str(std::string_view(kutf8)) : NveStrSlice{nullptr, 0},
          nve_value_from_objc_object(val, arena, depth + 1u),
      };
    }];
    return nve::make_map_value(entries, idx);
  }
  return nve::make_null_value();
}

} // namespace

@implementation NveAppleComponentHost {
  __weak id<NveAppleComponentViewAdapter> _adapter;
  NSString *_errorDomain;
  NveAppleSceneReconciler *_reconciler;
  std::unique_ptr<nve::NveRnComponentBridge> _bridge;
  NveFabricHostHandle _frame0Host;
  std::vector<uint8_t> _pendingSnapshotState;
  void *_dylibHandle;
  uint32_t _surfaceId;
  nve::BumpArena _propsArena;
}

- (instancetype)initWithAdapter:(id<NveAppleComponentViewAdapter>)adapter
                    errorDomain:(NSString *)errorDomain {
  self = [super init];
  if (self) {
    _adapter = adapter;
    _errorDomain = [errorDomain copy];
    _reconciler = [[NveAppleSceneReconciler alloc] initWithAdapter:adapter];
    _frame0Host = nullptr;
    _dylibHandle = nullptr;
    _surfaceId = 1;
  }
  return self;
}

- (void)dealloc {
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
  return _frame0Host;
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
      *error = make_init_error(_errorDomain, @"Missing NVE_IFACE_SURFACE interface");
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
      *error = make_init_error(_errorDomain, @"Failed to initialize NVE surface instance");
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
      *error = make_init_error(
          _errorDomain,
          [NSString stringWithFormat:@"Initial frame render failed (rc=%d)", rc]);
    }
    return NO;
  }

  // Replacement is live and verified — now atomically tear down old state and commit.
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

- (BOOL)replayFrame0SnapshotAtPath:(NSString *)snapshotPath
             expectedSchemaVersion:(uint32_t)expectedSchemaVersion
                     restoreTimeUs:(uint64_t * _Nullable)outRestoreUs {
  id<NveAppleComponentViewAdapter> adapter = _adapter;
  if (!adapter || snapshotPath.length == 0) {
    return NO;
  }
  const char *utf8 = snapshotPath.UTF8String;
  if (utf8 == nullptr) {
    return NO;
  }

  using Clock = std::chrono::steady_clock;
  const auto t0 = Clock::now();

  nve::MmapSnapshot snap;
  if (!nve::MmapSnapshot::open_with_schema(
          std::string_view(utf8), expectedSchemaVersion, snap, /*verify_checksum=*/true)) {
    return NO;
  }

  const NveBufferSlice frame0 = snap.frame0_slice();
  if (frame0.ptr == nullptr || frame0.len == 0u) {
    return NO;
  }
  const NveBufferSlice state = snap.state_slice();

  if (_bridge) {
    if (nve_fabric_host_submit_frame(_bridge->fabric_host(), frame0, nullptr) != 0) {
      return NO;
    }
    if (state.ptr != nullptr && state.len > 0u) {
      _bridge->restore_state(state, /*auto_render=*/false);
    }
  } else {
    const NveSurfaceConfig cfg = [adapter nveSurfaceConfigForSurfaceId:_surfaceId];
    NveFabricHostHandle nextHost = nve_fabric_host_create(cfg);
    if (nextHost == nullptr) {
      return NO;
    }
    nve::apple::install_native_text_measurer(nextHost);
    if (nve_fabric_host_submit_frame(nextHost, frame0, nullptr) != 0) {
      nve_fabric_host_destroy(nextHost);
      return NO;
    }
    if (_frame0Host != nullptr) {
      nve_fabric_host_destroy(_frame0Host);
    }
    _frame0Host = nextHost;
    if (state.ptr != nullptr && state.len > 0u) {
      _pendingSnapshotState.assign(state.ptr, state.ptr + state.len);
    } else {
      _pendingSnapshotState.clear();
    }
  }

  [_reconciler clearNativeNodeViews];
  [_reconciler syncFromFabricHost:[self _activeFabricHost] hasMutations:YES];

  const auto t1 = Clock::now();
  if (outRestoreUs != nullptr) {
    *outRestoreUs = static_cast<uint64_t>(
        std::chrono::duration_cast<std::chrono::microseconds>(t1 - t0).count());
  }
  return YES;
}

- (BOOL)saveSnapshotAtPath:(NSString *)snapshotPath
             schemaVersion:(uint32_t)schemaVersion {
  if (snapshotPath.length == 0) {
    return NO;
  }
  const char *utf8 = snapshotPath.UTF8String;
  if (utf8 == nullptr) {
    return NO;
  }
  NveFabricHostHandle host = [self _activeFabricHost];
  if (host == nullptr) {
    return NO;
  }
  const uint64_t frameId = (_bridge && _bridge->frame_count() > 0u) ? _bridge->frame_count() : 1u;
  const std::vector<uint8_t> frame0 = nve::fabric_host_serialize_frame0(host, frameId);
  if (frame0.empty()) {
    return NO;
  }
  const std::vector<uint8_t> state = _bridge ? _bridge->snapshot_state() : _pendingSnapshotState;
  return nve::write_snapshot_atomic(
             std::string_view(utf8),
             schemaVersion,
             NveBufferSlice{frame0.data(), frame0.size()},
             NveBufferSlice{state.empty() ? nullptr : state.data(), state.size()})
             ? YES
             : NO;
}

- (BOOL)applyPropsJson:(NSString *)propsJson {
  if (!_bridge) return NO;
  NSData *utf8 = [propsJson dataUsingEncoding:NSUTF8StringEncoding];
  if (!utf8) return NO;
  const std::string_view sv(reinterpret_cast<const char *>(utf8.bytes), utf8.length);
  const bool dirty = _bridge->update_props_json(sv, /*auto_render=*/false);
  if (dirty) {
    [self renderFrame];
  } else {
    [self drainEmittedEvents];
  }
  return dirty ? YES : NO;
}

- (BOOL)applyPropsDictionary:(NSDictionary<NSString *, id> *)props {
  if (!_bridge || !props) return NO;
  _propsArena.reset();
  const NveValue mapVal = nve_value_from_objc_object(props, _propsArena, 0u);
  const NveMapEntry *entries = (mapVal.tag == NVE_VAL_MAP) ? mapVal.data.map_val.ptr : nullptr;
  const uint32_t count =
      (mapVal.tag == NVE_VAL_MAP) ? static_cast<uint32_t>(mapVal.data.map_val.len) : 0u;
  const bool dirty = _bridge->update_props_direct(entries, count, /*auto_render=*/false);
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

- (void)_dispatchInboundEventsAndRender {
  const NveBufferSlice inbound = nve_fabric_host_drain_events(_bridge->fabric_host());
  const uint32_t dirty = _bridge->dispatch_native_events(inbound, /*auto_render=*/false);
  if (dirty != 0) {
    [self renderFrame];
  } else {
    [self drainEmittedEvents];
  }
}

- (void)dispatchPressOnNodeId:(uint32_t)nodeId x:(float)x y:(float)y {
  if (!_bridge) return;
  nve::fabric_host_enqueue_press(
      _bridge->fabric_host(), nodeId, nve::apple::current_timestamp_us(), x, y);
  [self _dispatchInboundEventsAndRender];
}

- (void)dispatchTextChangeOnNodeId:(uint32_t)nodeId text:(NSString *)text {
  if (!_bridge) return;
  NSData *utf8 = [(text ?: @"") dataUsingEncoding:NSUTF8StringEncoding];
  const std::string_view sv =
      utf8 ? std::string_view(reinterpret_cast<const char *>(utf8.bytes), utf8.length)
           : std::string_view{};
  nve::fabric_host_enqueue_text_change(
      _bridge->fabric_host(), nodeId, nve::apple::current_timestamp_us(), sv);
  [self _dispatchInboundEventsAndRender];
}

- (void)dispatchSubmitOnNodeId:(uint32_t)nodeId text:(NSString *)text {
  if (!_bridge) return;
  NSData *utf8 = [(text ?: @"") dataUsingEncoding:NSUTF8StringEncoding];
  const std::string_view sv =
      utf8 ? std::string_view(reinterpret_cast<const char *>(utf8.bytes), utf8.length)
           : std::string_view{};
  NveDirectEvent ev{};
  ev.surface_id = _surfaceId;
  ev.node_id = nodeId;
  ev.timestamp_us = nve::apple::current_timestamp_us();
  ev.kind = static_cast<uint8_t>(nve::EventKind::Custom);
  ev.payload_tag = NVE_EVENT_PAYLOAD_CUSTOM;
  ev.name = nve::make_str_slice("onSubmit");
  ev.text = nve::make_str_slice(sv);
  ev.value = nve::make_str_value(sv);
  const uint32_t dirty =
      _bridge->dispatch_direct_events(NveEventBatch{&ev, 1u}, /*auto_render=*/false);
  if (dirty != 0) {
    [self renderFrame];
  } else {
    [self drainEmittedEvents];
  }
}

- (void)dispatchFocusOnNodeId:(uint32_t)nodeId focused:(BOOL)focused {
  if (!_bridge) return;
  NveDirectEvent ev{};
  ev.surface_id = _surfaceId;
  ev.node_id = nodeId;
  ev.timestamp_us = nve::apple::current_timestamp_us();
  ev.kind = static_cast<uint8_t>(focused ? nve::EventKind::Focus : nve::EventKind::Blur);
  ev.payload_tag = NVE_EVENT_PAYLOAD_EMPTY;
  ev.value = nve::make_null_value();
  const uint32_t dirty =
      _bridge->dispatch_direct_events(NveEventBatch{&ev, 1u}, /*auto_render=*/false);
  if (dirty != 0) {
    [self renderFrame];
  } else {
    [self drainEmittedEvents];
  }
}

- (void)dispatchScrollOnNodeId:(uint32_t)nodeId
                             x:(float)x
                             y:(float)y
                  contentWidth:(float)contentWidth
                 contentHeight:(float)contentHeight {
  if (!_bridge || nodeId == 0u) return;
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
  const uint32_t dirty =
      _bridge->dispatch_direct_events(NveEventBatch{&ev, 1u}, /*auto_render=*/false);
  if (dirty != 0) {
    [self renderFrame];
  } else {
    [self drainEmittedEvents];
  }
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
