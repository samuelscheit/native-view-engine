#import "NveAppleComponentHostInternal.h"

#include "nve_snapshot.hpp"

#include <chrono>
#include <span>
#include <string>
#include <utility>
#include <vector>

namespace nve::apple {

BOOL host_connect_paired_endpoint(
    NveAppleComponentHost *host,
    NSString *endpoint,
    NSString *token,
    NSString * _Nullable transport,
    uint32_t surfaceId,
    NSError * _Nullable * _Nullable error) {
  nve::PairClientConfig cfg{};
  const char *epUtf8 = endpoint.UTF8String;
  const char *tokUtf8 = token.UTF8String;
  const char *trUtf8 = transport ? transport.UTF8String : "";
  std::string parseErr;
  if (!nve::parse_pair_endpoint(
          epUtf8 ? std::string_view(epUtf8) : std::string_view{},
          trUtf8 ? std::string_view(trUtf8) : std::string_view{},
          cfg,
          &parseErr)) {
    if (error) {
      *error = make_host_init_error(
          host->_errorDomain,
          [NSString stringWithUTF8String:parseErr.c_str()] ?: @"Invalid pairing endpoint");
    }
    return NO;
  }
  if (tokUtf8 != nullptr) {
    cfg.pairing_token = tokUtf8;
  }
  return host_connect_paired_config(host, cfg, surfaceId, error);
}

BOOL host_connect_paired_config(
    NveAppleComponentHost *host,
    const nve::PairClientConfig &config,
    uint32_t surfaceId,
    NSError * _Nullable * _Nullable error) {
  id<NveAppleComponentViewAdapter> adapter = host->_adapter;
  if (!adapter) {
    if (error) {
      *error = make_host_init_error(
          host->_errorDomain, @"Component view adapter is no longer alive");
    }
    return NO;
  }

  const NveSurfaceConfig surfaceCfg = [adapter nveSurfaceConfigForSurfaceId:surfaceId];
  NveFabricHostHandle nextHost = nve_fabric_host_create(surfaceCfg);
  if (nextHost == nullptr) {
    if (error) {
      *error = make_host_init_error(
          host->_errorDomain, @"Failed to create FabricHost for paired session");
    }
    return NO;
  }
  install_native_text_measurer(nextHost);

  __weak NveAppleComponentHost *weakHost = host;
  auto wakeCb = [weakHost]() {
    dispatch_async(dispatch_get_main_queue(), ^{
      __strong NveAppleComponentHost *strongHost = weakHost;
      if (strongHost) {
        [strongHost pollPairedFrames];
      }
    });
  };

  auto nextClient = std::make_unique<nve::PairClient>();
  std::string connectErr;
  if (!nextClient->connect(config, std::move(wakeCb), &connectErr)) {
    nve_fabric_host_destroy(nextHost);
    if (error) {
      *error = make_host_init_error(
          host->_errorDomain,
          [NSString stringWithUTF8String:connectErr.c_str()] ?: @"Failed to connect paired client");
    }
    return NO;
  }

  host_disconnect_paired(host);
  host->_bridge.reset();
  if (host->_frame0Host != nullptr) {
    nve_fabric_host_destroy(host->_frame0Host);
    host->_frame0Host = nullptr;
  }
  host->_pendingSnapshotState.clear();
  close_dylib(host->_dylibHandle);
  [host->_reconciler clearNativeNodeViews];

  host->_pairedHost = nextHost;
  host->_pairClient = std::move(nextClient);
  host->_pairedFrameCount = 0u;
  host->_surfaceId = surfaceId;
  (void)host_poll_paired_frames(host);
  return YES;
}

void host_disconnect_paired(NveAppleComponentHost *host) {
  if (host->_pairClient != nullptr) {
    host->_pairClient->disconnect();
    host->_pairClient.reset();
  }
  if (host->_pairedHost != nullptr) {
    nve_fabric_host_destroy(host->_pairedHost);
    host->_pairedHost = nullptr;
  }
  host->_pairedFrameCount = 0u;
}

NSUInteger host_poll_paired_frames(NveAppleComponentHost *host) {
  if (!host->_pairClient || host->_pairedHost == nullptr) {
    return 0u;
  }
  std::vector<std::pair<uint32_t, std::vector<uint8_t>>> frames;
  if (host->_pairClient->drain_pending_frames(frames) == 0u) {
    return 0u;
  }
  NSUInteger applied = 0u;
  BOOL anyMutations = NO;
  for (const auto &item : frames) {
    const auto &cmdBuf = item.second;
    if (cmdBuf.empty()) continue;
    NveFrameStats stats{};
    if (nve_fabric_host_submit_frame(
            host->_pairedHost, NveBufferSlice{cmdBuf.data(), cmdBuf.size()}, &stats) == 0) {
      ++applied;
      ++host->_pairedFrameCount;
      if (stats.mutation_count > 0u) {
        anyMutations = YES;
      }
      const NveBufferSlice ev = nve_fabric_host_drain_events(host->_pairedHost);
      if (ev.ptr != nullptr && ev.len > 4u) {
        (void)host->_pairClient->send_events(std::span<const uint8_t>(ev.ptr, ev.len));
      }
    }
  }
  if (applied > 0u) {
    [host->_reconciler syncFromFabricHost:host->_pairedHost hasMutations:anyMutations];
  }
  return applied;
}

BOOL host_replay_frame0_snapshot(
    NveAppleComponentHost *host,
    NSString *snapshotPath,
    uint32_t expectedSchemaVersion,
    uint64_t * _Nullable outRestoreUs) {
  id<NveAppleComponentViewAdapter> adapter = host->_adapter;
  if (!adapter || snapshotPath.length == 0) {
    return NO;
  }
  const char *utf8 = snapshotPath.UTF8String;
  if (utf8 == nullptr) {
    return NO;
  }

  using Clock = std::chrono::steady_clock;
  const auto t0 = Clock::now();

  nve::MmapSnapshot snap;
  if (!nve::MmapSnapshot::open_with_schema(
          std::string_view(utf8), expectedSchemaVersion, snap, /*verify_checksum=*/true)) {
    return NO;
  }

  const NveBufferSlice frame0 = snap.frame0_slice();
  if (frame0.ptr == nullptr || frame0.len == 0u) {
    return NO;
  }
  const NveBufferSlice state = snap.state_slice();

  if (host->_bridge) {
    if (nve_fabric_host_submit_frame(host->_bridge->fabric_host(), frame0, nullptr) != 0) {
      return NO;
    }
    if (state.ptr != nullptr && state.len > 0u) {
      host->_bridge->restore_state(state, /*auto_render=*/false);
    }
  } else {
    const NveSurfaceConfig cfg = [adapter nveSurfaceConfigForSurfaceId:host->_surfaceId];
    NveFabricHostHandle nextHost = nve_fabric_host_create(cfg);
    if (nextHost == nullptr) {
      return NO;
    }
    install_native_text_measurer(nextHost);
    if (nve_fabric_host_submit_frame(nextHost, frame0, nullptr) != 0) {
      nve_fabric_host_destroy(nextHost);
      return NO;
    }
    if (host->_frame0Host != nullptr) {
      nve_fabric_host_destroy(host->_frame0Host);
    }
    host->_frame0Host = nextHost;
    if (state.ptr != nullptr && state.len > 0u) {
      host->_pendingSnapshotState.assign(state.ptr, state.ptr + state.len);
    } else {
      host->_pendingSnapshotState.clear();
    }
  }

  [host->_reconciler clearNativeNodeViews];
  [host->_reconciler syncFromFabricHost:[host _activeFabricHost] hasMutations:YES];

  const auto t1 = Clock::now();
  if (outRestoreUs != nullptr) {
    *outRestoreUs = static_cast<uint64_t>(
        std::chrono::duration_cast<std::chrono::microseconds>(t1 - t0).count());
  }
  return YES;
}

BOOL host_save_snapshot(
    NveAppleComponentHost *host,
    NSString *snapshotPath,
    uint32_t schemaVersion) {
  if (snapshotPath.length == 0) {
    return NO;
  }
  const char *utf8 = snapshotPath.UTF8String;
  if (utf8 == nullptr) {
    return NO;
  }
  NveFabricHostHandle activeHost = [host _activeFabricHost];
  if (activeHost == nullptr) {
    return NO;
  }
  const uint64_t frameId =
      (host->_bridge && host->_bridge->frame_count() > 0u) ? host->_bridge->frame_count() : 1u;
  const std::vector<uint8_t> frame0 = nve::fabric_host_serialize_frame0(activeHost, frameId);
  if (frame0.empty()) {
    return NO;
  }
  const std::vector<uint8_t> state =
      host->_bridge ? host->_bridge->snapshot_state() : host->_pendingSnapshotState;
  return nve::write_snapshot_atomic(
             std::string_view(utf8),
             schemaVersion,
             NveBufferSlice{frame0.data(), frame0.size()},
             NveBufferSlice{state.empty() ? nullptr : state.data(), state.size()})
             ? YES
             : NO;
}

} // namespace nve::apple
