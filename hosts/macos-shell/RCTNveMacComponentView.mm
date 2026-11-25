#import "RCTNveMacComponentView.h"
#import <QuartzCore/QuartzCore.h>

#include "nve_rn_component.hpp"

#include <chrono>
#include <dlfcn.h>
#include <memory>
#include <unordered_map>
#include <unordered_set>

/// Flipped NSView container so Yoga's top-left origin maps 1:1 to AppKit `NSView.frame`.
@interface NveFlippedContainerView : NSView
@property (nonatomic, assign) uint32_t nveNodeId;
@end

@implementation NveFlippedContainerView
- (BOOL)isFlipped {
  return YES;
}
@end

namespace {

constexpr uint64_t kNveMacEventMaskPress = 1ULL << 0;

uint64_t currentTimestampUs() {
  using Clock = std::chrono::steady_clock;
  return static_cast<uint64_t>(
      std::chrono::duration_cast<std::chrono::microseconds>(Clock::now().time_since_epoch())
          .count());
}

NSColor *colorFromRgba(uint32_t rgba) {
  const CGFloat r = static_cast<CGFloat>(rgba & 0xFFu) / 255.0;
  const CGFloat g = static_cast<CGFloat>((rgba >> 8) & 0xFFu) / 255.0;
  const CGFloat b = static_cast<CGFloat>((rgba >> 16) & 0xFFu) / 255.0;
  const CGFloat a = static_cast<CGFloat>((rgba >> 24) & 0xFFu) / 255.0;
  return [NSColor colorWithSRGBRed:r green:g blue:b alpha:a];
}

NSDictionary<NSString *, id> *payloadDictionaryFromEvent(const nve::DecodedEvent &ev) {
  NSMutableDictionary<NSString *, id> *dict = [NSMutableDictionary dictionary];
  dict[@"surfaceId"] = @(ev.surface_id);
  dict[@"nodeId"] = @(ev.node_id);
  dict[@"timestampUs"] = @(ev.timestamp_us);

  switch (ev.kind) {
    case nve::EventKind::Press:
    case nve::EventKind::LongPress:
      dict[@"x"] = @(ev.x);
      dict[@"y"] = @(ev.y);
      break;
    case nve::EventKind::Scroll:
      dict[@"x"] = @(ev.x);
      dict[@"y"] = @(ev.y);
      dict[@"contentWidth"] = @(ev.width);
      dict[@"contentHeight"] = @(ev.height);
      break;
    case nve::EventKind::TextChange:
      dict[@"text"] = [NSString stringWithUTF8String:ev.text.c_str()] ?: @"";
      break;
    case nve::EventKind::Layout:
      dict[@"x"] = @(ev.x);
      dict[@"y"] = @(ev.y);
      dict[@"width"] = @(ev.width);
      dict[@"height"] = @(ev.height);
      break;
    case nve::EventKind::Custom: {
      if (!ev.payload_bytes.empty()) {
        NSData *data = [NSData dataWithBytes:ev.payload_bytes.data()
                                      length:ev.payload_bytes.size()];
        NSError *jsonError = nil;
        id parsed = [NSJSONSerialization JSONObjectWithData:data options:0 error:&jsonError];
        if (!jsonError && [parsed isKindOfClass:[NSDictionary class]]) {
          [dict addEntriesFromDictionary:(NSDictionary *)parsed];
        } else if (!jsonError && parsed != nil) {
          dict[@"value"] = parsed;
        } else {
          NSString *raw = [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding];
          if (raw != nil) {
            dict[@"raw"] = raw;
          }
        }
      }
      break;
    }
    case nve::EventKind::Focus:
    case nve::EventKind::Blur:
      break;
  }
  return [dict copy];
}

} // namespace

@implementation RCTNveMacComponentView {
  std::unique_ptr<nve::NveRnComponentBridge> _bridge;
  void *_dylibHandle;
  uint32_t _surfaceId;
  std::unordered_map<uint32_t, NSView *> _nodeViews;
  std::unordered_map<uint32_t, uint16_t> _nodeKinds;
}

