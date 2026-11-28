// C++20 end-to-end link-and-run test against the real compiled `libnve_runtime` static library.
//
// Exercises header include-order safety (`api.h` before `runtime_session.hpp`), the single
// exported `nve_runtime_get_interface` entry point, RAII `RuntimeSession`, real binary `NVE2`
// dynamic table + text + hierarchy submission, zero-copy `RuntimeSceneGuard` / `SceneReader`
// + `TableReader` + post-release/move/self-move empty-span safety, `NVE_RUNTIME_BUSY` exclusion,
// owner-thread `hit_test` under active reader lock, `set_viewport` + `tick`, `get_metrics`, live
// `set_wake_callback` firing (`g_wake_count > 0`), exact pairing URI/pin validation, live loopback
// WebSocket pairing roundtrip, `last_error`, move semantics, and clean RAII `destroy` under ASan/UBSan.

#include <array>
#include <atomic>
#include <chrono>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <span>
#include <string>
#include <string_view>
#include <thread>
#include <utility>
#include <vector>

#include "check.h"
#if defined(NVE_RUNTIME_TEST_HAS_WIRE)
#include "nve/host/testing/loopback_pair_server.hpp"
#endif
#include "nve/host/nve2_builder.hpp"
// Include `api.h` BEFORE `runtime_session.hpp` to verify `NVE_RUNTIME_API` and
// `NVE_STATIC_ASSERT` never clash with `NVE_API` regardless of include order.
#include "nve/host/api.h"
#include "nve/host/runtime_session.hpp"

