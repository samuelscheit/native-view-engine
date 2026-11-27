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

/// Returns the wire encoding of a runtime resource (`resource_bytes`), or `nil`.
typedef NSData *_Nullable (^NveAppleResourceProvider)(uint32_t resourceId);

/// Incremental Apple reconciler over the zero-copy runtime scene: realizes every live node as a
/// retained native view keyed by `NodeId`, recreates views whose kind changed, updates dirty
/// nodes in `O(dirty)`, refreshes frames, reorders subviews by paint order, sizes scroll content,
/// and resolves `nve.image` resources (cached until an image row changes).
@interface NveAppleSceneReconciler : NSObject

@property (nonatomic, readonly) uint32_t lastReconciledDirtyCount;
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
- (void)syncFromScene:(const nve::host::RuntimeSceneReader &)scene;
#endif

@end

NS_ASSUME_NONNULL_END