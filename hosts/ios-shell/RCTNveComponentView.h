#import <UIKit/UIKit.h>
#import "NveAppleComponentViewBase.h"

#define NVE_RN_COMPONENT_BASE_CLASS NVE_APPLE_COMPONENT_BASE_CLASS

NS_ASSUME_NONNULL_BEGIN

/// Callback block fired whenever the embedded NVE component emits an outbound event
/// via `NveComponentApi::drain_emitted_events`.
typedef NveAppleEventHandler RCTNveEventHandler;

@protocol RCTNveComponentViewDelegate <NSObject>
@optional
- (void)nveComponentView:(UIView *)view
            didEmitEvent:(NSString *)eventName
                 payload:(NSDictionary<NSString *, id> *)payload;
@end

/// Embeddable React Native Fabric `RCTViewComponentView`-compatible view that hosts
/// any NVE Rust component (`nve_get_interface` / `NveModule`) inside a standard React Native iOS app.
///
/// Inherits shared host/bridge/props/snapshot/paired/Fabric glue from `NveAppleComponentViewBase`
/// and implements thin UIKit view-factory and delegate adapters.
@interface RCTNveComponentView : NveAppleComponentViewBase

@property (nonatomic, weak, nullable) id<RCTNveComponentViewDelegate> delegate;

@end

NS_ASSUME_NONNULL_END
