#import <Foundation/Foundation.h>
#include "nve_abi.h"

NS_ASSUME_NONNULL_BEGIN

/// Prebuilt macOS Fabric Host Shell controller.
///
/// Loads and hot-swaps a Rust NVE app `.dylib` on macOS (Tier 2, preserving state via
/// `NveDevApi::snapshot_state` / `restore_state`) and applies WebSocket JSON template
/// patches (Tier 1) without rebuilding the host binary.
@interface NveMacFabricHostController : NSObject

@property (nonatomic, readonly) NveFabricHostHandle fabricHost;
@property (nonatomic, readonly) NveSurfaceConfig surfaceConfig;
@property (nonatomic, readonly) NveFrameStats lastFrameStats;
@property (nonatomic, readonly) uint32_t nodeCount;
@property (nonatomic, readonly, getter=isAppLoaded) BOOL appLoaded;

- (instancetype)initWithSurfaceConfig:(NveSurfaceConfig)config;
- (BOOL)attachInterface:(NveGetInterfaceFn)getInterface;
- (BOOL)loadRustAppDylibAtPath:(NSString *)dylibPath error:(NSError * _Nullable * _Nullable)error;
- (BOOL)reloadRustAppDylibAtPath:(NSString *)dylibPath error:(NSError * _Nullable * _Nullable)error;
- (void)setSurfaceWidth:(float)width height:(float)height scaleFactor:(float)scaleFactor;
- (int32_t)renderNextFrame;
- (BOOL)applyHotPatchJson:(NSString *)patchJson;

@end

NS_ASSUME_NONNULL_END
