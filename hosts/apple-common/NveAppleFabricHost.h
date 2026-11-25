#pragma once

#import <Foundation/Foundation.h>
#include "nve_abi.generated.h"

NS_ASSUME_NONNULL_BEGIN

/// Shared Apple (iOS & macOS) Fabric host controller.
///
/// Manages `NveFabricHostHandle`, statically-linked or dynamically-loaded Rust NVE apps,
/// Tier-1 JSON hot patching, and Tier-2 dylib hot reloading with state preservation.
@interface NveAppleFabricHostController : NSObject

@property (nonatomic, readonly) NveFabricHostHandle fabricHost;
@property (nonatomic, readonly) NveSurfaceConfig surfaceConfig;
@property (nonatomic, readonly) NveFrameStats lastFrameStats;
@property (nonatomic, readonly) uint32_t nodeCount;
@property (nonatomic, readonly, getter=isAppLoaded) BOOL appLoaded;

+ (NSString *)defaultErrorDomain;

- (instancetype)initWithSurfaceConfig:(NveSurfaceConfig)config
                          errorDomain:(NSString *)errorDomain NS_DESIGNATED_INITIALIZER;
- (instancetype)initWithSurfaceConfig:(NveSurfaceConfig)config;
- (instancetype)init NS_UNAVAILABLE;

- (BOOL)attachInterface:(NveGetInterfaceFn)getInterface;
- (BOOL)loadRustAppDylibAtPath:(NSString *)dylibPath
                         error:(NSError * _Nullable * _Nullable)error;
- (BOOL)reloadRustAppDylibAtPath:(NSString *)dylibPath
                           error:(NSError * _Nullable * _Nullable)error;
- (void)setSurfaceWidth:(float)width height:(float)height scaleFactor:(float)scaleFactor;
- (int32_t)renderNextFrame;
- (BOOL)applyHotPatchJson:(NSString *)patchJson;

@end

NS_ASSUME_NONNULL_END
