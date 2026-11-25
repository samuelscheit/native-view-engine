#import <AppKit/AppKit.h>
#include "nve_abi.h"

#if defined(NVE_WITH_RN_FABRIC) && NVE_WITH_RN_FABRIC
#import <React/RCTViewComponentView.h>
#define NVE_MAC_COMPONENT_BASE_CLASS RCTViewComponentView
#else
#define NVE_MAC_COMPONENT_BASE_CLASS NSView
#endif

NS_ASSUME_NONNULL_BEGIN

/// Callback block fired whenever the embedded NVE component emits an outbound event
/// via `NveComponentApi::drain_emitted_events`.
typedef void (^RCTNveMacEventHandler)(NSString *eventName, NSDictionary<NSString *, id> *payload);

@protocol RCTNveMacComponentViewDelegate <NSObject>
@optional
- (void)nveComponentView:(NSView *)view
            didEmitEvent:(NSString *)eventName
                 payload:(NSDictionary<NSString *, id> *)payload;
@end

/// Embeddable macOS AppKit / `react-native-macos` Fabric `RCTViewComponentView`-compatible view
/// that hosts any NVE Rust component (`nve_get_interface` / `NveModule`) inside a native macOS app.
///
/// Mirrors `RCTNveComponentView` (iOS) verbatim so `nve-codegen --export-rn` templates share
/// identical lifecycle, prop injection, and outbound event override hooks across iOS and macOS.
@interface RCTNveMacComponentView : NVE_MAC_COMPONENT_BASE_CLASS

@property (nonatomic, weak, nullable) id<RCTNveMacComponentViewDelegate> delegate;
@property (nonatomic, copy, nullable) RCTNveMacEventHandler onNveEvent;
@property (nonatomic, readonly) uint32_t nveSurfaceId;
@property (nonatomic, readonly) uint32_t nveNodeCount;

/// Attach an NVE Rust component interface resolver directly (for statically-linked Rust libraries).
- (BOOL)attachNveInterface:(NveGetInterfaceFn)getInterface surfaceId:(uint32_t)surfaceId;

/// Dynamically load an NVE Rust component `.dylib` exporting `nve_get_interface`.
- (BOOL)loadNveDylibAtPath:(NSString *)dylibPath
                 surfaceId:(uint32_t)surfaceId
                     error:(NSError * _Nullable * _Nullable)error;

/// Reload a previously-loaded `.dylib`, preserving component state via `NveDevApi::snapshot_state`/`restore_state`.
- (BOOL)reloadNveDylibAtPath:(NSString *)dylibPath
                       error:(NSError * _Nullable * _Nullable)error;

/// Zero-copy instant-boot Frame-0 replay from an `NVE_SNAP` snapshot file before loading a dylib.
- (BOOL)replayFrame0SnapshotAtPath:(NSString *)snapshotPath
             expectedSchemaVersion:(uint32_t)expectedSchemaVersion
                     restoreTimeUs:(uint64_t * _Nullable)outRestoreUs;

/// Atomically persist the current rendered Frame-0 tree and app state to `snapshotPath`.
- (BOOL)saveSnapshotAtPath:(NSString *)snapshotPath
             schemaVersion:(uint32_t)schemaVersion;

/// Update component props from a UTF-8 JSON object string and re-render if needed.
- (BOOL)applyPropsJson:(NSString *)propsJson;

/// Update component props from an NSDictionary (serialized to JSON) and re-render if needed.
- (BOOL)applyPropsDictionary:(NSDictionary<NSString *, id> *)props;

/// Trigger a render pass (`NveRnComponentBridge::render`), reconcile native AppKit child
/// views from the computed Yoga/Fabric shadow tree, and flush outbound emitted events.
- (void)renderNveFrame;

/// Flush pending outbound events from `NveComponentApi::drain_emitted_events` into
/// `-handleNveEvent:payload:`.
- (void)drainNveEmittedEvents;

/// Programmatically dispatch a native press event on a rendered node (useful for accessibility
/// actions and headless AppKit tests).
- (void)simulatePressOnNodeId:(uint32_t)nodeId atPoint:(NSPoint)point;

/// Inspect the live AppKit `NSView` mounted for a given NVE shadow `nodeId` (or `nil` if absent).
- (nullable NSView *)nativeViewForNodeId:(uint32_t)nodeId;

/// Override point for `nve-codegen`-generated subclasses to return their statically-linked
/// `nve_get_interface` function pointer (return `NULL` when attached manually).
- (nullable NveGetInterfaceFn)defaultNveGetInterface;

/// Override point for `nve-codegen`-generated subclasses to forward decoded NVE events into
/// their typed Fabric `EventEmitter`. Default implementation forwards to `onNveEvent` and `delegate`.
- (void)handleNveEvent:(NSString *)eventName payload:(NSDictionary<NSString *, id> *)payload;

@end

NS_ASSUME_NONNULL_END
