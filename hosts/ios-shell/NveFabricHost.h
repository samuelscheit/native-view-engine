#import <Foundation/Foundation.h>
#if __has_include("NveAppleFabricHost.h")
#import "NveAppleFabricHost.h"
#else
#import "../apple-common/NveAppleFabricHost.h"
#endif

NS_ASSUME_NONNULL_BEGIN

/// Prebuilt iOS Fabric Host Shell controller.
///
/// Built once against prebuilt React Native Fabric XCFrameworks + `nve_fabric_bridge`.
/// Loads and hot-swaps the user's Rust app `cdylib` on iOS Simulator (Tier 2, ~200-400ms)
/// and applies WebSocket template patches on both Simulator and physical iOS devices (Tier 1, <50ms).
@interface NveFabricHostController : NveAppleFabricHostController
@end

NS_ASSUME_NONNULL_END
