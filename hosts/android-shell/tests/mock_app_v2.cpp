#include "jni_test_recorder.h"
#include "nve_abi.generated.h"

#include <algorithm>
#include <cstring>
#include <vector>

namespace {

struct MockAppV2 {
  uint32_t surface_id{0};
  std::vector<uint8_t> frame;
};

std::vector<uint8_t> make_commit_frame(uint32_t surface_id, uint64_t frame_id) {
  std::vector<uint8_t> buf;
  auto push = [&](auto value) {
    const auto* p = reinterpret_cast<const uint8_t*>(&value);
    buf.insert(buf.end(), p, p + sizeof(value));
  };
  push(static_cast<uint32_t>(NVE_ABI_MAGIC));
  push(static_cast<uint16_t>(NVE_ABI_VERSION));
  push(static_cast<uint16_t>(0));
  push(surface_id);
  push(frame_id);
  push(static_cast<uint32_t>(1)); // op_count = 1 (COMMIT)
  push(static_cast<uint8_t>(NVE_OP_COMMIT));
  push(static_cast<uint8_t>(1));
  return buf;
}

NveAppHandle v2_create(uint32_t surface_id) {
  auto* log = nve_jni_test_log();
  log->v2_create_count += 1;
  log->v2_surface_id = surface_id;
  auto* app = new MockAppV2();
  app->surface_id = surface_id;
  return app;
}

void v2_destroy(NveAppHandle handle) {
  nve_jni_test_log()->v2_destroy_count += 1;
  delete static_cast<MockAppV2*>(handle);
}

NveBufferSlice v2_render(NveAppHandle handle, uint64_t frame_id) {
  auto* app = static_cast<MockAppV2*>(handle);
  nve_jni_test_log()->v2_last_frame_id = frame_id;
  app->frame = make_commit_frame(app->surface_id, frame_id);
  return NveBufferSlice{app->frame.data(), app->frame.size()};
}

uint32_t v2_dispatch_events(NveAppHandle, NveBufferSlice) {
  return 0;
}

uint32_t v2_apply_hot_patch(NveAppHandle, NveBufferSlice) {
  return 0;
}

NveBufferSlice v2_snapshot_state(NveAppHandle) {
  return NveBufferSlice{nullptr, 0};
}

void v2_restore_state(NveAppHandle, NveBufferSlice state) {
  auto* log = nve_jni_test_log();
  log->v2_restore_count += 1;
  const size_t copy_len = std::min(state.len, sizeof(log->v2_restored_state) - 1);
  if (state.ptr && copy_len > 0) {
    std::memcpy(log->v2_restored_state, state.ptr, copy_len);
  }
  log->v2_restored_state[copy_len] = '\0';
  log->v2_restored_len = copy_len;
}

} // namespace

extern "C" const void* nve_get_interface(uint32_t interface_id) {
  static const NveSurfaceApi kSurface{
      .create = &v2_create,
      .destroy = &v2_destroy,
      .render = &v2_render,
      .dispatch_events = &v2_dispatch_events,
      .dispatch_events_direct = nullptr,
  };
  static const NveDevApi kDev{
      .apply_hot_patch = &v2_apply_hot_patch,
      .snapshot_state = &v2_snapshot_state,
      .restore_state = &v2_restore_state,
  };
  switch (interface_id) {
    case NVE_IFACE_SURFACE:
      return &kSurface;
    case NVE_IFACE_DEV:
      return &kDev;
    default:
      return nullptr;
  }
}
