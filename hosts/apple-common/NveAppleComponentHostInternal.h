#pragma once

#import "NveAppleComponentHost.h"

#include "nve_value.hpp"

#include <memory>
#include <vector>

NS_ASSUME_NONNULL_BEGIN

@interface NveAppleComponentHost () {
 @package
  __weak id<NveAppleComponentViewAdapter> _adapter;
  NSString *_errorDomain;
  NveAppleSceneReconciler *_reconciler;
  std::unique_ptr<nve::NveRnComponentBridge> _bridge;
  std::unique_ptr<nve::PairClient> _pairClient;
  NveFabricHostHandle _Nullable _pairedHost;
  uint64_t _pairedFrameCount;
  NveFabricHostHandle _Nullable _frame0Host;
  std::vector<uint8_t> _pendingSnapshotState;
  void * _Nullable _dylibHandle;
  uint32_t _surfaceId;
  nve::BumpArena _propsArena;
}

- (NveFabricHostHandle _Nullable)_activeFabricHost;

@end

namespace nve::apple {

inline NSError *make_host_init_error(NSString *domain, NSString *description) {
  return [NSError errorWithDomain:domain
                             code:static_cast<NSInteger>(DylibErrorCode::MissingSurfaceInterface)
                         userInfo:@{NSLocalizedDescriptionKey : description}];
}

BOOL host_connect_paired_endpoint(
    NveAppleComponentHost *host,
    NSString *endpoint,
    NSString *token,
    NSString * _Nullable transport,
    uint32_t surfaceId,
    NSError * _Nullable * _Nullable error);

BOOL host_connect_paired_config(
    NveAppleComponentHost *host,
    const nve::PairClientConfig &config,
    uint32_t surfaceId,
    NSError * _Nullable * _Nullable error);

void host_disconnect_paired(NveAppleComponentHost *host);

NSUInteger host_poll_paired_frames(NveAppleComponentHost *host);

BOOL host_replay_frame0_snapshot(
    NveAppleComponentHost *host,
    NSString *snapshotPath,
    uint32_t expectedSchemaVersion,
    uint64_t * _Nullable outRestoreUs);

BOOL host_save_snapshot(
    NveAppleComponentHost *host,
    NSString *snapshotPath,
    uint32_t schemaVersion);

} // namespace nve::apple

NS_ASSUME_NONNULL_END
