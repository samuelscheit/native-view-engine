#import "NveAppleComponentHostInternal.h"

#include <string>

@implementation NveAppleComponentHost (Session)

#pragma mark - Tier-2 reload

- (BOOL)reloadDylibAtPath:(NSString *)dylibPath error:(NSError **)error {
  return [self _loadAppModule:dylibPath carryState:_appAttached error:error];  // failure: live app untouched
}

#pragma mark - Pairing

- (BOOL)connectPairedEndpoint:(NSString *)uri token:(NSString *)token surfaceId:(uint32_t)surfaceId error:(NSError **)error {
  _surfaceId = surfaceId;
  // `pair_connect` swaps atomically: on success the runtime has dropped any attached app (so its
  // code may unload); on failure the running app is untouched.
  if (_session.pair_connect(uri.UTF8String ?: "", token.UTF8String ?: "") != NVE_RUNTIME_OK) {
    if (error) {
      const std::string detail = _session.last_error();
      *error = [self _errorWithCode:static_cast<NSInteger>(nve::host::ModuleError::Open)
                        description:[NSString stringWithFormat:@"Pairing with %@ failed: %s", uri, detail.c_str()]];
    }
    return NO;
  }
  _appAttached = NO;
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
