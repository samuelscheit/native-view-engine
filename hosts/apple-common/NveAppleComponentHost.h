#pragma once

#import "NveAppleSceneReconciler.h"
#include "nve_pair_client.hpp"

NS_ASSUME_NONNULL_BEGIN

/// Shared Apple (iOS & macOS) component-view lifecycle controller.
///
/// Owns `nve::NveRnComponentBridge`, the optional `dlopen` handle, and snapshot replay while
/// delegating incremental `O(changed)` `NveSceneView` reconciliation to `NveAppleSceneReconciler`
/// and UIKit/AppKit view creation/styling/gestures to its adapter.
@interface NveAppleComponentHost : NSObject

@property (nonatomic, readonly) uint32_t surfaceId;
@property (nonatomic, readonly) uint32_t nodeCount;
@property (nonatomic, readonly) uint32_t lastReconciledDirtyCount;
@property (nonatomic, readonly) uint32_t lastReconciledRemovedCount;

- (instancetype)initWithAdapter:(id<NveAppleComponentViewAdapter>)adapter
                    errorDomain:(NSString *)errorDomain NS_DESIGNATED_INITIALIZER;
- (instancetype)init NS_UNAVAILABLE;

/// Look up the `NveAppleComponentHost` associated with a platform root view
/// (`RCTNveMacComponentView` / `RCTNveComponentView`).
+ (nullable NveAppleComponentHost *)hostForRootView:(NvePlatformView *)rootView;

- (BOOL)attachInterface:(NveGetInterfaceFn)getInterface surfaceId:(uint32_t)surfaceId;
- (BOOL)loadDylibAtPath:(NSString *)dylibPath
              surfaceId:(uint32_t)surfaceId
                  error:(NSError * _Nullable * _Nullable)error;
- (BOOL)reloadDylibAtPath:(NSString *)dylibPath
                    error:(NSError * _Nullable * _Nullable)error;

/// Connect in paired thin-client mode (zero local dylib required). Incoming `NVD1` frames feed
/// directly into `FabricHost` and reconcile on the main queue via `dispatch_async`; UI events
/// stream upstream as `TransportMsgKind::InputEvents = 11`.
- (BOOL)connectPairedEndpoint:(NSString *)endpoint
                        token:(NSString *)token
                    transport:(nullable NSString *)transport
                    surfaceId:(uint32_t)surfaceId
                        error:(NSError * _Nullable * _Nullable)error;
- (BOOL)connectPairedConfig:(const nve::PairClientConfig &)config
                  surfaceId:(uint32_t)surfaceId
                      error:(NSError * _Nullable * _Nullable)error;
- (void)disconnectPaired;
- (NSUInteger)pollPairedFrames;

@property (nonatomic, readonly) BOOL isPaired;
@property (nonatomic, readonly) BOOL isPairedConnected;

/// Zero-copy instant-boot Frame-0 replay from an `NVERELOC` snapshot file before any dylib loads.
/// Mounts native views immediately and caches the snapshot state for seamless `restore_state`
/// once the live app dylib or interface attaches.
- (BOOL)replayFrame0SnapshotAtPath:(NSString *)snapshotPath
             expectedSchemaVersion:(uint32_t)expectedSchemaVersion
                     restoreTimeUs:(uint64_t * _Nullable)outRestoreUs;

/// Freeze-dries the live shadow tree into a complete Frame-0 `CommandBuffer` + `snapshot_state()`
/// and persists it atomically to `snapshotPath`.
- (BOOL)saveSnapshotAtPath:(NSString *)snapshotPath
             schemaVersion:(uint32_t)schemaVersion;

@property (nonatomic, readonly) BOOL hasReplayedFrame0;

- (BOOL)applyPropsJson:(NSString *)propsJson;
- (BOOL)applyPropsDictionary:(NSDictionary<NSString *, id> *)props;
- (void)renderFrame;
- (void)drainEmittedEvents;
- (void)dispatchPressOnNodeId:(uint32_t)nodeId x:(float)x y:(float)y;
- (void)dispatchTextChangeOnNodeId:(uint32_t)nodeId text:(NSString *)text;
- (void)dispatchSubmitOnNodeId:(uint32_t)nodeId text:(NSString *)text;
- (void)dispatchFocusOnNodeId:(uint32_t)nodeId focused:(BOOL)focused;
- (void)dispatchScrollOnNodeId:(uint32_t)nodeId
                             x:(float)x
                             y:(float)y
                  contentWidth:(float)contentWidth
                 contentHeight:(float)contentHeight;

- (void)updateSurfaceWidth:(float)width height:(float)height scaleFactor:(float)scaleFactor;
- (BOOL)getRootLayoutSize:(CGSize *)outSize;

- (nullable NvePlatformView *)nativeViewForNodeId:(uint32_t)nodeId;
- (uint32_t)nodeIdForNativeView:(NvePlatformView *)view;
- (void)clearNativeNodeViews;
- (void)syncNativeViewsFromHost;

@end

NS_ASSUME_NONNULL_END