- (instancetype)initWithFrame:(NSRect)frameRect {
  self = [super initWithFrame:frameRect];
  if (self) {
    self.wantsLayer = YES;
    _dylibHandle = nullptr;
    _surfaceId = 1;
    NveGetInterfaceFn defaultIface = [self defaultNveGetInterface];
    if (defaultIface != nullptr) {
      [self attachNveInterface:defaultIface surfaceId:_surfaceId];
    }
  }
  return self;
}

- (void)dealloc {
  _bridge.reset();
  if (_dylibHandle) {
    dlclose(_dylibHandle);
    _dylibHandle = nullptr;
  }
}

- (BOOL)isFlipped {
  return YES;
}

- (uint32_t)nveSurfaceId {
  return _surfaceId;
}

- (uint32_t)nveNodeCount {
  return _bridge ? _bridge->node_count() : 0;
}

- (CGFloat)_currentBackingScale {
  if (self.window != nil && self.window.backingScaleFactor > 0.0) {
    return self.window.backingScaleFactor;
  }
  if ([NSScreen mainScreen] != nil && [NSScreen mainScreen].backingScaleFactor > 0.0) {
    return [NSScreen mainScreen].backingScaleFactor;
  }
  return 2.0;
}

- (NveSurfaceConfig)_currentSurfaceConfigWithId:(uint32_t)surfaceId {
  const NSRect bounds = self.bounds;
  NveSurfaceConfig cfg{};
  cfg.surface_id = surfaceId;
  cfg.width = static_cast<float>(bounds.size.width > 0.0 ? bounds.size.width : 480.0);
  cfg.height = static_cast<float>(bounds.size.height > 0.0 ? bounds.size.height : 320.0);
  cfg.scale_factor = static_cast<float>([self _currentBackingScale]);
  return cfg;
}

- (nullable NveGetInterfaceFn)defaultNveGetInterface {
  return nullptr;
}

- (BOOL)attachNveInterface:(NveGetInterfaceFn)getInterface surfaceId:(uint32_t)surfaceId {
  NveModule mod = nve_resolve_module(getInterface);
  if (mod.surface == nullptr || mod.surface->create == nullptr) {
    return NO;
  }
  _surfaceId = surfaceId;
  [self _clearNativeNodeViews];

  __weak RCTNveMacComponentView *weakSelf = self;
  auto bridge = std::make_unique<nve::NveRnComponentBridge>(
      mod,
      [self _currentSurfaceConfigWithId:surfaceId],
      [weakSelf](const nve::DecodedEvent &ev) {
        __strong RCTNveMacComponentView *strongSelf = weakSelf;
        if (!strongSelf) return;
        NSString *eventName = [NSString stringWithUTF8String:ev.name.c_str()] ?: @"onCustom";
        NSDictionary<NSString *, id> *payload = payloadDictionaryFromEvent(ev);
        [strongSelf handleNveEvent:eventName payload:payload];
      });
  if (!bridge->is_valid()) {
    return NO;
  }
  _bridge = std::move(bridge);
  [self renderNveFrame];
  return YES;
}

- (BOOL)loadNveDylibAtPath:(NSString *)dylibPath
                 surfaceId:(uint32_t)surfaceId
                     error:(NSError * _Nullable * _Nullable)error {
  void *handle = dlopen(dylibPath.UTF8String, RTLD_NOW | RTLD_LOCAL);
  if (!handle) {
    if (error) {
      *error = [NSError errorWithDomain:@"nve.macos.rncomponent"
                                   code:1
                               userInfo:@{NSLocalizedDescriptionKey : @(dlerror() ?: "dlopen failed")}];
    }
    return NO;
  }
  auto getIface = reinterpret_cast<NveGetInterfaceFn>(dlsym(handle, "nve_get_interface"));
  if (!getIface) {
    dlclose(handle);
    if (error) {
      *error = [NSError errorWithDomain:@"nve.macos.rncomponent"
                                   code:2
                               userInfo:@{NSLocalizedDescriptionKey : @"Missing nve_get_interface symbol"}];
    }
    return NO;
  }
  NveModule mod = nve_resolve_module(getIface);
  if (!mod.surface || !mod.surface->create) {
    dlclose(handle);
    if (error) {
      *error = [NSError errorWithDomain:@"nve.macos.rncomponent"
                                   code:3
                               userInfo:@{NSLocalizedDescriptionKey : @"Missing NVE_IFACE_SURFACE interface"}];
    }
    return NO;
  }

  _bridge.reset();
  if (_dylibHandle) {
    dlclose(_dylibHandle);
    _dylibHandle = nullptr;
  }
  _dylibHandle = handle;
  return [self attachNveInterface:getIface surfaceId:surfaceId];
}

