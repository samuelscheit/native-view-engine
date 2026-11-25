#import "RCTNveMacComponentView.h"
#import "NveAppleComponentHost.h"

/// Flipped NSView container so Yoga's top-left origin maps 1:1 to AppKit `NSView.frame`.
@interface NveFlippedContainerView : NSView
@property (nonatomic, assign) uint32_t nveNodeId;
@end

@implementation NveFlippedContainerView
- (BOOL)isFlipped {
  return YES;
}
@end

@interface RCTNveMacComponentView () <NveAppleComponentViewAdapter>
@end

@implementation RCTNveMacComponentView {
  NveAppleComponentHost *_host;
}

- (instancetype)initWithFrame:(NSRect)frameRect {
  self = [super initWithFrame:frameRect];
  if (self) {
    self.wantsLayer = YES;
    _host = [[NveAppleComponentHost alloc] initWithAdapter:self
                                               errorDomain:@"nve.macos.rncomponent"];
    NveGetInterfaceFn defaultIface = [self defaultNveGetInterface];
    if (defaultIface != nullptr) {
      [_host attachInterface:defaultIface surfaceId:_host.surfaceId];
    }
  }
  return self;
}

- (BOOL)isFlipped {
  return YES;
}

- (uint32_t)nveSurfaceId {
  return _host.surfaceId;
}

