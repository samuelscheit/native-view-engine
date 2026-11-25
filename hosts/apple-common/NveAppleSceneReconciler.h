#pragma once

#import "NveAppleSupport.h"

#include <string_view>

NS_ASSUME_NONNULL_BEGIN

/// Thin UIKit/AppKit view adapter protocol consumed by `NveAppleComponentHost` and
/// `NveAppleSceneReconciler`.
@protocol NveAppleComponentViewAdapter <NSObject>

- (NvePlatformView *)nveHostRootView;
- (NveSurfaceConfig)nveSurfaceConfigForSurfaceId:(uint32_t)surfaceId;
- (void)nveEmitEvent:(NSString *)eventName payload:(NSDictionary<NSString *, id> *)payload;
- (NvePlatformView *)nveCreateNativeViewForNode:(const NveRenderNode &)node
                                      semantics:(const nve::apple::NodeSemantics &)semantics;
- (void)nveApplyRootBackgroundRgba:(uint32_t)rgba;
- (void)nveUpdateNativeView:(NvePlatformView *)view
                    forNode:(const NveRenderNode &)node
                       text:(std::string_view)text
                  semantics:(const nve::apple::NodeSemantics &)semantics
                interactive:(BOOL)interactive;

@optional
/// Optional hook invoked right before a managed native view is removed and released.
- (void)nveWillDestroyNativeView:(NvePlatformView *)view forNodeId:(uint32_t)nodeId;
/// Optional container override (e.g. AppKit `NSScrollView.documentView`) for mounting children.
- (NvePlatformView *)nveChildContainerForParentView:(NvePlatformView *)parentView;
/// Optional scroll content-size hook invoked after laying out `NVE_NODE_KIND_SCROLL_VIEW` children.
- (void)nveUpdateScrollView:(NvePlatformView *)scrollView contentSize:(CGSize)contentSize;

@end

/// Shared Apple (iOS & macOS) incremental `O(changed)` `NveSceneView` reconciler managing the
/// retained UIKit/AppKit view table, raw-prop semantics cache, subview ordering, and scroll bounds.
@interface NveAppleSceneReconciler : NSObject

@property (nonatomic, readonly) uint32_t lastReconciledDirtyCount;
@property (nonatomic, readonly) uint32_t lastReconciledRemovedCount;

- (instancetype)initWithAdapter:(id<NveAppleComponentViewAdapter>)adapter NS_DESIGNATED_INITIALIZER;
- (instancetype)init NS_UNAVAILABLE;

- (nullable NvePlatformView *)nativeViewForNodeId:(uint32_t)nodeId;
- (uint32_t)nodeIdForNativeView:(NvePlatformView *)view;
- (void)clearNativeNodeViews;
- (void)syncFromFabricHost:(NveFabricHostHandle _Nullable)host hasMutations:(BOOL)hasMutations;

@end

NS_ASSUME_NONNULL_END
