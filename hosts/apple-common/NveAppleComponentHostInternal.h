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
  std::vector<uint8_t> _pendingSnapshotState;
  nve::apple::ValueArena _arena;
}

/// Attaches `getInterface` (detaching any previous app), restores pending snapshot state, pumps.
- (BOOL)_attachGetInterface:(NveGetInterfaceFn)getInterface error:(NSError *_Nullable *_Nullable)error;
- (NSError *)_errorWithCode:(NSInteger)code description:(NSString *)description;

@end

NS_ASSUME_NONNULL_END