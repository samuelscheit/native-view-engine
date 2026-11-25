#import "RCTNveComponentView.h"
#import "NveAppleCanvasView.h"
#import "NveAppleComponentHost.h"
#import "NveAppleValueBridge.h"

@interface RCTNveComponentView () <UITextFieldDelegate, UIScrollViewDelegate>
@end

@implementation RCTNveComponentView

- (void)handleNveEvent:(NSString *)eventName payload:(NSDictionary<NSString *, id> *)payload {
  [super handleNveEvent:eventName payload:payload];
  id<RCTNveComponentViewDelegate> delegate = self.delegate;
  if ([delegate respondsToSelector:@selector(nveComponentView:didEmitEvent:payload:)]) {
    [delegate nveComponentView:self didEmitEvent:eventName payload:payload];
  }
}

- (void)layoutSubviews {
  [super layoutSubviews];
  [self propagateBoundsToNveHost];
}

- (CGSize)sizeThatFits:(CGSize)size {
  CGSize rootSize = CGSizeZero;
  if ([self.nveHost getRootLayoutSize:&rootSize]) {
    return rootSize;
  }
  return [super sizeThatFits:size];
}

#pragma mark - NveAppleComponentViewAdapter (UIKit)

- (UIView *)nveCreateNativeViewForNode:(const NveRenderNode &)node
                             semantics:(const nve::apple::NodeSemantics &)semantics {
  (void)semantics;
  UIView *view = nil;
  if (node.kind == NVE_NODE_KIND_TEXT && (node.flags & NVE_NODE_LEAF_TEXT) == 0u) {
    view = [[UIView alloc] initWithFrame:CGRectZero];
  } else {
    switch (node.kind) {
      case NVE_NODE_KIND_TEXT: {
        UILabel *label = [[UILabel alloc] initWithFrame:CGRectZero];
        label.numberOfLines = 0;
        view = label;
        break;
      }
      case NVE_NODE_KIND_IMAGE:
        view = [[NveAppleImageView alloc] initWithFrame:CGRectZero];
        break;
      case NVE_NODE_KIND_SCROLL_VIEW: {
        UIScrollView *scroll = [[UIScrollView alloc] initWithFrame:CGRectZero];
        scroll.delegate = self;
        view = scroll;
        break;
      }
      case NVE_NODE_KIND_TEXT_INPUT: {
        UITextField *field = [[UITextField alloc] initWithFrame:CGRectZero];
        field.delegate = self;
        [field addTarget:self
                      action:@selector(_handleTextFieldChanged:)
            forControlEvents:UIControlEventEditingChanged];
        view = field;
        break;
      }
      case NVE_NODE_KIND_SKIA_CANVAS:
        view = [[NveAppleCanvasView alloc] initWithFrame:CGRectZero];
        break;
      default:
        view = [[UIView alloc] initWithFrame:CGRectZero];
        break;
    }
  }
  view.tag = static_cast<NSInteger>(node.node_id);
  return view;
}

- (void)nveWillDestroyNativeView:(UIView *)view forNodeId:(uint32_t)nodeId {
  (void)nodeId;
  if ([view isKindOfClass:[UIScrollView class]]) {
    ((UIScrollView *)view).delegate = nil;
  }
}

