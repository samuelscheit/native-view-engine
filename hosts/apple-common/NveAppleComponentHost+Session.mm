#import "NveAppleComponentHostInternal.h"

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

} // namespace nve::apple
