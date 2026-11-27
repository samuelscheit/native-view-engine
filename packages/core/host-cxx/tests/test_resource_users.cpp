// `ResourceUsers`: the reverse index resource id -> views stays exact under random re-pointing,
// clearing, and resets (checked against a naive oracle), its head map is O(referenced ids) for any
// id value (no table sized by the largest id), and a reset forgets everything and shrinks.
#include <algorithm>
#include <cstdint>
#include <cstdio>
#include <map>
#include <random>
#include <vector>

#include "check.h"
#include "nve/host/resource_users.hpp"

namespace {

using nve::host::ResourceUsers;

std::vector<uint32_t> users_of(const ResourceUsers& u, uint32_t id) {
  std::vector<uint32_t> out;
  u.for_each_user(id, [&](uint32_t v) { out.push_back(v); });
  std::sort(out.begin(), out.end());
  return out;
}

void test_random_ops_match_an_oracle() {
  constexpr uint32_t kViews = 64u;
  // Few ids, including huge ones and ids colliding in a 16-slot table, so chains and
  // backward-shift deletes are exercised.
  const std::vector<uint32_t> ids = {1u, 2u, 3u, 17u, 33u, 49u, 0x00FFFFFFu, 0xFFFFFFFEu, 0x80000000u, 977u};
  ResourceUsers u;
  u.reserve_views(kViews);
  std::vector<ResourceUsers::Ids> oracle(kViews, ResourceUsers::Ids{});
  std::mt19937 rng(7u);
  for (int step = 0; step < 20000; ++step) {
    const uint32_t view = rng() % kViews;
    switch (rng() % 8u) {
      case 0: u.clear(view); oracle[view] = {}; break;
      case 1:
        if (step % 2000 == 1999) {
          u.reset();
          std::fill(oracle.begin(), oracle.end(), ResourceUsers::Ids{});
        }
        break;
      default: {
        ResourceUsers::Ids next{};
        for (uint32_t& id : next) id = rng() % 3u == 0u ? 0u : ids[rng() % ids.size()];
        u.set(view, next);
        oracle[view] = next;
      }
    }
    if (step % 97 != 0) continue;
    size_t referenced = 0u;
    for (const uint32_t id : ids) {
      std::vector<uint32_t> expected;
      for (uint32_t v = 0u; v < kViews; ++v) {
        for (const uint32_t r : oracle[v]) {
          if (r == id) expected.push_back(v);
        }
      }
      std::sort(expected.begin(), expected.end());
      CHECK(users_of(u, id) == expected);
      referenced += expected.empty() ? 0u : 1u;
    }
    CHECK(u.referenced() == referenced);
    CHECK(users_of(u, 0u).empty() && users_of(u, 4u).empty());
  }
}

void test_memory_is_bounded_by_referenced_ids_not_id_values() {
  ResourceUsers u;
  u.reserve_views(2u);
  u.set(0u, {0xFFFFFFF0u, 0x7FFFFFFFu, 0u, 0u});
  u.set(1u, {0xFFFFFFF0u, 0u, 0u, 5u});
  CHECK(u.referenced() == 3u && u.capacity() <= 16u);
  CHECK((users_of(u, 0xFFFFFFF0u) == std::vector<uint32_t>{0u, 1u}));
  // Many ids grow the map; a reset after they are gone shrinks it back.
  ResourceUsers big;
  big.reserve_views(4096u);
  for (uint32_t v = 0u; v < 4096u; ++v) big.set(v, {v * 7919u + 1u, 0u, 0u, 0u});
  CHECK(big.referenced() == 4096u && big.capacity() >= 4096u && big.capacity() <= 4u * 4096u);
  for (uint32_t v = 0u; v < 4096u; ++v) big.clear(v);
  CHECK(big.referenced() == 0u);
  big.reset();
  CHECK(big.capacity() <= 16u);
  // Unlinked ids leave nothing behind.
  u.clear(0u);
  u.clear(1u);
  CHECK(u.referenced() == 0u && users_of(u, 0xFFFFFFF0u).empty());
}

}  // namespace

int main() {
  test_random_ops_match_an_oracle();
  test_memory_is_bounded_by_referenced_ids_not_id_values();
  if (g_failures != 0) {
    fprintf(stderr, "nve_host_resource_users_test: %d failure(s)\n", g_failures);
    return 1;
  }
  printf("nve_host_resource_users_test: ok\n");
  return 0;
}
