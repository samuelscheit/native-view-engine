#import "RCTNveComponentView.h"
#import "NveAppleCanvasView.h"
#import "NveAppleComponentHost.h"
#import "NveAppleImageCache.h"

@interface NveIosImageView : UIImageView
@property (nonatomic, copy, nullable) NSString *nveImageSrc;
@end

@implementation NveIosImageView
@end

@interface RCTNveComponentView () <NveAppleComponentViewAdapter, UITextFieldDelegate, UIScrollViewDelegate>
@end

@implementation RCTNveComponentView {
  NveAppleComponentHost *_host;
}

- (instancetype)initWithFrame:(CGRect)frame {
  self = [super initWithFrame:frame];
  if (self) {
    _host = [[NveAppleComponentHost alloc] initWithAdapter:self
                                               errorDomain:@"nve.ios.rncomponent"];
    NveGetInterfaceFn defaultIface = [self defaultNveGetInterface];
    if (defaultIface != nullptr) {
      [_host attachInterface:defaultIface surfaceId:_host.surfaceId];
    }
  }
  return self;
}

- (uint32_t)nveSurfaceId {
  return _host.surfaceId;
}

- (uint32_t)nveNodeCount {
  return _host.nodeCount;
}

- (CGFloat)_currentDisplayScale {
  const CGFloat scale = self.traitCollection.displayScale;
  return scale > 0.0 ? scale : 2.0;
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

- (BOOL)hasReplayedFrame0 {
  return _host.hasReplayedFrame0;
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

- (void)simulatePressOnNodeId:(uint32_t)nodeId atPoint:(CGPoint)point {
  [_host dispatchPressOnNodeId:nodeId x:static_cast<float>(point.x) y:static_cast<float>(point.y)];
}

- (nullable UIView *)nativeViewForNodeId:(uint32_t)nodeId {
  return [_host nativeViewForNodeId:nodeId];
}

- (void)handleNveEvent:(NSString *)eventName payload:(NSDictionary<NSString *, id> *)payload {
  if (self.onNveEvent) {
    self.onNveEvent(eventName, payload);
  }
  id<RCTNveComponentViewDelegate> delegate = self.delegate;
  if ([delegate respondsToSelector:@selector(nveComponentView:didEmitEvent:payload:)]) {
    [delegate nveComponentView:self didEmitEvent:eventName payload:payload];
  }
}

- (void)layoutSubviews {
  [super layoutSubviews];
  const CGRect b = self.bounds;
  [_host updateSurfaceWidth:static_cast<float>(b.size.width)
                     height:static_cast<float>(b.size.height)
                scaleFactor:static_cast<float>([self _currentDisplayScale])];
}

- (CGSize)sizeThatFits:(CGSize)size {
  CGSize rootSize = CGSizeZero;
  if ([_host getRootLayoutSize:&rootSize]) {
    return rootSize;
  }
  return [super sizeThatFits:size];
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

#pragma mark - NveAppleComponentViewAdapter (UIKit)

- (UIView *)nveHostRootView {
  return self;
}

- (NveSurfaceConfig)nveSurfaceConfigForSurfaceId:(uint32_t)surfaceId {
  const CGRect bounds = self.bounds;
  NveSurfaceConfig cfg{};
  cfg.surface_id = surfaceId;
  cfg.width = static_cast<float>(bounds.size.width > 0.0 ? bounds.size.width : 320.0);
  cfg.height = static_cast<float>(bounds.size.height > 0.0 ? bounds.size.height : 200.0);
  cfg.scale_factor = static_cast<float>([self _currentDisplayScale]);
  return cfg;
}

- (void)nveEmitEvent:(NSString *)eventName payload:(NSDictionary<NSString *, id> *)payload {
  [self handleNveEvent:eventName payload:payload];
}

- (void)nveApplyRootBackgroundRgba:(uint32_t)rgba {
  if (rgba != 0) {
    self.backgroundColor = nve::apple::color_from_rgba(rgba);
  }
}

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
      case NVE_NODE_KIND_IMAGE: {
        NveIosImageView *imgView = [[NveIosImageView alloc] initWithFrame:CGRectZero];
        view = imgView;
        break;
      }
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
      case NVE_NODE_KIND_SKIA_CANVAS: {
        view = [[NveAppleCanvasView alloc] initWithFrame:CGRectZero];
        break;
      }
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

  if ([view isKindOfClass:[NveIosImageView class]]) {
    NveIosImageView *imgView = (NveIosImageView *)view;
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
  } else if ([view isKindOfClass:[UILabel class]]) {
    UILabel *label = (UILabel *)view;
    NSString *nextText =
        text.empty() ? @""
                     : ([[NSString alloc] initWithBytes:text.data()
                                                 length:text.size()
                                               encoding:NSUTF8StringEncoding] ?: @"");
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
    NveAppleCanvasView *canvasView = (NveAppleCanvasView *)view;
    const std::span<const uint8_t> drawBytes(
        reinterpret_cast<const uint8_t *>(text.data()), text.size());
    [canvasView updateDrawListBytes:drawBytes];
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
  [_host dispatchScrollOnNodeId:nodeId
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
  [_host dispatchTextChangeOnNodeId:nodeId text:field.text ?: @""];
}

- (void)textFieldDidBeginEditing:(UITextField *)textField {
  const uint32_t nodeId = static_cast<uint32_t>(textField.tag);
  if (nodeId == 0) return;
  [_host dispatchFocusOnNodeId:nodeId focused:YES];
}

- (void)textFieldDidEndEditing:(UITextField *)textField {
  const uint32_t nodeId = static_cast<uint32_t>(textField.tag);
  if (nodeId == 0) return;
  [_host dispatchFocusOnNodeId:nodeId focused:NO];
}

- (BOOL)textFieldShouldReturn:(UITextField *)textField {
  if (nve::apple::is_text_input_composing_ime(textField)) {
    return NO;
  }
  const uint32_t nodeId = static_cast<uint32_t>(textField.tag);
  if (nodeId != 0) {
    [_host dispatchSubmitOnNodeId:nodeId text:textField.text ?: @""];
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
