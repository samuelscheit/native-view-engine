#import "RCTNveComponentView.h"
#import "NveAppleCanvasView.h"
#import "NveAppleComponentHost.h"

#include <nve/generated/nve_tables.generated.h>

using nve::apple::NodeKind;
using nve::apple::NodeView;

@interface RCTNveComponentView () <UITextFieldDelegate, UIScrollViewDelegate>
@end

@implementation RCTNveComponentView {
  BOOL _updatingScrollFrame;
}

- (void)handleNveEvent:(NSString *)eventName payload:(NSDictionary<NSString *, id> *)payload {
  [super handleNveEvent:eventName payload:payload];
  id<RCTNveComponentViewDelegate> delegate = self.delegate;
  if ([delegate respondsToSelector:@selector(nveComponentView:didEmitEvent:payload:)]) {
    [delegate nveComponentView:self didEmitEvent:eventName payload:payload];
  }
}

- (CGSize)sizeThatFits:(CGSize)size {
  CGSize root = CGSizeZero;
  return [self.nveHost getRootLayoutSize:&root] ? root : [super sizeThatFits:size];
}

#pragma mark - NveAppleComponentViewAdapter (UIKit)

- (UIView *)nveCreateNativeViewForNode:(const NodeView &)node {
  UIView *view = nil;
  switch (node.kind) {
    case NodeKind::Text: {
      UILabel *label = [[UILabel alloc] initWithFrame:CGRectZero];
      label.numberOfLines = 0;
      view = label;
      break;
    }
    case NodeKind::Image:
      view = [[NveAppleImageView alloc] initWithFrame:CGRectZero];
      break;
    case NodeKind::Scroll: {
      UIScrollView *scroll = [[UIScrollView alloc] initWithFrame:CGRectZero];
      scroll.delegate = self;
      view = scroll;
      break;
    }
    case NodeKind::TextInput: {
      UITextField *field = [[UITextField alloc] initWithFrame:CGRectZero];
      field.delegate = self;
      [field addTarget:self action:@selector(_handleTextFieldChanged:) forControlEvents:UIControlEventEditingChanged];
      view = field;
      break;
    }
    case NodeKind::Canvas:
      view = [[NveAppleCanvasView alloc] initWithFrame:CGRectZero];
      break;
    case NodeKind::Container:
      view = [[UIView alloc] initWithFrame:CGRectZero];
      break;
  }
  view.tag = static_cast<NSInteger>(node.node_id);
  return view;
}

- (void)nveWillDestroyNativeView:(UIView *)view forNodeId:(uint32_t)nodeId {
  (void)nodeId;
  if ([view isKindOfClass:[UIScrollView class]]) ((UIScrollView *)view).delegate = nil;
}

