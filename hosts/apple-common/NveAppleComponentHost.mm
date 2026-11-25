#import "NveAppleComponentHost.h"

#include <memory>
#include <unordered_map>
#include <unordered_set>
#include <vector>

@implementation NveAppleComponentHost {
  __weak id<NveAppleComponentViewAdapter> _adapter;
  NSString *_errorDomain;
  std::unique_ptr<nve::NveRnComponentBridge> _bridge;
  void *_dylibHandle;
  uint32_t _surfaceId;
  std::unordered_map<uint32_t, NvePlatformView *> _nodeViews;
  std::unordered_map<uint32_t, uint32_t> _nodeViewTags;
}

- (instancetype)initWithAdapter:(id<NveAppleComponentViewAdapter>)adapter
                    errorDomain:(NSString *)errorDomain {
  self = [super init];
  if (self) {
    _adapter = adapter;
    _errorDomain = [errorDomain copy];
    _dylibHandle = nullptr;
    _surfaceId = 1;
  }
  return self;
}

- (void)dealloc {
  // Destroy the bridge (which calls `surface->destroy`) before closing the dylib handle.
  _bridge.reset();
  nve::apple::close_dylib(_dylibHandle);
}

- (uint32_t)surfaceId {
  return _surfaceId;
}

- (uint32_t)nodeCount {
  return _bridge ? _bridge->node_count() : 0;
}

- (BOOL)_installModule:(NveModule)mod
           dylibHandle:(void *)dylibHandle
             surfaceId:(uint32_t)surfaceId
         restoredState:(const std::vector<uint8_t> &)savedState {
  id<NveAppleComponentViewAdapter> adapter = _adapter;
  if (!adapter || !mod.surface || !mod.surface->create) {
    nve::apple::close_dylib(dylibHandle);
    return NO;
  }

  _bridge.reset();
  nve::apple::close_dylib(_dylibHandle);
  [self clearNativeNodeViews];

  _dylibHandle = dylibHandle;
  _surfaceId = surfaceId;

  __weak NveAppleComponentHost *weakSelf = self;
  auto bridge = std::make_unique<nve::NveRnComponentBridge>(
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
  if (!bridge->is_valid()) {
    bridge.reset();
    nve::apple::close_dylib(_dylibHandle);
    return NO;
  }

  nve::apple::install_native_text_measurer(bridge->fabric_host());
  _bridge = std::move(bridge);
  if (!savedState.empty()) {
    _bridge->restore_state(
        NveBufferSlice{savedState.data(), savedState.size()}, /*auto_render=*/false);
  }
  [self renderFrame];
  return YES;
}

- (BOOL)attachInterface:(NveGetInterfaceFn)getInterface surfaceId:(uint32_t)surfaceId {
  const NveModule mod = nve_resolve_module(getInterface);
  return [self _installModule:mod dylibHandle:nullptr surfaceId:surfaceId restoredState:{}];
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
                restoredState:{}];
}

- (BOOL)reloadDylibAtPath:(NSString *)dylibPath
                    error:(NSError * _Nullable * _Nullable)error {
  // Validate the replacement dylib before tearing down the existing bridge so a failed reload
  // preserves the currently mounted component and its state.
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
                restoredState:saved];
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
  if (!props) return NO;
  NSError *error = nil;
  NSData *jsonData = [NSJSONSerialization dataWithJSONObject:props options:0 error:&error];
  if (error || !jsonData) {
    return NO;
  }
  NSString *jsonString = [[NSString alloc] initWithData:jsonData encoding:NSUTF8StringEncoding];
  if (!jsonString) {
    return NO;
  }
  return [self applyPropsJson:jsonString];
}

- (void)renderFrame {
  if (!_bridge) return;
  NveFrameStats stats{};
  const int32_t rc = _bridge->render(&stats);
  if (rc == 0) {
    [self syncNativeViewsFromHost];
  }
  [self drainEmittedEvents];
}

- (void)drainEmittedEvents {
  if (!_bridge) return;
  (void)_bridge->drain_emitted_events();
}

- (void)dispatchPressOnNodeId:(uint32_t)nodeId x:(float)x y:(float)y {
  if (!_bridge) return;
  nve::fabric_host_enqueue_press(
      _bridge->fabric_host(), nodeId, nve::apple::current_timestamp_us(), x, y);
  const NveBufferSlice inbound = nve_fabric_host_drain_events(_bridge->fabric_host());
  const uint32_t dirty = _bridge->dispatch_native_events(inbound, /*auto_render=*/false);
  if (dirty != 0) {
    [self renderFrame];
  } else {
    [self drainEmittedEvents];
  }
}

- (void)updateSurfaceWidth:(float)width height:(float)height scaleFactor:(float)scaleFactor {
  if (!_bridge) return;
  if (width > 0.0f && height > 0.0f) {
    _bridge->set_surface_size(width, height, scaleFactor);
    [self syncNativeViewsFromHost];
  }
}

