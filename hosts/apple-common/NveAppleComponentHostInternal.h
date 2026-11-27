#pragma once

#import "NveAppleComponentHost.h"

#include <nve/host/runtime_session.hpp>

#include <vector>

NS_ASSUME_NONNULL_BEGIN

@interface NveAppleComponentHost () {
 @package
  __weak id<NveAppleComponentViewAdapter> _adapter;
  NSString *_errorDomain;
  NveAppleSceneReconciler *_reconciler;
  nve::host::RuntimeSession _session;
  void *_Nullable _dylibHandle;
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
  /// App state carried across a Tier-2 reload, restored once the new app attaches.
  std::vector<uint8_t> _pendingSnapshotState;
  nve::apple::ValueArena _arena;
}

/// Attaches `getInterface` (detaching any previous app), restores pending snapshot state, pumps.
- (BOOL)_attachGetInterface:(NveGetInterfaceFn)getInterface error:(NSError *_Nullable *_Nullable)error;
- (NSError *)_errorWithCode:(NSInteger)code description:(NSString *)description;
/// Re-runs `block` on the next main-queue turn when called during reconcile; `NO` otherwise.
- (BOOL)_deferIfReconciling:(dispatch_block_t)block;

@end

NS_ASSUME_NONNULL_END