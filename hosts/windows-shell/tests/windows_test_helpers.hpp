#pragma once

#include "nve_abi.generated.h"
#include "nve_event_batch.hpp"
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
  nve::EventBatchBuilder pending;
  nve::EventBatchBuilder drained;

  void push_custom(uint32_t sid, uint32_t nid, uint64_t ts, std::string name, std::string payload) {
    (void)pending.push_custom(
        sid,
        nid,
        ts,
        name,
        payload.empty() ? nullptr : reinterpret_cast<const uint8_t*>(payload.data()),
        payload.size());
  }

  NveEventBatch drain() {
    drained = std::move(pending);
    pending.clear();
    return drained.as_batch();
  }
};

} // namespace nve::win_test
