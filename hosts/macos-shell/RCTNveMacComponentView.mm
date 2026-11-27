#import "RCTNveMacComponentView.h"
#import "NveAppleComponentHost.h"

#include <nve/generated/nve_tables.generated.h>

using nve::apple::NodeKind;
using nve::apple::NodeView;

/// Flipped container so runtime top-left layout maps 1:1 onto `NSView.frame`.
@interface NveFlippedContainerView : NSView
@property (nonatomic, assign) uint32_t nveNodeId;
@end

@implementation NveFlippedContainerView
- (BOOL)isFlipped {
  return YES;
}
@end

/// Non-editable label passing mouse hit-testing through to interactive ancestors unless it has
/// its own gesture recognizers.
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

- (void)dealloc {
  [[NSNotificationCenter defaultCenter] removeObserver:self];
}

- (BOOL)isFlipped {
  return YES;
}

- (NSSize)intrinsicContentSize {
  CGSize size = CGSizeZero;
  if ([self.nveHost getRootLayoutSize:&size]) return NSMakeSize(size.width, size.height);
  return [super intrinsicContentSize];
}

- (void)layout {
  [super layout];
  [self propagateBoundsToNveHost];
}

- (void)handleNveEvent:(NSString *)eventName payload:(NSDictionary<NSString *, id> *)payload {
  [super handleNveEvent:eventName payload:payload];
  id<RCTNveMacComponentViewDelegate> delegate = self.delegate;
  if ([delegate respondsToSelector:@selector(nveComponentView:didEmitEvent:payload:)]) {
    [delegate nveComponentView:self didEmitEvent:eventName payload:payload];
  }
}

#pragma mark - NveAppleComponentViewAdapter (AppKit)

