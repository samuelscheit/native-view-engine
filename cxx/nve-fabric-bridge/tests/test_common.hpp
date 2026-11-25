#pragma once

#include "command_decoder.hpp"
#include "command_encoder.hpp"
#include "mock_jsi_recorder.hpp"
#include "nve_abi.generated.h"
#include "nve_host_runtime.hpp"
#include "nve_rn_component.hpp"
#include "nve_scene_builder.hpp"

#include <chrono>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <limits>
#include <random>
#include <string>
#include <unordered_map>
#include <vector>

#define NVE_CHECK(expr)                                                                         \
  do {                                                                                          \
    if (!(expr)) {                                                                              \
      std::cerr << "NVE_CHECK failed at " << __FILE__ << ":" << __LINE__ << ": " #expr << "\n"; \
      std::abort();                                                                             \
    }                                                                                           \
  } while (0)

#undef assert
#define assert(expr) NVE_CHECK(expr)

namespace nve_test {

using nve::CommandBufferEncoder;

inline bool approx_eq(float a, float b, float eps = 0.5f) {
  return std::fabs(a - b) <= eps;
}

inline std::unordered_map<uint32_t, nve::RenderedNodeSnapshot> snapshot_by_id(
    NveFabricHostHandle host) {
  std::unordered_map<uint32_t, nve::RenderedNodeSnapshot> map;
  for (auto& snap : nve::fabric_host_snapshot_nodes(host)) {
    map.emplace(snap.node_id, std::move(snap));
  }
  return map;
}

struct MockEmbeddedRustComponent {
  uint32_t surface_id{1};
  std::string last_props_json;
  std::string last_direct_title;
  int64_t last_direct_initial_count{0};
  bool last_direct_enabled{false};
  double last_direct_ratio{0.0};
  bool saw_direct_null{false};
  uint32_t direct_props_calls{0};
  uint32_t direct_event_calls{0};
  std::string last_direct_event_name;
  int count{0};
  std::vector<uint8_t> last_cmd_buf;
  nve::EventRingWriter outbound_events;
  std::vector<uint8_t> outbound_snapshot;
  std::string saved_state_buf;

  static NveAppHandle create(uint32_t sid) {
    auto* self = new MockEmbeddedRustComponent();
    self->surface_id = sid;
    self->outbound_events.clear();
    return self;
  }

  static void destroy(NveAppHandle h) {
    delete static_cast<MockEmbeddedRustComponent*>(h);
  }

  static NveBufferSlice render(NveAppHandle h, uint64_t frame_id) {
    auto* self = static_cast<MockEmbeddedRustComponent*>(h);
    CommandBufferEncoder enc(self->surface_id, frame_id);
    enc.create_node(2, NVE_COMP_PRESSABLE);
    enc.create_node(3, NVE_COMP_TEXT);
    enc.set_children(NVE_ROOT_NODE_ID, {2});
    enc.set_children(2, {3});
    enc.set_text(3, "Count: " + std::to_string(self->count));
    nve::LayoutProps lp{};
    lp.width = {1, 200.0f};
    lp.height = {1, 60.0f};
    enc.set_layout(2, lp);
    enc.commit(true);
    auto slice = enc.finish();
    self->last_cmd_buf.assign(slice.ptr, slice.ptr + slice.len);
    return NveBufferSlice{self->last_cmd_buf.data(), self->last_cmd_buf.size()};
  }

  static uint32_t dispatch_events(NveAppHandle h, NveBufferSlice events) {
    auto* self = static_cast<MockEmbeddedRustComponent*>(h);
    std::vector<nve::DecodedEvent> decoded;
    if (!nve::decode_event_ring(events, decoded)) return 0;
    uint32_t dirty = 0;
    for (const auto& ev : decoded) {
      if (ev.kind == nve::EventKind::Press && ev.node_id == 2) {
        self->count += 1;
        const NveMapEntry entry{
            nve::make_str_slice("count"),
            nve::make_i64_value(static_cast<int64_t>(self->count)),
        };
        const NveValue payload = nve::make_map_value(&entry, 1);
        self->outbound_events.push_custom_value(
            self->surface_id, 2, ev.timestamp_us, "onIncrement", payload);
        dirty = 1;
      }
    }
    return dirty;
  }

  static uint32_t dispatch_events_direct(NveAppHandle h, NveEventBatch batch) {
    auto* self = static_cast<MockEmbeddedRustComponent*>(h);
    if (!self || !batch.ptr || batch.len == 0) return 0;
    uint32_t dirty = 0;
    for (uint32_t i = 0; i < batch.len; ++i) {
      const NveDirectEvent& ev = batch.ptr[i];
      ++self->direct_event_calls;
      if (ev.kind == 1 && ev.node_id == 2) {
        self->count += 1;
        dirty = 1;
      } else if (ev.kind == 8) {
        self->last_direct_event_name = std::string(nve::str_slice_view(ev.name));
        dirty = 1;
      }
    }
    return dirty;
  }

  static uint32_t apply_hot_patch(NveAppHandle h, NveBufferSlice patch) {
    auto* self = static_cast<MockEmbeddedRustComponent*>(h);
    self->last_props_json.assign(
        reinterpret_cast<const char*>(patch.ptr), patch.len);
    return 1;
  }

