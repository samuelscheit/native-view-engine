#include "nve_abi.h"
#include "nve_host_runtime.hpp"
#include "nve_rn_component.hpp"
#include "command_decoder.hpp"
#include "command_encoder.hpp"
#include "mock_jsi_recorder.hpp"

#include <cmath>
#include <cstdlib>
#include <cstring>
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

namespace {

using nve::CommandBufferEncoder;

bool approx_eq(float a, float b, float eps = 0.5f) {
  return std::fabs(a - b) <= eps;
}

std::unordered_map<uint32_t, nve::RenderedNodeSnapshot> snapshot_by_id(NveFabricHostHandle host) {
  std::unordered_map<uint32_t, nve::RenderedNodeSnapshot> map;
  for (auto& snap : nve::fabric_host_snapshot_nodes(host)) {
    map.emplace(snap.node_id, std::move(snap));
  }
  return map;
}

// 1. Test all 10 NveOpcode variants + custom component hooks + snapshot inspection.
void test_all_opcodes_and_custom_components() {
  NveSurfaceConfig cfg{.surface_id = 7, .width = 400.0f, .height = 600.0f, .scale_factor = 2.0f};
  NveFabricHostHandle host = nve_fabric_host_create(cfg);
  assert(host != nullptr);
  assert(nve_fabric_host_node_count(host) == 1);

  bool saw_custom_props = false;
  bool saw_custom_command = false;
  nve::fabric_host_register_custom_component(
      host,
      nve::CustomComponentDescriptor{
          .component_name = "RNSVGCircle",
          .on_update_props =
              [&](uint32_t node_id, const nve::RawPropMap& props) {
                assert(node_id == 4);
                assert(std::holds_alternative<std::nullptr_t>(props.at("nilProp")));
                assert(std::get<bool>(props.at("visible")) == true);
                assert(std::get<int64_t>(props.at("segments")) == 64);
                assert(std::fabs(std::get<double>(props.at("radius")) - 42.5) < 1e-6);
                assert(std::get<std::string>(props.at("fill")) == "#ff00aa");
                const auto& blob = std::get<std::vector<uint8_t>>(props.at("matrix"));
                assert(blob.size() == 3 && blob[0] == 1 && blob[2] == 3);
                saw_custom_props = true;
              },
          .on_dispatch_command =
              [&](uint32_t node_id, std::string_view cmd, NveBufferSlice payload) {
                assert(node_id == 4);
                assert(cmd == "animatePulse");
                assert(payload.len == 2 && payload.ptr[0] == 0xAA && payload.ptr[1] == 0x55);
                saw_custom_command = true;
              },
      });

  CommandBufferEncoder enc(7, 1);
  enc.create_node(2, NVE_COMP_VIEW);
  enc.create_node(3, NVE_COMP_TEXT);
  enc.create_node(4, NVE_COMP_CUSTOM, "RNSVGCircle");
  enc.create_node(5, NVE_COMP_VIEW);

  enc.set_children(NVE_ROOT_NODE_ID, {2});
  enc.set_children(2, {3, 4});
  enc.set_text(3, "Hello NVE Fabric");

  nve::LayoutProps container_layout{};
  container_layout.width = {1, 300.0f};
  container_layout.height = {1, 200.0f};
  container_layout.padding = {10.0f, 10.0f, 10.0f, 10.0f};
  container_layout.gap = 8.0f;
  enc.set_layout(2, container_layout);

  nve::StyleProps style{};
  style.background_rgba = 0xFF332211u;
  style.text_color_rgba = 0xFFFFFFFFu;
  style.border_color_rgba = 0xFF00FF00u;
  style.border_width = 2.0f;
  style.border_radius = 12.0f;
  style.opacity = 0.9f;
  style.font_size = 18.0f;
  style.z_index = 5;
  enc.set_style(2, style);

  enc.set_raw_prop_null(4, "nilProp");
  enc.set_raw_prop_bool(4, "visible", true);
  enc.set_raw_prop_i64(4, "segments", 64);
  enc.set_raw_prop_f64(4, "radius", 42.5);
  enc.set_raw_prop_string(4, "fill", "#ff00aa");
  enc.set_raw_prop_bytes(4, "matrix", {1, 2, 3});

  enc.set_event_mask(2, (1ULL << 0) | (1ULL << 6)); /* PRESS | LAYOUT */
  enc.dispatch_command(4, "animatePulse", {0xAA, 0x55});
  enc.remove_node(5);
  enc.commit(true);

  NveFrameStats stats{};
  const int32_t rc = nve_fabric_host_submit_frame(host, enc.finish(), &stats);
  assert(rc == 0);
  assert(stats.op_count == 19);
  assert(stats.mutation_count == 17); /* all ops except DispatchCommand and Commit */
  assert(nve_fabric_host_node_count(host) == 4); /* root + 2 + 3 + 4 (5 removed) */
  assert(saw_custom_props);
  assert(saw_custom_command);

  // Node 2 subscribed to EventMask::LAYOUT, so commit should have emitted a Layout event!
  const NveBufferSlice ev_slice = nve_fabric_host_drain_events(host);
  std::vector<nve::DecodedEvent> events;
  assert(nve::decode_event_ring(ev_slice, events));
  assert(events.size() == 1);
  assert(events[0].kind == nve::EventKind::Layout);
  assert(events[0].node_id == 2);
  assert(approx_eq(events[0].width, 300.0f));
  assert(approx_eq(events[0].height, 200.0f));

  // Snapshot check
  const auto snaps = nve::fabric_host_snapshot_nodes(host);
  assert(snaps.size() == 4);

  nve_fabric_host_destroy(host);
}

// 2. Test corrupt buffer error codes thoroughly.
void test_corrupt_buffer_error_codes() {
  NveSurfaceConfig cfg{.surface_id = 1, .width = 393.0f, .height = 852.0f, .scale_factor = 3.0f};
  NveFabricHostHandle host = nve_fabric_host_create(cfg);

  // Null host
  assert(nve_fabric_host_submit_frame(nullptr, NveBufferSlice{nullptr, 0}, nullptr) == -100);

  // Truncated header (< 24 bytes)
  const uint8_t short_buf[8] = {0};
  assert(nve_fabric_host_submit_frame(host, NveBufferSlice{short_buf, sizeof(short_buf)}, nullptr) == -1);

  // Null pointer with non-zero len
  assert(nve_fabric_host_submit_frame(host, NveBufferSlice{nullptr, 24}, nullptr) == -1);

  // Bad magic -> -2
  {
    CommandBufferEncoder enc(1, 1);
    auto slice = enc.finish();
    std::vector<uint8_t> bad(slice.ptr, slice.ptr + slice.len);
    bad[0] = 0xDE;
    assert(nve_fabric_host_submit_frame(host, NveBufferSlice{bad.data(), bad.size()}, nullptr) == -2);
  }

  // Bad version -> -2
  {
    CommandBufferEncoder enc(1, 1);
    auto slice = enc.finish();
    std::vector<uint8_t> bad(slice.ptr, slice.ptr + slice.len);
    bad[4] = 99;
    assert(nve_fabric_host_submit_frame(host, NveBufferSlice{bad.data(), bad.size()}, nullptr) == -2);
  }

  // Missing opcode byte (op_count=1 but 0 payload bytes) -> -3
  {
    CommandBufferEncoder enc(1, 1);
    enc.finish();
    uint32_t one = 1;
    std::memcpy(enc.raw().data() + 20, &one, 4);
    assert(nve_fabric_host_submit_frame(
               host, NveBufferSlice{enc.raw().data(), enc.raw().size()}, nullptr) == -3);
  }

  // Truncated op payload -> -4
  {
    CommandBufferEncoder enc(1, 1);
    enc.create_node(2, NVE_COMP_VIEW);
    auto slice = enc.finish();
    assert(nve_fabric_host_submit_frame(host, NveBufferSlice{slice.ptr, slice.len - 2}, nullptr) == -4);
  }

  // Unknown target node id -> -5
  {
    CommandBufferEncoder enc(1, 1);
    enc.set_text(999, "missing");
    assert(nve_fabric_host_submit_frame(host, enc.finish(), nullptr) == -5);
  }

  // Removing root node -> -5
  {
    CommandBufferEncoder enc(1, 1);
    enc.remove_node(NVE_ROOT_NODE_ID);
    assert(nve_fabric_host_submit_frame(host, enc.finish(), nullptr) == -5);
  }

  // Invalid raw prop tag -> -6
  {
    CommandBufferEncoder enc(1, 1);
    enc.create_node(10, NVE_COMP_VIEW);
    enc.set_raw_prop_null(10, "k");
    auto slice = enc.finish();
    std::vector<uint8_t> bad(slice.ptr, slice.ptr + slice.len);
    bad.back() = 99; /* corrupt tag byte */
    assert(nve_fabric_host_submit_frame(host, NveBufferSlice{bad.data(), bad.size()}, nullptr) == -6);
  }

  // Invalid ComponentKind -> -6
  {
    CommandBufferEncoder enc(1, 1);
    enc.create_node(11, 0x1234);
    assert(nve_fabric_host_submit_frame(host, enc.finish(), nullptr) == -6);
  }

  // Unknown opcode -> -7
  {
    CommandBufferEncoder enc(1, 1);
    enc.commit(false);
    auto slice = enc.finish();
    std::vector<uint8_t> bad(slice.ptr, slice.ptr + slice.len);
    bad[24] = 250;
    assert(nve_fabric_host_submit_frame(host, NveBufferSlice{bad.data(), bad.size()}, nullptr) == -7);
  }

  // Trailing garbage bytes -> -8
  {
    CommandBufferEncoder enc(1, 1);
    enc.commit(false);
    auto slice = enc.finish();
    std::vector<uint8_t> bad(slice.ptr, slice.ptr + slice.len);
    bad.push_back(0x42);
    assert(nve_fabric_host_submit_frame(host, NveBufferSlice{bad.data(), bad.size()}, nullptr) == -8);
  }

  nve_fabric_host_destroy(host);
}

// 3. Test real Yoga Flexbox layout (and standalone fallback layout)!
void test_flexbox_layout() {
  NveSurfaceConfig cfg{.surface_id = 1, .width = 400.0f, .height = 200.0f, .scale_factor = 2.0f};
  NveFabricHostHandle host = nve_fabric_host_create(cfg);

  // Build a Row container (width=400, height=200, padding=20, gap=10) with:
  //   - child 10: fixed width=100, height=80
  //   - child 11: flex_grow=1, height=80   -> remaining = 400 - 40(pad) - 100 - 20(2*gap) = 240; grow 1/(1+2) = 80
  //   - child 12: flex_grow=2, height=100% -> grow 2/(1+2) = 160, height = 160 (200 - 40 pad)
  CommandBufferEncoder enc(1, 1);
  enc.create_node(2, NVE_COMP_VIEW);
  enc.create_node(10, NVE_COMP_VIEW);
  enc.create_node(11, NVE_COMP_VIEW);
  enc.create_node(12, NVE_COMP_VIEW);

  enc.set_children(NVE_ROOT_NODE_ID, {2});
  enc.set_children(2, {10, 11, 12});

  nve::LayoutProps row{};
  row.width = {1, 400.0f};
  row.height = {1, 200.0f};
  row.flex_direction = 1; /* Row */
  row.padding = {20.0f, 20.0f, 20.0f, 20.0f};
  row.gap = 10.0f;
  enc.set_layout(2, row);

  nve::LayoutProps c10{};
  c10.width = {1, 100.0f};
  c10.height = {1, 80.0f};
  enc.set_layout(10, c10);

  nve::LayoutProps c11{};
  c11.height = {1, 80.0f};
  c11.flex_grow = 1.0f;
  enc.set_layout(11, c11);

  nve::LayoutProps c12{};
  c12.height = {2, 100.0f}; /* 100% of container inner height (160) */
  c12.flex_grow = 2.0f;
  enc.set_layout(12, c12);

  enc.commit(true);

  NveFrameStats stats{};
  assert(nve_fabric_host_submit_frame(host, enc.finish(), &stats) == 0);

  nve::LayoutRect r2{}, r10{}, r11{}, r12{};
  assert(nve::fabric_host_get_node_layout(host, 2, &r2));
  assert(nve::fabric_host_get_node_layout(host, 10, &r10));
  assert(nve::fabric_host_get_node_layout(host, 11, &r11));
  assert(nve::fabric_host_get_node_layout(host, 12, &r12));

  assert(approx_eq(r2.x, 0.0f) && approx_eq(r2.y, 0.0f));
  assert(approx_eq(r2.width, 400.0f) && approx_eq(r2.height, 200.0f));

  assert(approx_eq(r10.x, 20.0f) && approx_eq(r10.y, 20.0f));
  assert(approx_eq(r10.width, 100.0f) && approx_eq(r10.height, 80.0f));

  assert(approx_eq(r11.x, 130.0f) && approx_eq(r11.y, 20.0f));
  assert(approx_eq(r11.width, 80.0f) && approx_eq(r11.height, 80.0f));

  assert(approx_eq(r12.x, 220.0f) && approx_eq(r12.y, 20.0f));
  assert(approx_eq(r12.width, 160.0f) && approx_eq(r12.height, 160.0f));

#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
  assert(nve::fabric_host_has_yoga_layout() == true);

  // Additional Yoga-specific test: justifyContent=SpaceBetween + alignItems=Center + RowReverse + Text measure
  CommandBufferEncoder enc2(1, 2);
  enc2.create_node(20, NVE_COMP_VIEW);
  enc2.create_node(21, NVE_COMP_VIEW);
  enc2.create_node(22, NVE_COMP_TEXT);
  enc2.set_children(NVE_ROOT_NODE_ID, {20});
  enc2.set_children(20, {21, 22});
  enc2.set_text(22, "YogaText");

  nve::LayoutProps sb{};
  sb.width = {1, 300.0f};
  sb.height = {1, 100.0f};
  sb.flex_direction = 1; /* Row */
  sb.justify_content = 3; /* SpaceBetween */
  sb.align_items = 2; /* Center */
  enc2.set_layout(20, sb);

  nve::LayoutProps b21{};
  b21.width = {1, 60.0f};
  b21.height = {1, 40.0f};
  enc2.set_layout(21, b21);

  enc2.commit(true);
  assert(nve_fabric_host_submit_frame(host, enc2.finish(), &stats) == 0);

  nve::LayoutRect r21{}, r22{};
  assert(nve::fabric_host_get_node_layout(host, 21, &r21));
  assert(nve::fabric_host_get_node_layout(host, 22, &r22));
  assert(approx_eq(r21.x, 0.0f));
  assert(approx_eq(r21.y, 30.0f)); /* centered in 100px height: (100-40)/2 = 30 */
  assert(r22.width > 10.0f && r22.height > 10.0f); /* measured by Yoga text measure func */
  assert(approx_eq(r22.x + r22.width, 300.0f)); /* SpaceBetween pushes 2nd item flush right */
#else
  assert(nve::fabric_host_has_yoga_layout() == false);
#endif

  nve_fabric_host_destroy(host);
}

// 4. Test NveRnComponentBridge end-to-end (set_props, render, native events, drain_emitted_events, snapshot/restore).
struct MockEmbeddedRustComponent {
  uint32_t surface_id{1};
  std::string last_props_json;
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
        const std::string json = "{\"count\":" + std::to_string(self->count) + "}";
        self->outbound_events.push_custom(
            self->surface_id,
            2,
            ev.timestamp_us,
            "onIncrement",
            reinterpret_cast<const uint8_t*>(json.data()),
            json.size());
        dirty = 1;
      }
    }
    return dirty;
  }

  static uint32_t apply_hot_patch(NveAppHandle h, NveBufferSlice patch_json) {
    auto* self = static_cast<MockEmbeddedRustComponent*>(h);
    self->last_props_json.assign(
        reinterpret_cast<const char*>(patch_json.ptr), patch_json.len);
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
    const std::string payload = "{\"accepted\":true}";
    self->outbound_events.push_custom(
        self->surface_id,
        NVE_ROOT_NODE_ID,
        1000,
        "onPropsApplied",
        reinterpret_cast<const uint8_t*>(payload.data()),
        payload.size());
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
    };
    static const NveLayoutApi kLayout{
        .measure = &measure,
        .hit_test = &hit_test,
    };
    static const NveComponentApi kComponent{
        .set_props = &set_props,
        .drain_emitted_events = &drain_emitted_events,
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

void test_rn_component_bridge() {
  std::vector<nve::DecodedEvent> emitted_via_cb;
  NveSurfaceConfig cfg{.surface_id = 42, .width = 320.0f, .height = 120.0f, .scale_factor = 3.0f};
  nve::NveRnComponentBridge bridge(
      &MockEmbeddedRustComponent::get_interface,
      cfg,
      [&](const nve::DecodedEvent& ev) { emitted_via_cb.push_back(ev); });

  assert(bridge.is_valid());
  auto measured = bridge.measure(NveSizeConstraints{0.0f, 0.0f, 320.0f, 120.0f});
  assert(measured.has_value() && approx_eq(measured->width, 200.0f) && approx_eq(measured->height, 60.0f));
  assert(bridge.hit_test(10.0f, 10.0f, 1) == NVE_GESTURE_CLAIM);

  // 1. Update props via typed PropMap -> JSON -> vtable.set_props -> auto-render
  nve::PropMap props;
  props["title"] = std::string("Hello \"RN\"");
  props["initialCount"] = static_cast<int64_t>(5);
  props["enabled"] = true;
  props["ratio"] = 0.75;
  props["optional"] = nullptr;

  const std::string serialized = nve::serialize_props_json(props);
  assert(serialized.find("\"title\":\"Hello \\\"RN\\\"\"") != std::string::npos);
  assert(serialized.find("\"initialCount\":5") != std::string::npos);
  assert(serialized.find("\"enabled\":true") != std::string::npos);
  assert(serialized.find("\"optional\":null") != std::string::npos);

  assert(bridge.update_props(props, /*auto_render=*/true));
  assert(bridge.frame_count() == 1);
  assert(bridge.node_count() == 3);

  auto rect = bridge.node_layout(2);
  assert(rect.has_value());
  assert(approx_eq(rect->width, 200.0f) && approx_eq(rect->height, 60.0f));

  auto evs = bridge.drain_emitted_events();
  assert(evs.size() == 1);
  assert(evs[0].kind == nve::EventKind::Custom);
  assert(evs[0].name == "onPropsApplied");
  assert(evs[0].payload_as_string() == "{\"accepted\":true}");
  assert(emitted_via_cb.size() == 1);

  // 2. Simulate native press -> dispatch -> outbound onIncrement event
  nve::fabric_host_enqueue_press(bridge.fabric_host(), 2, 99999, 15.0f, 25.0f);
  const NveBufferSlice inbound = nve_fabric_host_drain_events(bridge.fabric_host());
  assert(bridge.dispatch_native_events(inbound, /*auto_render=*/true) == 1);
  assert(bridge.frame_count() == 2);

  auto evs2 = bridge.drain_emitted_events();
  assert(evs2.size() == 1);
  assert(evs2[0].name == "onIncrement");
  assert(evs2[0].payload_as_string() == "{\"count\":1}");
  assert(emitted_via_cb.size() == 2);

  // 3. Snapshot and restore state
  const auto snap = bridge.snapshot_state();
  assert(snap.size() == 1 && snap[0] == '1');
  const std::string next_state = "7";
  bridge.restore_state(
      NveBufferSlice{reinterpret_cast<const uint8_t*>(next_state.data()), next_state.size()},
      /*auto_render=*/true);
  assert(bridge.frame_count() == 3);
  const auto snap2 = bridge.snapshot_state();
  assert(snap2.size() == 1 && snap2[0] == '7');
}

#if defined(NVE_WITH_RN_JSI) && NVE_WITH_RN_JSI
// 5. Test real facebook::jsi::Runtime subclass (MockJsiRecorderRuntime) with zero Hermes!
void test_mock_jsi_recorder_runtime() {
  nve::MockJsiRecorderRuntime rt;
  facebook::jsi::Object root(rt);
  root.setProperty(rt, "event", "onSvgLoad");
  root.setProperty(rt, "width", 128.5);
  root.setProperty(rt, "animated", true);
  root.setProperty(rt, "tag", nullptr);

  facebook::jsi::Array pts(rt, 2);
  pts.setValueAtIndex(rt, 0, 10);
  pts.setValueAtIndex(rt, 1, 20);
  root.setProperty(rt, "points", std::move(pts));

  nve::EventRingWriter ring;
  ring.clear();
  rt.record_custom_event(ring, 1, 77, 123456, "onSvgLoad", facebook::jsi::Value(rt, root));

  const auto& bytes = ring.finalize();
  std::vector<nve::DecodedEvent> decoded;
  assert(nve::decode_event_ring(NveBufferSlice{bytes.data(), bytes.size()}, decoded));
  assert(decoded.size() == 1);
  assert(decoded[0].kind == nve::EventKind::Custom);
  assert(decoded[0].node_id == 77);
  assert(decoded[0].name == "onSvgLoad");
  const std::string_view json = decoded[0].payload_as_string();
  assert(json.find("\"event\":\"onSvgLoad\"") != std::string_view::npos);
  assert(json.find("\"width\":128.5") != std::string_view::npos);
  assert(json.find("\"animated\":true") != std::string_view::npos);
  assert(json.find("\"tag\":null") != std::string_view::npos);
  assert(json.find("\"points\":[10,20]") != std::string_view::npos);
}
#endif

// 6. Test NVE_COMP_TEXT multi-level style inheritance, explicit overrides, reparent reset, and default opaque black.
void test_text_style_inheritance() {
  NveSurfaceConfig cfg{.surface_id = 11, .width = 400.0f, .height = 600.0f, .scale_factor = 2.0f};
  NveFabricHostHandle host = nve_fabric_host_create(cfg);
  assert(host != nullptr);

  // Tree:
  //   root (1)
  //     ├── text_default (10)            [no explicit style -> default 0xFF000000, 16pt]
  //     └── text_parent (20)             [explicit 0xFF112233, 24pt]
  //           ├── text_child (21)        [inherits 0xFF112233, 24pt]
  //           ├── view_middle (22)       [non-text wrapper]
  //           │     └── text_grand (23)  [inherits 0xFF112233, 24pt across View]
  //           └── text_override (24)     [explicit 0xFFAA5500, 12pt]
  //                 └── text_nested (25) [inherits 0xFFAA5500, 12pt from override]
  CommandBufferEncoder enc(11, 1);
  enc.create_node(10, NVE_COMP_TEXT);
  enc.create_node(20, NVE_COMP_TEXT);
  enc.create_node(21, NVE_COMP_TEXT);
  enc.create_node(22, NVE_COMP_VIEW);
  enc.create_node(23, NVE_COMP_TEXT);
  enc.create_node(24, NVE_COMP_TEXT);
  enc.create_node(25, NVE_COMP_TEXT);

  enc.set_text(10, "Default black text");
  enc.set_text(21, "Inherited child");
  enc.set_text(23, "Inherited grandchild");
  enc.set_text(25, "Inherited under override");

  nve::StyleProps parent_style{};
  parent_style.text_color_rgba = 0xFF112233u;
  parent_style.font_size = 24.0f;
  enc.set_style(20, parent_style);

  nve::StyleProps override_style{};
  override_style.text_color_rgba = 0xFFAA5500u;
  override_style.font_size = 12.0f;
  enc.set_style(24, override_style);

  enc.set_children(NVE_ROOT_NODE_ID, {10, 20});
  enc.set_children(20, {21, 22, 24});
  enc.set_children(22, {23});
  enc.set_children(24, {25});
  enc.commit(true);

  assert(nve_fabric_host_submit_frame(host, enc.finish(), nullptr) == 0);

  auto snaps = snapshot_by_id(host);
  // Default opaque black (0xFF000000) & 16pt on standalone Text
  assert(snaps.at(10).text_color_rgba == 0xFF000000u);
  assert(approx_eq(snaps.at(10).font_size, 16.0f, 1e-3f));

  // Multi-level inheritance (direct child and grandchild across View)
  assert(snaps.at(20).text_color_rgba == 0xFF112233u);
  assert(approx_eq(snaps.at(20).font_size, 24.0f, 1e-3f));
  assert(snaps.at(21).text_color_rgba == 0xFF112233u);
  assert(approx_eq(snaps.at(21).font_size, 24.0f, 1e-3f));
  assert(snaps.at(23).text_color_rgba == 0xFF112233u);
  assert(approx_eq(snaps.at(23).font_size, 24.0f, 1e-3f));

  // Explicit child override and inheritance beneath the override
  assert(snaps.at(24).text_color_rgba == 0xFFAA5500u);
  assert(approx_eq(snaps.at(24).font_size, 12.0f, 1e-3f));
  assert(snaps.at(25).text_color_rgba == 0xFFAA5500u);
  assert(approx_eq(snaps.at(25).font_size, 12.0f, 1e-3f));

  // Verify inherited font_size affects measured height (24pt child > 12pt nested override)
  nve::LayoutRect r21{}, r25{};
  assert(nve::fabric_host_get_node_layout(host, 21, &r21));
  assert(nve::fabric_host_get_node_layout(host, 25, &r25));
  assert(r21.height > r25.height);

  // Frame 2: dynamically update parent style -> non-explicit descendants update, explicit subtree stays isolated
  CommandBufferEncoder enc2(11, 2);
  nve::StyleProps updated_parent{};
  updated_parent.text_color_rgba = 0xFF00CC88u;
  updated_parent.font_size = 32.0f;
  enc2.set_style(20, updated_parent);
  enc2.commit(true);
  assert(nve_fabric_host_submit_frame(host, enc2.finish(), nullptr) == 0);

  auto snaps2 = snapshot_by_id(host);
  assert(snaps2.at(21).text_color_rgba == 0xFF00CC88u);
  assert(approx_eq(snaps2.at(21).font_size, 32.0f, 1e-3f));
  assert(snaps2.at(23).text_color_rgba == 0xFF00CC88u);
  assert(approx_eq(snaps2.at(23).font_size, 32.0f, 1e-3f));
  assert(snaps2.at(24).text_color_rgba == 0xFFAA5500u);
  assert(approx_eq(snaps2.at(24).font_size, 12.0f, 1e-3f));
  assert(snaps2.at(25).text_color_rgba == 0xFFAA5500u);
  assert(approx_eq(snaps2.at(25).font_size, 12.0f, 1e-3f));

  // Frame 3: reparent node 23 directly under root (no Text ancestor) -> resets to default 0xFF000000 / 16pt
  CommandBufferEncoder enc3(11, 3);
  enc3.set_children(22, {});
  enc3.set_children(NVE_ROOT_NODE_ID, {10, 20, 23});
  enc3.commit(true);
  assert(nve_fabric_host_submit_frame(host, enc3.finish(), nullptr) == 0);

  auto snaps3 = snapshot_by_id(host);
  assert(snaps3.at(23).text_color_rgba == 0xFF000000u);
  assert(approx_eq(snaps3.at(23).font_size, 16.0f, 1e-3f));

  nve_fabric_host_destroy(host);
}

// 7. Test fabric_host_set_text_measurer callback, max_width wrapping, empty strings, and heuristic reset.
void test_text_measurer_callback() {
  NveSurfaceConfig cfg{.surface_id = 12, .width = 400.0f, .height = 600.0f, .scale_factor = 2.0f};
  NveFabricHostHandle host = nve_fabric_host_create(cfg);
  assert(host != nullptr);

  int call_count = 0;
  bool saw_empty = false;
  float last_narrow_max_w = -1.0f;
  float last_wide_max_w = -1.0f;
  float last_font_size = 0.0f;

  // Distinctive custom measurer: 10pt per char, 40pt line height (clearly distinct from 1.3x heuristic).
  nve::fabric_host_set_text_measurer(
      host,
      [&](std::string_view text, float font_size, float max_width) -> NveIntrinsicSize {
        ++call_count;
        if (text.empty()) {
          saw_empty = true;
        }
        last_font_size = font_size;
        const float raw_w = static_cast<float>(text.size()) * (font_size * 0.625f);
        const float line_h = font_size * 2.5f;
        if (text.find("NARROW:") == 0) {
          last_narrow_max_w = max_width;
        } else if (text.find("WIDE:") == 0) {
          last_wide_max_w = max_width;
        }
        if (max_width > 0.0f && raw_w > max_width) {
          const float lines = std::ceil(raw_w / max_width);
          return NveIntrinsicSize{max_width, lines * line_h};
        }
        return NveIntrinsicSize{raw_w, line_h};
      });

  CommandBufferEncoder enc(12, 1);
  enc.create_node(2, NVE_COMP_VIEW); /* narrow container: 120px wide, 10px padding -> 100px max_w */
  enc.create_node(3, NVE_COMP_TEXT); /* narrow wrapping text */
  enc.create_node(4, NVE_COMP_VIEW); /* wide container: 360px wide */
  enc.create_node(5, NVE_COMP_TEXT); /* wide single-line text */
  enc.create_node(6, NVE_COMP_TEXT); /* empty text */

  nve::LayoutProps narrow_box{};
  narrow_box.width = {1, 120.0f};
  narrow_box.padding = {0.0f, 10.0f, 0.0f, 10.0f};
  narrow_box.align_items = 1; /* FlexStart so width isn't stretched beyond measured width */
  enc.set_layout(2, narrow_box);

  nve::LayoutProps wide_box{};
  wide_box.width = {1, 360.0f};
  wide_box.align_items = 1; /* FlexStart */
  enc.set_layout(4, wide_box);

  nve::StyleProps st{};
  st.font_size = 16.0f;
  enc.set_style(3, st);
  enc.set_style(5, st);
  enc.set_style(6, st);

  // 30 chars @ 16pt * 0.625 = 300pt unwrapped: wraps to 3 lines (120pt) in 100pt box, 1 line (40pt) in 360pt box
  enc.set_text(3, "NARROW:01234567890123456789012");
  enc.set_text(5, "WIDE:0123456789012345678901234");
  enc.set_text(6, "");

  enc.set_children(NVE_ROOT_NODE_ID, {2, 4, 6});
  enc.set_children(2, {3});
  enc.set_children(4, {5});
  enc.commit(true);

  assert(nve_fabric_host_submit_frame(host, enc.finish(), nullptr) == 0);
  assert(call_count >= 2);
  assert(!saw_empty); /* empty text short-circuits without invoking callback */
  assert(approx_eq(last_font_size, 16.0f, 0.1f));
  assert(approx_eq(last_narrow_max_w, 100.0f, 1.0f));
  assert(approx_eq(last_wide_max_w, 360.0f, 1.0f));

  nve::LayoutRect r_narrow{}, r_wide{}, r_empty{};
  assert(nve::fabric_host_get_node_layout(host, 3, &r_narrow));
  assert(nve::fabric_host_get_node_layout(host, 5, &r_wide));
  assert(nve::fabric_host_get_node_layout(host, 6, &r_empty));

  // Narrow wraps to 3 lines @ 40pt = 120pt; wide stays 1 line @ 40pt
  assert(approx_eq(r_narrow.width, 100.0f, 1.0f));
  assert(approx_eq(r_narrow.height, 120.0f, 1.0f));
  assert(approx_eq(r_wide.width, 300.0f, 1.0f));
  assert(approx_eq(r_wide.height, 40.0f, 1.0f));
  assert(r_narrow.height > r_wide.height * 2.0f);
  assert(approx_eq(r_empty.height, 0.0f, 0.5f));

  // Reset measurer to empty callback -> falls back to built-in heuristic (16 * 1.3 = 20.8pt/line)
  const int calls_before_reset = call_count;
  nve::fabric_host_set_text_measurer(host, {});
  nve::fabric_host_set_surface_size(host, 400.0f, 600.0f, 2.0f);

  assert(call_count == calls_before_reset);
  nve::LayoutRect r_wide_heuristic{}, r_narrow_heuristic{};
  assert(nve::fabric_host_get_node_layout(host, 5, &r_wide_heuristic));
  assert(nve::fabric_host_get_node_layout(host, 3, &r_narrow_heuristic));
  // Heuristic single line height is ~20.8pt (far below custom 40pt)
  assert(approx_eq(r_wide_heuristic.height, 16.0f * 1.3f, 1.5f));
  assert(r_narrow_heuristic.height > r_wide_heuristic.height);

  // Misbehaving callback returning NaN / negative values must not crash or yield NaN layouts
  nve::fabric_host_set_text_measurer(
      host,
      [](std::string_view, float, float) -> NveIntrinsicSize {
        return NveIntrinsicSize{std::numeric_limits<float>::quiet_NaN(), -50.0f};
      });
  nve::fabric_host_set_surface_size(host, 400.0f, 600.0f, 2.0f);
  nve::LayoutRect r_sanitized{};
  assert(nve::fabric_host_get_node_layout(host, 5, &r_sanitized));
  assert(std::isfinite(r_sanitized.width) && r_sanitized.width >= 0.0f);
  assert(std::isfinite(r_sanitized.height) && r_sanitized.height >= 0.0f);

  nve_fabric_host_destroy(host);
}

// 8. Deterministic seeded fuzz corpus (600+ CommandBuffers + 300+ EventRings) asserting zero crashes/UB
//    and clean negative error codes on malformed inputs.
void test_seeded_fuzz_corpus() {
  std::mt19937_64 rng(0x4E56455F46555A5AuLL);
  auto rand_u8 = [&]() -> uint8_t { return static_cast<uint8_t>(rng() & 0xFFu); };
  auto rand_range = [&](size_t lo, size_t hi) -> size_t {
    return lo + static_cast<size_t>(rng() % (hi - lo + 1));
  };

  NveSurfaceConfig cfg{.surface_id = 1, .width = 393.0f, .height = 852.0f, .scale_factor = 3.0f};
  NveFabricHostHandle shared_host = nve_fabric_host_create(cfg);
  assert(shared_host != nullptr);

  // Build a rich valid baseline CommandBuffer containing all 10 opcodes for truncation/mutation fuzzing.
  CommandBufferEncoder baseline_enc(1, 1);
  baseline_enc.create_node(2, NVE_COMP_VIEW);
  baseline_enc.create_node(3, NVE_COMP_TEXT);
  baseline_enc.create_node(4, NVE_COMP_CUSTOM, "FuzzComp");
  baseline_enc.set_children(NVE_ROOT_NODE_ID, {2});
  baseline_enc.set_children(2, {3, 4});
  baseline_enc.set_text(3, "Fuzz baseline text");
  nve::LayoutProps base_lp{};
  base_lp.width = {1, 200.0f};
  base_lp.height = {1, 80.0f};
  base_lp.padding = {4.0f, 4.0f, 4.0f, 4.0f};
  baseline_enc.set_layout(2, base_lp);
  nve::StyleProps base_st{};
  base_st.background_rgba = 0xFF112233u;
  base_st.font_size = 16.0f;
  baseline_enc.set_style(3, base_st);
  baseline_enc.set_raw_prop_null(4, "n");
  baseline_enc.set_raw_prop_bool(4, "b", true);
  baseline_enc.set_raw_prop_i64(4, "i", -42);
  baseline_enc.set_raw_prop_f64(4, "f", 3.14);
  baseline_enc.set_raw_prop_string(4, "s", "hello");
  baseline_enc.set_raw_prop_bytes(4, "bytes", {1, 2, 3, 4});
  baseline_enc.set_event_mask(2, (1ULL << 0) | (1ULL << 6));
  baseline_enc.dispatch_command(4, "cmd", {9, 8});
  baseline_enc.remove_node(4);
  baseline_enc.commit(true);
  const NveBufferSlice base_slice = baseline_enc.finish();
  const std::vector<uint8_t> base_bytes(base_slice.ptr, base_slice.ptr + base_slice.len);

  size_t cmd_fuzz_cases = 0;

  // (a) Every prefix truncation of the baseline CommandBuffer MUST fail cleanly with a negative error code.
  for (size_t cut = 0; cut < base_bytes.size(); ++cut) {
    const int32_t rc = nve_fabric_host_submit_frame(
        shared_host, NveBufferSlice{base_bytes.data(), cut}, nullptr);
    assert(rc < 0);
    ++cmd_fuzz_cases;
  }

  // (b) Explicit adversarial/oversized/cyclic/NaN CommandBuffers (must all return < 0 without OOM/crash).
  {
    auto expect_err = [&](const std::vector<uint8_t>& buf, int32_t expected_or_any_neg = 0) {
      const int32_t rc = nve_fabric_host_submit_frame(
          shared_host, NveBufferSlice{buf.data(), buf.size()}, nullptr);
      if (expected_or_any_neg < 0) {
        assert(rc == expected_or_any_neg);
      } else {
        assert(rc < 0);
      }
      ++cmd_fuzz_cases;
    };

    // Oversized SET_CHILDREN count (0xFFFFFFFF) -> -4 without OOM
    {
      CommandBufferEncoder enc(1, 10);
      enc.create_node(50, NVE_COMP_VIEW);
      enc.finish();
      auto& raw = enc.raw();
      uint32_t op_count = 2;
      std::memcpy(raw.data() + 20, &op_count, 4);
      raw.push_back(NVE_OP_SET_CHILDREN);
      uint32_t pid = 50;
      uint32_t huge_cnt = 0xFFFFFFFFu;
      raw.insert(raw.end(), reinterpret_cast<uint8_t*>(&pid), reinterpret_cast<uint8_t*>(&pid) + 4);
      raw.insert(raw.end(), reinterpret_cast<uint8_t*>(&huge_cnt), reinterpret_cast<uint8_t*>(&huge_cnt) + 4);
      expect_err(raw, -4);
    }

    // Oversized SET_TEXT length (0xFFFFFFFF) -> -4 without OOM
    {
      CommandBufferEncoder enc(1, 11);
      enc.create_node(51, NVE_COMP_TEXT);
      enc.finish();
      auto& raw = enc.raw();
      uint32_t op_count = 2;
      std::memcpy(raw.data() + 20, &op_count, 4);
      raw.push_back(NVE_OP_SET_TEXT);
      uint32_t nid = 51;
      uint32_t huge_len = 0xFFFFFFFFu;
      raw.insert(raw.end(), reinterpret_cast<uint8_t*>(&nid), reinterpret_cast<uint8_t*>(&nid) + 4);
      raw.insert(raw.end(), reinterpret_cast<uint8_t*>(&huge_len), reinterpret_cast<uint8_t*>(&huge_len) + 4);
      expect_err(raw, -4);
    }

    // Self-parenting cycle (52 -> {52}) -> -6
    {
      CommandBufferEncoder enc(1, 12);
      enc.create_node(52, NVE_COMP_VIEW);
      enc.set_children(52, {52});
      enc.commit(true);
      enc.finish();
      expect_err(enc.raw(), -6);
    }

    // Root-as-child (53 -> {NVE_ROOT_NODE_ID}) -> -6
    {
      CommandBufferEncoder enc(1, 13);
      enc.create_node(53, NVE_COMP_VIEW);
      enc.set_children(53, {NVE_ROOT_NODE_ID});
      enc.commit(true);
      enc.finish();
      expect_err(enc.raw(), -6);
    }

    // 2-node cycle (54 -> 55 -> 54) -> -6
    {
      CommandBufferEncoder enc(1, 14);
      enc.create_node(54, NVE_COMP_VIEW);
      enc.create_node(55, NVE_COMP_VIEW);
      enc.set_children(54, {55});
      enc.set_children(55, {54});
      enc.commit(true);
      enc.finish();
      expect_err(enc.raw(), -6);
    }

    // Duplicate child in same parent (56 -> {57, 57}) -> -6
    {
      CommandBufferEncoder enc(1, 15);
      enc.create_node(56, NVE_COMP_VIEW);
      enc.create_node(57, NVE_COMP_VIEW);
      enc.set_children(56, {57, 57});
      enc.commit(true);
      enc.finish();
      expect_err(enc.raw(), -6);
    }

    // NaN / Inf layout dimensions and negative flex_grow -> -6
    {
      CommandBufferEncoder enc(1, 16);
      enc.create_node(58, NVE_COMP_VIEW);
      nve::LayoutProps bad_lp{};
      bad_lp.width = {1, std::numeric_limits<float>::quiet_NaN()};
      enc.set_layout(58, bad_lp);
      enc.finish();
      expect_err(enc.raw(), -6);
    }
    {
      CommandBufferEncoder enc(1, 17);
      enc.create_node(59, NVE_COMP_VIEW);
      nve::LayoutProps bad_lp{};
      bad_lp.flex_grow = -1.0f;
      enc.set_layout(59, bad_lp);
      enc.finish();
      expect_err(enc.raw(), -6);
    }
    {
      CommandBufferEncoder enc(1, 18);
      enc.create_node(60, NVE_COMP_TEXT);
      nve::StyleProps bad_st{};
      bad_st.font_size = std::numeric_limits<float>::infinity();
      enc.set_style(60, bad_st);
      enc.finish();
      expect_err(enc.raw(), -6);
    }
  }

  // (c) 200 pure random buffers (sizes 0..2048) -> must all return clean negative error codes.
  for (int i = 0; i < 200; ++i) {
    const size_t len = (i == 0) ? 0 : rand_range(1, 2048);
    std::vector<uint8_t> buf(len);
    for (auto& b : buf) b = rand_u8();
    const int32_t rc = nve_fabric_host_submit_frame(
        shared_host, NveBufferSlice{buf.data(), buf.size()}, nullptr);
    assert(rc < 0);
    ++cmd_fuzz_cases;
  }

  // (d) 200 valid-header + random op_count + random payload buffers.
  for (int i = 0; i < 200; ++i) {
    const size_t payload_len = rand_range(0, 512);
    std::vector<uint8_t> buf(24 + payload_len);
    const uint32_t magic = NVE_ABI_MAGIC;
    const uint16_t version = NVE_ABI_VERSION;
    const uint16_t flags = 0;
    const uint32_t sid = 1;
    const uint64_t fid = static_cast<uint64_t>(i + 100);
    const uint32_t op_count = (i % 5 == 0) ? 0xFFFFFFFFu : static_cast<uint32_t>(rand_range(1, 64));
    std::memcpy(buf.data() + 0, &magic, 4);
    std::memcpy(buf.data() + 4, &version, 2);
    std::memcpy(buf.data() + 6, &flags, 2);
    std::memcpy(buf.data() + 8, &sid, 4);
    std::memcpy(buf.data() + 12, &fid, 8);
    std::memcpy(buf.data() + 20, &op_count, 4);
    for (size_t j = 24; j < buf.size(); ++j) {
      buf[j] = rand_u8();
    }
    const int32_t rc = nve_fabric_host_submit_frame(
        shared_host, NveBufferSlice{buf.data(), buf.size()}, nullptr);
    // Random bytes with op_count >= 1 will almost always fail cleanly with < 0; even if by rare
    // chance a valid encoding occurs, rc <= 0 and zero crashes/UB.
    assert(rc <= 0);
    ++cmd_fuzz_cases;
  }

  // (e) 200 mutated copies of the rich baseline buffer (bit flips, random overwrites, random splice/append).
  for (int i = 0; i < 200; ++i) {
    std::vector<uint8_t> mut = base_bytes;
    const size_t num_edits = rand_range(1, 6);
    for (size_t e = 0; e < num_edits; ++e) {
      const size_t mode = rng() % 4;
      if (mode == 0 && !mut.empty()) {
        mut[rng() % mut.size()] ^= static_cast<uint8_t>(1u << (rng() % 8));
      } else if (mode == 1 && !mut.empty()) {
        mut[rng() % mut.size()] = rand_u8();
      } else if (mode == 2 && mut.size() > 1) {
        mut.resize(rand_range(1, mut.size() - 1));
      } else {
        const size_t extra = rand_range(1, 32);
        for (size_t k = 0; k < extra; ++k) mut.push_back(rand_u8());
      }
    }
    NveFabricHostHandle fresh = nve_fabric_host_create(cfg);
    const int32_t rc = nve_fabric_host_submit_frame(
        fresh, NveBufferSlice{mut.data(), mut.size()}, nullptr);
    assert(rc <= 0);
    nve_fabric_host_destroy(fresh);
    ++cmd_fuzz_cases;
  }

  assert(cmd_fuzz_cases >= 600);

  // Verify shared_host survived 600+ hostile fuzz buffers and still mounts a clean valid frame!
  {
    CommandBufferEncoder recovery(1, 9999);
    recovery.create_node(9001, NVE_COMP_VIEW);
    recovery.create_node(9002, NVE_COMP_TEXT);
    recovery.set_text(9002, "Recovered cleanly");
    nve::LayoutProps lp{};
    lp.width = {1, 250.0f};
    lp.height = {1, 100.0f};
    recovery.set_layout(9001, lp);
    recovery.set_children(NVE_ROOT_NODE_ID, {9001});
    recovery.set_children(9001, {9002});
    recovery.commit(true);
    assert(nve_fabric_host_submit_frame(shared_host, recovery.finish(), nullptr) == 0);
    nve::LayoutRect r{};
    assert(nve::fabric_host_get_node_layout(shared_host, 9001, &r));
    assert(approx_eq(r.width, 250.0f) && approx_eq(r.height, 100.0f));
  }

  // Deep chain stress (1,000 nested nodes -> exercises iterative text style propagation & safe layout).
  {
    NveFabricHostHandle deep_host = nve_fabric_host_create(cfg);
    CommandBufferEncoder deep_enc(1, 1);
    uint32_t prev = NVE_ROOT_NODE_ID;
    for (uint32_t id = 2; id <= 1001; ++id) {
      deep_enc.create_node(id, (id == 1001) ? NVE_COMP_TEXT : NVE_COMP_VIEW);
      deep_enc.set_children(prev, {id});
      prev = id;
    }
    deep_enc.set_text(1001, "Deep leaf");
    deep_enc.commit(true);
    assert(nve_fabric_host_submit_frame(deep_host, deep_enc.finish(), nullptr) == 0);
    nve_fabric_host_destroy(deep_host);
  }

  nve_fabric_host_destroy(shared_host);

  // (f) EventRing fuzz corpus (300+ cases): truncations, oversized counts/lengths, trailing garbage, random bytes.
  nve::EventRingWriter valid_ring;
  valid_ring.clear();
  valid_ring.push_press(1, 2, 100, 12.5f, 34.5f);
  valid_ring.push_long_press(1, 2, 200, 15.0f, 36.0f);
  valid_ring.push_scroll(1, 3, 300, 0.0f, 120.0f, 393.0f, 2000.0f);
  valid_ring.push_text_change(1, 4, 400, "hello world");
  valid_ring.push_empty(1, 5, 500, 5); /* Focus */
  valid_ring.push_empty(1, 5, 600, 6); /* Blur */
  valid_ring.push_layout(1, 6, 700, 0.0f, 0.0f, 300.0f, 200.0f);
  const std::string custom_json = "{\"ok\":true}";
  valid_ring.push_custom(
      1, 7, 800, "onCustom", reinterpret_cast<const uint8_t*>(custom_json.data()), custom_json.size());
  const std::vector<uint8_t> valid_ring_bytes = valid_ring.finalize();

  std::vector<nve::DecodedEvent> out_events;
  assert(nve::decode_event_ring(
      NveBufferSlice{valid_ring_bytes.data(), valid_ring_bytes.size()}, out_events));
  assert(out_events.size() == 8);

  // Null pointer + non-zero len -> false; zero len -> true
  assert(!nve::decode_event_ring(NveBufferSlice{nullptr, 16}, out_events));
  assert(nve::decode_event_ring(NveBufferSlice{nullptr, 0}, out_events));
  assert(out_events.empty());

  size_t ring_fuzz_cases = 0;

  // Every non-empty prefix truncation of valid_ring_bytes must return false and leave out_events empty.
  for (size_t cut = 1; cut < valid_ring_bytes.size(); ++cut) {
    assert(!nve::decode_event_ring(NveBufferSlice{valid_ring_bytes.data(), cut}, out_events));
    assert(out_events.empty());
    ++ring_fuzz_cases;
  }

  // Trailing garbage byte after valid ring -> false and out_events cleared.
  {
    std::vector<uint8_t> trailing = valid_ring_bytes;
    trailing.push_back(0xAA);
    assert(!nve::decode_event_ring(NveBufferSlice{trailing.data(), trailing.size()}, out_events));
    assert(out_events.empty());
    ++ring_fuzz_cases;
  }

  // Oversized count (0xFFFFFFFF) -> false without OOM.
  {
    const uint8_t huge_cnt[4] = {0xFF, 0xFF, 0xFF, 0xFF};
    assert(!nve::decode_event_ring(NveBufferSlice{huge_cnt, sizeof(huge_cnt)}, out_events));
    assert(out_events.empty());
    ++ring_fuzz_cases;
  }

  // 150 random EventRing buffers + 100 mutated copies of valid_ring_bytes.
  for (int i = 0; i < 150; ++i) {
    const size_t len = rand_range(1, 1024);
    std::vector<uint8_t> buf(len);
    for (auto& b : buf) b = rand_u8();
    const bool ok = nve::decode_event_ring(NveBufferSlice{buf.data(), buf.size()}, out_events);
    if (!ok) {
      assert(out_events.empty());
    }
    ++ring_fuzz_cases;
  }
  for (int i = 0; i < 100; ++i) {
    std::vector<uint8_t> mut = valid_ring_bytes;
    const size_t flips = rand_range(1, 4);
    for (size_t f = 0; f < flips; ++f) {
      mut[rng() % mut.size()] ^= static_cast<uint8_t>(1u << (rng() % 8));
    }
    const bool ok = nve::decode_event_ring(NveBufferSlice{mut.data(), mut.size()}, out_events);
    if (!ok) {
      assert(out_events.empty());
    }
    ++ring_fuzz_cases;
  }

  assert(ring_fuzz_cases >= 300);
}

// 9. Test nve_host_runtime.hpp utilities: unpack_rgba, current_timestamp_us, view_tag_for_snapshot,
//    and load_nve_dylib error codes.
void test_host_runtime_utilities_and_dylib_errors() {
  const uint64_t t1 = nve::current_timestamp_us();
  const uint64_t t2 = nve::current_timestamp_us();
  assert(t2 >= t1);

  // RGBA unpacking (little-endian: R low byte, A high byte)
  {
    const nve::RgbaColor clear = nve::unpack_rgba(0x00000000u);
    assert(clear.r8 == 0 && clear.g8 == 0 && clear.b8 == 0 && clear.a8 == 0);
    assert(approx_eq(clear.r, 0.0f, 1e-6f) && approx_eq(clear.a, 0.0f, 1e-6f));

    const nve::RgbaColor black = nve::unpack_rgba(0xFF000000u);
    assert(black.r8 == 0 && black.g8 == 0 && black.b8 == 0 && black.a8 == 255);
    assert(approx_eq(black.r, 0.0f, 1e-6f) && approx_eq(black.a, 1.0f, 1e-6f));

    const nve::RgbaColor white = nve::unpack_rgba(0xFFFFFFFFu);
    assert(white.r8 == 255 && white.g8 == 255 && white.b8 == 255 && white.a8 == 255);
    assert(approx_eq(white.r, 1.0f, 1e-6f) && approx_eq(white.g, 1.0f, 1e-6f) &&
           approx_eq(white.b, 1.0f, 1e-6f) && approx_eq(white.a, 1.0f, 1e-6f));

    const nve::RgbaColor sample = nve::unpack_rgba(0x80336699u);
    assert(sample.r8 == 0x99u && sample.g8 == 0x66u && sample.b8 == 0x33u && sample.a8 == 0x80u);
    assert(approx_eq(sample.r, 0x99 / 255.0f, 1e-5f));
    assert(approx_eq(sample.g, 0x66 / 255.0f, 1e-5f));
    assert(approx_eq(sample.b, 0x33 / 255.0f, 1e-5f));
    assert(approx_eq(sample.a, 0x80 / 255.0f, 1e-5f));
  }

  // View tags distinguish leaf text vs wrapper text
  {
    nve::RenderedNodeSnapshot leaf_text{};
    leaf_text.kind = NVE_COMP_TEXT;
    nve::RenderedNodeSnapshot wrapper_text{};
    wrapper_text.kind = NVE_COMP_TEXT;
    wrapper_text.children = {10};
    assert(nve::view_tag_for_snapshot(leaf_text) != nve::view_tag_for_snapshot(wrapper_text));

    nve::RenderedNodeSnapshot view_empty{};
    view_empty.kind = NVE_COMP_VIEW;
    nve::RenderedNodeSnapshot view_with_kids{};
    view_with_kids.kind = NVE_COMP_VIEW;
    view_with_kids.children = {10};
    assert(nve::view_tag_for_snapshot(view_empty) == nve::view_tag_for_snapshot(view_with_kids));
  }

  // load_nve_dylib error codes: empty path -> 1, missing file -> 1
  {
    nve::LoadedModule empty_res = nve::load_nve_dylib("");
    assert(!empty_res.ok());
    assert(empty_res.error_code == 1);
    assert(!empty_res.error_message.empty());
    assert(!empty_res.lib.is_open());

    nve::LoadedModule missing_res = nve::load_nve_dylib("/nonexistent/path/nve_missing_module.so");
    assert(!missing_res.ok());
    assert(missing_res.error_code == 1);
    assert(!missing_res.error_message.empty());
    assert(!missing_res.lib.is_open());
  }

#ifdef NVE_BRIDGE_FIXTURE_NO_ENTRY
  {
    nve::LoadedModule no_entry = nve::load_nve_dylib(NVE_BRIDGE_FIXTURE_NO_ENTRY);
    assert(!no_entry.ok());
    assert(no_entry.error_code == 2);
    assert(!no_entry.error_message.empty());
    assert(!no_entry.lib.is_open());
  }
#endif

#ifdef NVE_BRIDGE_FIXTURE_NO_SURFACE
  {
    nve::LoadedModule no_surface = nve::load_nve_dylib(NVE_BRIDGE_FIXTURE_NO_SURFACE);
    assert(!no_surface.ok());
    assert(no_surface.error_code == 3);
    assert(!no_surface.error_message.empty());
    assert(!no_surface.lib.is_open());
  }
#endif

#ifdef NVE_BRIDGE_FIXTURE_VALID
  {
    nve::LoadedModule valid = nve::load_nve_dylib(NVE_BRIDGE_FIXTURE_VALID);
    assert(valid.ok());
    assert(valid.error_code == 0);
    assert(valid.lib.is_open());
    assert(valid.get_interface != nullptr);
    assert(valid.module.surface != nullptr);
  }
#endif
}

struct MockReconciledView {
  uint32_t node_id{0};
  uint32_t instance_id{0};
  bool is_text_wrapper{false};
  std::string text;
  int update_count{0};
  std::vector<uint32_t> mounted_children;
};

struct MockReconcilerAdapter {
  uint32_t next_instance_id{1};
  int root_updates{0};
  uint32_t last_root_bg{0};
  int creates{0};
  std::vector<uint32_t> destroyed_nodes;
  std::vector<uint32_t> root_children;

  void update_root(const nve::RenderedNodeSnapshot& snap) {
    ++root_updates;
    last_root_bg = snap.background_rgba;
  }

  std::unique_ptr<MockReconciledView> create_view(const nve::RenderedNodeSnapshot& snap) {
    ++creates;
    auto view = std::make_unique<MockReconciledView>();
    view->node_id = snap.node_id;
    view->instance_id = next_instance_id++;
    view->is_text_wrapper = (snap.kind == NVE_COMP_TEXT && !snap.children.empty());
    return view;
  }

  void update_view(
      std::unique_ptr<MockReconciledView>& view,
      const nve::RenderedNodeSnapshot& snap) {
    assert(view != nullptr);
    view->text = snap.text;
    view->update_count += 1;
  }

  void destroy_view(uint32_t node_id, std::unique_ptr<MockReconciledView>& view) {
    assert(view != nullptr);
    assert(view->node_id == node_id);
    destroyed_nodes.push_back(node_id);
    view.reset();
  }

  template <typename LookupFn>
  void sync_children(
      uint32_t parent_id,
      std::unique_ptr<MockReconciledView>* parent_view,
      const std::vector<uint32_t>& child_ids,
      LookupFn&& lookup_fn) {
    std::vector<uint32_t> resolved;
    for (uint32_t cid : child_ids) {
      auto* child_ptr = lookup_fn(cid);
      if (child_ptr != nullptr && *child_ptr != nullptr) {
        resolved.push_back((*child_ptr)->node_id);
      }
    }
    assert(lookup_fn(999999u) == nullptr);
    if (parent_id == NVE_ROOT_NODE_ID) {
      assert(parent_view == nullptr);
      root_children = std::move(resolved);
    } else {
      assert(parent_view != nullptr && *parent_view != nullptr);
      (*parent_view)->mounted_children = std::move(resolved);
    }
  }
};

// 10. Test nve::RetainedViewReconciler across mount, update, text leaf<->wrapper tag swap,
//     child reordering, pruning, and clear.
void test_retained_view_reconciler() {
  nve::RetainedViewReconciler<std::unique_ptr<MockReconciledView>, MockReconcilerAdapter>
      reconciler;
  MockReconcilerAdapter adapter;

  auto make_snap = [](uint32_t id,
                      uint16_t kind,
                      std::string text = {},
                      std::vector<uint32_t> children = {},
                      uint32_t bg = 0) {
    nve::RenderedNodeSnapshot s{};
    s.node_id = id;
    s.kind = kind;
    s.text = std::move(text);
    s.children = std::move(children);
    s.background_rgba = bg;
    return s;
  };

  // Frame 1: initial mount (root -> [2(VIEW)], 2 -> [3(leaf TEXT), 4(PRESSABLE)])
  reconciler.sync(
      {
          make_snap(NVE_ROOT_NODE_ID, NVE_COMP_ROOT, "", {2}, 0xFF102030u),
          make_snap(2, NVE_COMP_VIEW, "", {3, 4}),
          make_snap(3, NVE_COMP_TEXT, "Leaf"),
          make_snap(4, NVE_COMP_PRESSABLE, ""),
      },
      adapter);
  assert(reconciler.size() == 3);
  assert(adapter.root_updates == 1);
  assert(adapter.last_root_bg == 0xFF102030u);
  assert(adapter.creates == 3);
  assert(adapter.destroyed_nodes.empty());
  assert(adapter.root_children == std::vector<uint32_t>({2}));
  assert(reconciler.find(2) != nullptr);
  assert((*reconciler.find(2))->mounted_children == std::vector<uint32_t>({3, 4}));
  assert(!(*reconciler.find(3))->is_text_wrapper);
  assert((*reconciler.find(3))->text == "Leaf");
  const uint32_t leaf_text_inst = (*reconciler.find(3))->instance_id;
  const uint32_t view2_inst = (*reconciler.find(2))->instance_id;

  // Frame 2: incremental update + child reordering (no view recreation!)
  reconciler.sync(
      {
          make_snap(NVE_ROOT_NODE_ID, NVE_COMP_ROOT, "", {2}, 0xFF102030u),
          make_snap(2, NVE_COMP_VIEW, "", {4, 3}),
          make_snap(3, NVE_COMP_TEXT, "Leaf updated"),
          make_snap(4, NVE_COMP_PRESSABLE, ""),
      },
      adapter);
  assert(adapter.creates == 3);
  assert(adapter.destroyed_nodes.empty());
  assert((*reconciler.find(2))->instance_id == view2_inst);
  assert((*reconciler.find(3))->instance_id == leaf_text_inst);
  assert((*reconciler.find(3))->text == "Leaf updated");
  assert((*reconciler.find(3))->update_count == 2);
  assert((*reconciler.find(2))->mounted_children == std::vector<uint32_t>({4, 3}));

  // Frame 3: text leaf -> wrapper tag swap (node 3 gains child 5) => destroys & recreates node 3
  reconciler.sync(
      {
          make_snap(NVE_ROOT_NODE_ID, NVE_COMP_ROOT, "", {2}),
          make_snap(2, NVE_COMP_VIEW, "", {4, 3}),
          make_snap(3, NVE_COMP_TEXT, "", {5}),
          make_snap(5, NVE_COMP_TEXT, "Nested span"),
          make_snap(4, NVE_COMP_PRESSABLE, ""),
      },
      adapter);
  assert(reconciler.size() == 4);
  assert(adapter.creates == 5); /* +1 recreated node 3, +1 new node 5 */
  assert(adapter.destroyed_nodes == std::vector<uint32_t>({3}));
  assert((*reconciler.find(3))->instance_id != leaf_text_inst);
  assert((*reconciler.find(3))->is_text_wrapper);
  assert((*reconciler.find(3))->mounted_children == std::vector<uint32_t>({5}));
  const uint32_t wrapper_text_inst = (*reconciler.find(3))->instance_id;

  // Frame 4: wrapper -> leaf tag swap + pruning node 4 and node 5
  reconciler.sync(
      {
          make_snap(NVE_ROOT_NODE_ID, NVE_COMP_ROOT, "", {2}),
          make_snap(2, NVE_COMP_VIEW, "", {3}),
          make_snap(3, NVE_COMP_TEXT, "Back to leaf"),
      },
      adapter);
  assert(reconciler.size() == 2);
  assert(reconciler.find(4) == nullptr);
  assert(reconciler.find(5) == nullptr);
  assert((*reconciler.find(3))->instance_id != wrapper_text_inst);
  assert(!(*reconciler.find(3))->is_text_wrapper);
  assert((*reconciler.find(3))->text == "Back to leaf");

  // Clear destroys remaining views
  reconciler.clear(adapter);
  assert(reconciler.size() == 0);
  assert(reconciler.empty());
  assert(reconciler.find(2) == nullptr);
  assert(reconciler.find(3) == nullptr);
}

// 11. Test nve::NativeHostSession (attach, custom text measurer re-applied across reload,
//     press dispatch, props update, atomic failed reload, and real dylib hot-reload).
void test_native_host_session() {
  NveSurfaceConfig cfg{.surface_id = 55, .width = 360.0f, .height = 240.0f, .scale_factor = 2.0f};
  nve::NativeHostSession session;
  assert(!session.is_valid());
  assert(session.bridge() == nullptr);
  assert(session.render() == -100);
  assert(session.dispatch_press(2, 5.0f, 5.0f) == 0);
  assert(session.drain_events().empty());
  assert(session.snapshot_nodes().empty());

  int measurer_calls = 0;
  session.set_text_measurer(
      [&](std::string_view text, float, float) -> NveIntrinsicSize {
        ++measurer_calls;
        return NveIntrinsicSize{static_cast<float>(text.size()) * 11.0f, 44.0f};
      });

  std::vector<nve::DecodedEvent> callback_events;
  session.set_event_callback(
      [&](const nve::DecodedEvent& ev) { callback_events.push_back(ev); });

  // Attach in-process interface (automatically installs text measurer & renders frame 1)
  assert(session.attach_interface(&MockEmbeddedRustComponent::get_interface, cfg));
  assert(session.is_valid());
  assert(session.bridge() != nullptr);
  assert(session.snapshot_nodes().size() == 3);
#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
  assert(measurer_calls >= 1);
  auto text_rect = session.bridge()->node_layout(3);
  assert(text_rect.has_value() && approx_eq(text_rect->height, 44.0f, 1.0f));
#endif

  // Update props via JSON -> emits onPropsApplied
  assert(session.update_props_json("{\"title\":\"HostSession\"}"));
  auto props_evs = session.drain_events();
  assert(props_evs.size() == 1);
  assert(props_evs[0].name == "onPropsApplied");
  assert(callback_events.size() == 1);

  // Dispatch press -> increments counter to 1 and emits onIncrement
  assert(session.dispatch_press(2, 12.0f, 18.0f) == 1);
  auto press_evs = session.drain_events();
  assert(press_evs.size() == 1);
  assert(press_evs[0].name == "onIncrement");
  assert(press_evs[0].payload_as_string() == "{\"count\":1}");
  assert(callback_events.size() == 2);

  // Resize surface
  session.set_surface_size(480.0f, 320.0f, 2.0f);
  assert(approx_eq(session.surface_config().width, 480.0f));
  assert(approx_eq(session.surface_config().height, 320.0f));

  // Failed reload_dylib must be atomic: returns false with code 1 and preserves active session/state
  int err_code = 0;
  std::string err_msg;
  assert(!session.reload_dylib("/nonexistent/bad_reload.dylib", &err_code, &err_msg));
  assert(err_code == 1);
  assert(!err_msg.empty());
  assert(session.is_valid());
  const auto preserved_state = session.bridge()->snapshot_state();
  assert(preserved_state.size() == 1 && preserved_state[0] == '1');

#ifdef NVE_BRIDGE_FIXTURE_VALID
  // Full real-dylib load_dylib + press + reload_dylib verifying state preservation AND
  // custom text measurer re-application across reload!
  {
    nve::NativeHostSession dylib_session;
    int dylib_measurer_calls = 0;
    dylib_session.set_text_measurer(
        [&](std::string_view text, float, float) -> NveIntrinsicSize {
          ++dylib_measurer_calls;
          return NveIntrinsicSize{static_cast<float>(text.size()) * 10.0f, 52.0f};
        });

    int load_err = -1;
    std::string load_msg;
    assert(dylib_session.load_dylib(NVE_BRIDGE_FIXTURE_VALID, cfg, &load_err, &load_msg));
    assert(load_err == 0 && load_msg.empty());
    assert(dylib_session.is_valid());
    assert(dylib_session.dispatch_press(2, 10.0f, 10.0f) == 1);
    assert(dylib_session.dispatch_press(2, 10.0f, 10.0f) == 1);
    assert(dylib_session.update_props_json("{\"v\":1}"));

    const int calls_before_reload = dylib_measurer_calls;
    int reload_err = -1;
    std::string reload_msg;
    assert(dylib_session.reload_dylib(NVE_BRIDGE_FIXTURE_VALID, &reload_err, &reload_msg));
    assert(reload_err == 0 && reload_msg.empty());
    assert(dylib_session.is_valid());

    // State ("2") must survive reload_dylib, and text node 3 reflects "Fixture:2"
    const auto reloaded_snaps = dylib_session.snapshot_nodes();
    bool saw_preserved_text = false;
    for (const auto& s : reloaded_snaps) {
      if (s.node_id == 3 && s.text == "Fixture:2") {
        saw_preserved_text = true;
      }
    }
    assert(saw_preserved_text);

#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
    // Custom text measurer must have been re-applied on the reloaded bridge
    assert(dylib_measurer_calls > calls_before_reload);
    auto reloaded_rect = dylib_session.bridge()->node_layout(3);
    assert(reloaded_rect.has_value() && approx_eq(reloaded_rect->height, 52.0f, 1.0f));
#else
    (void)calls_before_reload;
#endif
  }
#endif
}

} // namespace

int main() {
  test_all_opcodes_and_custom_components();
  test_corrupt_buffer_error_codes();
  test_flexbox_layout();
  test_rn_component_bridge();
#if defined(NVE_WITH_RN_JSI) && NVE_WITH_RN_JSI
  test_mock_jsi_recorder_runtime();
#endif
  test_text_style_inheritance();
  test_text_measurer_callback();
  test_seeded_fuzz_corpus();
  test_host_runtime_utilities_and_dylib_errors();
  test_retained_view_reconciler();
  test_native_host_session();

  std::cout << "nve_fabric_bridge_test passed ("
            << (nve::fabric_host_has_yoga_layout() ? "Yoga+JSI" : "Standalone")
            << " mode)\n";
  return 0;
}
