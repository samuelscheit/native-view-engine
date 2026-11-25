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

#if defined(NVE_WITH_RN_FABRIC) && NVE_WITH_RN_FABRIC
#import <React/RCTViewComponentView.h>
#define NVE_APPLE_COMPONENT_BASE_CLASS RCTViewComponentView
#elif TARGET_OS_OSX
#define NVE_APPLE_COMPONENT_BASE_CLASS NSView
#else
#define NVE_APPLE_COMPONENT_BASE_CLASS UIView
#endif

#include "nve_abi.generated.h"

#ifdef __cplusplus
#import "NveAppleSupport.h"
#include "nve_native_view.hpp"
#include <functional>
#include <string_view>
#endif

NS_ASSUME_NONNULL_BEGIN

@class NveAppleComponentHost;

/// Callback block fired whenever the embedded NVE component emits an outbound event
/// via `NveComponentApi::drain_emitted_events`.
typedef void (^NveAppleEventHandler)(NSString *eventName, NSDictionary<NSString *, id> *payload);

/// Shared `UIImageView` / `NSImageView` subclass caching its bound `src` string so incremental
/// renders skip redundant image-cache lookups (and passing clicks through on macOS when non-interactive).
@interface NveAppleImageView : NveAppleBaseImageView
@property (nonatomic, copy, nullable) NSString *nveImageSrc;
@end

/// Shared iOS & macOS component-view base class owning `NveAppleComponentHost` and forwarding
/// all lifecycle, dylib, paired-session, snapshot, prop, event, and React Native Fabric hooks.
///
/// `RCTNveComponentView` (UIKit) and `RCTNveMacComponentView` (AppKit) subclass this view and
/// implement only thin platform view-factory, layout, and delegate glue.
@interface NveAppleComponentViewBase : NVE_APPLE_COMPONENT_BASE_CLASS

@property (nonatomic, strong, readonly) NveAppleComponentHost *nveHost;
@property (nonatomic, copy, nullable) NveAppleEventHandler onNveEvent;
@property (nonatomic, readonly) uint32_t nveSurfaceId;
@property (nonatomic, readonly) uint32_t nveNodeCount;
@property (nonatomic, readonly) uint32_t nveLastReconciledDirtyCount;
@property (nonatomic, readonly) uint32_t nveLastReconciledRemovedCount;
@property (nonatomic, readonly) BOOL isPaired;
@property (nonatomic, readonly) BOOL isPairedConnected;
@property (nonatomic, readonly) BOOL hasReplayedFrame0;

/// Attach an NVE Rust component interface resolver directly (for statically-linked Rust libraries).
- (BOOL)attachNveInterface:(NveGetInterfaceFn)getInterface surfaceId:(uint32_t)surfaceId;

/// Dynamically load an NVE Rust component `cdylib` exporting `nve_get_interface`.
- (BOOL)loadNveDylibAtPath:(NSString *)dylibPath
                 surfaceId:(uint32_t)surfaceId
                     error:(NSError * _Nullable * _Nullable)error;

/// Reload a previously-loaded `cdylib`, preserving component state via `NveDevApi::snapshot_state`/`restore_state`.
- (BOOL)reloadNveDylibAtPath:(NSString *)dylibPath
                       error:(NSError * _Nullable * _Nullable)error;

/// Connect in zero-cdylib paired thin-client mode using `NVT1`/`NVD1` over UDP/TCP/WebSocket.
- (BOOL)connectPairedEndpoint:(NSString *)endpoint
                        token:(NSString *)token
                    transport:(nullable NSString *)transport
                    surfaceId:(uint32_t)surfaceId
                        error:(NSError * _Nullable * _Nullable)error;

/// Disconnect any active paired thin-client session.
- (void)disconnectPaired;

/// Zero-copy instant-boot Frame-0 replay from an `NVERELOC` snapshot file before loading a dylib.
- (BOOL)replayFrame0SnapshotAtPath:(NSString *)snapshotPath
             expectedSchemaVersion:(uint32_t)expectedSchemaVersion
                     restoreTimeUs:(uint64_t * _Nullable)outRestoreUs;

/// Atomically persist the current rendered Frame-0 tree and app state to `snapshotPath`.
- (BOOL)saveSnapshotAtPath:(NSString *)snapshotPath
             schemaVersion:(uint32_t)schemaVersion;

/// Update component props from a UTF-8 JSON object string and re-render if needed.
- (BOOL)applyPropsJson:(NSString *)propsJson;