  static NveBufferSlice snapshot_state(NveAppHandle h) {
    auto* self = static_cast<MockEmbeddedRustComponent*>(h);
    self->saved_state_buf = std::to_string(self->count);
    return NveBufferSlice{
        reinterpret_cast<const uint8_t*>(self->saved_state_buf.data()),
        self->saved_state_buf.size()};
  }

  static void restore_state(NveAppHandle h, NveBufferSlice state) {
    auto* self = static_cast<MockEmbeddedRustComponent*>(h);
    std::string s(reinterpret_cast<const char*>(state.ptr), state.len);
    self->count = std::stoi(s);
  }

  static uint32_t set_props(NveAppHandle h, NveBufferSlice props) {
    auto* self = static_cast<MockEmbeddedRustComponent*>(h);
    self->last_props_json.assign(reinterpret_cast<const char*>(props.ptr), props.len);
    const NveMapEntry entry{nve::make_str_slice("accepted"), nve::make_bool_value(true)};
    const NveValue payload = nve::make_map_value(&entry, 1);
    self->outbound_events.push_custom_value(
        self->surface_id, NVE_ROOT_NODE_ID, 1000, "onPropsApplied", payload);
    return 1;
  }

  static uint32_t set_props_direct(NveAppHandle h, const NveMapEntry* entries, uint32_t count) {
    auto* self = static_cast<MockEmbeddedRustComponent*>(h);
    ++self->direct_props_calls;
    for (uint32_t i = 0; i < count; ++i) {
      const std::string_view k = nve::str_slice_view(entries[i].key);
      const NveValue& v = entries[i].value;
      if (k == "title" && v.tag == NVE_VAL_STR) {
        self->last_direct_title = std::string(nve::str_slice_view(v.data.str_val));
      } else if (k == "initialCount" && v.tag == NVE_VAL_I64) {
        self->last_direct_initial_count = v.data.i64_val;
      } else if (k == "enabled" && v.tag == NVE_VAL_BOOL) {
        self->last_direct_enabled = v.data.bool_val != 0;
      } else if (k == "ratio" && v.tag == NVE_VAL_F64) {
        self->last_direct_ratio = v.data.f64_val;
      } else if (k == "optional" && v.tag == NVE_VAL_NULL) {
        self->saw_direct_null = true;
      }
    }
    const NveMapEntry entry{nve::make_str_slice("accepted"), nve::make_bool_value(true)};
    const NveValue payload = nve::make_map_value(&entry, 1);
    self->outbound_events.push_custom_value(
        self->surface_id, NVE_ROOT_NODE_ID, 1000, "onPropsApplied", payload);
    return 1;
  }

  static NveBufferSlice drain_emitted_events(NveAppHandle h) {
    auto* self = static_cast<MockEmbeddedRustComponent*>(h);
    self->outbound_snapshot = self->outbound_events.finalize();
    self->outbound_events.clear();
    return NveBufferSlice{self->outbound_snapshot.data(), self->outbound_snapshot.size()};
  }

  static NveIntrinsicSize measure(NveAppHandle, NveSizeConstraints c) {
    return NveIntrinsicSize{
        .width = c.max_w > 0.0f ? (c.max_w < 200.0f ? c.max_w : 200.0f) : 200.0f,
        .height = 60.0f,
    };
  }

  static uint8_t hit_test(NveAppHandle, float, float, uint8_t) {
    return NVE_GESTURE_CLAIM;
  }

  static const void* get_interface(uint32_t interface_id) {
    static const NveSurfaceApi kSurface{
        .create = &create,
        .destroy = &destroy,
        .render = &render,
        .dispatch_events = &dispatch_events,
        .dispatch_events_direct = &dispatch_events_direct,
    };
    static const NveLayoutApi kLayout{
        .measure = &measure,
        .hit_test = &hit_test,
    };
    static const NveComponentApi kComponent{
        .set_props = &set_props,
        .drain_emitted_events = &drain_emitted_events,
        .set_props_direct = &set_props_direct,
    };
    static const NveDevApi kDev{
        .apply_hot_patch = &apply_hot_patch,
        .snapshot_state = &snapshot_state,
        .restore_state = &restore_state,
    };
    switch (interface_id) {
      case NVE_IFACE_SURFACE: return &kSurface;
      case NVE_IFACE_LAYOUT: return &kLayout;
      case NVE_IFACE_COMPONENT: return &kComponent;
      case NVE_IFACE_DEV: return &kDev;
      default: return nullptr;
    }
  }
};

void test_all_opcodes_and_custom_components();
void test_corrupt_buffer_error_codes();
void test_flexbox_layout();
void test_standalone_flexbox_parity();
void test_rn_component_bridge();
#if defined(NVE_WITH_RN_JSI) && NVE_WITH_RN_JSI
void test_mock_jsi_recorder_runtime();
#endif
void test_bump_arena_and_bin_value_fuzz();
void test_text_style_inheritance();
void test_text_measurer_callback();
void test_seeded_fuzz_corpus();
void test_host_runtime_utilities_and_dylib_errors();
void test_retained_view_reconciler();
void test_native_host_session();
void test_mmap_snapshot_and_session_instant_boot();
void test_direct_scene_view_and_reconciler();

} // namespace nve_test
