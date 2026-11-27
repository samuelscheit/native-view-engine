#import "NveAppleComponentHostInternal.h"

#include <string>

@implementation NveAppleComponentHost (Session)

#pragma mark - Tier-2 reload

- (BOOL)reloadDylibAtPath:(NSString *)dylibPath error:(NSError **)error {
  nve::apple::LoadedDylib next;
  if (!nve::apple::open_nve_dylib(dylibPath, _errorDomain, &next, error)) return NO;  // live app untouched
  std::span<const uint8_t> state;
  if (_appAttached && _session.snapshot_state(&state) == NVE_RUNTIME_OK) {
    _pendingSnapshotState.assign(state.begin(), state.end());  // copy: the slice dies with the old app
  }
  if (![self _attachGetInterface:next.get_interface error:error]) {
    _pendingSnapshotState.clear();
    nve::apple::close_dylib(next.handle);
    return NO;
  }
  nve::apple::close_dylib(_dylibHandle);  // the old app was destroyed by the attach above
  _dylibHandle = next.handle;
  return YES;
}

#pragma mark - Pairing

- (BOOL)connectPairedEndpoint:(NSString *)uri token:(NSString *)token surfaceId:(uint32_t)surfaceId error:(NSError **)error {
  _surfaceId = surfaceId;
  if (_appAttached) {
    // Only unload the app's code once the runtime has really dropped the app.
    if (_session.detach_app() != NVE_RUNTIME_OK) {
      if (error) *error = [self _errorWithCode:1 description:@"Failed to detach the running app before pairing"];
      return NO;
    }
    _appAttached = NO;
    nve::apple::close_dylib(_dylibHandle);
  }
  if (_session.pair_connect(uri.UTF8String ?: "", token.UTF8String ?: "") != NVE_RUNTIME_OK) {
    if (error) {
      const std::string detail = _session.last_error();
      *error = [self _errorWithCode:static_cast<NSInteger>(nve::apple::DylibErrorCode::OpenFailed)
                        description:[NSString stringWithFormat:@"Pairing with %@ failed: %s", uri, detail.c_str()]];
    }
    return NO;
  }
  _pairRequested = YES;
  [self renderFrame];
  return YES;
}

- (void)disconnectPaired {
  if (!_pairRequested) return;
  (void)_session.pair_disconnect();
  _pairRequested = NO;
}

#pragma mark - Frame-0 replay

- (BOOL)replayFrame0:(NSData *)frame0 {
  if (_appAttached || frame0.length == 0) return NO;
  const std::span<const uint8_t> bytes(static_cast<const uint8_t *>(frame0.bytes), frame0.length);
  if (_session.submit_nve2(bytes) != NVE_RUNTIME_OK) return NO;
  [self renderFrame];
  [self syncNativeViewsFromHost];  // `submit_nve2` publishes immediately; mount it now
  _hasReplayedFrame0 = YES;
  return YES;
}

@end
