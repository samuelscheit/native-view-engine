#pragma once

#import "NveAppleComponentHost.h"

#include <nve/host/controlled_text.hpp>
#include <nve/host/host_session.hpp>

#include <optional>
#include <string>
#include <unordered_map>
#include <vector>

namespace nve::apple {

struct TextEventSpec {
  NveTextInputEvent kind;
  std::string text;
};

struct AppleTextInputState {
  nve::host::ControlledText controlled;
  bool synced{false};
  bool composing{false};
  bool focused{false};
  uint32_t max_length{0u};
  std::string last_committed;
  std::string pre_comp;
  std::string comp_text;
  std::optional<std::string> pending_app;
};

}  // namespace nve::apple

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
  std::unordered_map<uint32_t, nve::apple::AppleTextInputState> _textInputs;
}

/// Attaches `getInterface` (an in-process app; `NULL` detaches), replacing any app or pairing, and pumps.
- (BOOL)_attachGetInterface:(NveGetInterfaceFn)getInterface error:(NSError *_Nullable *_Nullable)error;
/// Loads the app `.dylib` at `path` through the runtime (`load_app_module`; `carry` restores the
/// running app's state into it), replacing any app or pairing, and pumps.
- (BOOL)_loadAppModule:(NSString *)path carryState:(BOOL)carry error:(NSError *_Nullable *_Nullable)error;
- (NSError *)_errorWithCode:(NSInteger)code description:(NSString *)description;
- (NSError *)_runtimeError:(NSString *)what status:(NveRuntimeStatus)status;
- (NSError *)_frame0Error:(NSString *)what status:(NveRuntimeStatus)status;
- (void)_dispatchTextEvent:(NveTextInputEvent)kind nodeId:(uint32_t)nodeId text:(NSString *)text;
- (void)_dispatchTextEvents:(const std::vector<nve::apple::TextEventSpec> &)specs nodeId:(uint32_t)nodeId;
/// Runs `block` with this host on the next main-queue turn, unless the host is gone by then (the
/// block never retains it). For calls made during reconcile, which must not re-enter the runtime.
- (void)_deferToNextTurn:(void (^)(NveAppleComponentHost *host))block;

@end

NS_ASSUME_NONNULL_END