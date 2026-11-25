#pragma once

#import <AppKit/AppKit.h>
#import "RCTNveMacComponentView.h"

#include <cstdint>

NS_ASSUME_NONNULL_BEGIN

typedef BOOL (^NveMacWindowCloseVetoHandler)(uint32_t surfaceId);
typedef void (^NveMacWindowClosedHandler)(uint32_t surfaceId);
typedef void (^NveMacAllWindowsClosedHandler)(void);

/// Multi-window AppKit lifecycle manager mapping `surfaceId -> (NSWindow, RCTNveMacComponentView)`.
///
/// Reuses one `NveAppleComponentHost` per window (via `RCTNveMacComponentView`), enforces
/// parent-before-child modal focus blocking, supports cancelable close-request vetoes and
/// bottom-up cascading close of owned sheets/popovers/panels, and fires `onAllWindowsClosed`
/// only when the final managed window closes.
@interface NveMacWindowManager : NSObject <NSWindowDelegate>

@property (nonatomic, copy, nullable) NveMacWindowCloseVetoHandler shouldCloseWindow;
@property (nonatomic, copy, nullable) NveMacWindowClosedHandler onWindowClosed;
@property (nonatomic, copy, nullable) NveMacAllWindowsClosedHandler onAllWindowsClosed;
@property (nonatomic, readonly) NSUInteger windowCount;
@property (nonatomic, readonly) uint32_t focusedSurfaceId;

/// Opens a new AppKit window hosting an independent `RCTNveMacComponentView` for `surfaceId`.
/// Returns `nil` if `surfaceId` is already open or `parentSurfaceId` is non-zero and unknown.
- (nullable RCTNveMacComponentView *)openWindowWithSurfaceId:(uint32_t)surfaceId
                                                       title:(NSString *)title
                                                       frame:(NSRect)frame
                                                   styleMask:(NSWindowStyleMask)styleMask
                                             parentSurfaceId:(uint32_t)parentSurfaceId
                                                       modal:(BOOL)modal
                                                 alwaysOnTop:(BOOL)alwaysOnTop
                                                cascadeClose:(BOOL)cascadeClose
                                                     present:(BOOL)present;

/// Convenience overload for opening a standard top-level resizable window.
- (nullable RCTNveMacComponentView *)openWindowWithSurfaceId:(uint32_t)surfaceId
                                                       title:(NSString *)title
                                                       frame:(NSRect)frame
                                                     present:(BOOL)present;

- (nullable NSWindow *)windowForSurfaceId:(uint32_t)surfaceId;
- (nullable RCTNveMacComponentView *)componentViewForSurfaceId:(uint32_t)surfaceId;
- (NSArray<NSNumber *> *)activeSurfaceIds;

/// Returns the deepest open modal child surface blocking `surfaceId`, or `0` if unblocked.
- (uint32_t)activeModalBlockerForSurfaceId:(uint32_t)surfaceId;

/// Focuses `surfaceId` unless blocked by an open modal child (returns `NO` when blocked/missing).
- (BOOL)focusWindowWithSurfaceId:(uint32_t)surfaceId;

- (BOOL)setTitle:(NSString *)title forSurfaceId:(uint32_t)surfaceId;
- (BOOL)setFrame:(NSRect)frame forSurfaceId:(uint32_t)surfaceId;
- (BOOL)setAlwaysOnTop:(BOOL)alwaysOnTop forSurfaceId:(uint32_t)surfaceId;

/// Runs `shouldCloseWindow` veto check; if allowed, cascades close to owned children bottom-up.
/// Returns `YES` if closed, `NO` if missing or vetoed.
- (BOOL)requestCloseSurfaceId:(uint32_t)surfaceId;

/// Immediately closes `surfaceId` and any owned cascading children bottom-up without veto.
- (BOOL)closeWindowWithSurfaceId:(uint32_t)surfaceId;

/// Immediately closes every managed window.
- (void)closeAllWindows;

@end

NS_ASSUME_NONNULL_END
