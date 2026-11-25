#import <AppKit/AppKit.h>
#import "NveAppleComponentViewBase.h"

#define NVE_MAC_COMPONENT_BASE_CLASS NVE_APPLE_COMPONENT_BASE_CLASS

NS_ASSUME_NONNULL_BEGIN

/// Callback block fired whenever the embedded NVE component emits an outbound event
/// via `NveComponentApi::drain_emitted_events`.
typedef NveAppleEventHandler RCTNveMacEventHandler;

@protocol RCTNveMacComponentViewDelegate <NSObject>
@optional
- (void)nveComponentView:(NSView *)view
            didEmitEvent:(NSString *)eventName
                 payload:(NSDictionary<NSString *, id> *)payload;
@end

/// Embeddable macOS AppKit / `react-native-macos` Fabric `RCTViewComponentView`-compatible view
/// that hosts any NVE Rust component (`nve_get_interface` / `NveModule`) inside a native macOS app.
///
/// Inherits shared host/bridge/props/snapshot/paired/Fabric glue from `NveAppleComponentViewBase`
/// and implements thin AppKit view-factory and delegate adapters.
@interface RCTNveMacComponentView : NveAppleComponentViewBase

@property (nonatomic, weak, nullable) id<RCTNveMacComponentViewDelegate> delegate;

@end

NS_ASSUME_NONNULL_END
