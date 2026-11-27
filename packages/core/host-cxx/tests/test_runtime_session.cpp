// C++20 end-to-end link-and-run test against the real compiled `libnve_runtime` static library.
//
// Exercises the single exported `nve_runtime_get_interface` entry point, RAII `RuntimeSession`,
// real binary `NVE2` dynamic table + text + hierarchy submission, zero-copy `RuntimeSceneGuard` /
// `RuntimeSceneReader` + `TableReader`, `NVE_RUNTIME_BUSY` exclusion, owner-thread `hit_test`
// under active reader lock, `set_viewport` + `tick`, `get_metrics`, `set_wake_callback`, pairing
// validation + `last_error`, move semantics, and clean RAII `destroy` under ASan/UBSan.

#include <atomic>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <span>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

#include "check.h"
#include "nve/host/runtime_session.hpp"

namespace {

using nve::host::field;
using nve::host::Key64;
using nve::host::RuntimeSceneGuard;
using nve::host::RuntimeSceneReader;
using nve::host::RuntimeSession;
using nve::host::SceneError;
using nve::host::TableReader;

constexpr uint32_t kNve2Magic = 0x3245564Eu;  // "NVE2" LE
constexpr uint32_t kNve2Version = 1u;
constexpr uint8_t kOpDeclareTables = 1u;
constexpr uint8_t kOpCreateNodes = 3u;
constexpr uint8_t kOpPatch = 6u;
constexpr uint8_t kOpSetStrings = 8u;
constexpr uint8_t kOpSetChildren = 9u;

void push_u32(std::vector<uint8_t>& out, uint32_t v) {
  out.push_back(static_cast<uint8_t>(v & 0xFFu));
  out.push_back(static_cast<uint8_t>((v >> 8u) & 0xFFu));
  out.push_back(static_cast<uint8_t>((v >> 16u) & 0xFFu));
  out.push_back(static_cast<uint8_t>((v >> 24u) & 0xFFu));
}

void write_u32_at(std::vector<uint8_t>& out, size_t off, uint32_t v) {
  out[off + 0u] = static_cast<uint8_t>(v & 0xFFu);
  out[off + 1u] = static_cast<uint8_t>((v >> 8u) & 0xFFu);
  out[off + 2u] = static_cast<uint8_t>((v >> 16u) & 0xFFu);
  out[off + 3u] = static_cast<uint8_t>((v >> 24u) & 0xFFu);
}

void push_padded_bytes(std::vector<uint8_t>& out, std::string_view sv) {
  for (const char ch : sv) out.push_back(static_cast<uint8_t>(ch));
  while ((out.size() & 3u) != 0u) out.push_back(0u);
}

/// Builds a valid binary `NVE2` frame declaring dynamic sparse table `"cxx.demo.badge"` (3 words:
/// word 0 = u32 badge_count, words 1..2 = StrSpan label), creating ROOT(1) + child(2), patching
/// badge_count = `badge_value`, setting UTF-8 label = `label`, and attaching child(2) under ROOT(1).
std::vector<uint8_t> build_demo_nve2(uint32_t badge_value, std::string_view label) {
  constexpr std::string_view kTableName = "cxx.demo.badge";
  constexpr Key64 kTableKey = Key64::of(kTableName);
  constexpr Key64 kSchemaHash = Key64::of("cxx.demo.badge.schema.v1");
  constexpr uint8_t kWireSlot = 40u;

  std::vector<uint8_t> out;
  for (int i = 0; i < 6; ++i) push_u32(out, 0u);  // 24-byte NVE2 header placeholder
  uint32_t ops = 0u;

  auto begin_op = [&](uint8_t op, uint8_t table, uint16_t count) -> size_t {
    const size_t start = out.size();
    const uint32_t head = static_cast<uint32_t>(op) | (static_cast<uint32_t>(table) << 8u) |
                          (static_cast<uint32_t>(count) << 16u);
    push_u32(out, head);
    push_u32(out, 0u);  // payload_words placeholder
    return start;
  };
  auto end_op = [&](size_t start) {
    const uint32_t payload_words = static_cast<uint32_t>((out.size() - start) / 4u - 2u);
    write_u32_at(out, start + 4u, payload_words);
    ++ops;
  };

  // 1. DeclareTables (1 sparse 3-word table with StrSpan at word 1).
  {
    const size_t op = begin_op(kOpDeclareTables, 0u, 1u);
    push_u32(out, kTableKey.lo());
    push_u32(out, kTableKey.hi());
    push_u32(out, kSchemaHash.lo());
    push_u32(out, kSchemaHash.hi());
    const uint32_t storage_sparse = 1u;
    const uint32_t flags = uint32_t{1} << 1u;  // table_flags::PUBLISH_TO_SCENE
    const uint32_t words = 3u;
    push_u32(out, static_cast<uint32_t>(kWireSlot) | (storage_sparse << 8u) | (flags << 16u) |
                      (words << 24u));
    push_u32(out, uint32_t{1} << 1u);  // str_words: bit 1 -> words 1 & 2 hold StrSpan
    push_u32(out, 0u);                 // resource_words
    push_u32(out, 0u);                 // default[0]
    push_u32(out, 0u);                 // default[1]
    push_u32(out, 0u);                 // default[2]
    push_u32(out, 0x00010101u);        // word_effects[0..3] padded to 4 bytes
    push_u32(out, static_cast<uint32_t>(kTableName.size()));
    push_padded_bytes(out, kTableName);
    end_op(op);
  }

  // 2. CreateNodes: ROOT (1) and child (2).
  {
    const size_t op = begin_op(kOpCreateNodes, 0u, 2u);
    push_u32(out, 1u);
    push_u32(out, 2u);
    end_op(op);
  }

  // 3. Patch: node 2 word 0 = badge_value.
  {
    const size_t op = begin_op(kOpPatch, kWireSlot, 1u);
    push_u32(out, 2u);
    push_u32(out, 1u);  // mask: word 0
    push_u32(out, badge_value);
    end_op(op);
  }

  // 4. SetStrings: node 2 word 1 = label.
  {
    const size_t op = begin_op(kOpSetStrings, kWireSlot, 1u);
    push_u32(out, 2u);
    push_u32(out, 1u | (static_cast<uint32_t>(label.size()) << 8u));
    push_padded_bytes(out, label);
    end_op(op);
  }

  // 5. SetChildren: ROOT (1) -> [2].
  {
    const size_t op = begin_op(kOpSetChildren, 0u, 1u);
    push_u32(out, 1u);
    push_u32(out, 1u);
    push_u32(out, 2u);
    end_op(op);
  }

  // Patch header.
  const uint32_t total_words = static_cast<uint32_t>(out.size() / 4u);
  write_u32_at(out, 0u, kNve2Magic);
  write_u32_at(out, 4u, kNve2Version);
  write_u32_at(out, 8u, 1u);   // frame_lo = 1
  write_u32_at(out, 12u, 0u);  // frame_hi = 0
  write_u32_at(out, 16u, ops);
  write_u32_at(out, 20u, total_words);
  return out;
}

std::atomic<int> g_wake_count{0};

extern "C" void test_wake_cb(void* ctx) {
  auto* counter = static_cast<std::atomic<int>*>(ctx);
  counter->fetch_add(1, std::memory_order_release);
}

void test_live_runtime_session() {
  // ABI version mismatch returns NULL; matching version returns valid 17-slot vtable.
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

  // Acquire zero-copy scene view under RAII guard.
  {
    NveRuntimeStatus st = NVE_RUNTIME_ERROR;
    SceneError serr = SceneError::NullView;
    std::optional<RuntimeSceneGuard> guard = session.acquire_scene(&st, &serr);
    CHECK(guard.has_value());
    CHECK(st == NVE_RUNTIME_OK);
    CHECK(serr == SceneError::None);

    const RuntimeSceneReader& scene = guard->reader();
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

  // Pairing validation + last_error roundtrip.
  const NveRuntimeStatus pair_rc = session.pair_connect("", "");
  CHECK(pair_rc == NVE_RUNTIME_INVALID_ARG || pair_rc == NVE_RUNTIME_UNSUPPORTED);
  CHECK(!session.last_error().empty());
  CHECK(!session.pair_is_connected());
  const NveRuntimeStatus disc_rc = session.pair_disconnect();
  CHECK(disc_rc == NVE_RUNTIME_OK || disc_rc == NVE_RUNTIME_UNSUPPORTED);

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