- (uint32_t)nveNodeCount {
  return _host.nodeCount;
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

- (nullable NveGetInterfaceFn)defaultNveGetInterface {
  return nullptr;
}

- (BOOL)attachNveInterface:(NveGetInterfaceFn)getInterface surfaceId:(uint32_t)surfaceId {
  return [_host attachInterface:getInterface surfaceId:surfaceId];
}

- (BOOL)loadNveDylibAtPath:(NSString *)dylibPath
                 surfaceId:(uint32_t)surfaceId
                     error:(NSError * _Nullable * _Nullable)error {
  return [_host loadDylibAtPath:dylibPath surfaceId:surfaceId error:error];
}

- (BOOL)reloadNveDylibAtPath:(NSString *)dylibPath
                       error:(NSError * _Nullable * _Nullable)error {
  return [_host reloadDylibAtPath:dylibPath error:error];
}

- (BOOL)applyPropsJson:(NSString *)propsJson {
  return [_host applyPropsJson:propsJson];
}

- (BOOL)applyPropsDictionary:(NSDictionary<NSString *, id> *)props {
  return [_host applyPropsDictionary:props];
}

- (void)renderNveFrame {
  [_host renderFrame];
}

- (void)drainNveEmittedEvents {
  [_host drainEmittedEvents];
}

- (void)simulatePressOnNodeId:(uint32_t)nodeId atPoint:(NSPoint)point {
  [_host dispatchPressOnNodeId:nodeId x:static_cast<float>(point.x) y:static_cast<float>(point.y)];
}

- (nullable NSView *)nativeViewForNodeId:(uint32_t)nodeId {
  return [_host nativeViewForNodeId:nodeId];
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

- (void)_propagateBoundsToHost {
  const NSRect b = self.bounds;
  [_host updateSurfaceWidth:static_cast<float>(b.size.width)
                     height:static_cast<float>(b.size.height)
                scaleFactor:static_cast<float>([self _currentBackingScale])];
}

- (void)setFrameSize:(NSSize)newSize {
  [super setFrameSize:newSize];
  [self _propagateBoundsToHost];
}

- (void)layout {
  [super layout];
  [self _propagateBoundsToHost];
}

- (NSSize)intrinsicContentSize {
  CGSize rootSize = CGSizeZero;
  if ([_host getRootLayoutSize:&rootSize]) {
    return NSMakeSize(rootSize.width, rootSize.height);
  }
  return [super intrinsicContentSize];
}

#if defined(NVE_WITH_RN_FABRIC) && NVE_WITH_RN_FABRIC
- (void)updateLayoutMetrics:(const facebook::react::LayoutMetrics &)layoutMetrics
           oldLayoutMetrics:(const facebook::react::LayoutMetrics &)oldLayoutMetrics {
  [super updateLayoutMetrics:layoutMetrics oldLayoutMetrics:oldLayoutMetrics];
  const auto &size = layoutMetrics.frame.size;
  [_host updateSurfaceWidth:static_cast<float>(size.width)
                     height:static_cast<float>(size.height)
                scaleFactor:static_cast<float>(layoutMetrics.pointScaleFactor)];
}

- (void)prepareForRecycle {
  [super prepareForRecycle];
  [_host clearNativeNodeViews];
}
#endif

#pragma mark - NveAppleComponentViewAdapter (AppKit)

- (NSView *)nveHostRootView {
  return self;
}

- (NveSurfaceConfig)nveSurfaceConfigForSurfaceId:(uint32_t)surfaceId {
  const NSRect bounds = self.bounds;
  NveSurfaceConfig cfg{};
  cfg.surface_id = surfaceId;
  cfg.width = static_cast<float>(bounds.size.width > 0.0 ? bounds.size.width : 480.0);
  cfg.height = static_cast<float>(bounds.size.height > 0.0 ? bounds.size.height : 320.0);
  cfg.scale_factor = static_cast<float>([self _currentBackingScale]);
  return cfg;
}

- (void)nveEmitEvent:(NSString *)eventName payload:(NSDictionary<NSString *, id> *)payload {
  [self handleNveEvent:eventName payload:payload];
}

- (void)nveApplyRootBackgroundRgba:(uint32_t)rgba {
  self.wantsLayer = YES;
  if (rgba != 0) {
    self.layer.backgroundColor = nve::apple::color_from_rgba(rgba).CGColor;
  }
}

- (NSView *)nveCreateNativeViewForSnapshot:(const nve::RenderedNodeSnapshot &)snap {
  if (snap.kind == NVE_COMP_TEXT && !snap.children.empty()) {
    NveFlippedContainerView *container =
        [[NveFlippedContainerView alloc] initWithFrame:NSZeroRect];
    container.nveNodeId = snap.node_id;
    container.wantsLayer = YES;
    return container;
  }
  switch (snap.kind) {
    case NVE_COMP_TEXT: {
      NSTextField *label = [[NSTextField alloc] initWithFrame:NSZeroRect];
      label.editable = NO;
      label.selectable = NO;
      label.bordered = NO;
      label.drawsBackground = NO;
      label.usesSingleLineMode = NO;
      label.lineBreakMode = NSLineBreakByWordWrapping;
      label.cell.wraps = YES;
      label.cell.scrollable = NO;
      label.wantsLayer = YES;
      return label;
    }
    case NVE_COMP_TEXT_INPUT: {
      NSTextField *input = [[NSTextField alloc] initWithFrame:NSZeroRect];
      input.editable = YES;
      input.bezeled = YES;
      input.wantsLayer = YES;
      return input;
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
      return scroll;
    }
    default: {
      NveFlippedContainerView *container =
          [[NveFlippedContainerView alloc] initWithFrame:NSZeroRect];
      container.nveNodeId = snap.node_id;
      container.wantsLayer = YES;
      return container;
    }
  }
}

- (void)nveUpdateNativeView:(NSView *)view
                forSnapshot:(const nve::RenderedNodeSnapshot &)snap
                interactive:(BOOL)interactive {
  view.wantsLayer = YES;
  view.alphaValue = static_cast<CGFloat>(snap.opacity);
  view.layer.backgroundColor = nve::apple::color_from_rgba(snap.background_rgba).CGColor;

  if ([view isKindOfClass:[NSTextField class]]) {
    NSTextField *field = (NSTextField *)view;
    field.stringValue = [NSString stringWithUTF8String:snap.text.c_str()] ?: @"";
    field.textColor = nve::apple::color_from_rgba(snap.text_color_rgba);
    if (snap.font_size > 0.0f) {
      field.font = [NSFont systemFontOfSize:static_cast<CGFloat>(snap.font_size)];
    }
  }

  if (interactive && view.gestureRecognizers.count == 0) {
    NSClickGestureRecognizer *click =
        [[NSClickGestureRecognizer alloc] initWithTarget:self action:@selector(_handleNodeClick:)];
    [view addGestureRecognizer:click];
  }
}

- (NSView *)nveChildContainerForParentView:(NSView *)parentView {
  if ([parentView isKindOfClass:[NSScrollView class]]) {
    NSScrollView *scroll = (NSScrollView *)parentView;
    if (scroll.documentView != nil) {
      return scroll.documentView;
    }
  }
  return parentView;
}

- (void)_handleNodeClick:(NSClickGestureRecognizer *)recognizer {
  if (!recognizer.view) return;
  uint32_t nodeId = 0;
  if ([recognizer.view isKindOfClass:[NveFlippedContainerView class]]) {
    nodeId = ((NveFlippedContainerView *)recognizer.view).nveNodeId;
  } else {
    nodeId = [_host nodeIdForNativeView:recognizer.view];
  }
  if (nodeId == 0) return;
  const NSPoint loc = [recognizer locationInView:recognizer.view];
  [self simulatePressOnNodeId:nodeId atPoint:loc];
}

@end
