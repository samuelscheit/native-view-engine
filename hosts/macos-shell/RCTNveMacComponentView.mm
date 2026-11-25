#import "RCTNveMacComponentView.h"
#import "NveAppleCanvasView.h"
#import "NveAppleComponentHost.h"
#import "NveAppleImageCache.h"

/// Flipped NSView container so Yoga's top-left origin maps 1:1 to AppKit `NSView.frame`.
@interface NveFlippedContainerView : NSView
@property (nonatomic, assign) uint32_t nveNodeId;
@end

@implementation NveFlippedContainerView
- (BOOL)isFlipped {
  return YES;
}
@end

/// Non-editable label that passes mouse hit-testing through to ancestor Pressable containers
/// unless the label itself has interactive gesture recognizers attached.
@interface NveLabelTextField : NSTextField
@end

@implementation NveLabelTextField
- (NSView *)hitTest:(NSPoint)point {
  return self.gestureRecognizers.count > 0 ? [super hitTest:point] : nil;
}
@end

/// Image view that caches its last bound source string and passes clicks through when non-interactive.
@interface NveMacImageView : NSImageView
@property (nonatomic, copy, nullable) NSString *nveImageSrc;
@end

@implementation NveMacImageView
- (NSView *)hitTest:(NSPoint)point {
  return self.gestureRecognizers.count > 0 ? [super hitTest:point] : nil;
}
@end

@interface RCTNveMacComponentView () <NveAppleComponentViewAdapter, NSTextFieldDelegate>
@end

@implementation RCTNveMacComponentView {
  NveAppleComponentHost *_host;
  BOOL _updatingScrollFrame;
}

- (instancetype)initWithFrame:(NSRect)frameRect {
  self = [super initWithFrame:frameRect];
  if (self) {
    self.wantsLayer = YES;
    _updatingScrollFrame = NO;
    _host = [[NveAppleComponentHost alloc] initWithAdapter:self
                                               errorDomain:@"nve.macos.rncomponent"];
    NveGetInterfaceFn defaultIface = [self defaultNveGetInterface];
    if (defaultIface != nullptr) {
      [_host attachInterface:defaultIface surfaceId:_host.surfaceId];
    }
  }
  return self;
}