- (void)nveUpdateNativeView:(UIView *)view forNode:(const NodeView &)node image:(id)image {
  [super nveUpdateNativeView:view forNode:node image:image];
  view.tag = static_cast<NSInteger>(node.node_id);
  UIFont *font = nve::apple::platform_font(
      node.style.font_size, node.style.font_weight, node.style.italic, node.style.font_family);
  if ([view isKindOfClass:[NveAppleImageView class]]) {
    nve::apple::update_image_view((NveAppleImageView *)view, image, node.image_fit, node.style.border_radius);
    // A cleared tint must clear the view's tint too, not keep the last one.
    view.tintColor = (node.image_tint_rgba >> 24u) != 0u ? nve::apple::color_from_rgba(node.image_tint_rgba) : nil;
  } else if ([view isKindOfClass:[UILabel class]]) {
    UILabel *label = (UILabel *)view;
    const nve::apple::TextSpec spec = nve::apple::text_spec_from_style(node.style);
    const bool noWrap = node.style.white_space == NVE_WHITE_SPACE_NO_WRAP ||
                        node.style.white_space == NVE_WHITE_SPACE_PRE;
    label.textColor = nve::apple::color_from_rgba(node.style.text_rgba);
    label.font = font;
    label.lineBreakMode = nve::apple::line_break_mode_for(spec, false);
    label.numberOfLines = noWrap ? 1 : static_cast<NSInteger>(node.style.max_lines);
    label.textAlignment = node.style.text_align == NVE_TEXT_ALIGN_CENTER ? NSTextAlignmentCenter
                          : (node.style.text_align == NVE_TEXT_ALIGN_RIGHT || node.style.text_align == NVE_TEXT_ALIGN_END)
                              ? NSTextAlignmentRight
                              : NSTextAlignmentNatural;
    NSAttributedString *attributed = nve::apple::attributed_text(node.text, spec);
    if (![label.attributedText isEqualToAttributedString:attributed]) {
      label.attributedText = attributed;
    }
  } else if ([view isKindOfClass:[UITextField class]]) {
    UITextField *field = (UITextField *)view;
    nve::apple::apply_text_input_props(field, node.text, node.semantics);
    field.textColor = nve::apple::color_from_rgba(node.style.text_rgba);
    field.font = font;
  }

  view.userInteractionEnabled = node.interactive || node.kind == NodeKind::Scroll || node.kind == NodeKind::TextInput ||
                                node.kind == NodeKind::Container;
  NSArray<UIGestureRecognizer *> *taps = [view.gestureRecognizers
      filteredArrayUsingPredicate:[NSPredicate predicateWithBlock:^BOOL(id gr, NSDictionary *) {
        return [gr isKindOfClass:[UITapGestureRecognizer class]];
      }]];
  if (node.interactive && taps.count == 0) {
    [view addGestureRecognizer:[[UITapGestureRecognizer alloc] initWithTarget:self action:@selector(_handleNodeTap:)]];
  } else if (!node.interactive) {
    for (UIGestureRecognizer *gr in taps) [view removeGestureRecognizer:gr];
  }
}

- (void)nveUpdateScrollView:(UIView *)scrollView contentSize:(CGSize)contentSize {
  if (![scrollView isKindOfClass:[UIScrollView class]]) return;
  UIScrollView *scroll = (UIScrollView *)scrollView;
  if (CGSizeEqualToSize(scroll.contentSize, contentSize)) return;
  // A shrinking content size clamps the offset and fires `scrollViewDidScroll:`: that is the
  // runtime's own layout, not a user scroll to report back.
  _updatingScrollFrame = YES;
  scroll.contentSize = contentSize;
  _updatingScrollFrame = NO;
}

#pragma mark - Input

- (void)scrollViewDidScroll:(UIScrollView *)scrollView {
  if (_updatingScrollFrame) return;
  const auto nodeId = static_cast<uint32_t>(scrollView.tag);
  if (nodeId == 0u) return;
  [self.nveHost dispatchScrollOnNodeId:nodeId
                                     x:static_cast<float>(scrollView.contentOffset.x)
                                     y:static_cast<float>(scrollView.contentOffset.y)
                          contentWidth:static_cast<float>(scrollView.contentSize.width)
                         contentHeight:static_cast<float>(scrollView.contentSize.height)];
}

- (void)_handleTextFieldChanged:(UITextField *)field {
  if (nve::apple::is_text_input_composing_ime(field)) return;  // never send half-composed IME text
  if (const auto nodeId = static_cast<uint32_t>(field.tag)) [self.nveHost dispatchTextChangeOnNodeId:nodeId text:field.text ?: @""];
}

- (void)textFieldDidBeginEditing:(UITextField *)textField {
  if (const auto nodeId = static_cast<uint32_t>(textField.tag)) [self.nveHost dispatchFocusOnNodeId:nodeId focused:YES];
}

- (void)textFieldDidEndEditing:(UITextField *)textField {
  if (const auto nodeId = static_cast<uint32_t>(textField.tag)) [self.nveHost dispatchFocusOnNodeId:nodeId focused:NO];
}

- (BOOL)textFieldShouldReturn:(UITextField *)textField {
  if (nve::apple::is_text_input_composing_ime(textField)) return NO;
  if (const auto nodeId = static_cast<uint32_t>(textField.tag)) [self.nveHost dispatchSubmitOnNodeId:nodeId text:textField.text ?: @""];
  return YES;
}

- (void)_handleNodeTap:(UITapGestureRecognizer *)recognizer {
  if (recognizer.view == nil) return;
  [self simulatePressOnNodeId:static_cast<uint32_t>(recognizer.view.tag) atPoint:[recognizer locationInView:recognizer.view]];
}

@end
