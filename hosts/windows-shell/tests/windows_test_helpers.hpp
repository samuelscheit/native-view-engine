#pragma once

#include "nve_abi.generated.h"
#include "nve_value.hpp"

#include <cmath>
#include <cstdint>
#include <cstdlib>
#include <iostream>
#include <span>
#include <string>
#include <string_view>
#include <vector>

#define NVE_CHECK(expr)                                                                         \
  do {                                                                                          \
    if (!(expr)) {                                                                              \
      std::cerr << "NVE_CHECK failed at " << __FILE__ << ":" << __LINE__ << ": " #expr << "\n"; \
      std::abort();                                                                             \
    }                                                                                           \
  } while (0)

namespace nve::win_test {

inline bool approx_eq(float a, float b, float eps = 1.5f) {
  return std::fabs(a - b) <= eps;
}

inline std::span<const uint8_t> nve_test_patch(std::string_view text) {
  return {reinterpret_cast<const uint8_t*>(text.data()), text.size()};
}

inline std::string get_env_string(const char* name) {
#if defined(_WIN32)
  char* buf = nullptr;
  size_t len = 0;
  if (_dupenv_s(&buf, &len, name) != 0 || buf == nullptr) {
    return {};
  }
  std::string out(buf);
  std::free(buf);
  return out;
#else
  const char* val = std::getenv(name);
  return (val != nullptr) ? std::string(val) : std::string{};
#endif
}

struct DirectOutboundQueue {
  struct Pending {
    uint32_t surface_id{0};
    uint32_t node_id{0};
    uint64_t ts{0};
    std::string name;
    std::string payload;
  };
  std::vector<Pending> pending;
  std::vector<Pending> drained;
  std::vector<NveDirectEvent> batch_storage;

  void push_custom(uint32_t sid, uint32_t nid, uint64_t ts, std::string name, std::string payload) {
    pending.push_back(Pending{sid, nid, ts, std::move(name), std::move(payload)});
  }

  NveEventBatch drain() {
    drained.swap(pending);
    pending.clear();
    batch_storage.clear();
    batch_storage.reserve(drained.size());
    for (const auto& p : drained) {
      NveDirectEvent ev{};
      ev.surface_id = p.surface_id;
      ev.node_id = p.node_id;
      ev.timestamp_us = p.ts;
      ev.kind = NVE_EVENT_CUSTOM;
      ev.payload_tag = NVE_EVENT_PAYLOAD_CUSTOM;
      ev.name = nve::make_str_slice(p.name);
      ev.value = nve::make_bytes_value(NveBufferSlice{
          reinterpret_cast<const uint8_t*>(p.payload.data()),
          p.payload.size(),
      });
      batch_storage.push_back(ev);
    }
    return NveEventBatch{
        batch_storage.empty() ? nullptr : batch_storage.data(),
        static_cast<uint32_t>(batch_storage.size()),
        0u,
    };
  }
};

} // namespace nve::win_test
