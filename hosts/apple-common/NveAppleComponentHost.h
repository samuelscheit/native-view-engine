#pragma once

#import "NveAppleSceneReconciler.h"

NS_ASSUME_NONNULL_BEGIN

/// `NSError.userInfo` key carrying the exact `NveRuntimeStatus` integer (`NSNumber`).
extern NSErrorUserInfoKey const NveAppleRuntimeStatusErrorKey;

/// Shared Apple (iOS & macOS) surface controller over `libnve_runtime`.
///
/// Owns one `nve::host::RuntimeSession` (the runtime hosts the app, reconcile, layout with the
/// native text measurer, and pairing), drives frames from the runtime wake callback, and hands the
/// zero-copy scene to `NveAppleSceneReconciler`, which realizes native views through the adapter.
@interface NveAppleComponentHost : NSObject

@property (nonatomic, readonly) uint32_t surfaceId;
/// Live runtime nodes (`NveRuntimeMetrics.live_nodes`).
@property (nonatomic, readonly) uint32_t nodeCount;
@property (nonatomic, readonly) uint32_t lastReconciledDirtyCount;
@property (nonatomic, readonly) uint32_t lastReconciledRemovedCount;
/// Views only re-placed (geometry moved, content clean) by the last frame.
@property (nonatomic, readonly) uint32_t lastReconciledMovedCount;
@property (nonatomic, readonly) BOOL isAppAttached;
@property (nonatomic, readonly) BOOL isPaired;
@property (nonatomic, readonly) BOOL isPairedConnected;
@property (nonatomic, readonly) BOOL hasReplayedFrame0;

/// `nil` when `libnve_runtime` is not linked or its ABI does not match this build.
- (nullable instancetype)initWithAdapter:(id<NveAppleComponentViewAdapter>)adapter
                             errorDomain:(NSString *)errorDomain NS_DESIGNATED_INITIALIZER;
- (instancetype)init NS_UNAVAILABLE;

/// The host associated with a platform root view (`RCTNveMacComponentView` / `RCTNveComponentView`).
+ (nullable NveAppleComponentHost *)hostForRootView:(NvePlatformView *)rootView;

- (BOOL)attachInterface:(NveGetInterfaceFn)getInterface surfaceId:(uint32_t)surfaceId;
- (BOOL)loadDylibAtPath:(NSString *)dylibPath surfaceId:(uint32_t)surfaceId error:(NSError *_Nullable *_Nullable)error;

/// Forwards props to the app; `NO` when the runtime refused them. Called from a view callback
/// during reconcile, it is deferred to the next main-queue turn and returns `YES` (queued).
- (BOOL)applyPropsDictionary:(NSDictionary<NSString *, id> *)props;
/// Applies one NVP1 hot patch; `YES` when it changed the app (or was queued during reconcile).
- (BOOL)applyHotPatch:(NSData *)patchNvp1;
/// Ticks the runtime, reconciles a published frame, and forwards outbound app events.
- (void)renderFrame;
- (void)drainEmittedEvents;
- (void)dispatchPressOnNodeId:(uint32_t)nodeId x:(float)x y:(float)y;
- (void)dispatchTextChangeOnNodeId:(uint32_t)nodeId text:(NSString *)text;
- (void)dispatchSubmitOnNodeId:(uint32_t)nodeId text:(NSString *)text;
- (void)dispatchCompositionStartOnNodeId:(uint32_t)nodeId text:(NSString *)text;
- (void)dispatchCompositionUpdateOnNodeId:(uint32_t)nodeId text:(NSString *)text;
- (void)dispatchCompositionEndOnNodeId:(uint32_t)nodeId text:(NSString *)text;
- (void)dispatchFocusOnNodeId:(uint32_t)nodeId focused:(BOOL)focused;
- (void)dispatchScrollOnNodeId:(uint32_t)nodeId
                             x:(float)x
                             y:(float)y
                  contentWidth:(float)contentWidth
                 contentHeight:(float)contentHeight;

