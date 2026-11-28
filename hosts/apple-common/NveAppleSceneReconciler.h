#pragma once

#import "NveAppleSceneNodes.h"

NS_ASSUME_NONNULL_BEGIN

/// Thin UIKit / AppKit adapter consumed by `NveAppleSceneReconciler` and `NveAppleComponentHost`.
@protocol NveAppleComponentViewAdapter <NSObject>

/// Container every runtime root view mounts into.
- (NvePlatformView *)nveHostRootView;
- (void)nveEmitEvent:(NSString *)eventName payload:(NSDictionary<NSString *, id> *)payload;
/// Creates the platform view realizing `node.kind` (label, text field, image view, scroll view, container).
- (NvePlatformView *)nveCreateNativeViewForNode:(const nve::apple::NodeView &)node;
/// Applies content, style, and semantics; `image` is the decoded `nve.image` source (if any).
- (void)nveUpdateNativeView:(NvePlatformView *)view
                    forNode:(const nve::apple::NodeView &)node
                      image:(nullable id)image;

@optional
/// Invoked right before a managed native view is removed and released.
- (void)nveWillDestroyNativeView:(NvePlatformView *)view forNodeId:(uint32_t)nodeId;
/// Container override for mounting children (e.g. AppKit `NSScrollView.documentView`).
- (NvePlatformView *)nveChildContainerForParentView:(NvePlatformView *)parentView;
/// Content size of a scroll node after its children were laid out.
- (void)nveUpdateScrollView:(NvePlatformView *)scrollView contentSize:(CGSize)contentSize;

@end

/// Returns the canonical payload of a runtime resource (`resource_payload`), or `nil`.
typedef NSData *_Nullable (^NveAppleResourceProvider)(uint32_t resourceId);

/// Apple adapter of host-cxx's `RetainedTree` over the zero-copy runtime scene: one retained
/// native view per live node, O(changed) per frame (removed, dirty, moved — geometry only — and the
/// clean views whose resource bytes were rewritten), inherited typography cascades, subviews in
/// paint order (scene order by `z_index`), scroll content sizes, and `nve.image` sources decoded
/// once per resource version.
@interface NveAppleSceneReconciler : NSObject

/// Views (re)applied by the last sync (`update`s), geometry-only moves, and removals.
@property (nonatomic, readonly) uint32_t lastReconciledDirtyCount;
@property (nonatomic, readonly) uint32_t lastReconciledMovedCount;
@property (nonatomic, readonly) uint32_t lastReconciledRemovedCount;
@property (nonatomic, readonly) NSUInteger mountedViewCount;

- (instancetype)initWithAdapter:(id<NveAppleComponentViewAdapter>)adapter
                      resources:(NveAppleResourceProvider)resources NS_DESIGNATED_INITIALIZER;
- (instancetype)init NS_UNAVAILABLE;

- (nullable NvePlatformView *)nativeViewForNodeId:(uint32_t)nodeId;
- (uint32_t)nodeIdForNativeView:(NvePlatformView *)view;
- (void)clearNativeNodeViews;

#ifdef __cplusplus
/// Reconciles the published scene held by `guard`.
- (void)syncFromScene:(const nve::host::SceneReader &)scene;
#endif

@end

NS_ASSUME_NONNULL_END