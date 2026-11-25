#import <Foundation/Foundation.h>
#if __has_include("NveAppleFabricHost.h")
#import "NveAppleFabricHost.h"
#else
#import "../apple-common/NveAppleFabricHost.h"
#endif

NS_ASSUME_NONNULL_BEGIN

/// Prebuilt macOS Fabric Host Shell controller.
///
/// Loads and hot-swaps a Rust NVE app `.dylib` on macOS (Tier 2, preserving state via
/// `NveDevApi::snapshot_state` / `restore_state`) and applies WebSocket JSON template
/// patches (Tier 1) without rebuilding the host binary.
@interface NveMacFabricHostController : NveAppleFabricHostController
@end

NS_ASSUME_NONNULL_END