- (NSView *)nveCreateNativeViewForNode:(const NodeView &)node {
  switch (node.kind) {
    case NodeKind::Text: {
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
    case NodeKind::Image: {
      NveAppleImageView *image = [[NveAppleImageView alloc] initWithFrame:NSZeroRect];
      image.imageFrameStyle = NSImageFrameNone;
      image.imageAlignment = NSImageAlignCenter;
      image.wantsLayer = YES;
      return image;
    }
    case NodeKind::TextInput: {
      NSTextField *input = node.semantics.secure_text_entry ? [[NSSecureTextField alloc] initWithFrame:NSZeroRect]
                                                            : [[NSTextField alloc] initWithFrame:NSZeroRect];
      input.editable = YES;
      input.bezeled = YES;
      input.delegate = self;
      input.wantsLayer = YES;
      return input;
    }
    case NodeKind::Scroll: {
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
    case NodeKind::Container: {
      NveFlippedContainerView *container = [[NveFlippedContainerView alloc] initWithFrame:NSZeroRect];
      container.nveNodeId = node.node_id;
      container.wantsLayer = YES;
      return container;
    }
  }
  return [[NveFlippedContainerView alloc] initWithFrame:NSZeroRect];
}

- (void)nveUpdateNativeView:(NSView *)view forNode:(const NodeView &)node image:(id)image {
  [super nveUpdateNativeView:view forNode:node image:image];
  if ([view isKindOfClass:[NveFlippedContainerView class]]) ((NveFlippedContainerView *)view).nveNodeId = node.node_id;
  if ([view isKindOfClass:[NveAppleImageView class]]) {
    nve::apple::update_image_view((NveAppleImageView *)view, image, node.image_fit, node.style.border_radius);
    if (node.image_tint_rgba >> 24u) ((NveAppleImageView *)view).contentTintColor = nve::apple::color_from_rgba(node.image_tint_rgba);
  } else if ([view isKindOfClass:[NSTextField class]]) {
    NSTextField *field = (NSTextField *)view;
    if (node.kind == NodeKind::TextInput) {
      nve::apple::apply_text_input_props(field, node.text, node.semantics);
    } else {
      NSString *next = nve::apple::nsstring_from_sv(node.text);
      if (![field.stringValue isEqualToString:next]) field.stringValue = next;
      field.maximumNumberOfLines = static_cast<NSInteger>(node.style.max_lines);
      field.alignment = node.style.text_align == NVE_TEXT_ALIGN_CENTER ? NSTextAlignmentCenter
                        : (node.style.text_align == NVE_TEXT_ALIGN_RIGHT || node.style.text_align == NVE_TEXT_ALIGN_END)
                            ? NSTextAlignmentRight
                            : NSTextAlignmentLeft;
    }
    field.textColor = nve::apple::color_from_rgba(node.style.text_rgba);
    field.font = nve::apple::platform_font(node.style.font_size, node.style.font_weight, node.style.italic);
  }

  NSArray<NSGestureRecognizer *> *clicks = [view.gestureRecognizers
      filteredArrayUsingPredicate:[NSPredicate predicateWithBlock:^BOOL(id gr, NSDictionary *) {
        return [gr isKindOfClass:[NSClickGestureRecognizer class]] && ((NSGestureRecognizer *)gr).target == self;
      }]];
  if (node.interactive && clicks.count == 0) {
    [view addGestureRecognizer:[[NSClickGestureRecognizer alloc] initWithTarget:self action:@selector(_handleNodeClick:)]];
  } else if (!node.interactive) {
    for (NSGestureRecognizer *gr in clicks) [view removeGestureRecognizer:gr];
  }
}

- (void)nveWillDestroyNativeView:(NSView *)view forNodeId:(uint32_t)nodeId {
  (void)nodeId;
  if ([view isKindOfClass:[NSScrollView class]]) {
    [[NSNotificationCenter defaultCenter] removeObserver:self
                                                    name:NSViewBoundsDidChangeNotification
                                                  object:((NSScrollView *)view).contentView];
  }
}

- (NSView *)nveChildContainerForParentView:(NSView *)parentView {
  if ([parentView isKindOfClass:[NSScrollView class]] && ((NSScrollView *)parentView).documentView != nil) {
    return ((NSScrollView *)parentView).documentView;
  }
  return parentView;
}

- (void)nveUpdateScrollView:(NSView *)scrollView contentSize:(CGSize)contentSize {
  if (![scrollView isKindOfClass:[NSScrollView class]]) return;
  NSScrollView *scroll = (NSScrollView *)scrollView;
  const NSRect next = NSMakeRect(0.0, 0.0, contentSize.width, contentSize.height);
  if (scroll.documentView != nil && !NSEqualRects(scroll.documentView.frame, next)) {
    _updatingScrollFrame = YES;
    scroll.documentView.frame = next;
    _updatingScrollFrame = NO;
  }
}

#pragma mark - Input

- (void)_handleScrollViewBoundsChanged:(NSNotification *)note {
  if (_updatingScrollFrame || ![note.object isKindOfClass:[NSClipView class]]) return;
  NSClipView *clip = (NSClipView *)note.object;
  if (![clip.superview isKindOfClass:[NSScrollView class]]) return;
  NSScrollView *scroll = (NSScrollView *)clip.superview;
  const uint32_t nodeId = [self.nveHost nodeIdForNativeView:scroll];
  if (nodeId == 0u) return;
  const NSSize doc = scroll.documentView != nil ? scroll.documentView.frame.size : NSZeroSize;
  [self.nveHost dispatchScrollOnNodeId:nodeId
                                     x:static_cast<float>(clip.bounds.origin.x)
                                     y:static_cast<float>(clip.bounds.origin.y)
                          contentWidth:static_cast<float>(doc.width)
                         contentHeight:static_cast<float>(doc.height)];
}

- (uint32_t)_nodeIdForControl:(id)object {
  return [object isKindOfClass:[NSTextField class]] ? [self.nveHost nodeIdForNativeView:(NSTextField *)object] : 0u;
}

- (void)controlTextDidBeginEditing:(NSNotification *)obj {
  if (const uint32_t nodeId = [self _nodeIdForControl:obj.object]) [self.nveHost dispatchFocusOnNodeId:nodeId focused:YES];
}

- (void)controlTextDidEndEditing:(NSNotification *)obj {
  if (const uint32_t nodeId = [self _nodeIdForControl:obj.object]) [self.nveHost dispatchFocusOnNodeId:nodeId focused:NO];
}

- (void)controlTextDidChange:(NSNotification *)obj {
  if (![obj.object isKindOfClass:[NSTextField class]]) return;
  NSTextField *field = (NSTextField *)obj.object;
  if (nve::apple::is_text_input_composing_ime(field)) return;  // never send half-composed IME text
  id editor = obj.userInfo[@"NSFieldEditor"];
  if ([editor isKindOfClass:[NSTextView class]] && (((NSTextView *)editor).hasMarkedText || ((NSTextView *)editor).markedRange.length > 0)) {
    return;
  }
  if (const uint32_t nodeId = [self _nodeIdForControl:field]) {
    [self.nveHost dispatchTextChangeOnNodeId:nodeId text:field.stringValue ?: @""];
  }
}

- (BOOL)control:(NSControl *)control textView:(NSTextView *)textView doCommandBySelector:(SEL)commandSelector {
  if (commandSelector != @selector(insertNewline:)) return NO;
  if (textView != nil && (textView.hasMarkedText || textView.markedRange.length > 0)) return NO;
  if (![control isKindOfClass:[NSTextField class]] || !((NSTextField *)control).usesSingleLineMode) return NO;
  const uint32_t nodeId = [self _nodeIdForControl:control];
  if (nodeId == 0u) return NO;
  [self.nveHost dispatchSubmitOnNodeId:nodeId text:((NSTextField *)control).stringValue ?: @""];
  return YES;
}

- (void)_handleNodeClick:(NSClickGestureRecognizer *)recognizer {
  if (recognizer.view == nil) return;
  const uint32_t nodeId = [recognizer.view isKindOfClass:[NveFlippedContainerView class]]
                              ? ((NveFlippedContainerView *)recognizer.view).nveNodeId
                              : [self.nveHost nodeIdForNativeView:recognizer.view];
  if (nodeId == 0u) return;
  [self simulatePressOnNodeId:nodeId atPoint:[recognizer locationInView:recognizer.view]];
}

@end