namespace {

using nve::host::field;
using nve::host::Key64;
using nve::host::RuntimeSceneGuard;
using nve::host::SceneReader;
using nve::host::RuntimeSession;
using nve::host::SceneError;
using nve::host::TableReader;


/// Builds a valid binary `NVE2` frame declaring dynamic sparse table `"cxx.demo.badge"` (3 words:
/// word 0 = u32 badge_count, words 1..2 = StrSpan label), creating ROOT(1) + child(2), patching
/// badge_count = `badge_value`, setting UTF-8 label = `label`, and attaching child(2) under ROOT(1).
std::vector<uint8_t> build_demo_nve2(uint32_t badge_value, std::string_view label) {
  constexpr uint8_t kWireSlot = 40u;
  constexpr std::array<uint32_t, 3> kDefaults{0u, 0u, 0u};
  constexpr std::array<uint8_t, 3> kEffects{1u, 1u, 1u};
  const std::array<uint32_t, 3> row{badge_value, 0u, 0u};
  return nve::host::Nve2FrameBuilder()
      .declare({Key64::of("cxx.demo.badge"), Key64::of("cxx.demo.badge.schema.v1"), kWireSlot, true /* sparse */,
                2u /* PUBLISH_TO_SCENE */, kDefaults, kEffects, uint32_t{1} << 1u /* words 1..2 = StrSpan label */,
                0u, "cxx.demo.badge"})
      .create_nodes({1u, 2u})
      .patch(kWireSlot, 2u, 1u, row)
      .set_string(kWireSlot, 2u, 1u, label)
      .set_children(1u, {2u})
      .finish();
}


std::atomic<int> g_wake_count{0};

extern "C" void test_wake_cb(void* ctx) {
  auto* counter = static_cast<std::atomic<int>*>(ctx);
  counter->fetch_add(1, std::memory_order_release);
}

void test_live_runtime_session() {
  // ABI version mismatch returns NULL; matching version returns a complete vtable.
  CHECK(nve_runtime_get_interface(NVE_RUNTIME_ABI_VERSION + 1u) == nullptr);
  const NveRuntimeApi* api = nve_runtime_get_interface(NVE_RUNTIME_ABI_VERSION);
  CHECK(api != nullptr);
  CHECK(RuntimeSession::validate_api(api));

  const NveRuntimeConfig cfg = RuntimeSession::default_config(1u, 400.0f, 300.0f, 1.0f);
  std::optional<RuntimeSession> opt_session = RuntimeSession::create(api, cfg);
  CHECK(opt_session.has_value());

  // Exercise move semantics cleanly.
  RuntimeSession session = std::move(*opt_session);
  CHECK(session.valid());
  CHECK(!opt_session->valid());

  // Install and clear wake callback cleanly.
  g_wake_count.store(0, std::memory_order_release);
  CHECK(session.set_wake_callback(test_wake_cb, &g_wake_count) == NVE_RUNTIME_OK);
  CHECK(session.clear_wake_callback() == NVE_RUNTIME_OK);

  // Submit real binary NVE2 frame with dynamic table, node hierarchy, and UTF-8 string.
  const std::vector<uint8_t> frame = build_demo_nve2(42u, "Hello C++20 Runtime!");
  CHECK(session.submit_nve2(frame) == NVE_RUNTIME_OK);

  // Acquire zero-copy scene view under RAII guard + verify move/release empty-span safety.
  {
    NveRuntimeStatus st = NVE_RUNTIME_ERROR;
    SceneError serr = SceneError::NullView;
    std::optional<RuntimeSceneGuard> opt_guard = session.acquire_scene(&st, &serr);
    CHECK(opt_guard.has_value());
    CHECK(st == NVE_RUNTIME_OK);
    CHECK(serr == SceneError::None);

    // Move-construct guard; moved-from guard must clear raw_ & reader_ spans immediately.
    RuntimeSceneGuard guard = std::move(*opt_guard);
    CHECK(!opt_guard->active());
    CHECK(opt_guard->reader().rows().empty() && opt_guard->reader().rects().empty());
    CHECK(opt_guard->raw().rows == nullptr && opt_guard->raw().node_capacity == 0u);

    // Self-move assignment must preserve live guard cleanly.
    RuntimeSceneGuard* self_ptr = &guard;
    guard = std::move(*self_ptr);
    CHECK(guard.active());

    const SceneReader& scene = guard.reader();
    CHECK(scene.frame_id() == 1u);
    CHECK(scene.node_capacity() >= 3u);
    CHECK(scene.is_live(1u));
    CHECK(scene.is_live(2u));

    const NveLayoutRect* root_rect = scene.rect(1u);
    CHECK(root_rect != nullptr);
    CHECK(root_rect->width == 400.0f);
    CHECK(root_rect->height == 300.0f);

    const auto root_kids = scene.node_children(1u);
    CHECK(root_kids.has_value() && root_kids->size() == 1u && (*root_kids)[0] == 2u);

    const std::optional<TableReader> badge = scene.find_table(Key64::of("cxx.demo.badge"));
    CHECK(badge.has_value() && badge->is_sparse() && badge->row_count() == 1u);
    if (badge.has_value()) {
      const std::optional<uint32_t> row = badge->row_of(2u);
      CHECK(row.has_value());
      if (row.has_value()) {
        CHECK(badge->u32(*row, field(0, 0, 32, NVE_FIELD_U32)) == 42u);
        const auto str_span = badge->str(*row, field(1, 0, 32, NVE_FIELD_STR));
        CHECK(str_span.has_value());
        if (str_span.has_value()) {
          CHECK(scene.text(str_span->offset, str_span->len) == "Hello C++20 Runtime!");
        }
      }
    }

    // While reader guard is held: mutating `tick` MUST return `NVE_RUNTIME_BUSY`,
    // while owner-thread `hit_test` MUST succeed (0 = no Interaction table subscribed).
    bool busy_changed = false;
    CHECK(session.tick(0.016f, &busy_changed) == NVE_RUNTIME_BUSY);
    uint32_t hit_id = 999u;
    CHECK(session.hit_test(20.0f, 20.0f, &hit_id) == NVE_RUNTIME_OK);
    CHECK(hit_id == 0u);

    // Explicit release() clears spans and releases the reader lock immediately.
    guard.release();
    CHECK(!guard.active());
    CHECK(guard.reader().rows().empty() && guard.reader().rects().empty());
    CHECK(guard.raw().rows == nullptr && guard.raw().node_capacity == 0u);
  }

  // After releasing guard: `set_viewport` + `tick` advances `frame_id` and updates ROOT rect.
  CHECK(session.set_viewport(640.0f, 480.0f, 2.0f) == NVE_RUNTIME_OK);
  bool changed = false;
  CHECK(session.tick(0.016f, &changed) == NVE_RUNTIME_OK);
  CHECK(changed);

  NveRuntimeMetrics metrics{};
  CHECK(session.metrics(&metrics) == NVE_RUNTIME_OK);
  CHECK(metrics.frame_id == 2u);
  CHECK(metrics.live_nodes == 2u);
  CHECK(metrics.pair_connected == 0u);

  {
    auto guard2 = session.acquire_scene();
    CHECK(guard2.has_value());
    const NveLayoutRect* r1 = (*guard2)->rect(1u);
    CHECK(r1 != nullptr && r1->width == 640.0f && r1->height == 480.0f);
  }

  // Exact pairing URI + TLS cert-pin validation + last_error roundtrip.
#if defined(NVE_RUNTIME_TEST_HAS_WIRE)
  CHECK(session.pair_connect("", "") == NVE_RUNTIME_INVALID_ARG);
  CHECK(!session.last_error().empty());
  CHECK(session.pair_connect("not-a-url", "tok") == NVE_RUNTIME_INVALID_ARG);
  CHECK(session.pair_connect("wss://127.0.0.1:8092/nve-wire", "tok") == NVE_RUNTIME_INVALID_ARG);
  CHECK(!session.pair_is_connected());
  CHECK(session.pair_disconnect() == NVE_RUNTIME_OK);
#else
  CHECK(session.pair_connect("", "") == NVE_RUNTIME_UNSUPPORTED);
  CHECK(!session.last_error().empty());
  CHECK(!session.pair_is_connected());
  CHECK(session.pair_disconnect() == NVE_RUNTIME_UNSUPPORTED);
#endif

#if defined(NVE_RUNTIME_TEST_HAS_WIRE)
  // Live loopback WebSocket pairing roundtrip verifying `g_wake_count > 0` + remote scene sync!
  {
    nve::host::testing::LoopbackPairServer server;
    CHECK(server.listening());
    server.send_keyframe(build_demo_nve2(77u, "Paired C++20!"));
    g_wake_count.store(0, std::memory_order_release);
    CHECK(session.set_wake_callback(test_wake_cb, &g_wake_count) == NVE_RUNTIME_OK);
    CHECK(session.pair_connect(server.uri(), "cxx-live-token") == NVE_RUNTIME_OK);

    bool paired_synced = false;
    const auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(3);
    while (std::chrono::steady_clock::now() < deadline) {
      bool tick_changed = false;
      CHECK(session.tick(0.016f, &tick_changed) == NVE_RUNTIME_OK);
      if (tick_changed) {
        auto pg = session.acquire_scene();
        if (pg.has_value()) {
          const auto badge = (*pg)->find_table(Key64::of("cxx.demo.badge"));
          if (badge.has_value()) {
            const auto row = badge->row_of(2u);
            if (row.has_value() && badge->u32(*row, field(0, 0, 32, NVE_FIELD_U32)) == 77u) {
              const auto s = badge->str(*row, field(1, 0, 32, NVE_FIELD_STR));
              CHECK(s.has_value() && (*pg)->text(s->offset, s->len) == "Paired C++20!");
              paired_synced = true;
              break;
            }
          }
        }
      }
      std::this_thread::sleep_for(std::chrono::milliseconds(10));
    }
    CHECK(paired_synced);
    CHECK(g_wake_count.load(std::memory_order_acquire) > 0);
    CHECK(session.pair_is_connected());

    CHECK(server.paired());
    CHECK(session.pair_disconnect() == NVE_RUNTIME_OK);
    CHECK(!session.pair_is_connected());
  }
#endif

  // Explicit reset destroys cleanly; second reset is a no-op.
  session.reset();
  CHECK(!session.valid());
  session.reset();
}

}  // namespace

int main() {
  test_live_runtime_session();
  if (g_failures != 0) {
    fprintf(stderr, "nve_host_runtime_session_test: %d failure(s)\n", g_failures);
    return 1;
  }
  printf("nve_host_runtime_session_test: ok\n");
  return 0;
}