- (void)updateSurfaceWidth:(float)width height:(float)height scaleFactor:(float)scaleFactor;
/// Intrinsic size: the laid-out root.
- (BOOL)getRootLayoutSize:(CGSize *)outSize;

- (nullable NvePlatformView *)nativeViewForNodeId:(uint32_t)nodeId;
- (uint32_t)nodeIdForNativeView:(NvePlatformView *)view;
- (void)clearNativeNodeViews;
- (void)syncNativeViewsFromHost;
/// Returns the canonical payload of resource `resourceId` matching `expectedKind`, or `nil`.
- (nullable NSData *)resourceBytesForId:(uint32_t)resourceId expectedKind:(uint64_t)expectedKind;

@end

/// Dev loop, pairing, Frame-0 replay, and controlled-text IME state (`NveAppleComponentHost+Session.mm`).
@interface NveAppleComponentHost (Session)

/// Controlled-text + IME helpers backed by `nve::host::ControlledText` (shared by AppKit & UIKit).
- (BOOL)shouldApplyAppText:(NSString *)value
                  toNodeId:(uint32_t)nodeId
                 shownText:(NSString *)shown
             hasMarkedText:(BOOL)hasMarkedText;
- (void)handleTextInputEditOnNodeId:(uint32_t)nodeId
                               text:(NSString *)text
                      hasMarkedText:(BOOL)hasMarkedText
                         markedText:(nullable NSString *)markedText
                          writeBack:(void (^)(NSString *nextText))writeBack;
- (void)handleTextInputFocusOnNodeId:(uint32_t)nodeId focused:(BOOL)focused;
- (BOOL)handleTextInputSubmitOnNodeId:(uint32_t)nodeId text:(NSString *)text hasMarkedText:(BOOL)hasMarkedText;
- (void)clearTextInputStateForNodeId:(uint32_t)nodeId;
- (nullable NSString *)textInputValueForNodeId:(uint32_t)nodeId;

/// Tier-2 hot swap: snapshots app state, attaches the rebuilt dylib, restores state. A failed
/// reload keeps the running app.
- (BOOL)reloadDylibAtPath:(NSString *)dylibPath error:(NSError *_Nullable *_Nullable)error;

/// Thin-client pairing with `nve dev` (`nve://pair?...` / `ws(s)://...`): the runtime replicates the
/// remote scene (reconnect + keyframe recovery) and forwards input upstream.
- (BOOL)connectPairedEndpoint:(NSString *)uri
                        token:(NSString *)token
                    surfaceId:(uint32_t)surfaceId
                        error:(NSError *_Nullable *_Nullable)error;
- (void)disconnectPaired;

/// Atomically persists the active scene as a checksummed ABI v6 `frame0.nve2` file on disk.
- (BOOL)saveFrame0ToPath:(NSString *)path
              appBuildId:(uint64_t)appBuildId
                 envHash:(uint64_t)envHash
                   error:(NSError *_Nullable *_Nullable)error;

/// Validates and boots a persisted ABI v6 `frame0.nve2` file while no app is attached.
- (BOOL)bootFrame0FromPath:(NSString *)path
                appBuildId:(uint64_t)appBuildId
                   envHash:(uint64_t)envHash
                     error:(NSError *_Nullable *_Nullable)error;

/// Validates and boots an in-memory ABI v6 `frame0.nve2` envelope while no app is attached.
- (BOOL)bootFrame0FromBytes:(NSData *)envelope
                 appBuildId:(uint64_t)appBuildId
                    envHash:(uint64_t)envHash
                      error:(NSError *_Nullable *_Nullable)error;

/// Submits a raw `NVE2` keyframe or delta frame directly (`submit_nve2`) while no app is attached.
- (BOOL)submitNve2Frame:(NSData *)frame;

@end

NS_ASSUME_NONNULL_END