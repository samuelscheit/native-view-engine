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

#pragma mark - Frame-0 (ABI v6) & raw NVE2 submission

- (BOOL)saveFrame0ToPath:(NSString *)path
              appBuildId:(uint64_t)appBuildId
                 envHash:(uint64_t)envHash
                   error:(NSError **)error {
  if (path.length == 0) {
    if (error) *error = [self _errorWithCode:static_cast<NSInteger>(nve::host::ModuleError::Open) description:@"Empty Frame-0 path"];
    return NO;
  }
  const NveRuntimeStatus st = _session.save_frame0_file(path.UTF8String ?: "", appBuildId, envHash);
  if (st != NVE_RUNTIME_OK) {
    if (error) *error = [self _runtimeError:[NSString stringWithFormat:@"Saving Frame-0 %@ failed", path] status:st];
    return NO;
  }
  return YES;
}

- (BOOL)bootFrame0FromPath:(NSString *)path
                appBuildId:(uint64_t)appBuildId
                   envHash:(uint64_t)envHash
                     error:(NSError **)error {
  if (_appAttached) {
    if (error) {
      *error = [self _errorWithCode:static_cast<NSInteger>(nve::host::ModuleError::Attach)
                        description:@"Cannot boot Frame-0 while an app is attached"];
    }
    return NO;
  }
  if (path.length == 0) {
    if (error) *error = [self _errorWithCode:static_cast<NSInteger>(nve::host::ModuleError::Open) description:@"Empty Frame-0 path"];
    return NO;
  }
  const NveRuntimeStatus st = _session.boot_frame0_file(path.UTF8String ?: "", appBuildId, envHash);
  if (st != NVE_RUNTIME_OK) {
    if (error) *error = [self _runtimeError:[NSString stringWithFormat:@"Booting Frame-0 %@ failed", path] status:st];
    return NO;
  }
  [self renderFrame];
  [self syncNativeViewsFromHost];
  _hasReplayedFrame0 = YES;
  return YES;
}

- (BOOL)bootFrame0FromBytes:(NSData *)envelope
                 appBuildId:(uint64_t)appBuildId
                    envHash:(uint64_t)envHash
                      error:(NSError **)error {
  if (_appAttached) {
    if (error) {
      *error = [self _errorWithCode:static_cast<NSInteger>(nve::host::ModuleError::Attach)
                        description:@"Cannot boot Frame-0 while an app is attached"];
    }
    return NO;
  }
  if (envelope.length == 0) {
    if (error) *error = [self _errorWithCode:static_cast<NSInteger>(nve::host::ModuleError::Open) description:@"Empty Frame-0 envelope"];
    return NO;
  }
  const std::span<const uint8_t> bytes(static_cast<const uint8_t *>(envelope.bytes), envelope.length);
  const NveRuntimeStatus st = _session.boot_frame0_bytes(bytes, appBuildId, envHash);
  if (st != NVE_RUNTIME_OK) {
    if (error) *error = [self _runtimeError:@"Booting Frame-0 bytes failed" status:st];
    return NO;
  }
  [self renderFrame];
  [self syncNativeViewsFromHost];
  _hasReplayedFrame0 = YES;
  return YES;
}

- (BOOL)submitNve2Frame:(NSData *)frame {
  if (_appAttached || frame.length == 0) return NO;
  const std::span<const uint8_t> bytes(static_cast<const uint8_t *>(frame.bytes), frame.length);
  if (_session.submit_nve2(bytes) != NVE_RUNTIME_OK) return NO;
  [self renderFrame];
  [self syncNativeViewsFromHost];  // `submit_nve2` publishes immediately; mount it now
  return YES;
}

@end
