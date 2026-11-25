#pragma once

#import "NveAppleSupport.h"

NS_ASSUME_NONNULL_BEGIN

/// Thin UIKit/AppKit view adapter protocol consumed by `NveAppleComponentHost`.
@protocol NveAppleComponentViewAdapter <NSObject>

- (NvePlatformView *)nveHostRootView;
- (NveSurfaceConfig)nveSurfaceConfigForSurfaceId:(uint32_t)surfaceId;
- (void)nveEmitEvent:(NSString *)eventName payload:(NSDictionary<NSString *, id> *)payload;
- (NvePlatformView *)nveCreateNativeViewForSnapshot:(const nve::RenderedNodeSnapshot &)snap;
- (void)nveApplyRootBackgroundRgba:(uint32_t)rgba;
- (void)nveUpdateNativeView:(NvePlatformView *)view
                forSnapshot:(const nve::RenderedNodeSnapshot &)snap
                interactive:(BOOL)interactive;

@optional
/// Optional container override (e.g. AppKit `NSScrollView.documentView`) for mounting children.
- (NvePlatformView *)nveChildContainerForParentView:(NvePlatformView *)parentView;
/// Optional scroll content-size hook invoked after laying out `NVE_COMP_SCROLL_VIEW` children.
- (void)nveUpdateScrollView:(NvePlatformView *)scrollView contentSize:(CGSize)contentSize;

@end

/// Shared Apple (iOS & macOS) component-view lifecycle controller and 3-pass view reconciler.
///
/// Owns `nve::NveRnComponentBridge`, the optional `dlopen` handle, and the mounted native view
/// table while delegating UIKit/AppKit view creation, styling, and gesture wiring to its adapter.
@interface NveAppleComponentHost : NSObject

@property (nonatomic, readonly) uint32_t surfaceId;
@property (nonatomic, readonly) uint32_t nodeCount;

- (instancetype)initWithAdapter:(id<NveAppleComponentViewAdapter>)adapter
                    errorDomain:(NSString *)errorDomain NS_DESIGNATED_INITIALIZER;
- (instancetype)init NS_UNAVAILABLE;

- (BOOL)attachInterface:(NveGetInterfaceFn)getInterface surfaceId:(uint32_t)surfaceId;
- (BOOL)loadDylibAtPath:(NSString *)dylibPath
              surfaceId:(uint32_t)surfaceId
                  error:(NSError * _Nullable * _Nullable)error;
- (BOOL)reloadDylibAtPath:(NSString *)dylibPath
                    error:(NSError * _Nullable * _Nullable)error;

/// Zero-copy instant-boot Frame-0 replay from an `NVE_SNAP` snapshot file before any dylib loads.
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

- (void)updateSurfaceWidth:(float)width height:(float)height scaleFactor:(float)scaleFactor;
- (BOOL)getRootLayoutSize:(CGSize *)outSize;

- (nullable NvePlatformView *)nativeViewForNodeId:(uint32_t)nodeId;
- (uint32_t)nodeIdForNativeView:(NvePlatformView *)view;
- (void)clearNativeNodeViews;
- (void)syncNativeViewsFromHost;

@end

NS_ASSUME_NONNULL_END