- (BOOL)getRootLayoutSize:(CGSize *)outSize {
  if (!_bridge || !outSize) return NO;
  const auto rootRect = _bridge->node_layout(NVE_ROOT_NODE_ID);
  if (rootRect.has_value() && rootRect->width > 0.0f && rootRect->height > 0.0f) {
    *outSize = CGSizeMake(rootRect->width, rootRect->height);
    return YES;
  }
  return NO;
}

- (nullable NvePlatformView *)nativeViewForNodeId:(uint32_t)nodeId {
  if (nodeId == NVE_ROOT_NODE_ID) {
    return [_adapter nveHostRootView];
  }
  const auto it = _nodeViews.find(nodeId);
  return (it != _nodeViews.end()) ? it->second : nil;
}

- (uint32_t)nodeIdForNativeView:(NvePlatformView *)view {
  for (const auto &[nid, v] : _nodeViews) {
    if (v == view) {
      return nid;
    }
  }
  return 0;
}

- (void)clearNativeNodeViews {
  for (auto &[_, view] : _nodeViews) {
    [view removeFromSuperview];
  }
  _nodeViews.clear();
  _nodeViewTags.clear();
}

- (uint32_t)_viewTagForSnapshot:(const nve::RenderedNodeSnapshot &)snap {
  // Distinguish leaf text nodes (UILabel / NSTextField) from wrapper text containers.
  const uint32_t isContainerText =
      (snap.kind == NVE_COMP_TEXT && !snap.children.empty()) ? 0x10000u : 0u;
  return static_cast<uint32_t>(snap.kind) | isContainerText;
}

- (void)syncNativeViewsFromHost {
  id<NveAppleComponentViewAdapter> adapter = _adapter;
  if (!_bridge || !adapter) return;
  const auto snapshots = nve::fabric_host_snapshot_nodes(_bridge->fabric_host());

  // Pass 1: Prune removed nodes.
  std::unordered_set<uint32_t> liveIds;
  liveIds.reserve(snapshots.size());
  for (const auto &snap : snapshots) {
    liveIds.insert(snap.node_id);
  }
  for (auto it = _nodeViews.begin(); it != _nodeViews.end();) {
    if (liveIds.find(it->first) == liveIds.end()) {
      [it->second removeFromSuperview];
      _nodeViewTags.erase(it->first);
      it = _nodeViews.erase(it);
    } else {
      ++it;
    }
  }

  // Pass 2: Create or update mounted native views and apply layout + style properties.
  for (const auto &snap : snapshots) {
    if (snap.node_id == NVE_ROOT_NODE_ID) {
      [adapter nveApplyRootBackgroundRgba:snap.background_rgba];
      continue;
    }

    NvePlatformView *view = nil;
    const uint32_t viewTag = [self _viewTagForSnapshot:snap];
    const auto existingIt = _nodeViews.find(snap.node_id);
    if (existingIt != _nodeViews.end() && _nodeViewTags[snap.node_id] == viewTag) {
      view = existingIt->second;
    } else {
      if (existingIt != _nodeViews.end()) {
        [existingIt->second removeFromSuperview];
      }
      view = [adapter nveCreateNativeViewForSnapshot:snap];
      _nodeViews[snap.node_id] = view;
      _nodeViewTags[snap.node_id] = viewTag;
    }

    view.frame = CGRectMake(snap.layout.x, snap.layout.y, snap.layout.width, snap.layout.height);
    const BOOL interactive =
        (snap.kind == NVE_COMP_PRESSABLE || (snap.event_mask & nve::apple::kEventMaskPress) != 0);
    [adapter nveUpdateNativeView:view forSnapshot:snap interactive:interactive];
    nve::apple::apply_common_layer_style(view.layer, snap);
  }

  // Pass 3: Reconcile parent-child view hierarchy.
  const BOOL hasCustomContainer =
      [adapter respondsToSelector:@selector(nveChildContainerForParentView:)];
  NvePlatformView *rootView = [adapter nveHostRootView];
  for (const auto &snap : snapshots) {
    NvePlatformView *parentView = (snap.node_id == NVE_ROOT_NODE_ID) ? rootView : nil;
    if (!parentView) {
      const auto pit = _nodeViews.find(snap.node_id);
      if (pit != _nodeViews.end()) {
        parentView = pit->second;
      }
    }
    if (!parentView) continue;

    if (hasCustomContainer) {
      parentView = [adapter nveChildContainerForParentView:parentView];
      if (!parentView) continue;
    }

    for (uint32_t childId : snap.children) {
      const auto cit = _nodeViews.find(childId);
      if (cit == _nodeViews.end()) continue;
      NvePlatformView *childView = cit->second;
      if (childView.superview != parentView) {
        [parentView addSubview:childView];
      }
    }
  }
}

@end
