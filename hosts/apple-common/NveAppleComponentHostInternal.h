#pragma once

#import "NveAppleComponentHost.h"

#include <nve/host/host_session.hpp>

#include <vector>

NS_ASSUME_NONNULL_BEGIN

@interface NveAppleComponentHost () {
 @package
  __weak id<NveAppleComponentViewAdapter> _adapter;
  NSString *_errorDomain;
  NveAppleSceneReconciler *_reconciler;
  nve::host::RuntimeSession _session;
  uint32_t _surfaceId;
  BOOL _appAttached;
  BOOL _pairRequested;
  BOOL _hasReplayedFrame0;
  BOOL _pumpScheduled;
  /// `YES` while views are being reconciled: UIKit / AppKit callbacks fired by view updates
  /// (scrolling, text, focus) are deferred instead of re-entering the runtime.
  BOOL _reconciling;
  /// The scene guard held during reconcile (`resource_payload` borrows from it).
  const nve::host::RuntimeSceneGuard *_Nullable _activeScene;
  nve::apple::ValueArena _arena;
}

/// Attaches `getInterface` (an in-process app; `NULL` detaches), replacing any app or pairing, and pumps.
- (BOOL)_attachGetInterface:(NveGetInterfaceFn)getInterface error:(NSError *_Nullable *_Nullable)error;
/// Loads the app `.dylib` at `path` through the runtime (`load_app_module`; `carry` restores the
/// running app's state into it), replacing any app or pairing, and pumps.
- (BOOL)_loadAppModule:(NSString *)path carryState:(BOOL)carry error:(NSError *_Nullable *_Nullable)error;
- (NSError *)_errorWithCode:(NSInteger)code description:(NSString *)description;
/// Runs `block` with this host on the next main-queue turn, unless the host is gone by then (the
/// block never retains it). For calls made during reconcile, which must not re-enter the runtime.
- (void)_deferToNextTurn:(void (^)(NveAppleComponentHost *host))block;

@end

NS_ASSUME_NONNULL_END