#include "nve_abi.h"

#include <cstdint>
#include <cstring>
#include <string>
#include <vector>

namespace {

struct MockSceneApp {
  uint32_t surface_id{1};
  uint32_t press_count{0};
  std::vector<uint8_t> frame;
};

template <typename T>
void push_le(std::vector<uint8_t>& buf, T value) {
  const auto* p = reinterpret_cast<const uint8_t*>(&value);
  buf.insert(buf.end(), p, p + sizeof(T));
}

void push_create_node(std::vector<uint8_t>& buf, uint32_t id, uint16_t kind) {
  push_le<uint8_t>(buf, NVE_OP_CREATE_NODE);
  push_le<uint32_t>(buf, id);
  push_le<uint16_t>(buf, kind);
  push_le<uint16_t>(buf, 0); // custom_name len
}

void push_set_text(std::vector<uint8_t>& buf, uint32_t id, const std::string& text) {
  push_le<uint8_t>(buf, NVE_OP_SET_TEXT);
  push_le<uint32_t>(buf, id);
  push_le<uint32_t>(buf, static_cast<uint32_t>(text.size()));
  buf.insert(buf.end(), text.begin(), text.end());
}

void push_set_children(std::vector<uint8_t>& buf, uint32_t id, const std::vector<uint32_t>& kids) {
  push_le<uint8_t>(buf, NVE_OP_SET_CHILDREN);
  push_le<uint32_t>(buf, id);
  push_le<uint32_t>(buf, static_cast<uint32_t>(kids.size()));
  for (uint32_t c : kids) {
    push_le<uint32_t>(buf, c);
  }
}

void push_remove_node(std::vector<uint8_t>& buf, uint32_t id) {
  push_le<uint8_t>(buf, NVE_OP_REMOVE_NODE);
  push_le<uint32_t>(buf, id);
}

void push_set_event_mask(std::vector<uint8_t>& buf, uint32_t id, uint64_t mask) {
  push_le<uint8_t>(buf, NVE_OP_SET_EVENT_MASK);
  push_le<uint32_t>(buf, id);
  push_le<uint64_t>(buf, mask);
}

std::vector<uint8_t> build_frame(uint32_t surface_id, uint64_t frame_id, uint32_t press_count) {
  std::vector<uint8_t> ops;
  uint32_t op_count = 0;

  if (frame_id == 1) {
    push_create_node(ops, 1, NVE_COMP_ROOT);
    ++op_count;
    push_create_node(ops, 2, NVE_COMP_TEXT);
    ++op_count;
    push_set_text(ops, 2, "Count: 0 🚀");
    ++op_count;
    push_create_node(ops, 3, NVE_COMP_PRESSABLE);
    ++op_count;
    push_set_event_mask(ops, 3, 1ULL);
    ++op_count;
    push_create_node(ops, 4, NVE_COMP_TEXT);
    ++op_count;
    push_set_text(ops, 4, "Temporary Banner");
    ++op_count;
    push_set_children(ops, 1, {2, 3, 4});
    ++op_count;
  } else if (press_count == 1) {
    push_set_text(ops, 2, "Count: 1 🚀");
    ++op_count;
  } else {
    push_set_text(ops, 2, "Count: " + std::to_string(press_count));
    ++op_count;
    push_set_children(ops, 1, {2, 3});
    ++op_count;
    push_remove_node(ops, 4);
    ++op_count;
  }

  push_le<uint8_t>(ops, NVE_OP_COMMIT);
  push_le<uint8_t>(ops, 1);
  ++op_count;

  std::vector<uint8_t> buf;
  buf.reserve(24 + ops.size());
  push_le<uint32_t>(buf, static_cast<uint32_t>(NVE_ABI_MAGIC));
  push_le<uint16_t>(buf, static_cast<uint16_t>(NVE_ABI_VERSION));
  push_le<uint16_t>(buf, 0u);
  push_le<uint32_t>(buf, surface_id);
  push_le<uint64_t>(buf, frame_id);
  push_le<uint32_t>(buf, op_count);
  buf.insert(buf.end(), ops.begin(), ops.end());
  return buf;
}

NveAppHandle scene_create(uint32_t surface_id) {
  auto* app = new MockSceneApp();
  app->surface_id = surface_id;
  return app;
}

void scene_destroy(NveAppHandle handle) {
  delete static_cast<MockSceneApp*>(handle);
}

NveBufferSlice scene_render(NveAppHandle handle, uint64_t frame_id) {
  auto* app = static_cast<MockSceneApp*>(handle);
  app->frame = build_frame(app->surface_id, frame_id, app->press_count);
  return NveBufferSlice{app->frame.data(), app->frame.size()};
}

uint32_t scene_dispatch_events(NveAppHandle handle, NveBufferSlice slice) {
  auto* app = static_cast<MockSceneApp*>(handle);
  if (slice.ptr != nullptr && slice.len > 0) {
    app->press_count += 1;
    return 1;
  }
  return 0;
}

} // namespace

extern "C" const void* nve_get_interface(uint32_t interface_id) {
  static const NveSurfaceApi kSurface{
      .create = &scene_create,
      .destroy = &scene_destroy,
      .render = &scene_render,
      .dispatch_events = &scene_dispatch_events,
      .dispatch_events_direct = nullptr,
  };
  return interface_id == NVE_IFACE_SURFACE ? &kSurface : nullptr;
}
