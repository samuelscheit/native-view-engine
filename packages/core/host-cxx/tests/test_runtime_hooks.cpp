// C++20 coverage of the embedding-host runtime hooks against the real `libnve_runtime`:
// vtable completeness, native text measurer install / restore / BUSY, scene-guarded
// `resource_payload` (incl. codec-less opaque module kinds and stale ids), the app capability
// hooks without an app (UNSUPPORTED) and with bad arguments (INVALID_ARG, out slots reset), and
// published `scroll_offsets` reader validation / lookup.

#include <array>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <optional>
#include <span>
#include <utility>
#include <vector>

#include "check.h"
#include "nve/host/runtime_session.hpp"
#include "nve/host/nve2_builder.hpp"

namespace {

using nve::host::Key64;
using nve::host::RuntimeResource;
using nve::host::RuntimeSceneGuard;
using nve::host::RuntimeSceneReader;
using nve::host::RuntimeSession;
using nve::host::SceneError;
using nve::host::Nve2FrameBuilder;

constexpr uint8_t kBlobSlot = 41u;
constexpr uint32_t kBlobId = 7u;
constexpr Key64 kBlobKind = Key64::of("cxx.hooks.blob");
constexpr std::array<uint8_t, 5> kBlobBytes{1u, 2u, 3u, 4u, 5u};

const NveRuntimeApi* runtime_api() { return nve_runtime_get_interface(NVE_RUNTIME_ABI_VERSION); }

RuntimeSession make_session() {
  std::optional<RuntimeSession> s =
      RuntimeSession::create(runtime_api(), RuntimeSession::default_config(1u, 400.0f, 300.0f, 1.0f));
  CHECK(s.has_value());
  return s ? std::move(*s) : RuntimeSession();
}

/// Dynamic table `cxx.hooks.blob_ref` (1 resource word) on node 2 pointing at `resource`
/// (`kBlobId` is defined in the same frame with a kind no codec in the runtime knows).
std::vector<uint8_t> blob_frame(uint32_t frame, uint32_t resource) {
  constexpr std::array<uint32_t, 1> kDefaults{0u};
  constexpr std::array<uint8_t, 1> kEffects{1u};
  Nve2FrameBuilder w(frame);
  w.declare({Key64::of("cxx.hooks.blob_ref"), Key64::of("cxx.hooks.blob_ref.v1"), kBlobSlot, true /* sparse */,
             2u /* PUBLISH_TO_SCENE */, kDefaults, kEffects, 0u, 1u /* word 0 = ResourceId */, "cxx.hooks.blob_ref"});
  if (frame == 1u) w.create_nodes({1u, 2u}).define_resource(kBlobId, kBlobKind, kBlobBytes);
  const std::array<uint32_t, 1> row{resource};
  w.patch(kBlobSlot, 2u, 1u, row);
  if (frame == 1u) w.set_children(1u, {2u});
  return w.finish();
}

void test_vtable_complete_and_validated() {
  const NveRuntimeApi* api = runtime_api();
  CHECK(api != nullptr);
  CHECK(nve_runtime_api_slots_complete(api) == 1);
  CHECK(nve_runtime_api_slots_complete(nullptr) == 0);
  CHECK(RuntimeSession::validate_api(api));
  NveRuntimeApi copy = *api;
  copy.resource_payload = nullptr;
  CHECK(nve_runtime_api_slots_complete(&copy) == 0);
  CHECK(!RuntimeSession::validate_api(&copy));
  CHECK(!RuntimeSession::create(&copy).has_value());
  copy = *api;
  copy.schema_hash ^= 1u;
  CHECK(!RuntimeSession::validate_api(&copy));
}

struct MeasureCtx {
  uint32_t calls = 0u;
};

void measure_fixed(void* ctx, NveStrSlice, float, uint32_t, float, float* out_w, float* out_h) {
  ++static_cast<MeasureCtx*>(ctx)->calls;
  *out_w = 10.0f;
  *out_h = 10.0f;
}

void test_text_measurer() {
  RuntimeSession session = make_session();
  MeasureCtx ctx;
  CHECK(session.set_text_measurer(&measure_fixed, &ctx) == NVE_RUNTIME_OK);
  CHECK(session.clear_text_measurer() == NVE_RUNTIME_OK);
  CHECK(session.set_text_measurer(&measure_fixed, &ctx) == NVE_RUNTIME_OK);
  {
    std::optional<RuntimeSceneGuard> guard = session.acquire_scene();
    CHECK(guard.has_value());
    // Installing relayouts: a held scene reader blocks it.
    CHECK(session.set_text_measurer(&measure_fixed, &ctx) == NVE_RUNTIME_BUSY);
    CHECK(session.clear_text_measurer() == NVE_RUNTIME_BUSY);
  }
  CHECK(session.clear_text_measurer() == NVE_RUNTIME_OK);
  CHECK(runtime_api()->set_text_measurer(nullptr, &measure_fixed, &ctx) == NVE_RUNTIME_INVALID_ARG);
  CHECK(ctx.calls == 0u);  // no text nodes: never invoked
  RuntimeSession empty;
  CHECK(empty.set_text_measurer(&measure_fixed, &ctx) == NVE_RUNTIME_INVALID_ARG);
}

bool same_bytes(std::span<const uint8_t> a, std::span<const uint8_t> b) {
  return a.size() == b.size() && (a.empty() || std::memcmp(a.data(), b.data(), a.size()) == 0);
}

void test_resource_payload() {
  RuntimeSession session = make_session();
  CHECK(session.submit_nve2(blob_frame(1u, kBlobId)) == NVE_RUNTIME_OK);

  // Raw slot without an active `scene_acquire`: INVALID_ARG, out slots untouched.
  uint64_t kind = 99u;
  NveBufferSlice payload{nullptr, 0u};
  CHECK(session.api()->resource_payload(session.handle(), kBlobId, &kind, &payload) ==
        NVE_RUNTIME_INVALID_ARG);
  CHECK(kind == 99u && payload.ptr == nullptr);
  {
    std::optional<RuntimeSceneGuard> guard = session.acquire_scene();
    CHECK(guard.has_value());
    NveRuntimeStatus st = NVE_RUNTIME_ERROR;
    const std::optional<RuntimeResource> blob = guard->resource(kBlobId, &st);
    CHECK(st == NVE_RUNTIME_OK && blob.has_value());
    if (blob) {
      CHECK(blob->kind == kBlobKind);  // opaque module kind survives byte-exact
      CHECK(same_bytes(blob->bytes, kBlobBytes));
    }
    CHECK(!guard->resource(0u, &st).has_value() && st == NVE_RUNTIME_NOT_FOUND);
    CHECK(!guard->resource(kBlobId + 1000u, &st).has_value() && st == NVE_RUNTIME_NOT_FOUND);
    // Kind slot is optional; the payload slot is required.
    CHECK(session.api()->resource_payload(session.handle(), kBlobId, nullptr, &payload) == NVE_RUNTIME_OK);
    CHECK(same_bytes(nve::host::buffer_span(payload), kBlobBytes));
    CHECK(session.api()->resource_payload(session.handle(), kBlobId, &kind, nullptr) ==
          NVE_RUNTIME_INVALID_ARG);

    RuntimeSceneGuard moved = std::move(*guard);
    CHECK(!guard->resource(kBlobId, &st).has_value() && st == NVE_RUNTIME_INVALID_ARG);
    CHECK(moved.resource(kBlobId).has_value());
  }

  // Dropping the last reference frees the id at frame end: stale ids report NOT_FOUND.
  CHECK(session.submit_nve2(blob_frame(2u, 0u)) == NVE_RUNTIME_OK);
  std::optional<RuntimeSceneGuard> guard = session.acquire_scene();
  CHECK(guard.has_value());
  NveRuntimeStatus st = NVE_RUNTIME_OK;
  CHECK(!guard->resource(kBlobId, &st).has_value() && st == NVE_RUNTIME_NOT_FOUND);
}

void test_app_hooks_without_app() {
  RuntimeSession session = make_session();
  bool changed = true;
  CHECK(session.set_props(nullptr, 0u, &changed) == NVE_RUNTIME_UNSUPPORTED);
  CHECK(!changed);
  CHECK(session.set_props(nullptr, 2u) == NVE_RUNTIME_INVALID_ARG);
  uint32_t raw_changed = 5u;
  CHECK(session.api()->set_props(session.handle(), nullptr, 2u, &raw_changed) == NVE_RUNTIME_INVALID_ARG);

  NveEventBatch events{reinterpret_cast<const NveDirectEvent*>(&session), 3u, 0u};
  CHECK(session.drain_emitted_events(&events) == NVE_RUNTIME_UNSUPPORTED);
  CHECK(events.ptr == nullptr && events.len == 0u);
  CHECK(session.drain_emitted_events(nullptr) == NVE_RUNTIME_INVALID_ARG);
  CHECK(session.api()->drain_emitted_events(session.handle(), nullptr) == NVE_RUNTIME_INVALID_ARG);

  std::span<const uint8_t> state = kBlobBytes;
  CHECK(session.snapshot_state(&state) == NVE_RUNTIME_UNSUPPORTED);
  CHECK(state.empty());
  CHECK(session.snapshot_state(nullptr) == NVE_RUNTIME_INVALID_ARG);
  CHECK(session.api()->snapshot_state(session.handle(), nullptr) == NVE_RUNTIME_INVALID_ARG);
  CHECK(session.restore_state(kBlobBytes) == NVE_RUNTIME_UNSUPPORTED);
  CHECK(session.restore_state({}) == NVE_RUNTIME_UNSUPPORTED);

  // NVP1 framing is validated before the app capability.
  CHECK(session.apply_hot_patch(kBlobBytes, &changed) == NVE_RUNTIME_INVALID_ARG);
  CHECK(session.apply_hot_patch({}) == NVE_RUNTIME_INVALID_ARG);
  CHECK(!session.last_error().empty());
  CHECK(session.api()->restore_state(session.handle(), NveBufferSlice{nullptr, 4u}) ==
        NVE_RUNTIME_INVALID_ARG);

  RuntimeSession empty;
  CHECK(empty.set_props(nullptr, 0u) == NVE_RUNTIME_INVALID_ARG);
  CHECK(empty.drain_emitted_events(&events) == NVE_RUNTIME_INVALID_ARG);
  CHECK(empty.snapshot_state(&state) == NVE_RUNTIME_INVALID_ARG);
  CHECK(empty.restore_state({}) == NVE_RUNTIME_INVALID_ARG);
  CHECK(empty.apply_hot_patch({}) == NVE_RUNTIME_INVALID_ARG);
}

void test_scroll_offsets() {
  // Live runtime without scroll containers publishes none.
  RuntimeSession session = make_session();
  CHECK(session.submit_nve2(blob_frame(1u, kBlobId)) == NVE_RUNTIME_OK);
  {
    std::optional<RuntimeSceneGuard> guard = session.acquire_scene();
    CHECK(guard.has_value() && guard->raw().scroll_count == 0u && guard->raw().scroll_offsets == nullptr);
    CHECK(guard->reader().scroll_offsets().empty());
  }

  // Synthetic view: sorted lookup, absent containers are zero, NULL slab with a count is rejected.
  const std::array<NveScrollOffset, 2> offsets{NveScrollOffset{2u, 0.0f, 40.0f},
                                               NveScrollOffset{5u, 3.0f, 0.0f}};
  NveRuntimeSceneView view{};
  view.schema_hash = NVE_RUNTIME_SCHEMA_HASH;
  view.scroll_count = static_cast<uint32_t>(offsets.size());
  view.scroll_offsets = offsets.data();
  const std::optional<RuntimeSceneReader> reader = RuntimeSceneReader::open(&view);
  CHECK(reader.has_value());
  if (reader) {
    CHECK(reader->scroll_offsets().size() == 2u);
    CHECK(reader->scroll_offset(2u).y == 40.0f);
    CHECK(reader->scroll_offset(5u).x == 3.0f);
    const NveScrollOffset none = reader->scroll_offset(4u);
    CHECK(none.node_id == 4u && none.x == 0.0f && none.y == 0.0f);
    CHECK(reader->scroll_offset(9u).y == 0.0f);
  }
  // Unsorted or duplicate ids would break the binary search: rejected on open.
  SceneError err = SceneError::None;
  const std::array<NveScrollOffset, 2> unsorted{NveScrollOffset{5u, 0.0f, 1.0f},
                                                NveScrollOffset{2u, 0.0f, 2.0f}};
  view.scroll_offsets = unsorted.data();
  CHECK(!RuntimeSceneReader::open(&view, Key64{NVE_RUNTIME_SCHEMA_HASH}, &err).has_value());
  CHECK(err == SceneError::UnsortedScrollOffsets);
  const std::array<NveScrollOffset, 2> duplicate{NveScrollOffset{3u, 0.0f, 1.0f},
                                                 NveScrollOffset{3u, 0.0f, 2.0f}};
  view.scroll_offsets = duplicate.data();
  err = SceneError::None;
  CHECK(!RuntimeSceneReader::open(&view, Key64{NVE_RUNTIME_SCHEMA_HASH}, &err).has_value());
  CHECK(err == SceneError::UnsortedScrollOffsets);
  const NveScrollOffset single{7u, 1.0f, 0.0f};
  view.scroll_offsets = &single;
  view.scroll_count = 1u;
  CHECK(RuntimeSceneReader::open(&view).has_value());
  view.scroll_count = 2u;

  view.scroll_offsets = nullptr;
  err = SceneError::None;
  CHECK(!RuntimeSceneReader::open(&view, Key64{NVE_RUNTIME_SCHEMA_HASH}, &err).has_value());
  CHECK(err == SceneError::NullSlab);
}

std::vector<uint32_t> dirty_resources_of(RuntimeSession& session, uint32_t* epoch) {
  std::optional<RuntimeSceneGuard> guard = session.acquire_scene();
  CHECK(guard.has_value());
  if (!guard) return {};
  *epoch = guard->reader().scene_epoch();
  CHECK(guard->reader().scene_epoch() == guard->raw().scene_epoch);
  const std::span<const uint32_t> ids = guard->reader().dirty_resources();
  return {ids.begin(), ids.end()};
}

void test_scene_epoch_and_dirty_resources() {
  RuntimeSession session = make_session();
  uint32_t e0 = 0u;
  CHECK(session.submit_nve2(blob_frame(1u, kBlobId)) == NVE_RUNTIME_OK);
  CHECK(dirty_resources_of(session, &e0) == std::vector<uint32_t>{kBlobId});

  // A delta that only re-points the row defines no resource: nothing dirty, same epoch.
  uint32_t e1 = 0u;
  CHECK(session.submit_nve2(blob_frame(2u, kBlobId)) == NVE_RUNTIME_OK);
  CHECK(dirty_resources_of(session, &e1).empty());
  CHECK(e1 == e0);

  // A new session's keyframe over the live replica resets the scene: new epoch, ids redefined.
  uint32_t e2 = 0u;
  CHECK(session.submit_nve2(blob_frame(1u, kBlobId)) == NVE_RUNTIME_OK);
  CHECK(dirty_resources_of(session, &e2) == std::vector<uint32_t>{kBlobId});
  CHECK(e2 == e0 + 1u);

  // A rejected frame keeps the scene and its epoch.
  const std::array<uint8_t, 4> junk{1u, 2u, 3u, 4u};
  CHECK(session.submit_nve2(junk) != NVE_RUNTIME_OK);
  uint32_t e3 = 0u;
  (void)dirty_resources_of(session, &e3);
  CHECK(e3 == e2);

  // Synthetic views: unsorted / duplicate ids are rejected, ascending ones are served.
  NveRuntimeSceneView view{};
  view.schema_hash = NVE_RUNTIME_SCHEMA_HASH;
  view.scene_epoch = 42u;
  const std::array<uint32_t, 3> sorted{2u, 5u, 9u};
  view.dirty_resources = sorted.data();
  view.dirty_resource_count = static_cast<uint32_t>(sorted.size());
  const std::optional<RuntimeSceneReader> reader = RuntimeSceneReader::open(&view);
  CHECK(reader.has_value() && reader->scene_epoch() == 42u && reader->dirty_resources().size() == 3u);
  SceneError err = SceneError::None;
  const std::array<uint32_t, 2> unsorted{5u, 2u};
  view.dirty_resources = unsorted.data();
  view.dirty_resource_count = 2u;
  CHECK(!RuntimeSceneReader::open(&view, Key64{NVE_RUNTIME_SCHEMA_HASH}, &err).has_value());
  CHECK(err == SceneError::UnsortedDirtyResources);
  const std::array<uint32_t, 2> duplicate{3u, 3u};
  view.dirty_resources = duplicate.data();
  err = SceneError::None;
  CHECK(!RuntimeSceneReader::open(&view, Key64{NVE_RUNTIME_SCHEMA_HASH}, &err).has_value());
  CHECK(err == SceneError::UnsortedDirtyResources);
  view.dirty_resources = nullptr;
  err = SceneError::None;
  CHECK(!RuntimeSceneReader::open(&view, Key64{NVE_RUNTIME_SCHEMA_HASH}, &err).has_value());
  CHECK(err == SceneError::NullSlab);
}

}  // namespace

int main() {
  test_vtable_complete_and_validated();
  test_text_measurer();
  test_resource_payload();
  test_app_hooks_without_app();
  test_scroll_offsets();
  test_scene_epoch_and_dirty_resources();
  if (g_failures != 0) {
    std::fprintf(stderr, "test_runtime_hooks: %d failure(s)\n", g_failures);
    return 1;
  }
  std::printf("test_runtime_hooks: OK\n");
  return 0;
}