- (BOOL)reloadNveDylibAtPath:(NSString *)dylibPath
                       error:(NSError * _Nullable * _Nullable)error {
  std::vector<uint8_t> saved;
  if (_bridge) {
    saved = _bridge->snapshot_state();
  }
  if (![self loadNveDylibAtPath:dylibPath surfaceId:_surfaceId error:error]) {
    return NO;
  }
  if (!saved.empty() && _bridge) {
    _bridge->restore_state(NveBufferSlice{saved.data(), saved.size()}, /*auto_render=*/false);
    [self renderNveFrame];
  }
  return YES;
}

- (BOOL)applyPropsJson:(NSString *)propsJson {
  if (!_bridge) return NO;
  NSData *utf8 = [propsJson dataUsingEncoding:NSUTF8StringEncoding];
  if (!utf8) return NO;
  std::string_view sv(reinterpret_cast<const char *>(utf8.bytes), utf8.length);
  const bool dirty = _bridge->update_props_json(sv, /*auto_render=*/false);
  if (dirty) {
    [self renderNveFrame];
  } else {
    [self drainNveEmittedEvents];
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

- (void)renderNveFrame {
  if (!_bridge) return;
  NveFrameStats stats{};
  const int32_t rc = _bridge->render(&stats);
  if (rc == 0) {
    [self _syncNativeViewsFromHost];
  }
  [self drainNveEmittedEvents];
}

- (void)drainNveEmittedEvents {
  if (!_bridge) return;
  (void)_bridge->drain_emitted_events();
}

- (void)simulatePressOnNodeId:(uint32_t)nodeId atPoint:(NSPoint)point {
  if (!_bridge) return;
  nve::fabric_host_enqueue_press(
      _bridge->fabric_host(),
      nodeId,
      currentTimestampUs(),
      static_cast<float>(point.x),
      static_cast<float>(point.y));
  const NveBufferSlice inbound = nve_fabric_host_drain_events(_bridge->fabric_host());
  const uint32_t dirty = _bridge->dispatch_native_events(inbound, /*auto_render=*/false);
  if (dirty != 0) {
    [self renderNveFrame];
  } else {
    [self drainNveEmittedEvents];
  }
}

- (nullable NSView *)nativeViewForNodeId:(uint32_t)nodeId {
  if (nodeId == NVE_ROOT_NODE_ID) {
    return self;
  }
  auto it = _nodeViews.find(nodeId);
  return (it != _nodeViews.end()) ? it->second : nil;
}

- (void)handleNveEvent:(NSString *)eventName payload:(NSDictionary<NSString *, id> *)payload {
  if (self.onNveEvent) {
    self.onNveEvent(eventName, payload);
  }
  id<RCTNveMacComponentViewDelegate> delegate = self.delegate;
  if ([delegate respondsToSelector:@selector(nveComponentView:didEmitEvent:payload:)]) {
    [delegate nveComponentView:self didEmitEvent:eventName payload:payload];
  }
}

- (void)setFrameSize:(NSSize)newSize {
  [super setFrameSize:newSize];
  [self _propagateBoundsToBridge];
}

- (void)layout {
  [super layout];
  [self _propagateBoundsToBridge];
}

- (NSSize)intrinsicContentSize {
  if (_bridge) {
    auto rootRect = _bridge->node_layout(NVE_ROOT_NODE_ID);
    if (rootRect.has_value() && rootRect->width > 0.0f && rootRect->height > 0.0f) {
      return NSMakeSize(rootRect->width, rootRect->height);
    }
  }
  return [super intrinsicContentSize];
}

#if defined(NVE_WITH_RN_FABRIC) && NVE_WITH_RN_FABRIC
- (void)updateLayoutMetrics:(const facebook::react::LayoutMetrics &)layoutMetrics
           oldLayoutMetrics:(const facebook::react::LayoutMetrics &)oldLayoutMetrics {
  [super updateLayoutMetrics:layoutMetrics oldLayoutMetrics:oldLayoutMetrics];
  if (_bridge) {
    const auto &size = layoutMetrics.frame.size;
    _bridge->set_surface_size(
        static_cast<float>(size.width),
        static_cast<float>(size.height),
        static_cast<float>(layoutMetrics.pointScaleFactor));
    [self _syncNativeViewsFromHost];
  }
}

- (void)prepareForRecycle {
  [super prepareForRecycle];
  [self _clearNativeNodeViews];
}
#endif

#pragma mark - Native AppKit View Reconciliation

- (void)_propagateBoundsToBridge {
  if (!_bridge) return;
  const NSRect b = self.bounds;
  if (b.size.width > 0.0 && b.size.height > 0.0) {
    _bridge->set_surface_size(
        static_cast<float>(b.size.width),
        static_cast<float>(b.size.height),
        static_cast<float>([self _currentBackingScale]));
    [self _syncNativeViewsFromHost];
  }
}

- (void)_clearNativeNodeViews {
  for (auto &[_, view] : _nodeViews) {
    [view removeFromSuperview];
  }
  _nodeViews.clear();
  _nodeKinds.clear();
}

- (NSView *)_createNativeViewForSnapshot:(const nve::RenderedNodeSnapshot &)snap {
  NSView *view = nil;
  switch (snap.kind) {
    case NVE_COMP_TEXT: {
      NSTextField *label = [[NSTextField alloc] initWithFrame:NSZeroRect];
      label.editable = NO;
      label.selectable = NO;
      label.bordered = NO;
      label.drawsBackground = NO;
      label.usesSingleLineMode = NO;
      label.lineBreakMode = NSLineBreakByWordWrapping;
      label.wantsLayer = YES;
      view = label;
      break;
    }
    case NVE_COMP_TEXT_INPUT: {
      NSTextField *input = [[NSTextField alloc] initWithFrame:NSZeroRect];
      input.editable = YES;
      input.bezeled = YES;
      input.wantsLayer = YES;
      view = input;
      break;
    }
    case NVE_COMP_SCROLL_VIEW: {
      NSScrollView *scroll = [[NSScrollView alloc] initWithFrame:NSZeroRect];
      scroll.hasVerticalScroller = YES;
      scroll.drawsBackground = NO;
      scroll.wantsLayer = YES;
      NveFlippedContainerView *doc = [[NveFlippedContainerView alloc] initWithFrame:NSZeroRect];
      doc.nveNodeId = snap.node_id;
      doc.wantsLayer = YES;
      scroll.documentView = doc;
      view = scroll;
      break;
    }
    default: {
      NveFlippedContainerView *container =
          [[NveFlippedContainerView alloc] initWithFrame:NSZeroRect];
      container.nveNodeId = snap.node_id;
      container.wantsLayer = YES;
      view = container;
      break;
    }
  }
  return view;
}

- (void)_syncNativeViewsFromHost {
  if (!_bridge) return;
  const auto snapshots = nve::fabric_host_snapshot_nodes(_bridge->fabric_host());

  std::unordered_set<uint32_t> liveIds;
  liveIds.reserve(snapshots.size());
  for (const auto &snap : snapshots) {
    liveIds.insert(snap.node_id);
  }

  for (auto it = _nodeViews.begin(); it != _nodeViews.end();) {
    if (liveIds.find(it->first) == liveIds.end()) {
      [it->second removeFromSuperview];
      _nodeKinds.erase(it->first);
      it = _nodeViews.erase(it);
    } else {
      ++it;
    }
  }

  for (const auto &snap : snapshots) {
    if (snap.node_id == NVE_ROOT_NODE_ID) {
      self.wantsLayer = YES;
      if (snap.background_rgba != 0) {
        self.layer.backgroundColor = colorFromRgba(snap.background_rgba).CGColor;
      }
      continue;
    }

    NSView *view = nil;
    auto existingIt = _nodeViews.find(snap.node_id);
    if (existingIt != _nodeViews.end() && _nodeKinds[snap.node_id] == snap.kind) {
      view = existingIt->second;
    } else {
      if (existingIt != _nodeViews.end()) {
        [existingIt->second removeFromSuperview];
      }
      view = [self _createNativeViewForSnapshot:snap];
      _nodeViews[snap.node_id] = view;
      _nodeKinds[snap.node_id] = snap.kind;
    }

    view.frame = NSMakeRect(snap.layout.x, snap.layout.y, snap.layout.width, snap.layout.height);
    view.wantsLayer = YES;
    view.alphaValue = static_cast<CGFloat>(snap.opacity);
    view.layer.backgroundColor = colorFromRgba(snap.background_rgba).CGColor;
    view.layer.cornerRadius = static_cast<CGFloat>(snap.border_radius);
    view.layer.borderWidth = static_cast<CGFloat>(snap.border_width);
    view.layer.borderColor = colorFromRgba(snap.border_color_rgba).CGColor;
    view.layer.zPosition = static_cast<CGFloat>(snap.z_index);

    if ([view isKindOfClass:[NSTextField class]]) {
      NSTextField *field = (NSTextField *)view;
      field.stringValue = [NSString stringWithUTF8String:snap.text.c_str()] ?: @"";
      field.textColor = colorFromRgba(snap.text_color_rgba);
      if (snap.font_size > 0.0f) {
        field.font = [NSFont systemFontOfSize:static_cast<CGFloat>(snap.font_size)];
      }
    }

    const BOOL interactive =
        (snap.kind == NVE_COMP_PRESSABLE || (snap.event_mask & kNveMacEventMaskPress) != 0);
    if (interactive && view.gestureRecognizers.count == 0) {
      NSClickGestureRecognizer *click =
          [[NSClickGestureRecognizer alloc] initWithTarget:self action:@selector(_handleNodeClick:)];
      [view addGestureRecognizer:click];
    }
  }

  for (const auto &snap : snapshots) {
    NSView *parentView = (snap.node_id == NVE_ROOT_NODE_ID) ? self : nil;
    if (!parentView) {
      auto pit = _nodeViews.find(snap.node_id);
      if (pit != _nodeViews.end()) {
        parentView = pit->second;
      }
    }
    if (!parentView) continue;

    if ([parentView isKindOfClass:[NSScrollView class]]) {
      NSScrollView *scroll = (NSScrollView *)parentView;
      if (scroll.documentView != nil) {
        parentView = scroll.documentView;
      }
    }

    for (uint32_t childId : snap.children) {
      auto cit = _nodeViews.find(childId);
      if (cit == _nodeViews.end()) continue;
      NSView *childView = cit->second;
      if (childView.superview != parentView) {
        [parentView addSubview:childView];
      }
    }
  }
}

- (void)_handleNodeClick:(NSClickGestureRecognizer *)recognizer {
  if (!_bridge || !recognizer.view) return;
  uint32_t nodeId = 0;
  if ([recognizer.view isKindOfClass:[NveFlippedContainerView class]]) {
    nodeId = ((NveFlippedContainerView *)recognizer.view).nveNodeId;
  } else {
    for (const auto &[nid, v] : _nodeViews) {
      if (v == recognizer.view) {
        nodeId = nid;
        break;
      }
    }
  }
  if (nodeId == 0) return;
  const NSPoint loc = [recognizer locationInView:recognizer.view];
  [self simulatePressOnNodeId:nodeId atPoint:loc];
}

@end