- (void)nveUpdateNativeView:(UIView *)view
                    forNode:(const NveRenderNode &)node
                       text:(std::string_view)text
                  semantics:(const nve::apple::NodeSemantics &)semantics
                interactive:(BOOL)interactive {
  view.backgroundColor = nve::apple::color_from_rgba(node.bg_rgba);
  view.alpha = static_cast<CGFloat>(node.opacity_u8) / 255.0;

  if ([view isKindOfClass:[NveAppleImageView class]]) {
    nve::apple::update_image_view(
        (NveAppleImageView *)view, text, semantics.resize_mode, node.border_radius);
  } else if ([view isKindOfClass:[UILabel class]]) {
    UILabel *label = (UILabel *)view;
    NSString *nextText = nve::apple::nsstring_from_sv(text);
    if (![label.text isEqualToString:nextText]) {
      label.text = nextText;
    }
    label.textColor = nve::apple::color_from_rgba(node.text_rgba);
    if (node.font_size > 0.0f) {
      label.font = [UIFont systemFontOfSize:static_cast<CGFloat>(node.font_size)];
    }
  } else if ([view isKindOfClass:[UITextField class]]) {
    UITextField *field = (UITextField *)view;
    field.textColor = nve::apple::color_from_rgba(node.text_rgba);
    if (node.font_size > 0.0f) {
      field.font = [UIFont systemFontOfSize:static_cast<CGFloat>(node.font_size)];
    }
  } else if ([view isKindOfClass:[NveAppleCanvasView class]]) {
    nve::apple::update_canvas_view(view, text);
  }

  view.userInteractionEnabled =
      (interactive || node.child_count > 0u || node.kind == NVE_NODE_KIND_SCROLL_VIEW ||
       node.kind == NVE_NODE_KIND_TEXT_INPUT);
  if (interactive) {
    if (view.gestureRecognizers.count == 0) {
      UITapGestureRecognizer *tap =
          [[UITapGestureRecognizer alloc] initWithTarget:self action:@selector(_handleNodeTap:)];
      [view addGestureRecognizer:tap];
    }
  } else if (view.gestureRecognizers.count > 0) {
    NSArray<UIGestureRecognizer *> *recognizers = [view.gestureRecognizers copy];
    for (UIGestureRecognizer *gr in recognizers) {
      if ([gr isKindOfClass:[UITapGestureRecognizer class]]) {
        [view removeGestureRecognizer:gr];
      }
    }
  }
}

- (void)nveUpdateScrollView:(UIView *)scrollView contentSize:(CGSize)contentSize {
  if ([scrollView isKindOfClass:[UIScrollView class]]) {
    ((UIScrollView *)scrollView).contentSize = contentSize;
  }
}

- (void)scrollViewDidScroll:(UIScrollView *)scrollView {
  const uint32_t nodeId = static_cast<uint32_t>(scrollView.tag);
  if (nodeId == 0u) return;
  const CGPoint offset = scrollView.contentOffset;
  const CGSize contentSize = scrollView.contentSize;
  [self.nveHost dispatchScrollOnNodeId:nodeId
                                     x:static_cast<float>(offset.x)
                                     y:static_cast<float>(offset.y)
                          contentWidth:static_cast<float>(contentSize.width)
                         contentHeight:static_cast<float>(contentSize.height)];
}

- (void)_handleTextFieldChanged:(UITextField *)field {
  if (nve::apple::is_text_input_composing_ime(field)) {
    return;
  }
  const uint32_t nodeId = static_cast<uint32_t>(field.tag);
  if (nodeId == 0) return;
  [self.nveHost dispatchTextChangeOnNodeId:nodeId text:field.text ?: @""];
}

- (void)textFieldDidBeginEditing:(UITextField *)textField {
  const uint32_t nodeId = static_cast<uint32_t>(textField.tag);
  if (nodeId == 0) return;
  [self.nveHost dispatchFocusOnNodeId:nodeId focused:YES];
}

- (void)textFieldDidEndEditing:(UITextField *)textField {
  const uint32_t nodeId = static_cast<uint32_t>(textField.tag);
  if (nodeId == 0) return;
  [self.nveHost dispatchFocusOnNodeId:nodeId focused:NO];
}

- (BOOL)textFieldShouldReturn:(UITextField *)textField {
  if (nve::apple::is_text_input_composing_ime(textField)) {
    return NO;
  }
  const uint32_t nodeId = static_cast<uint32_t>(textField.tag);
  if (nodeId != 0) {
    [self.nveHost dispatchSubmitOnNodeId:nodeId text:textField.text ?: @""];
  }
  return YES;
}

- (void)_handleNodeTap:(UITapGestureRecognizer *)recognizer {
  if (!recognizer.view) return;
  const uint32_t nodeId = static_cast<uint32_t>(recognizer.view.tag);
  const CGPoint loc = [recognizer locationInView:recognizer.view];
  [self simulatePressOnNodeId:nodeId atPoint:loc];
}

@end
