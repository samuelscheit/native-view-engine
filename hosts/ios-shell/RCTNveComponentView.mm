#import "RCTNveComponentView.h"
#import "NveAppleCanvasView.h"
#import "NveAppleComponentHost.h"

#include <nve/generated/nve_tables.generated.h>

#include <unordered_set>

using nve::apple::NodeKind;
using nve::apple::NodeView;

@interface NveIosLabelView : UILabel
@property (nonatomic, assign) BOOL nveOverline;
@property (nonatomic, strong, nullable) UIColor *nveOverlineColor;
@end

@implementation NveIosLabelView
- (void)drawTextInRect:(CGRect)rect {
  [super drawTextInRect:rect];
  if (self.nveOverline && self.text.length > 0 && rect.size.width > 0.0) {
    CGContextRef ctx = UIGraphicsGetCurrentContext();
    if (ctx != nullptr) {
      UIColor *col = self.nveOverlineColor ?: self.textColor ?: UIColor.labelColor;
      CGContextSaveGState(ctx);
      CGContextSetFillColorWithColor(ctx, col.CGColor);
      CGContextFillRect(ctx, CGRectMake(rect.origin.x, rect.origin.y + 0.5, rect.size.width, 1.0));
      CGContextRestoreGState(ctx);
    }
  }
}
@end

@interface NveIosTextView : UITextView
@property (nonatomic, readonly) UILabel *nvePlaceholderLabel;
- (void)setNvePlaceholder:(nullable NSString *)placeholder font:(UIFont *)font color:(UIColor *)color;
- (void)updateNvePlaceholderVisibility;
@end

@implementation NveIosTextView
- (instancetype)initWithFrame:(CGRect)frame textContainer:(NSTextContainer *)textContainer {
  if ((self = [super initWithFrame:frame textContainer:textContainer])) {
    _nvePlaceholderLabel = [[UILabel alloc] initWithFrame:CGRectZero];
    _nvePlaceholderLabel.numberOfLines = 0;
    _nvePlaceholderLabel.userInteractionEnabled = NO;
    _nvePlaceholderLabel.translatesAutoresizingMaskIntoConstraints = NO;
    [self addSubview:_nvePlaceholderLabel];
    [NSLayoutConstraint activateConstraints:@[
      [_nvePlaceholderLabel.topAnchor constraintEqualToAnchor:self.topAnchor constant:8.0],
      [_nvePlaceholderLabel.leadingAnchor constraintEqualToAnchor:self.leadingAnchor constant:5.0],
      [_nvePlaceholderLabel.trailingAnchor constraintLessThanOrEqualToAnchor:self.frameLayoutGuide.trailingAnchor
                                                                    constant:-5.0],
    ]];
  }
  return self;
}
- (void)setNvePlaceholder:(nullable NSString *)placeholder font:(UIFont *)font color:(UIColor *)color {
  _nvePlaceholderLabel.text = placeholder;
  _nvePlaceholderLabel.font = font;
  _nvePlaceholderLabel.textColor = [color colorWithAlphaComponent:0.45];
  [self updateNvePlaceholderVisibility];
}
- (void)updateNvePlaceholderVisibility {
  _nvePlaceholderLabel.hidden = (self.text.length > 0u) || (_nvePlaceholderLabel.text.length == 0u);
}
@end

@interface RCTNveComponentView () <UITextFieldDelegate, UITextViewDelegate, UIScrollViewDelegate>
@end

@implementation RCTNveComponentView {
  BOOL _updatingScrollFrame;
  std::unordered_set<uint32_t> _readOnlyInputs;
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
      NveIosLabelView *label = [[NveIosLabelView alloc] initWithFrame:CGRectZero];
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
      if (node.semantics.multiline) {
        NveIosTextView *tv = [[NveIosTextView alloc] initWithFrame:CGRectZero textContainer:nil];
        tv.backgroundColor = UIColor.clearColor;
        tv.delegate = self;
        view = tv;
      } else {
        UITextField *field = [[UITextField alloc] initWithFrame:CGRectZero];
        field.delegate = self;
        [field addTarget:self action:@selector(_handleTextFieldChanged:) forControlEvents:UIControlEventEditingChanged];
        view = field;
      }
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
  _readOnlyInputs.erase(nodeId);
  [self.nveHost clearTextInputStateForNodeId:nodeId];
  if ([view isKindOfClass:[UIScrollView class]]) ((UIScrollView *)view).delegate = nil;
  if ([view isKindOfClass:[UITextField class]]) {
    UITextField *f = (UITextField *)view;
    [f removeTarget:self action:@selector(_handleTextFieldChanged:) forControlEvents:UIControlEventEditingChanged];
    f.delegate = nil;
  } else if ([view isKindOfClass:[UITextView class]]) {
    ((UITextView *)view).delegate = nil;
  }
}

