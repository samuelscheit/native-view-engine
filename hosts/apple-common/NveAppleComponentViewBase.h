#pragma once

#import <Foundation/Foundation.h>
#import <TargetConditionals.h>

#if TARGET_OS_OSX
#import <AppKit/AppKit.h>
typedef NSView NveAppleBasePlatformView;
typedef NSImageView NveAppleBaseImageView;
#else
#import <UIKit/UIKit.h>
typedef UIView NveAppleBasePlatformView;
typedef UIImageView NveAppleBaseImageView;
#endif

// React Native Fabric glue stays opt-in (`NVE_WITH_RN_FABRIC`) so it can move into
// `packages/modules/react-native-fabric` without touching the runtime host path.
#if defined(NVE_WITH_RN_FABRIC) && NVE_WITH_RN_FABRIC
#import <React/RCTViewComponentView.h>
#define NVE_APPLE_COMPONENT_BASE_CLASS RCTViewComponentView
#elif TARGET_OS_OSX
#define NVE_APPLE_COMPONENT_BASE_CLASS NSView
#else
#define NVE_APPLE_COMPONENT_BASE_CLASS UIView
#endif

#include <nve/host/nve_runtime_abi.generated.h>

#ifdef __cplusplus
#import "NveAppleSceneReconciler.h"
#endif

NS_ASSUME_NONNULL_BEGIN

@class NveAppleComponentHost;

/// Fired for every outbound app event (`drain_emitted_events`).
typedef void (^NveAppleEventHandler)(NSString *eventName, NSDictionary<NSString *, id> *payload);

/// `UIImageView` / `NSImageView` for `nve.image` nodes (macOS passes clicks through unless it has
/// gesture recognizers).
@interface NveAppleImageView : NveAppleBaseImageView
@end

/// Shared iOS & macOS view hosting one runtime surface through `NveAppleComponentHost`.
///
/// `RCTNveComponentView` (UIKit) and `RCTNveMacComponentView` (AppKit) subclass it and implement
/// only the platform view factory / updater and input glue.
@interface NveAppleComponentViewBase : NVE_APPLE_COMPONENT_BASE_CLASS

/// `nil` when `libnve_runtime` is not linked into the process.
@property (nonatomic, strong, readonly, nullable) NveAppleComponentHost *nveHost;
@property (nonatomic, copy, nullable) NveAppleEventHandler onNveEvent;
@property (nonatomic, readonly) uint32_t nveSurfaceId;
@property (nonatomic, readonly) uint32_t nveNodeCount;
@property (nonatomic, readonly) uint32_t nveLastReconciledDirtyCount;
@property (nonatomic, readonly) uint32_t nveLastReconciledRemovedCount;
@property (nonatomic, readonly) BOOL isPaired;
@property (nonatomic, readonly) BOOL isPairedConnected;
@property (nonatomic, readonly) BOOL hasReplayedFrame0;

/// Attach a statically linked app entry point (`nve_get_interface`).
- (BOOL)attachNveInterface:(NveGetInterfaceFn)getInterface surfaceId:(uint32_t)surfaceId;
/// `dlopen` an app `cdylib` exporting `nve_get_interface`.
- (BOOL)loadNveDylibAtPath:(NSString *)dylibPath surfaceId:(uint32_t)surfaceId error:(NSError *_Nullable *_Nullable)error;
/// Tier-2 reload preserving app state (`snapshot_state` / `restore_state`).
- (BOOL)reloadNveDylibAtPath:(NSString *)dylibPath error:(NSError *_Nullable *_Nullable)error;
/// Thin-client pairing with `nve dev` through the runtime (`pair_connect`).
- (BOOL)connectPairedEndpoint:(NSString *)uri
                        token:(NSString *)token
                    surfaceId:(uint32_t)surfaceId
                        error:(NSError *_Nullable *_Nullable)error;
- (void)disconnectPaired;
/// Instant boot from a `nve::host::SnapshotFile` before loading the app.
- (BOOL)replayFrame0SnapshotAtPath:(NSString *)snapshotPath
             expectedSchemaVersion:(uint32_t)expectedSchemaVersion
                     restoreTimeUs:(uint64_t *_Nullable)outRestoreUs;
- (BOOL)saveSnapshotAtPath:(NSString *)snapshotPath schemaVersion:(uint32_t)schemaVersion;
/// Props from an `NSDictionary` (`set_props`).
- (BOOL)applyPropsDictionary:(NSDictionary<NSString *, id> *)props;
/// Ticks, reconciles native views, and flushes outbound events.
- (void)renderNveFrame;
- (void)drainNveEmittedEvents;
- (void)simulatePressOnNodeId:(uint32_t)nodeId atPoint:(CGPoint)point;
- (nullable NveAppleBasePlatformView *)nativeViewForNodeId:(uint32_t)nodeId;

/// Override points.
- (NSString *)nveErrorDomain;
- (CGSize)nveDefaultSurfaceSize;
- (CGFloat)nveCurrentDisplayScale;
- (void)propagateBoundsToNveHost;
/// Codegen-generated subclasses return their statically linked `nve_get_interface` (or `NULL`).
- (nullable NveGetInterfaceFn)defaultNveGetInterface;
/// Codegen-generated subclasses forward events into typed Fabric emitters; default: `onNveEvent`.
- (void)handleNveEvent:(NSString *)eventName payload:(NSDictionary<NSString *, id> *)payload;

@end

#ifdef __cplusplus
/// The base view is the `NveAppleComponentHost` adapter; subclasses override the factory /
/// updater and call `super` for the shared layer style + accessibility.
@interface NveAppleComponentViewBase (NveAdapter) <NveAppleComponentViewAdapter>
@end

namespace nve::apple {

/// Applies `image` (already decoded from `nve.image.src`), its `object-fit`, and corner radius.
void update_image_view(NveAppleImageView *_Nonnull view, NvePlatformImage *_Nullable image, ImageResizeMode fit, float border_radius);

}  // namespace nve::apple
#endif

NS_ASSUME_NONNULL_END