#import <Foundation/Foundation.h>
#include "nve_abi.h"

NS_ASSUME_NONNULL_BEGIN

/// Prebuilt iOS Fabric Host Shell controller.
///
/// Built once against prebuilt React Native Fabric XCFrameworks + `nve_fabric_bridge`.
/// Loads and hot-swaps the user's Rust app `cdylib` on iOS Simulator (Tier 2, ~200-400ms)
/// and applies WebSocket template patches on both Simulator and physical iOS devices (Tier 1, <50ms).
@interface NveFabricHostController : NSObject

- (instancetype)initWithSurfaceConfig:(NveSurfaceConfig)config;
- (BOOL)loadRustAppDylibAtPath:(NSString *)dylibPath error:(NSError **)error;
- (BOOL)reloadRustAppDylibAtPath:(NSString *)dylibPath error:(NSError **)error;
- (void)renderNextFrame;
- (void)applyHotPatchJson:(NSString *)patchJson;

@end

NS_ASSUME_NONNULL_END
