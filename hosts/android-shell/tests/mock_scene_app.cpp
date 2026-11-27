#include <nve/generated/nve_abi.generated.h>
#include "nve_rn_component.hpp"

#include <cmath>
#include <cstdint>
#include <cstring>
#include <limits>
#include <string>
#include <string_view>
#include <vector>

namespace {

constexpr const char* kAvatarPng =
    "data:image/png;base64,"
    "iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAJElEQVR42mOQ07T+jw8zgAgerzysGEWBxd4fKHggFOB1JD4MAKjRfBnTmv6zAAAAAElFTkSuQmCC";

struct MockSceneApp {
  uint32_t surface_id{1};
  uint32_t press_count{0};
  uint32_t scroll_count{0};
  float last_scroll_x{0.0f};
  float last_scroll_y{0.0f};
  uint32_t submit_count{0};
  std::string last_submit;
  uint32_t keydown_count{0};
  std::string last_keydown;
  uint32_t props_count{0};
  std::string last_props_summary;
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

void push_set_raw_str(
    std::vector<uint8_t>& buf,
    uint32_t id,
    std::string_view key,
    std::string_view val) {
  push_le<uint8_t>(buf, NVE_OP_SET_RAW_PROP);
  push_le<uint32_t>(buf, id);
  push_le<uint16_t>(buf, static_cast<uint16_t>(key.size()));
  buf.insert(buf.end(), key.begin(), key.end());
  push_le<uint8_t>(buf, 4); // string tag
  push_le<uint32_t>(buf, static_cast<uint32_t>(val.size()));
  buf.insert(buf.end(), val.begin(), val.end());
}

void push_set_raw_bool(
    std::vector<uint8_t>& buf,
    uint32_t id,
    std::string_view key,
    bool val) {
  push_le<uint8_t>(buf, NVE_OP_SET_RAW_PROP);
  push_le<uint32_t>(buf, id);
  push_le<uint16_t>(buf, static_cast<uint16_t>(key.size()));
  buf.insert(buf.end(), key.begin(), key.end());
  push_le<uint8_t>(buf, 1); // bool tag
  push_le<uint8_t>(buf, val ? 1u : 0u);
}

void push_set_raw_bytes(
    std::vector<uint8_t>& buf,
    uint32_t id,
    std::string_view key,
    const std::vector<uint8_t>& val) {
  push_le<uint8_t>(buf, NVE_OP_SET_RAW_PROP);
  push_le<uint32_t>(buf, id);
  push_le<uint16_t>(buf, static_cast<uint16_t>(key.size()));
  buf.insert(buf.end(), key.begin(), key.end());
  push_le<uint8_t>(buf, 5); // bytes tag
  push_le<uint32_t>(buf, static_cast<uint32_t>(val.size()));
  buf.insert(buf.end(), val.begin(), val.end());
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

uint32_t rgba_le(uint8_t r, uint8_t g, uint8_t b, uint8_t a) {
  return static_cast<uint32_t>(r) |
         (static_cast<uint32_t>(g) << 8) |
         (static_cast<uint32_t>(b) << 16) |
         (static_cast<uint32_t>(a) << 24);
}

std::vector<uint8_t> build_valid_draw1_all_ops() {
  std::vector<uint8_t> d;
  const char magic[9] = {'N', 'V', 'E', '_', 'D', 'R', 'A', 'W', '1'};
  d.insert(d.end(), magic, magic + 9);
  push_le<uint16_t>(d, 1u); // version
  push_le<uint32_t>(d, 8u); // op_count

  // 1: Clear
  push_le<uint8_t>(d, 1u);
  push_le<uint32_t>(d, rgba_le(15, 23, 42, 255));
  // 2: FillRect
  push_le<uint8_t>(d, 2u);
  push_le<float>(d, 4.0f);
  push_le<float>(d, 6.0f);
  push_le<float>(d, 40.0f);
  push_le<float>(d, 20.0f);
  push_le<uint32_t>(d, rgba_le(56, 189, 248, 255));
  // 3: FillRoundedRect
  push_le<uint8_t>(d, 3u);
  push_le<float>(d, 10.0f);
  push_le<float>(d, 12.0f);
  push_le<float>(d, 30.0f);
  push_le<float>(d, 14.0f);
  push_le<float>(d, 4.0f);
  push_le<uint32_t>(d, rgba_le(34, 197, 94, 255));
  // 4: FillCircle
  push_le<uint8_t>(d, 4u);
  push_le<float>(d, 25.0f);
  push_le<float>(d, 25.0f);
  push_le<float>(d, 5.0f);
  push_le<uint32_t>(d, rgba_le(250, 204, 21, 255));
  // 5: StrokeLine
  push_le<uint8_t>(d, 5u);
  push_le<float>(d, 0.0f);
  push_le<float>(d, 0.0f);
  push_le<float>(d, 50.0f);
  push_le<float>(d, 20.0f);
  push_le<float>(d, 2.0f);
  push_le<uint32_t>(d, rgba_le(248, 113, 113, 255));
  // 6: ClipRect
  push_le<uint8_t>(d, 7u);
  push_le<float>(d, 2.0f);
  push_le<float>(d, 2.0f);
  push_le<float>(d, 60.0f);
  push_le<float>(d, 40.0f);
  // 7: StrokePolyline (3 points)
  push_le<uint8_t>(d, 6u);
  push_le<float>(d, 1.5f);
  push_le<uint32_t>(d, rgba_le(168, 85, 247, 255));
  push_le<uint32_t>(d, 3u);
  push_le<float>(d, 2.0f); push_le<float>(d, 10.0f);
  push_le<float>(d, 20.0f); push_le<float>(d, 4.0f);
  push_le<float>(d, 40.0f); push_le<float>(d, 18.0f);
  // 8: RestoreClip
  push_le<uint8_t>(d, 8u);
  return d;
}

std::vector<uint8_t> build_corrupt_draw1_nan() {
  std::vector<uint8_t> d;
  const char magic[9] = {'N', 'V', 'E', '_', 'D', 'R', 'A', 'W', '1'};
  d.insert(d.end(), magic, magic + 9);
  push_le<uint16_t>(d, 1u);
  push_le<uint32_t>(d, 1u);
  push_le<uint8_t>(d, 2u); // FillRect with NaN x
  push_le<float>(d, std::numeric_limits<float>::quiet_NaN());
  push_le<float>(d, 0.0f);
  push_le<float>(d, 10.0f);
  push_le<float>(d, 10.0f);
  push_le<uint32_t>(d, rgba_le(255, 0, 0, 255));
  return d;
}

std::string encode_base64_attr(const std::vector<uint8_t>& bytes) {
  static constexpr char kAlphabet[] =
      "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  std::string out = "nve-draw1;b64,";
  size_t i = 0;
  while (i + 3 <= bytes.size()) {
    const uint32_t n = (static_cast<uint32_t>(bytes[i]) << 16) |
                       (static_cast<uint32_t>(bytes[i + 1]) << 8) |
                       static_cast<uint32_t>(bytes[i + 2]);
    out.push_back(kAlphabet[(n >> 18) & 0x3F]);
    out.push_back(kAlphabet[(n >> 12) & 0x3F]);
    out.push_back(kAlphabet[(n >> 6) & 0x3F]);
    out.push_back(kAlphabet[n & 0x3F]);
    i += 3;
  }
  const size_t rem = bytes.size() - i;
  if (rem == 1) {
    const uint32_t n = static_cast<uint32_t>(bytes[i]) << 16;
    out.push_back(kAlphabet[(n >> 18) & 0x3F]);
    out.push_back(kAlphabet[(n >> 12) & 0x3F]);
    out.push_back('=');
    out.push_back('=');
  } else if (rem == 2) {
    const uint32_t n =
        (static_cast<uint32_t>(bytes[i]) << 16) | (static_cast<uint32_t>(bytes[i + 1]) << 8);
    out.push_back(kAlphabet[(n >> 18) & 0x3F]);
    out.push_back(kAlphabet[(n >> 12) & 0x3F]);
    out.push_back(kAlphabet[(n >> 6) & 0x3F]);
    out.push_back('=');
  }
  return out;
}

std::vector<uint8_t> build_frame(const MockSceneApp& app, uint64_t frame_id) {
  std::vector<uint8_t> ops;
  uint32_t op_count = 0;

  if (frame_id == 1) {
    push_create_node(ops, 1, NVE_COMP_ROOT); ++op_count;
    push_create_node(ops, 2, NVE_COMP_TEXT); ++op_count;
    push_set_text(ops, 2, "Count: 0 🚀"); ++op_count;
    push_create_node(ops, 3, NVE_COMP_PRESSABLE); ++op_count;
    push_set_event_mask(ops, 3, 1ULL); ++op_count;
    push_set_raw_str(ops, 3, "accessibilityLabel", "Increment counter"); ++op_count;
    push_set_raw_str(ops, 3, "accessibilityRole", "button"); ++op_count;
    push_set_raw_str(ops, 3, "accessibilityHint", "Adds one"); ++op_count;
    push_set_raw_bool(ops, 3, "disabled", false); ++op_count;
    push_create_node(ops, 4, NVE_COMP_TEXT); ++op_count;
    push_set_text(ops, 4, "Temporary Banner"); ++op_count;
    push_create_node(ops, 5, NVE_COMP_IMAGE); ++op_count;
    push_set_raw_str(ops, 5, "src", kAvatarPng); ++op_count;
    push_set_raw_str(ops, 5, "resizeMode", "contain"); ++op_count;
    push_create_node(ops, 6, NVE_COMP_SCROLL_VIEW); ++op_count;
    push_create_node(ops, 7, NVE_COMP_TEXT_INPUT); ++op_count;
    push_set_raw_str(ops, 7, "placeholder", "Enter secret"); ++op_count;
    push_set_raw_bool(ops, 7, "secureTextEntry", true); ++op_count;
    push_set_raw_str(ops, 7, "accessibilityLabel", "Secret input"); ++op_count;
    push_create_node(ops, 8, NVE_COMP_SKIA_CANVAS); ++op_count;
    push_set_raw_bytes(ops, 8, "drawList", build_valid_draw1_all_ops()); ++op_count;
    push_set_children(ops, 1, {2, 3, 4, 5, 6, 7, 8}); ++op_count;
  } else if (app.props_count > 0) {
    push_set_text(ops, 2, app.last_props_summary); ++op_count;
  } else if (app.keydown_count > 0) {
    if (app.last_keydown == "Cmd+k") {
      // Raw-prop-only corrupt drawList update to verify safety and zero-crash degradation.
      push_set_raw_bytes(ops, 8, "drawList", build_corrupt_draw1_nan()); ++op_count;
      push_set_text(ops, 2, "Key: Cmd+k"); ++op_count;
    } else if (app.last_keydown == "Escape") {
      // Base64 prefixed `nve-draw1;b64,...` recovery update.
      push_set_raw_str(ops, 8, "drawList", encode_base64_attr(build_valid_draw1_all_ops())); ++op_count;
      push_set_text(ops, 2, "Key: Escape"); ++op_count;
    }
  } else if (app.submit_count > 0) {
    // Mutate accessibility & TextInput raw props on nodes 3 & 7 ONLY (node 5 & 8 unchanged).
    push_set_raw_bool(ops, 3, "disabled", true); ++op_count;
    push_set_raw_str(ops, 3, "accessibilityLabel", "Counter locked"); ++op_count;
    push_set_raw_str(ops, 7, "placeholder", "Submitted"); ++op_count;
    push_set_raw_bool(ops, 7, "secureTextEntry", false); ++op_count;
    push_set_text(ops, 2, "Submit: " + app.last_submit); ++op_count;
  } else if (app.scroll_count > 0) {
    const std::string label =
        "Scroll: " + std::to_string(static_cast<int>(std::lround(app.last_scroll_x))) + "," +
        std::to_string(static_cast<int>(std::lround(app.last_scroll_y)));
    push_set_text(ops, 2, label); ++op_count;
  } else if (app.press_count == 1) {
    push_set_text(ops, 2, "Count: 1 🚀"); ++op_count;
  } else {
    push_set_text(ops, 2, "Count: " + std::to_string(app.press_count)); ++op_count;
    push_set_children(ops, 1, {2, 3, 5, 6, 7, 8}); ++op_count;
    push_remove_node(ops, 4); ++op_count;
  }

  push_le<uint8_t>(ops, NVE_OP_COMMIT);
  push_le<uint8_t>(ops, 1);
  ++op_count;

  std::vector<uint8_t> buf;
  buf.reserve(24 + ops.size());
  push_le<uint32_t>(buf, static_cast<uint32_t>(NVE_ABI_MAGIC));
  push_le<uint16_t>(buf, static_cast<uint16_t>(NVE_ABI_VERSION));
  push_le<uint16_t>(buf, 0u);
  push_le<uint32_t>(buf, app.surface_id);
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
  app->frame = build_frame(*app, frame_id);
  return NveBufferSlice{app->frame.data(), app->frame.size()};
}

std::string extract_event_payload_str(const NveDirectEvent& ev) {
  if (ev.value.tag == NVE_VAL_STR) {
    return std::string(nve::str_slice_view(ev.value.data.str_val));
  }
  if (ev.value.tag == NVE_VAL_BYTES && ev.value.data.bytes_val.ptr != nullptr) {
    return std::string(
        reinterpret_cast<const char*>(ev.value.data.bytes_val.ptr),
        ev.value.data.bytes_val.len);
  }
  return {};
}

uint32_t scene_dispatch_events(NveAppHandle handle, NveEventBatch batch) {
  auto* app = static_cast<MockSceneApp*>(handle);
  if (batch.ptr == nullptr || batch.len == 0) {
    return 0;
  }
  uint32_t dirty = 0;
  for (uint32_t i = 0; i < batch.len; ++i) {
    const NveDirectEvent& ev = batch.ptr[i];
    const std::string_view ev_name = nve::str_slice_view(ev.name);
    if (ev.kind == static_cast<uint8_t>(nve::EventKind::Scroll) && ev.node_id == 6) {
      app->scroll_count += 1;
      app->last_scroll_x = ev.x;
      app->last_scroll_y = ev.y;
      dirty = 1;
    } else if (ev.kind == static_cast<uint8_t>(nve::EventKind::Custom) && ev_name == "submit") {
      app->submit_count += 1;
      app->last_submit = extract_event_payload_str(ev);
      dirty = 1;
    } else if (ev.kind == static_cast<uint8_t>(nve::EventKind::Submit)) {
      app->submit_count += 1;
      app->last_submit = std::string(nve::str_slice_view(ev.text));
      dirty = 1;
    } else if (ev.kind == static_cast<uint8_t>(nve::EventKind::Custom) && ev_name == "keydown") {
      app->keydown_count += 1;
      app->last_keydown = extract_event_payload_str(ev);
      dirty = 1;
    } else if (ev.kind == static_cast<uint8_t>(nve::EventKind::KeyDown)) {
      app->keydown_count += 1;
      app->last_keydown = std::string(nve::str_slice_view(ev.text));
      dirty = 1;
    } else if (ev.kind == static_cast<uint8_t>(nve::EventKind::Press)) {
      app->press_count += 1;
      dirty = 1;
    }
  }
  return dirty;
}

uint32_t scene_set_props(NveAppHandle handle, const NveMapEntry* entries, uint32_t count) {
  auto* app = static_cast<MockSceneApp*>(handle);
  if (!app || (count > 0u && entries == nullptr)) {
    return 0;
  }
  app->props_count += 1;
  std::string summary = "Props: count=" + std::to_string(count);
  if (count > 0u) {
    summary += " k0=";
    summary += nve::str_slice_view(entries[0].key);
    if (entries[0].value.tag == NVE_VAL_I64) {
      summary += " i64=" + std::to_string(entries[0].value.data.i64_val);
    } else if (entries[0].value.tag == NVE_VAL_BYTES) {
      summary += " blen=" + std::to_string(entries[0].value.data.bytes_val.len);
    } else if (entries[0].value.tag == NVE_VAL_STR) {
      summary += " slen=" + std::to_string(entries[0].value.data.str_val.len);
    }
  }
  app->last_props_summary = std::move(summary);
  return 1;
}

NveEventBatch scene_drain_emitted_events(NveAppHandle) {
  return NveEventBatch{nullptr, 0u, 0u};
}

} // namespace

extern "C" const void* nve_get_interface(uint32_t interface_id) {
  static const NveSurfaceApi kSurface{
      .create = &scene_create,
      .destroy = &scene_destroy,
      .render = &scene_render,
      .dispatch_events = &scene_dispatch_events,
  };
  static const NveComponentApi kComponent{
      .set_props = &scene_set_props,
      .drain_emitted_events = &scene_drain_emitted_events,
  };
  switch (interface_id) {
    case NVE_IFACE_SURFACE:
      return &kSurface;
    case NVE_IFACE_COMPONENT:
      return &kComponent;
    default:
      return nullptr;
  }
}
