#include "jni_test_recorder.h"
#include "nve_abi.h"

#include <cstring>
#include <vector>

namespace {

struct MockAppV1 {
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

NveAppHandle v1_create(uint32_t surface_id) {
  auto* log = nve_jni_test_log();
  log->v1_create_count += 1;
  log->v1_surface_id = surface_id;
  auto* app = new MockAppV1();
  app->surface_id = surface_id;
  return app;
}

void v1_destroy(NveAppHandle handle) {
  nve_jni_test_log()->v1_destroy_count += 1;
  delete static_cast<MockAppV1*>(handle);
}

NveBufferSlice v1_render(NveAppHandle handle, uint64_t frame_id) {
  auto* app = static_cast<MockAppV1*>(handle);
  nve_jni_test_log()->v1_last_frame_id = frame_id;
  app->frame = make_commit_frame(app->surface_id, frame_id);
  return NveBufferSlice{app->frame.data(), app->frame.size()};
}

uint32_t v1_dispatch_events(NveAppHandle, NveBufferSlice) {
  return 0;
}

uint32_t v1_apply_hot_patch(NveAppHandle, NveBufferSlice) {
  return 0;
}

NveBufferSlice v1_snapshot_state(NveAppHandle) {
  static constexpr char kSnapshot[] = "v1-counter=42";
  nve_jni_test_log()->v1_snapshot_count += 1;
  return NveBufferSlice{
      reinterpret_cast<const uint8_t*>(kSnapshot),
      sizeof(kSnapshot) - 1,
  };
}

void v1_restore_state(NveAppHandle, NveBufferSlice) {}

} // namespace

extern "C" const void* nve_get_interface(uint32_t interface_id) {
  static const NveSurfaceApi kSurface{
      .create = &v1_create,
      .destroy = &v1_destroy,
      .render = &v1_render,
      .dispatch_events = &v1_dispatch_events,
  };
  static const NveDevApi kDev{
      .apply_hot_patch = &v1_apply_hot_patch,
      .snapshot_state = &v1_snapshot_state,
      .restore_state = &v1_restore_state,
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