- (void)nveUpdateNativeView:(UIView *)view forNode:(const NodeView &)node image:(id)image {
  [super nveUpdateNativeView:view forNode:node image:image];
  view.tag = static_cast<NSInteger>(node.node_id);
  UIFont *font = nve::apple::platform_font(
      node.style.font_size, node.style.font_weight, node.style.italic, node.style.font_family);
  UIColor *color = nve::apple::color_from_rgba(node.style.text_rgba);
  if ([view isKindOfClass:[NveAppleImageView class]]) {
    nve::apple::update_image_view((NveAppleImageView *)view, image, node.image_fit, node.style.border_radius);
    view.tintColor = (node.image_tint_rgba >> 24u) != 0u ? nve::apple::color_from_rgba(node.image_tint_rgba) : nil;
  } else if ([view isKindOfClass:[UILabel class]]) {
    UILabel *label = (UILabel *)view;
    const nve::apple::TextSpec spec = nve::apple::text_spec_from_style(node.style);
    const bool singleLineNoWrap = node.style.white_space == NVE_WHITE_SPACE_NO_WRAP;
    label.textColor = color;
    label.font = font;
    label.lineBreakMode = nve::apple::line_break_mode_for(spec, false);
    label.numberOfLines = singleLineNoWrap ? 1 : static_cast<NSInteger>(node.style.max_lines);
    label.textAlignment = node.style.text_align == NVE_TEXT_ALIGN_CENTER ? NSTextAlignmentCenter
                          : (node.style.text_align == NVE_TEXT_ALIGN_RIGHT || node.style.text_align == NVE_TEXT_ALIGN_END)
                              ? NSTextAlignmentRight
                              : NSTextAlignmentNatural;
    if ([label isKindOfClass:[NveIosLabelView class]]) {
      NveIosLabelView *lbl = (NveIosLabelView *)label;
      const BOOL overline = (node.style.text_decoration & NVE_TEXT_DECORATION_OVERLINE) != 0u;
      if (lbl.nveOverline != overline || lbl.nveOverlineColor != color) {
        lbl.nveOverline = overline;
        lbl.nveOverlineColor = color;
        [lbl setNeedsDisplay];
      }
    }
    NSAttributedString *attributed = nve::apple::attributed_text(node.text, spec);
    if (![label.attributedText isEqualToAttributedString:attributed]) {
      label.attributedText = attributed;
    }
  } else if ([view isKindOfClass:[UITextField class]] || [view isKindOfClass:[UITextView class]]) {
    if (node.semantics.read_only || node.semantics.disabled) {
      _readOnlyInputs.insert(node.node_id);
    } else {
      _readOnlyInputs.erase(node.node_id);
    }
    [self.nveHost setTextInputMaxLength:node.semantics.max_length forNodeId:node.node_id];
    NSString *nextVal = nve::apple::nsstring_from_sv(node.text);
    NSString *shown = [view isKindOfClass:[UITextField class]] ? (((UITextField *)view).text ?: @"")
                                                               : (((UITextView *)view).text ?: @"");
    const BOOL shouldWrite = [self.nveHost shouldApplyAppText:nextVal
                                                     toNodeId:node.node_id
                                                    shownText:shown
                                                hasMarkedText:nve::apple::is_text_input_composing_ime(view)];
    nve::apple::apply_text_input_props(view, node.text, node.semantics, shouldWrite == YES);
    if ([view isKindOfClass:[UITextField class]]) {
      UITextField *field = (UITextField *)view;
      field.textColor = color;
      field.font = font;
    } else {
      UITextView *tv = (UITextView *)view;
      tv.textColor = color;
      tv.font = font;
      if ([tv isKindOfClass:[NveIosTextView class]]) {
        [(NveIosTextView *)tv setNvePlaceholder:nve::apple::nsstring_or_nil_from_sv(node.semantics.placeholder)
                                           font:font
                                          color:color];
      }
    }
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
  _updatingScrollFrame = YES;
  scroll.contentSize = contentSize;
  _updatingScrollFrame = NO;
}

#pragma mark - Input

- (void)scrollViewDidScroll:(UIScrollView *)scrollView {
  if (_updatingScrollFrame || [scrollView isKindOfClass:[UITextView class]]) return;
  const auto nodeId = static_cast<uint32_t>(scrollView.tag);
  if (nodeId == 0u) return;
  [self.nveHost dispatchScrollOnNodeId:nodeId
                                     x:static_cast<float>(scrollView.contentOffset.x)
                                     y:static_cast<float>(scrollView.contentOffset.y)
                          contentWidth:static_cast<float>(scrollView.contentSize.width)
                         contentHeight:static_cast<float>(scrollView.contentSize.height)];
}

- (BOOL)textFieldShouldBeginEditing:(UITextField *)textField {
  return _readOnlyInputs.count(static_cast<uint32_t>(textField.tag)) == 0u;
}

- (BOOL)textField:(UITextField *)textField shouldChangeCharactersInRange:(NSRange)range replacementString:(NSString *)string {
  (void)range;
  (void)string;
  return _readOnlyInputs.count(static_cast<uint32_t>(textField.tag)) == 0u;
}

- (void)_handleTextFieldChanged:(UITextField *)field {
  const auto nodeId = static_cast<uint32_t>(field.tag);
  if (nodeId == 0u) return;
  UITextRange *marked = field.markedTextRange;
  NSString *markedText = marked != nil ? [field textInRange:marked] : nil;
  __weak UITextField *weakField = field;
  [self.nveHost handleTextInputEditOnNodeId:nodeId
                                       text:(field.text ?: @"")
                              hasMarkedText:(marked != nil)
                                 markedText:markedText
                                  writeBack:^(NSString *nextText) {
                                    if (weakField != nil) nve::apple::write_text_input_preserving_selection(weakField, nextText);
                                  }];
}

- (void)textFieldDidBeginEditing:(UITextField *)textField {
  if (const auto nodeId = static_cast<uint32_t>(textField.tag)) {
    [self.nveHost handleTextInputFocusOnNodeId:nodeId focused:YES];
  }
}

- (void)textFieldDidEndEditing:(UITextField *)textField {
  if (const auto nodeId = static_cast<uint32_t>(textField.tag)) {
    [self.nveHost handleTextInputFocusOnNodeId:nodeId focused:NO];
  }
}

- (BOOL)textFieldShouldReturn:(UITextField *)textField {
  const auto nodeId = static_cast<uint32_t>(textField.tag);
  if (nodeId == 0u || _readOnlyInputs.count(nodeId) != 0u) return NO;
  return [self.nveHost handleTextInputSubmitOnNodeId:nodeId
                                                text:(textField.text ?: @"")
                                       hasMarkedText:(textField.markedTextRange != nil)];
}

- (BOOL)textView:(UITextView *)textView shouldChangeTextInRange:(NSRange)range replacementText:(NSString *)text {
  (void)range;
  (void)text;
  return _readOnlyInputs.count(static_cast<uint32_t>(textView.tag)) == 0u;
}

- (void)textViewDidBeginEditing:(UITextView *)textView {
  if (const auto nodeId = static_cast<uint32_t>(textView.tag)) {
    [self.nveHost handleTextInputFocusOnNodeId:nodeId focused:YES];
  }
}

- (void)textViewDidEndEditing:(UITextView *)textView {
  if (const auto nodeId = static_cast<uint32_t>(textView.tag)) {
    [self.nveHost handleTextInputFocusOnNodeId:nodeId focused:NO];
  }
}

- (void)textViewDidChangeSelection:(UITextView *)textView {
  if (textView.markedTextRange != nil) return;
  const auto nodeId = static_cast<uint32_t>(textView.tag);
  if (nodeId != 0u && [self.nveHost isTextInputComposingForNodeId:nodeId]) {
    [self textViewDidChange:textView];
  }
}

- (void)textViewDidChange:(UITextView *)textView {
  if ([textView isKindOfClass:[NveIosTextView class]]) {
    [(NveIosTextView *)textView updateNvePlaceholderVisibility];
  }
  const auto nodeId = static_cast<uint32_t>(textView.tag);
  if (nodeId == 0u) return;
  UITextRange *marked = textView.markedTextRange;
  NSString *markedText = marked != nil ? [textView textInRange:marked] : nil;
  __weak UITextView *weakTv = textView;
  [self.nveHost handleTextInputEditOnNodeId:nodeId
                                       text:(textView.text ?: @"")
                              hasMarkedText:(marked != nil)
                                 markedText:markedText
                                  writeBack:^(NSString *nextText) {
                                    UITextView *strongTv = weakTv;
                                    if (strongTv == nil) return;
                                    nve::apple::write_text_input_preserving_selection(strongTv, nextText);
                                    if ([strongTv isKindOfClass:[NveIosTextView class]]) {
                                      [(NveIosTextView *)strongTv updateNvePlaceholderVisibility];
                                    }
                                  }];
}

- (void)_handleNodeTap:(UITapGestureRecognizer *)recognizer {
  if (recognizer.view == nil) return;
  [self simulatePressOnNodeId:static_cast<uint32_t>(recognizer.view.tag) atPoint:[recognizer locationInView:recognizer.view]];
}

@end
