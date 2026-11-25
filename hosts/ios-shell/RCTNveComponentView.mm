#import "RCTNveComponentView.h"
#import "NveAppleComponentHost.h"

@interface RCTNveComponentView () <NveAppleComponentViewAdapter, UITextFieldDelegate>
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

- (UIView *)nveCreateNativeViewForSnapshot:(const nve::RenderedNodeSnapshot &)snap {
  UIView *view = nil;
  if (snap.kind == NVE_COMP_TEXT && !snap.children.empty()) {
    view = [[UIView alloc] initWithFrame:CGRectZero];
  } else {
    switch (snap.kind) {
      case NVE_COMP_TEXT: {
        UILabel *label = [[UILabel alloc] initWithFrame:CGRectZero];
        label.numberOfLines = 0;
        view = label;
        break;
      }
      case NVE_COMP_SCROLL_VIEW:
        view = [[UIScrollView alloc] initWithFrame:CGRectZero];
        break;
      case NVE_COMP_TEXT_INPUT: {
        UITextField *field = [[UITextField alloc] initWithFrame:CGRectZero];
        field.delegate = self;
        [field addTarget:self
                      action:@selector(_handleTextFieldChanged:)
            forControlEvents:UIControlEventEditingChanged];
        view = field;
        break;
      }
      default:
        view = [[UIView alloc] initWithFrame:CGRectZero];
        break;
    }
  }
  view.tag = static_cast<NSInteger>(snap.node_id);
  return view;
}

- (void)nveUpdateNativeView:(UIView *)view
                forSnapshot:(const nve::RenderedNodeSnapshot &)snap
                interactive:(BOOL)interactive {
  view.backgroundColor = nve::apple::color_from_rgba(snap.background_rgba);
  view.alpha = static_cast<CGFloat>(snap.opacity);

  if ([view isKindOfClass:[UILabel class]]) {
    UILabel *label = (UILabel *)view;
    NSString *nextText = [NSString stringWithUTF8String:snap.text.c_str()] ?: @"";
    if (![label.text isEqualToString:nextText]) {
      label.text = nextText;
    }
    label.textColor = nve::apple::color_from_rgba(snap.text_color_rgba);
    if (snap.font_size > 0.0f) {
      label.font = [UIFont systemFontOfSize:static_cast<CGFloat>(snap.font_size)];
    }
  } else if ([view isKindOfClass:[UITextField class]]) {
    UITextField *field = (UITextField *)view;
    field.textColor = nve::apple::color_from_rgba(snap.text_color_rgba);
    if (snap.font_size > 0.0f) {
      field.font = [UIFont systemFontOfSize:static_cast<CGFloat>(snap.font_size)];
    }
  }

  view.userInteractionEnabled =
      (interactive || !snap.children.empty() || snap.kind == NVE_COMP_SCROLL_VIEW ||
       snap.kind == NVE_COMP_TEXT_INPUT);
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