- (void)dealloc {
  [[NSNotificationCenter defaultCenter] removeObserver:self];
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

- (uint32_t)nveLastReconciledDirtyCount {
  return _host.lastReconciledDirtyCount;
}

- (uint32_t)nveLastReconciledRemovedCount {
  return _host.lastReconciledRemovedCount;
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

- (BOOL)replayFrame0SnapshotAtPath:(NSString *)snapshotPath
             expectedSchemaVersion:(uint32_t)expectedSchemaVersion
                     restoreTimeUs:(uint64_t * _Nullable)outRestoreUs {
  return [_host replayFrame0SnapshotAtPath:snapshotPath
                     expectedSchemaVersion:expectedSchemaVersion
                             restoreTimeUs:outRestoreUs];
}

- (BOOL)saveSnapshotAtPath:(NSString *)snapshotPath
             schemaVersion:(uint32_t)schemaVersion {
  return [_host saveSnapshotAtPath:snapshotPath schemaVersion:schemaVersion];
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

- (NSView *)nveCreateNativeViewForNode:(const NveRenderNode &)node
                             semantics:(const nve::apple::NodeSemantics &)semantics {
  if (node.kind == NVE_NODE_KIND_TEXT && (node.flags & NVE_NODE_LEAF_TEXT) == 0u) {
    NveFlippedContainerView *container =
        [[NveFlippedContainerView alloc] initWithFrame:NSZeroRect];
    container.nveNodeId = node.node_id;
    container.wantsLayer = YES;
    return container;
  }
  switch (node.kind) {
    case NVE_NODE_KIND_TEXT: {
      NveLabelTextField *label = [[NveLabelTextField alloc] initWithFrame:NSZeroRect];
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
    case NVE_NODE_KIND_IMAGE: {
      NveMacImageView *imgView = [[NveMacImageView alloc] initWithFrame:NSZeroRect];
      imgView.imageFrameStyle = NSImageFrameNone;
      imgView.imageAlignment = NSImageAlignCenter;
      imgView.wantsLayer = YES;
      return imgView;
    }
    case NVE_NODE_KIND_TEXT_INPUT: {
      NSTextField *input = semantics.secure_text_entry
                               ? [[NSSecureTextField alloc] initWithFrame:NSZeroRect]
                               : [[NSTextField alloc] initWithFrame:NSZeroRect];
      input.editable = YES;
      input.bezeled = YES;
      input.delegate = self;
      input.wantsLayer = YES;
      return input;
    }
    case NVE_NODE_KIND_SCROLL_VIEW: {
      NSScrollView *scroll = [[NSScrollView alloc] initWithFrame:NSZeroRect];
      scroll.hasVerticalScroller = YES;
      scroll.drawsBackground = NO;
      scroll.wantsLayer = YES;
      NveFlippedContainerView *doc = [[NveFlippedContainerView alloc] initWithFrame:NSZeroRect];
      doc.nveNodeId = node.node_id;
      doc.wantsLayer = YES;
      scroll.documentView = doc;
      scroll.contentView.postsBoundsChangedNotifications = YES;
      [[NSNotificationCenter defaultCenter] addObserver:self
                                               selector:@selector(_handleScrollViewBoundsChanged:)
                                                   name:NSViewBoundsDidChangeNotification
                                                 object:scroll.contentView];
      return scroll;
    }
    case NVE_NODE_KIND_SKIA_CANVAS: {
      NveAppleCanvasView *canvas = [[NveAppleCanvasView alloc] initWithFrame:NSZeroRect];
      canvas.wantsLayer = YES;
      return canvas;
    }
    default: {
      NveFlippedContainerView *container =
          [[NveFlippedContainerView alloc] initWithFrame:NSZeroRect];
      container.nveNodeId = node.node_id;
      container.wantsLayer = YES;
      return container;
    }
  }
}

- (void)nveWillDestroyNativeView:(NSView *)view forNodeId:(uint32_t)nodeId {
  (void)nodeId;
  if ([view isKindOfClass:[NSScrollView class]]) {
    NSScrollView *scroll = (NSScrollView *)view;
    [[NSNotificationCenter defaultCenter] removeObserver:self
                                                    name:NSViewBoundsDidChangeNotification
                                                  object:scroll.contentView];
  }
}

- (void)nveUpdateNativeView:(NSView *)view
                    forNode:(const NveRenderNode &)node
                       text:(std::string_view)text
                  semantics:(const nve::apple::NodeSemantics &)semantics
                interactive:(BOOL)interactive {
  view.wantsLayer = YES;
  view.alphaValue = static_cast<CGFloat>(node.opacity_u8) / 255.0;
  view.layer.backgroundColor = nve::apple::color_from_rgba(node.bg_rgba).CGColor;

  if ([view isKindOfClass:[NveMacImageView class]]) {
    NveMacImageView *imgView = (NveMacImageView *)view;
    NSString *nextSrc =
        text.empty() ? nil
                     : [[NSString alloc] initWithBytes:text.data()
                                                length:text.size()
                                              encoding:NSUTF8StringEncoding];
    const BOOL sameSrc =
        (nextSrc == nil && imgView.nveImageSrc == nil) ||
        (nextSrc != nil && [nextSrc isEqualToString:imgView.nveImageSrc]);
    if (!sameSrc) {
      imgView.nveImageSrc = nextSrc;
      imgView.image =
          text.empty() ? nil : nve::apple::shared_image_cache().image_for_source(text);
    }
    nve::apple::apply_image_resize_mode(imgView, semantics.resize_mode, node.border_radius);
  } else if ([view isKindOfClass:[NSTextField class]]) {
    NSTextField *field = (NSTextField *)view;
    if (node.kind != NVE_NODE_KIND_TEXT_INPUT) {
      NSString *nextText =
          text.empty() ? @""
                       : ([[NSString alloc] initWithBytes:text.data()
                                                   length:text.size()
                                                 encoding:NSUTF8StringEncoding] ?: @"");
      if (![field.stringValue isEqualToString:nextText]) {
        field.stringValue = nextText;
      }
    }
    field.textColor = nve::apple::color_from_rgba(node.text_rgba);
    if (node.font_size > 0.0f) {
      field.font = [NSFont systemFontOfSize:static_cast<CGFloat>(node.font_size)];
    }
  } else if ([view isKindOfClass:[NveAppleCanvasView class]]) {
    NveAppleCanvasView *canvasView = (NveAppleCanvasView *)view;
    const std::span<const uint8_t> drawBytes(
        reinterpret_cast<const uint8_t *>(text.data()), text.size());
    [canvasView updateDrawListBytes:drawBytes];
  }

  if (interactive) {
    if (view.gestureRecognizers.count == 0) {
      NSClickGestureRecognizer *click =
          [[NSClickGestureRecognizer alloc] initWithTarget:self action:@selector(_handleNodeClick:)];
      [view addGestureRecognizer:click];
    }
  } else if (view.gestureRecognizers.count > 0) {
    NSArray<NSGestureRecognizer *> *recognizers = [view.gestureRecognizers copy];
    for (NSGestureRecognizer *gr in recognizers) {
      if ([gr isKindOfClass:[NSClickGestureRecognizer class]] && gr.target == self) {
        [view removeGestureRecognizer:gr];
      }
    }
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

- (void)nveUpdateScrollView:(NSView *)scrollView contentSize:(CGSize)contentSize {
  if ([scrollView isKindOfClass:[NSScrollView class]]) {
    NSScrollView *scroll = (NSScrollView *)scrollView;
    if (scroll.documentView != nil) {
      const NSRect nextFrame = NSMakeRect(0.0, 0.0, contentSize.width, contentSize.height);
      if (!NSEqualRects(scroll.documentView.frame, nextFrame)) {
        _updatingScrollFrame = YES;
        scroll.documentView.frame = nextFrame;
        _updatingScrollFrame = NO;
      }
    }
  }
}

- (void)_handleScrollViewBoundsChanged:(NSNotification *)note {
  if (_updatingScrollFrame) return;
  if (![note.object isKindOfClass:[NSClipView class]]) return;
  NSClipView *clipView = (NSClipView *)note.object;
  if (![clipView.superview isKindOfClass:[NSScrollView class]]) return;
  NSScrollView *scroll = (NSScrollView *)clipView.superview;
  const uint32_t nodeId = [_host nodeIdForNativeView:scroll];
  if (nodeId == 0u) return;
  const NSPoint origin = clipView.bounds.origin;
  const NSSize docSize = scroll.documentView != nil ? scroll.documentView.frame.size : NSZeroSize;
  [_host dispatchScrollOnNodeId:nodeId
                              x:static_cast<float>(origin.x)
                              y:static_cast<float>(origin.y)
                   contentWidth:static_cast<float>(docSize.width)
                  contentHeight:static_cast<float>(docSize.height)];
}

- (void)controlTextDidBeginEditing:(NSNotification *)obj {
  if (![obj.object isKindOfClass:[NSTextField class]]) return;
  NSTextField *field = (NSTextField *)obj.object;
  const uint32_t nodeId = [_host nodeIdForNativeView:field];
  if (nodeId == 0) return;
  [_host dispatchFocusOnNodeId:nodeId focused:YES];
}

- (void)controlTextDidEndEditing:(NSNotification *)obj {
  if (![obj.object isKindOfClass:[NSTextField class]]) return;
  NSTextField *field = (NSTextField *)obj.object;
  const uint32_t nodeId = [_host nodeIdForNativeView:field];
  if (nodeId == 0) return;
  [_host dispatchFocusOnNodeId:nodeId focused:NO];
}

- (void)controlTextDidChange:(NSNotification *)obj {
  if (![obj.object isKindOfClass:[NSTextField class]]) return;
  NSTextField *field = (NSTextField *)obj.object;
  if (nve::apple::is_text_input_composing_ime(field)) {
    return;
  }
  id fieldEditor = obj.userInfo[@"NSFieldEditor"];
  if ([fieldEditor isKindOfClass:[NSTextView class]]) {
    NSTextView *tv = (NSTextView *)fieldEditor;
    if (tv.hasMarkedText || tv.markedRange.length > 0) {
      return;
    }
  }
  const uint32_t nodeId = [_host nodeIdForNativeView:field];
  if (nodeId == 0) return;
  [_host dispatchTextChangeOnNodeId:nodeId text:field.stringValue ?: @""];
}

- (BOOL)control:(NSControl *)control
       textView:(NSTextView *)textView
doCommandBySelector:(SEL)commandSelector {
  if (commandSelector == @selector(insertNewline:)) {
    if (textView != nil && (textView.hasMarkedText || textView.markedRange.length > 0)) {
      return NO;
    }
    if ([control isKindOfClass:[NSTextField class]]) {
      NSTextField *field = (NSTextField *)control;
      if (!field.usesSingleLineMode) {
        return NO;
      }
      const uint32_t nodeId = [_host nodeIdForNativeView:field];
      if (nodeId != 0) {
        [_host dispatchSubmitOnNodeId:nodeId text:field.stringValue ?: @""];
        return YES;
      }
    }
  }
  return NO;
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
