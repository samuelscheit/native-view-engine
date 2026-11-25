#import "RCTNveMacComponentView.h"
#import "NveAppleCanvasView.h"
#import "NveAppleComponentHost.h"
#import "NveAppleValueBridge.h"

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

@interface RCTNveMacComponentView () <NSTextFieldDelegate>
@end

@implementation RCTNveMacComponentView {
  BOOL _updatingScrollFrame;
}

- (instancetype)initWithFrame:(NSRect)frameRect {
  self = [super initWithFrame:frameRect];
  if (self) {
    _updatingScrollFrame = NO;
  }
  return self;
}

- (void)dealloc {
  [[NSNotificationCenter defaultCenter] removeObserver:self];
}

- (BOOL)isFlipped {
  return YES;
}

- (void)handleNveEvent:(NSString *)eventName payload:(NSDictionary<NSString *, id> *)payload {
  [super handleNveEvent:eventName payload:payload];
  id<RCTNveMacComponentViewDelegate> delegate = self.delegate;
  if ([delegate respondsToSelector:@selector(nveComponentView:didEmitEvent:payload:)]) {
    [delegate nveComponentView:self didEmitEvent:eventName payload:payload];
  }
}

- (void)setFrameSize:(NSSize)newSize {
  [super setFrameSize:newSize];
  [self propagateBoundsToNveHost];
}

- (void)layout {
  [super layout];
  [self propagateBoundsToNveHost];
}

- (NSSize)intrinsicContentSize {
  CGSize rootSize = CGSizeZero;
  if ([self.nveHost getRootLayoutSize:&rootSize]) {
    return NSMakeSize(rootSize.width, rootSize.height);
  }
  return [super intrinsicContentSize];
}

#pragma mark - NveAppleComponentViewAdapter (AppKit)

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
      NveAppleImageView *imgView = [[NveAppleImageView alloc] initWithFrame:NSZeroRect];
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

  if ([view isKindOfClass:[NveAppleImageView class]]) {
    nve::apple::update_image_view(
        (NveAppleImageView *)view, text, semantics.resize_mode, node.border_radius);
  } else if ([view isKindOfClass:[NSTextField class]]) {
    NSTextField *field = (NSTextField *)view;
    if (node.kind != NVE_NODE_KIND_TEXT_INPUT) {
      NSString *nextText = nve::apple::nsstring_from_sv(text);
      if (![field.stringValue isEqualToString:nextText]) {
        field.stringValue = nextText;
      }
    }
    field.textColor = nve::apple::color_from_rgba(node.text_rgba);
    if (node.font_size > 0.0f) {
      field.font = [NSFont systemFontOfSize:static_cast<CGFloat>(node.font_size)];
    }
  } else if ([view isKindOfClass:[NveAppleCanvasView class]]) {
    nve::apple::update_canvas_view(view, text);
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
  const uint32_t nodeId = [self.nveHost nodeIdForNativeView:scroll];
  if (nodeId == 0u) return;
  const NSPoint origin = clipView.bounds.origin;
  const NSSize docSize = scroll.documentView != nil ? scroll.documentView.frame.size : NSZeroSize;
  [self.nveHost dispatchScrollOnNodeId:nodeId
                                     x:static_cast<float>(origin.x)
                                     y:static_cast<float>(origin.y)
                          contentWidth:static_cast<float>(docSize.width)
                         contentHeight:static_cast<float>(docSize.height)];
}

- (void)controlTextDidBeginEditing:(NSNotification *)obj {
  if (![obj.object isKindOfClass:[NSTextField class]]) return;
  const uint32_t nodeId = [self.nveHost nodeIdForNativeView:(NSTextField *)obj.object];
  if (nodeId == 0) return;
  [self.nveHost dispatchFocusOnNodeId:nodeId focused:YES];
}

- (void)controlTextDidEndEditing:(NSNotification *)obj {
  if (![obj.object isKindOfClass:[NSTextField class]]) return;
  const uint32_t nodeId = [self.nveHost nodeIdForNativeView:(NSTextField *)obj.object];
  if (nodeId == 0) return;
  [self.nveHost dispatchFocusOnNodeId:nodeId focused:NO];
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
  const uint32_t nodeId = [self.nveHost nodeIdForNativeView:field];
  if (nodeId == 0) return;
  [self.nveHost dispatchTextChangeOnNodeId:nodeId text:field.stringValue ?: @""];
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
      const uint32_t nodeId = [self.nveHost nodeIdForNativeView:field];
      if (nodeId != 0) {
        [self.nveHost dispatchSubmitOnNodeId:nodeId text:field.stringValue ?: @""];
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
    nodeId = [self.nveHost nodeIdForNativeView:recognizer.view];
  }
  if (nodeId == 0) return;
  const NSPoint loc = [recognizer locationInView:recognizer.view];
  [self simulatePressOnNodeId:nodeId atPoint:loc];
}

@end