/// Update component props from an NSDictionary via direct FFI (`set_props_direct`) and re-render.
- (BOOL)applyPropsDictionary:(NSDictionary<NSString *, id> *)props;

/// Trigger a render pass (`NveRnComponentBridge::render`), reconcile native child views, and flush events.
- (void)renderNveFrame;

/// Flush pending outbound events from `NveComponentApi::drain_emitted_events` into `-handleNveEvent:payload:`.
- (void)drainNveEmittedEvents;

/// Programmatically dispatch a native press event on a rendered node.
- (void)simulatePressOnNodeId:(uint32_t)nodeId atPoint:(CGPoint)point;

/// Inspect the live native view mounted for a given NVE shadow `nodeId` (or `nil` if absent).
- (nullable NveAppleBasePlatformView *)nativeViewForNodeId:(uint32_t)nodeId;

/// Override point returning the NSError domain for dylib/paired load errors.
- (NSString *)nveErrorDomain;

/// Override point returning the fallback surface dimensions when `bounds` is empty.
- (CGSize)nveDefaultSurfaceSize;

/// Current display backing scale factor for this view.
- (CGFloat)nveCurrentDisplayScale;

/// Push the current view bounds and scale factor into `nveHost`.
- (void)propagateBoundsToNveHost;

/// Override point for `nve-codegen`-generated subclasses to return their statically-linked
/// `nve_get_interface` function pointer (return `NULL` when attached manually).
- (nullable NveGetInterfaceFn)defaultNveGetInterface;

/// Override point for `nve-codegen`-generated subclasses to forward decoded NVE events into
/// their typed Fabric `EventEmitter`. Default implementation forwards to `onNveEvent`.
- (void)handleNveEvent:(NSString *)eventName payload:(NSDictionary<NSString *, id> *)payload;

@end

#ifdef __cplusplus
namespace nve::apple {

/// Optional runtime hook that registers native view adapters into a `NativeViewRegistry`.
using NativeViewRegistrar = std::function<void(nve::NativeViewRegistry &registry)>;

/// Optional runtime hook that binds registered native view kinds to an active `FabricHost`.
using NativeViewBinder = std::function<void(
    NveFabricHostHandle _Nullable host,
    nve::NativeViewRegistry &registry)>;

/// Configure process-wide runtime native-view hooks used by newly created
/// `NveAppleComponentViewBase` instances (overrides compile-time `NVE_ENABLE_NATIVE_VIEWS`
/// defaults when set).
void set_default_native_view_hooks(NativeViewRegistrar registrar, NativeViewBinder binder);

/// Clear any process-wide runtime native-view hooks.
void clear_default_native_view_hooks();

/// Update a shared `NveAppleImageView` from `text` (`src`) + `resizeMode` + `borderRadius`
/// with source-string caching via `shared_image_cache()`.
void update_image_view(
    NveAppleImageView * _Nonnull imgView,
    std::string_view src,
    ImageResizeMode resizeMode,
    float borderRadius);

/// Update a shared `NveAppleCanvasView` from raw binary `NVE_DRAW1` bytes carried in `text`.
void update_canvas_view(NvePlatformView * _Nonnull view, std::string_view drawBytes);

} // namespace nve::apple

@interface NveAppleComponentViewBase (NveNativeViewRegistry)

/// Returns the active `nve::NativeViewRegistry` attached to this view, or `nullptr` when
/// opt-in native views are disabled and no runtime registry has been attached.
- (nve::NativeViewRegistry * _Nullable)nativeViewRegistry;

/// Attach an external (non-owning) `NativeViewRegistry` and optional host binder.
/// Passing `nullptr` detaches any external or owned registry. Immediately binds to the
/// currently active `FabricHost` if one is mounted.
- (void)attachNativeViewRegistry:(nve::NativeViewRegistry * _Nullable)registry
                          binder:(nve::apple::NativeViewBinder)binder;

/// Create and own a `NativeViewRegistry` populated by `registrar` and bound by `binder`.
- (void)installNativeViewHooksWithRegistrar:(nve::apple::NativeViewRegistrar)registrar
                                     binder:(nve::apple::NativeViewBinder)binder;

/// Re-run the configured `NativeViewBinder` against the currently active `FabricHost` (if any).
- (void)bindNativeViewsToActiveFabricHost;

@end
#endif

NS_ASSUME_NONNULL_END
