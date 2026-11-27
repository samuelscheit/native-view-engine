#import "NveAppleComponentHostInternal.h"

#include <nve/host/snapshot_file.hpp>

#include <chrono>
#include <string>

using nve::host::SnapshotFile;

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
    (void)_session.detach_app();
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

#pragma mark - Instant boot

- (BOOL)replayFrame0:(NSData *)frame0 {
  if (_appAttached || frame0.length == 0) return NO;
  const std::span<const uint8_t> bytes(static_cast<const uint8_t *>(frame0.bytes), frame0.length);
  if (_session.submit_nve2(bytes) != NVE_RUNTIME_OK) return NO;
  [self renderFrame];
  [self syncNativeViewsFromHost];  // `submit_nve2` publishes immediately; mount it now
  _hasReplayedFrame0 = YES;
  return YES;
}

- (BOOL)replayFrame0SnapshotAtPath:(NSString *)snapshotPath
             expectedSchemaVersion:(uint32_t)expectedSchemaVersion
                     restoreTimeUs:(uint64_t *)outRestoreUs {
  const auto start = std::chrono::steady_clock::now();
  const auto file = SnapshotFile::read(snapshotPath.fileSystemRepresentation, expectedSchemaVersion);
  if (!file) return NO;
  NSData *frame0 = [NSData dataWithBytes:file->frame0.data() length:file->frame0.size()];
  if (![self replayFrame0:frame0]) return NO;
  _pendingSnapshotState = file->state;  // restored once the live app attaches
  if (outRestoreUs != nullptr) {
    *outRestoreUs = static_cast<uint64_t>(
        std::chrono::duration_cast<std::chrono::microseconds>(std::chrono::steady_clock::now() - start).count());
  }
  return YES;
}

- (BOOL)saveSnapshotAtPath:(NSString *)snapshotPath schemaVersion:(uint32_t)schemaVersion {
  SnapshotFile file;
  file.schema = schemaVersion;
  if (_session.encode_scene(file.frame0) != NVE_RUNTIME_OK || file.frame0.empty()) return NO;
  if (_appAttached) {
    std::span<const uint8_t> state;
    if (_session.snapshot_state(&state) == NVE_RUNTIME_OK) file.state.assign(state.begin(), state.end());
  } else {
    file.state = _pendingSnapshotState;  // replayed but never attached: keep the original state
  }
  return file.write(snapshotPath.fileSystemRepresentation);
}

@end
