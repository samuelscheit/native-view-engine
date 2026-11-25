#if defined(_WIN32)
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <winsock2.h>
#include <ws2tcpip.h>
#include <windows.h>
#else
#include <arpa/inet.h>
#include <netinet/in.h>
#include <poll.h>
#include <sys/socket.h>
#include <unistd.h>
#endif

#include "NveRnwComponentView.hpp"
#include "NveWinHostWindow.hpp"
#include "NveWinWindowManager.hpp"
#include "NveWinTextMeasurer.hpp"
#include "command_encoder.hpp"
#include "mock_jsi_recorder.hpp"
#include "nve_pair_client.hpp"
#include "nve_scene_builder.hpp"

#include <atomic>
#include <chrono>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <initializer_list>
#include <iostream>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

#include <span>
#include <string_view>

// Mock modules treat the forwarded hot-patch frame as opaque bytes (hosts never decode NVP1).
[[maybe_unused]] static std::span<const uint8_t> nve_test_patch(std::string_view text) {
  return {reinterpret_cast<const uint8_t*>(text.data()), text.size()};
}

#define NVE_CHECK(expr)                                                                         \
  do {                                                                                          \
    if (!(expr)) {                                                                              \
      std::cerr << "NVE_CHECK failed at " << __FILE__ << ":" << __LINE__ << ": " #expr << "\n"; \
      std::abort();                                                                             \
    }                                                                                           \
  } while (0)

namespace {

bool approx_eq(float a, float b, float eps = 1.5f) {
  return std::fabs(a - b) <= eps;
}

std::string get_env_string(const char* name) {
#if defined(_WIN32)
  char* buf = nullptr;
  size_t len = 0;
  if (_dupenv_s(&buf, &len, name) != 0 || buf == nullptr) {
    return {};
  }
  std::string out(buf);
  std::free(buf);
  return out;
#else
  const char* val = std::getenv(name);
  return (val != nullptr) ? std::string(val) : std::string{};
#endif
}

// Mock Rust component exposing NVE_IFACE_SURFACE + NVE_IFACE_COMPONENT + NVE_IFACE_DEV +
// NVE_IFACE_LAYOUT to test NveRnwComponentHost, props JSON, outbound events, z-order hit testing,
// and NVE_COMP_TEXT wrapper <-> leaf tag transitions.
struct MockWinRustComponent {
  uint32_t surface_id{1};
  int count{0};
  std::string title{"Initial"};
  bool wrap_text_node{false};
  std::vector<uint8_t> cmd_buf;
  nve::EventRingWriter outbound;
  std::vector<uint8_t> outbound_snap;

  static NveAppHandle create(uint32_t sid) {
    auto* self = new MockWinRustComponent();
    self->surface_id = sid;
    self->outbound.clear();
    return self;
  }

  static void destroy(NveAppHandle h) { delete static_cast<MockWinRustComponent*>(h); }

  template <typename T>
  void push(T v) {
    const auto* p = reinterpret_cast<const uint8_t*>(&v);
    cmd_buf.insert(cmd_buf.end(), p, p + sizeof(T));
  }

  void push_dim(uint8_t tag, float val) {
    push<uint8_t>(tag);
    push<float>(val);
  }

  void push_insets(float t, float r, float b, float l) {
    push<float>(t);
    push<float>(r);
    push<float>(b);
    push<float>(l);
  }

  static NveBufferSlice render(NveAppHandle h, uint64_t frame_id) {
    auto* self = static_cast<MockWinRustComponent*>(h);
    self->cmd_buf.clear();
    uint32_t ops = 0;

    self->push<uint32_t>(NVE_ABI_MAGIC);
    self->push<uint16_t>(NVE_ABI_VERSION);
    self->push<uint16_t>(0);
    self->push<uint32_t>(self->surface_id);
    self->push<uint64_t>(frame_id);
    self->push<uint32_t>(0);

    auto create_node = [&](uint32_t id, uint16_t kind) {
      self->push<uint8_t>(NVE_OP_CREATE_NODE);
      self->push<uint32_t>(id);
      self->push<uint16_t>(kind);
      self->push<uint16_t>(0);
      ++ops;
    };

    auto set_children = [&](uint32_t parent, std::initializer_list<uint32_t> kids) {
      self->push<uint8_t>(NVE_OP_SET_CHILDREN);
      self->push<uint32_t>(parent);
      self->push<uint32_t>(static_cast<uint32_t>(kids.size()));
      for (uint32_t k : kids) {
        self->push<uint32_t>(k);
      }
      ++ops;
    };

    auto set_text = [&](uint32_t id, const std::string& s) {
      self->push<uint8_t>(NVE_OP_SET_TEXT);
      self->push<uint32_t>(id);
      self->push<uint32_t>(static_cast<uint32_t>(s.size()));
      self->cmd_buf.insert(self->cmd_buf.end(), s.begin(), s.end());
      ++ops;
    };

    auto set_layout = [&](uint32_t id,
                          uint8_t dir,
                          uint8_t wtag,
                          float w,
                          uint8_t htag,
                          float h,
                          float grow,
                          float pad,
                          float gap) {
      self->push<uint8_t>(NVE_OP_SET_LAYOUT);
      self->push<uint32_t>(id);
      self->push_dim(wtag, w);
      self->push_dim(htag, h);
      self->push_dim(0, 0.0f);
      self->push_dim(0, 0.0f);
      self->push_dim(0, 0.0f);
      self->push_dim(0, 0.0f);
      self->push<uint8_t>(dir);
      self->push<uint8_t>(0);
      self->push<uint8_t>(0);
      self->push<float>(grow);
      self->push<float>(1.0f);
      self->push<float>(gap);
      self->push_insets(pad, pad, pad, pad);
      self->push_insets(0, 0, 0, 0);
      ++ops;
    };

    auto set_style = [&](uint32_t id,
                         uint32_t bg,
                         uint32_t fg,
                         uint32_t border,
                         float bw,
                         float radius,
                         float opacity,
                         float font_size,
                         int32_t z_index) {
      self->push<uint8_t>(NVE_OP_SET_STYLE);
      self->push<uint32_t>(id);
      self->push<uint32_t>(bg);
      self->push<uint32_t>(fg);
      self->push<uint32_t>(border);
      self->push<float>(bw);
      self->push<float>(radius);
      self->push<float>(opacity);
      self->push<float>(font_size);
      self->push<int32_t>(z_index);
      ++ops;
    };

    create_node(2, NVE_COMP_VIEW);
    create_node(3, NVE_COMP_TEXT);
    create_node(4, NVE_COMP_PRESSABLE);
    if (self->wrap_text_node) {
      create_node(5, NVE_COMP_TEXT);
    }

    set_children(NVE_ROOT_NODE_ID, {2});
    set_children(2, {3, 4});
    if (self->wrap_text_node) {
      set_children(3, {5});
    } else {
      set_children(3, {});
    }

    // Row container: 100% x 100%, pad=20, gap=10
    set_layout(2, 1, 2, 100.0f, 2, 100.0f, 0.0f, 20.0f, 10.0f);
    set_style(2, 0xFF202020u, 0xFFFFFFFFu, 0xFF404040u, 1.0f, 8.0f, 1.0f, 16.0f, 0);

    // Label node 3: fixed 120x40
    set_layout(3, 0, 1, 120.0f, 1, 40.0f, 0.0f, 0.0f, 0.0f);
    set_style(3, 0x00000000u, 0xFFFFFFFFu, 0u, 0.0f, 0.0f, 1.0f, 18.0f, 1);
    if (self->wrap_text_node) {
      set_text(5, self->title + ":" + std::to_string(self->count));
    } else {
      set_text(3, self->title + ":" + std::to_string(self->count));
    }

    // Pressable node 4: flex_grow=1, height=40
    set_layout(4, 0, 0, 0.0f, 1, 40.0f, 1.0f, 0.0f, 0.0f);
    set_style(4, 0xFFEB6325u, 0xFFFFFFFFu, 0xFFFFFFFFu, 1.5f, 12.0f, 0.9f, 16.0f, 2);

    self->push<uint8_t>(NVE_OP_COMMIT);
    self->push<uint8_t>(1);
    ++ops;

    std::memcpy(self->cmd_buf.data() + 20, &ops, sizeof(uint32_t));
    return NveBufferSlice{self->cmd_buf.data(), self->cmd_buf.size()};
  }

  static uint32_t dispatch_events(NveAppHandle h, NveBufferSlice events) {
    auto* self = static_cast<MockWinRustComponent*>(h);
    std::vector<nve::DecodedEvent> decoded;
    if (!nve::decode_event_ring(events, decoded)) {
      return 0;
    }
    for (const auto& ev : decoded) {
      if (ev.kind == nve::EventKind::Press && ev.node_id == 4) {
        ++self->count;
        const std::string payload = "{\"count\":" + std::to_string(self->count) + "}";
        self->outbound.push_custom(
            self->surface_id,
            4,
            ev.timestamp_us,
            "onIncrement",
            reinterpret_cast<const uint8_t*>(payload.data()),
            payload.size());
        return 1;
      }
    }
    return 0;
  }

  static NveIntrinsicSize measure(NveAppHandle, NveSizeConstraints c) {
    const float w = std::clamp(400.0f, c.min_w, c.max_w > 0.0f ? c.max_w : 400.0f);
    const float h = std::clamp(160.0f, c.min_h, c.max_h > 0.0f ? c.max_h : 160.0f);
    return NveIntrinsicSize{w, h};
  }

  static uint8_t hit_test(NveAppHandle, float, float, uint8_t) {
    return static_cast<uint8_t>(NVE_GESTURE_CLAIM);
  }

  static uint32_t set_props(NveAppHandle h, NveBufferSlice props) {
    auto* self = static_cast<MockWinRustComponent*>(h);
    const std::string json(reinterpret_cast<const char*>(props.ptr), props.len);
    if (json.find("Updated") != std::string::npos) {
      self->title = "Updated";
    }
    if (json.find("\"wrap\":true") != std::string::npos) {
      self->wrap_text_node = true;
    } else if (json.find("\"wrap\":false") != std::string::npos) {
      self->wrap_text_node = false;
    }
    const std::string payload = "{\"ok\":true}";
    self->outbound.push_custom(
        self->surface_id,
        NVE_ROOT_NODE_ID,
        500,
        "onPropsApplied",
        reinterpret_cast<const uint8_t*>(payload.data()),
        payload.size());
    return 1;
  }

  static NveBufferSlice drain_emitted_events(NveAppHandle h) {
    auto* self = static_cast<MockWinRustComponent*>(h);
    self->outbound_snap = self->outbound.finalize();
    self->outbound.clear();
    return NveBufferSlice{self->outbound_snap.data(), self->outbound_snap.size()};
  }

  static uint32_t apply_hot_patch(NveAppHandle h, NveBufferSlice patch) {
    auto* self = static_cast<MockWinRustComponent*>(h);
    self->title.assign(reinterpret_cast<const char*>(patch.ptr), patch.len);
    return 1;
  }

  static NveBufferSlice snapshot_state(NveAppHandle) { return {nullptr, 0}; }
  static void restore_state(NveAppHandle, NveBufferSlice) {}

  static const void* get_interface(uint32_t interface_id) {
    static const NveSurfaceApi kSurface{
        .create = &create,
        .destroy = &destroy,
        .render = &render,
        .dispatch_events = &dispatch_events,
        .dispatch_events_direct = nullptr,
    };
    static const NveLayoutApi kLayout{
        .measure = &measure,
        .hit_test = &hit_test,
    };
    static const NveComponentApi kComponent{
        .set_props = &set_props,
        .drain_emitted_events = &drain_emitted_events,
        .set_props_direct = nullptr,
    };
    static const NveDevApi kDev{
        .apply_hot_patch = &apply_hot_patch,
        .snapshot_state = &snapshot_state,
        .restore_state = &restore_state,
    };
    switch (interface_id) {
      case NVE_IFACE_SURFACE:
        return &kSurface;
      case NVE_IFACE_LAYOUT:
        return &kLayout;
      case NVE_IFACE_COMPONENT:
        return &kComponent;
      case NVE_IFACE_DEV:
        return &kDev;
      default:
        return nullptr;
    }
  }
};

void test_directwrite_text_measurer() {
  nve::win::DirectWriteTextMeasurer measurer;
  NVE_CHECK(measurer.is_valid());

  // Empty string -> exact 0x0.
  const NveIntrinsicSize empty = measurer.measure("", 16.0f, 0.0f);
  NVE_CHECK(empty.width == 0.0f);
  NVE_CHECK(empty.height == 0.0f);

  // Non-empty single line -> strictly positive dimensions.
  const NveIntrinsicSize title = measurer.measure("NATIVE VIEW ENGINE", 14.0f, 0.0f);
  NVE_CHECK(title.width > 40.0f);
  NVE_CHECK(title.height >= 14.0f);

  // Larger font size increases measured height and width.
  const NveIntrinsicSize small_count = measurer.measure("Count: 0", 14.0f, 0.0f);
  const NveIntrinsicSize large_count = measurer.measure("Count: 0", 28.0f, 0.0f);
  NVE_CHECK(large_count.height > small_count.height);
  NVE_CHECK(large_count.width > small_count.width);

  // Wrapping under a tight max_width increases height relative to unconstrained text.
  const std::string paragraph =
      "The quick brown fox jumps over the lazy dog inside Native View Engine";
  const NveIntrinsicSize unwrapped = measurer.measure(paragraph, 16.0f, 0.0f);
  const NveIntrinsicSize wrapped = measurer.measure(paragraph, 16.0f, 90.0f);
  NVE_CHECK(unwrapped.width > 120.0f);
  NVE_CHECK(wrapped.width > 0.0f && wrapped.width <= 90.5f);
  NVE_CHECK(wrapped.height > unwrapped.height);

  // Non-positive / NaN font size falls back cleanly to 16pt.
  const NveIntrinsicSize fallback = measurer.measure("Fallback", 0.0f, 0.0f);
  NVE_CHECK(fallback.width > 10.0f && fallback.height >= 14.0f);

  // Move semantics transfer ownership cleanly without leaks or crashes.
  nve::win::DirectWriteTextMeasurer moved(std::move(measurer));
  NVE_CHECK(moved.is_valid());
  const NveIntrinsicSize after_move = moved.measure("AfterMove", 16.0f, 0.0f);
  NVE_CHECK(after_move.width > 10.0f && after_move.height >= 14.0f);
}

void test_visual_tree_reconciler_and_hit_testing() {
  nve::win::NveWinVisualTree tree;
  nve::SceneTableBuilder builder;

  nve::RenderedNodeSnapshot root{};
  root.node_id = NVE_ROOT_NODE_ID;
  root.kind = NVE_COMP_ROOT;
  root.layout = {0.0f, 0.0f, 400.0f, 300.0f};
  root.background_rgba = 0xFF101010u;
  root.children = {10, 20, 30};

  // Overlapping pressables: node 10 has higher z_index (5) than node 20 (z_index 1).
  nve::RenderedNodeSnapshot btn_top{};
  btn_top.node_id = 10;
  btn_top.kind = NVE_COMP_PRESSABLE;
  btn_top.layout = {20.0f, 20.0f, 100.0f, 50.0f};
  btn_top.z_index = 5;
  btn_top.children = {11};

  // Non-interactive leaf text inside btn_top.
  nve::RenderedNodeSnapshot btn_label{};
  btn_label.node_id = 11;
  btn_label.kind = NVE_COMP_TEXT;
  btn_label.text = "Top";
  btn_label.layout = {10.0f, 10.0f, 60.0f, 20.0f};

  nve::RenderedNodeSnapshot btn_under{};
  btn_under.node_id = 20;
  btn_under.kind = NVE_COMP_PRESSABLE;
  btn_under.layout = {20.0f, 20.0f, 100.0f, 50.0f};
  btn_under.z_index = 1;

  // Transparent interactive node (opacity 0) must be ignored during hit-testing.
  nve::RenderedNodeSnapshot btn_hidden{};
  btn_hidden.node_id = 30;
  btn_hidden.kind = NVE_COMP_PRESSABLE;
  btn_hidden.layout = {20.0f, 20.0f, 100.0f, 50.0f};
  btn_hidden.z_index = 10;
  btn_hidden.opacity = 0.0f;

  builder.sync(1u, {root, btn_top, btn_label, btn_under, btn_hidden});
  NVE_CHECK(tree.sync(builder.view()));
  NVE_CHECK(tree.size() == 4);
  NVE_CHECK(tree.created_count() == 4);
  NVE_CHECK(tree.destroyed_count() == 0);
  NVE_CHECK(tree.find(11) != nullptr && tree.find(11)->is_text_leaf);

  // Verify draw order respects z_index ascending: 20 (z=1), 10 (z=5), 11 (child of 10), 30 (z=10).
  const auto order = tree.build_draw_order();
  NVE_CHECK(order.size() == 4);
  NVE_CHECK(order[0].node_id == 20);
  NVE_CHECK(order[1].node_id == 10);
  NVE_CHECK(order[2].node_id == 11);
  NVE_CHECK(order[3].node_id == 30);

  // Hit-testing inside label 11 (surface 45, 35) skips opacity-0 node 30 and non-interactive
  // label 11, hitting topmost visible pressable node 10 with local coords (25, 15).
  const auto hit = tree.hit_test_interactive(45.0f, 35.0f);
  NVE_CHECK(hit.has_value());
  NVE_CHECK(hit->node_id == 10);
  NVE_CHECK(approx_eq(hit->local_x, 25.0f));
  NVE_CHECK(approx_eq(hit->local_y, 15.0f));

  // Verify absolute_rect accumulates parent layout offset.
  const auto abs_label = tree.absolute_rect(11);
  NVE_CHECK(abs_label.has_value());
  NVE_CHECK(approx_eq(abs_label->x, 30.0f));
  NVE_CHECK(approx_eq(abs_label->y, 30.0f));

  // Incremental O(changed) sync: mutating ONLY node 11 updates ONLY 1 visual node!
  const uint64_t created_before = tree.created_count();
  const uint64_t updated_before = tree.updated_count();
  btn_label.text = "TopUpdated";
  builder.sync(2u, {root, btn_top, btn_label, btn_under, btn_hidden});
  NVE_CHECK(tree.sync(builder.view()));
  NVE_CHECK(tree.created_count() == created_before);
  NVE_CHECK(tree.updated_count() == updated_before + 1);
  NVE_CHECK(tree.find(11)->text == "TopUpdated");

  // Transition node 11 from leaf NVE_COMP_TEXT to wrapper NVE_COMP_TEXT (children = {12}).
  // IncrementalSceneReconciler must detect view_tag_for_render_node change and recreate node 11!
  nve::RenderedNodeSnapshot nested_leaf{};
  nested_leaf.node_id = 12;
  nested_leaf.kind = NVE_COMP_TEXT;
  nested_leaf.text = "NestedLeaf";
  nested_leaf.layout = {0.0f, 0.0f, 50.0f, 18.0f};
  btn_label.text.clear();
  btn_label.children = {12};

  const uint64_t destroyed_before = tree.destroyed_count();
  builder.sync(3u, {root, btn_top, btn_label, nested_leaf, btn_under, btn_hidden});
  NVE_CHECK(tree.sync(builder.view()));
  NVE_CHECK(tree.size() == 5);
  NVE_CHECK(tree.destroyed_count() == destroyed_before + 1);
  NVE_CHECK(tree.created_count() == created_before + 2);
  NVE_CHECK(!tree.find(11)->is_text_leaf);
  NVE_CHECK((tree.find(11)->view_tag & 0x10000u) != 0u);
  NVE_CHECK(tree.find(12)->is_text_leaf);
  NVE_CHECK((tree.find(12)->view_tag & 0x10000u) == 0u);
}

void test_rnw_component_host_and_dylib_lifecycle() {
  nve::win::NveRnwComponentHost host;
  std::vector<nve::DecodedEvent> received_events;
  int visual_notifications = 0;

  host.set_event_callback([&](const nve::DecodedEvent& ev) { received_events.push_back(ev); });
  host.set_on_visual_tree_changed([&]() { ++visual_notifications; });

  const NveSurfaceConfig cfg{.surface_id = 9, .width = 400.0f, .height = 160.0f, .scale_factor = 1.0f};
  NVE_CHECK(host.attach_interface(&MockWinRustComponent::get_interface, cfg));
  NVE_CHECK(host.is_valid());
  NVE_CHECK(host.surface_id() == 9);
  NVE_CHECK(host.node_count() == 4);
  NVE_CHECK(visual_notifications >= 1);

  const auto* container = host.visual_tree().find(2);
  const auto* label = host.visual_tree().find(3);
  const auto* button = host.visual_tree().find(4);
  NVE_CHECK(container != nullptr && label != nullptr && button != nullptr);
  NVE_CHECK(label->is_text_leaf);
  NVE_CHECK(label->text == "Initial:0");
  NVE_CHECK(button->interactive);
  NVE_CHECK(approx_eq(container->layout.width, 400.0f));
  NVE_CHECK(approx_eq(label->layout.x, 20.0f));
  NVE_CHECK(approx_eq(label->layout.width, 120.0f));
  NVE_CHECK(approx_eq(button->layout.x, 150.0f));
  NVE_CHECK(approx_eq(button->layout.width, 230.0f));

  // Measure via NVE_IFACE_LAYOUT.
  const auto measured = host.measure(NveSizeConstraints{0.0f, 0.0f, 640.0f, 480.0f});
  NVE_CHECK(measured.has_value());
  NVE_CHECK(approx_eq(measured->width, 400.0f));
  NVE_CHECK(approx_eq(measured->height, 160.0f));

  // Props JSON update -> re-render + outbound event.
  NVE_CHECK(host.update_props_json("{\"title\":\"Updated\"}"));
  NVE_CHECK(host.visual_tree().find(3)->text == "Updated:0");
  NVE_CHECK(received_events.size() == 1);
  NVE_CHECK(received_events[0].name == "onPropsApplied");

  // Click button via surface coordinates (inside 150..380, 20..60).
  NVE_CHECK(host.dispatch_click_at(200.0f, 35.0f) != 0);
  NVE_CHECK(host.visual_tree().find(3)->text == "Updated:1");
  NVE_CHECK(received_events.size() == 2);
  NVE_CHECK(received_events[1].name == "onIncrement");
  NVE_CHECK(received_events[1].payload_as_string() == "{\"count\":1}");

  // Resize surface -> flex_grow button expands to 330px.
  host.set_surface_size(500.0f, 160.0f, 1.5f);
  NVE_CHECK(approx_eq(host.visual_tree().find(4)->layout.width, 330.0f));

  // Transition node 3 from leaf text to wrapper text via props -> reconciler swaps tag cleanly.
  NVE_CHECK(host.update_props_json("{\"wrap\":true}"));
  NVE_CHECK(!host.visual_tree().find(3)->is_text_leaf);
  NVE_CHECK(host.visual_tree().find(5) != nullptr && host.visual_tree().find(5)->is_text_leaf);
  NVE_CHECK(host.visual_tree().find(5)->text == "Updated:1");

  // Shared error codes 1 / 2 / 3 on load_dylib and reload_dylib.
  int err_code = 0;
  std::string err_msg;

  // Code 1: missing and empty path.
  NVE_CHECK(!host.load_dylib("/nonexistent/missing_nve_app.dll", cfg, &err_code, &err_msg));
  NVE_CHECK(err_code == 1 && !err_msg.empty());

  NVE_CHECK(!host.load_dylib("", cfg, &err_code, &err_msg));
  NVE_CHECK(err_code == 1 && !err_msg.empty());

  // Code 2: valid DLL missing `nve_get_interface`.
  NVE_CHECK(!host.load_dylib(NVE_WIN_TEST_NO_ENTRY_DYLIB, cfg, &err_code, &err_msg));
  NVE_CHECK(err_code == 2 && !err_msg.empty());

  // Code 3: exports `nve_get_interface` but missing `NVE_IFACE_SURFACE`.
  NVE_CHECK(!host.load_dylib(NVE_WIN_TEST_NO_SURFACE_DYLIB, cfg, &err_code, &err_msg));
  NVE_CHECK(err_code == 3 && !err_msg.empty());

  // Prior live app must remain intact after failed loads!
  NVE_CHECK(host.is_valid());
  NVE_CHECK(host.visual_tree().find(5)->text == "Updated:1");

  // Stateful DLL fixture lifecycle: cold reload -> mutate -> failed reload preserves state ->
  // warm reload preserves state -> fresh load resets state.
  nve::win::NveRnwComponentHost reload_host;
  NVE_CHECK(!reload_host.is_valid());
  NVE_CHECK(reload_host.load_dylib(NVE_WIN_TEST_STATEFUL_DYLIB, cfg, &err_code, &err_msg));
  NVE_CHECK(err_code == 0 && err_msg.empty());
  NVE_CHECK(reload_host.visual_tree().find(2)->text == "state=0");

  NVE_CHECK(reload_host.apply_hot_patch(nve_test_patch("77")));
  NVE_CHECK(reload_host.visual_tree().find(2)->text == "state=77");

  // Failed reload codes 1/2/3 must preserve state=77.
  NVE_CHECK(!reload_host.reload_dylib("/nonexistent/fail.dll", &err_code, &err_msg));
  NVE_CHECK(err_code == 1);
  NVE_CHECK(reload_host.visual_tree().find(2)->text == "state=77");

  NVE_CHECK(!reload_host.reload_dylib(NVE_WIN_TEST_NO_ENTRY_DYLIB, &err_code, &err_msg));
  NVE_CHECK(err_code == 2);
  NVE_CHECK(reload_host.visual_tree().find(2)->text == "state=77");

  NVE_CHECK(!reload_host.reload_dylib(NVE_WIN_TEST_NO_SURFACE_DYLIB, &err_code, &err_msg));
  NVE_CHECK(err_code == 3);
  NVE_CHECK(reload_host.visual_tree().find(2)->text == "state=77");

  // Warm reload preserves state=77 via snapshot_state / restore_state.
  NVE_CHECK(reload_host.reload_dylib(NVE_WIN_TEST_STATEFUL_DYLIB, &err_code, &err_msg));
  NVE_CHECK(err_code == 0);
  NVE_CHECK(reload_host.visual_tree().find(2)->text == "state=77");

  // Press increments to 78, props updates prefix.
  NVE_CHECK(reload_host.dispatch_press(3, 5.0f, 5.0f) != 0);
  NVE_CHECK(reload_host.visual_tree().find(2)->text == "state=78");
  NVE_CHECK(reload_host.update_props_json("{\"prefix\":\"Updated\"}"));
  NVE_CHECK(reload_host.visual_tree().find(2)->text == "Updated=78");

  // Fresh load resets counter back to 0.
  NVE_CHECK(reload_host.load_dylib(NVE_WIN_TEST_STATEFUL_DYLIB, cfg, &err_code, &err_msg));
  NVE_CHECK(err_code == 0);
  NVE_CHECK(reload_host.visual_tree().find(2)->text == "state=0");

#if defined(_WIN32)
  // Verify headless Direct2D/DirectWrite BMP rendering on Windows.
  const std::string bmp_path = "nve_windows_shell_unit_test.bmp";
  std::string bmp_err;
  NVE_CHECK(nve::win::render_visual_tree_to_bmp(
      host.visual_tree(), *host.text_measurer(), 400, 160, 1.0f, bmp_path, &bmp_err));
  {
    std::ifstream in(bmp_path, std::ios::binary | std::ios::ate);
    NVE_CHECK(in.is_open());
    const auto sz = in.tellg();
    NVE_CHECK(sz == static_cast<std::streamoff>(54 + 400 * 160 * 4));
    in.seekg(0);
    char magic[2]{};
    in.read(magic, 2);
    NVE_CHECK(magic[0] == 'B' && magic[1] == 'M');
  }
  (void)std::remove(bmp_path.c_str());
#endif
}

void test_live_counter_dylib_if_configured() {
  const std::string dylib_env = get_env_string("NVE_TEST_COUNTER_DYLIB");
  if (dylib_env.empty()) {
    return;
  }

  nve::win::NveRnwComponentHost host;
  const NveSurfaceConfig cfg{.surface_id = 1, .width = 640.0f, .height = 420.0f, .scale_factor = 1.0f};
  int err_code = 0;
  std::string err_msg;
  NVE_CHECK(host.load_dylib(dylib_env, cfg, &err_code, &err_msg));
  NVE_CHECK(err_code == 0);

  auto find_leaf_by_text = [&](std::string_view expected) -> const nve::win::NveWinNodeVisual* {
    for (const auto& flat : host.visual_tree().build_draw_order()) {
      const auto* node = host.visual_tree().find(flat.node_id);
      if (node != nullptr && node->is_text_leaf && node->text == expected) {
        return node;
      }
    }
    return nullptr;
  };

  auto find_pressable_ancestor = [&](uint32_t node_id) -> const nve::win::NveWinNodeVisual* {
    const auto* cur = host.visual_tree().find(node_id);
    uint32_t hops = 0;
    while (cur != nullptr && cur->parent_id != 0 && cur->parent_id != NVE_ROOT_NODE_ID &&
           hops < 64) {
      const auto* parent = host.visual_tree().find(cur->parent_id);
      if (parent == nullptr) {
        break;
      }
      if (parent->interactive) {
        return parent;
      }
      cur = parent;
      ++hops;
    }
    return nullptr;
  };

  // Assert all 4 counter labels exist, have non-zero frames, and opaque text color.
  const auto* lbl_title = find_leaf_by_text("NATIVE VIEW ENGINE");
  const auto* lbl_count = find_leaf_by_text("Count: 0");
  const auto* lbl_inc = find_leaf_by_text("Increment +");
  const auto* lbl_reset = find_leaf_by_text("Reset");
  NVE_CHECK(lbl_title != nullptr);
  NVE_CHECK(lbl_count != nullptr);
  NVE_CHECK(lbl_inc != nullptr);
  NVE_CHECK(lbl_reset != nullptr);

  for (const auto* lbl : {lbl_title, lbl_count, lbl_inc, lbl_reset}) {
    NVE_CHECK(lbl->layout.width > 20.0f);
    NVE_CHECK(lbl->layout.height >= 14.0f);
    const nve::RgbaColor fg = nve::unpack_rgba(lbl->text_color_rgba);
    NVE_CHECK(fg.a8 == 255);
  }
  NVE_CHECK(approx_eq(lbl_count->font_size, 28.0f, 0.5f));

  const auto* inc_btn = find_pressable_ancestor(lbl_inc->node_id);
  const auto* reset_btn = find_pressable_ancestor(lbl_reset->node_id);
  NVE_CHECK(inc_btn != nullptr && reset_btn != nullptr);

  if (nve::fabric_host_has_yoga_layout()) {
    // Assert top card (rounded opaque view) has non-zero frame.
    bool saw_top_card = false;
    for (const auto& flat : host.visual_tree().build_draw_order()) {
      const auto* node = host.visual_tree().find(flat.node_id);
      if (node != nullptr && node->kind == NVE_COMP_VIEW && node->border_radius >= 15.0f) {
        const nve::RgbaColor bg = nve::unpack_rgba(node->background_rgba);
        if (bg.a8 == 255 && node->layout.width > 100.0f && node->layout.height > 40.0f) {
          saw_top_card = true;
        }
      }
    }
    NVE_CHECK(saw_top_card);

    // Assert centered button labels inside Increment + and Reset buttons.
    auto assert_centered = [&](const nve::win::NveWinNodeVisual* btn,
                               const nve::win::NveWinNodeVisual* lbl) {
      const auto btn_rect = host.visual_tree().absolute_rect(btn->node_id);
      const auto lbl_rect = host.visual_tree().absolute_rect(lbl->node_id);
      NVE_CHECK(btn_rect.has_value() && lbl_rect.has_value());
      const float btn_cx = btn_rect->x + btn_rect->width * 0.5f;
      const float btn_cy = btn_rect->y + btn_rect->height * 0.5f;
      const float lbl_cx = lbl_rect->x + lbl_rect->width * 0.5f;
      const float lbl_cy = lbl_rect->y + lbl_rect->height * 0.5f;
      NVE_CHECK(approx_eq(lbl_cx, btn_cx, 3.0f));
      NVE_CHECK(approx_eq(lbl_cy, btn_cy, 3.0f));
    };
    assert_centered(inc_btn, lbl_inc);
    assert_centered(reset_btn, lbl_reset);

    // Click Increment + via surface coordinates -> Count: 1.
    const auto inc_rect = host.visual_tree().absolute_rect(inc_btn->node_id);
    const auto reset_rect = host.visual_tree().absolute_rect(reset_btn->node_id);
    NVE_CHECK(inc_rect.has_value() && reset_rect.has_value());

    NVE_CHECK(
        host.dispatch_click_at(
            inc_rect->x + inc_rect->width * 0.5f, inc_rect->y + inc_rect->height * 0.5f) != 0);
    NVE_CHECK(find_leaf_by_text("Count: 1") != nullptr);

    // Click Reset via surface coordinates -> Count: 0.
    NVE_CHECK(
        host.dispatch_click_at(
            reset_rect->x + reset_rect->width * 0.5f, reset_rect->y + reset_rect->height * 0.5f) !=
        0);
    NVE_CHECK(find_leaf_by_text("Count: 0") != nullptr);

    // Advance count to 2, then verify state-preserving hot-reload.
    NVE_CHECK(
        host.dispatch_click_at(
            inc_rect->x + inc_rect->width * 0.5f, inc_rect->y + inc_rect->height * 0.5f) != 0);
    NVE_CHECK(
        host.dispatch_click_at(
            inc_rect->x + inc_rect->width * 0.5f, inc_rect->y + inc_rect->height * 0.5f) != 0);
    NVE_CHECK(find_leaf_by_text("Count: 2") != nullptr);
  } else {
    // Non-Yoga standalone fallback: dispatch press directly by button node ID.
    NVE_CHECK(host.dispatch_press(inc_btn->node_id, 10.0f, 10.0f) != 0);
    NVE_CHECK(find_leaf_by_text("Count: 1") != nullptr);

    NVE_CHECK(host.dispatch_press(reset_btn->node_id, 10.0f, 10.0f) != 0);
    NVE_CHECK(find_leaf_by_text("Count: 0") != nullptr);

    NVE_CHECK(host.dispatch_press(inc_btn->node_id, 10.0f, 10.0f) != 0);
    NVE_CHECK(host.dispatch_press(inc_btn->node_id, 10.0f, 10.0f) != 0);
    NVE_CHECK(find_leaf_by_text("Count: 2") != nullptr);
  }

  // State-preserving live reload of the Rust counter dylib!
  NVE_CHECK(host.reload_dylib(dylib_env, &err_code, &err_msg));
  NVE_CHECK(err_code == 0);
  NVE_CHECK(find_leaf_by_text("Count: 2") != nullptr);

  // Instant-boot Frame-0 snapshot roundtrip with real Rust counter dylib!
  const std::filesystem::path snap_path =
      std::filesystem::temp_directory_path() / "nve_win_counter_test.nvesnap";
  std::error_code ec;
  std::filesystem::remove(snap_path, ec);
  NVE_CHECK(host.save_snapshot(snap_path.string(), 1u));
  NVE_CHECK(host.update_props_json("{\"initialCount\":0}"));

  nve::win::NveRnwComponentHost cold_counter;
  uint64_t counter_restore_us = 0u;
  NVE_CHECK(cold_counter.replay_frame0_snapshot(snap_path.string(), 1u, cfg, &counter_restore_us));
  std::cout << "[windows_shell_test] Cold mmap Frame-0 restore (counter dylib): "
            << counter_restore_us << " us\n";
  NVE_CHECK(!cold_counter.is_valid() && cold_counter.has_replayed_frame0());

  auto has_text_in = [](const nve::win::NveRnwComponentHost& h, std::string_view expected) {
    for (const auto& flat : h.visual_tree().build_draw_order()) {
      const auto* node = h.visual_tree().find(flat.node_id);
      if (node != nullptr && node->is_text_leaf && node->text == expected) {
        return true;
      }
    }
    return false;
  };
  NVE_CHECK(has_text_in(cold_counter, "Count: 2"));
  NVE_CHECK(cold_counter.load_dylib(dylib_env, cfg, &err_code, &err_msg));
  NVE_CHECK(err_code == 0);
  NVE_CHECK(has_text_in(cold_counter, "Count: 2"));
  std::filesystem::remove(snap_path, ec);

#if defined(_WIN32)
  const std::string shot_env = get_env_string("NVE_TEST_SCREENSHOT_PATH");
  if (!shot_env.empty()) {
    std::string bmp_err;
    NVE_CHECK(nve::win::render_visual_tree_to_bmp(
        host.visual_tree(), *host.text_measurer(), 640, 420, 1.0f, shot_env, &bmp_err));
  }
#endif
}

void test_frame0_mmap_snapshot_roundtrip() {
  namespace fs = std::filesystem;
  const fs::path snap_path =
      fs::temp_directory_path() / "nve_win_stateful_snapshot_test.nvesnap";
  std::error_code ec;
  fs::remove(snap_path, ec);

  const NveSurfaceConfig cfg{.surface_id = 1, .width = 400.0f, .height = 160.0f, .scale_factor = 1.0f};
  {
    nve::win::NveRnwComponentHost writer;
    NVE_CHECK(writer.load_dylib(NVE_WIN_TEST_STATEFUL_DYLIB, cfg));
    NVE_CHECK(writer.apply_hot_patch(nve_test_patch("42")));
    NVE_CHECK(writer.dispatch_press(3, 5.0f, 5.0f) != 0);
    NVE_CHECK(writer.visual_tree().find(2)->text == "state=43");
    NVE_CHECK(writer.save_snapshot(snap_path.string(), 3u));
  }

  nve::win::NveRnwComponentHost cold;
  uint64_t restore_us = 0u;
  NVE_CHECK(!cold.replay_frame0_snapshot(snap_path.string(), 99u, cfg, &restore_us));
  NVE_CHECK(cold.replay_frame0_snapshot(snap_path.string(), 3u, cfg, &restore_us));
  std::cout << "[windows_shell_test] Cold mmap Frame-0 restore (stateful dylib): "
            << restore_us << " us\n";

  // Visual tree immediately reflects mutated state=43 BEFORE load_dylib runs!
  NVE_CHECK(!cold.is_valid() && cold.has_replayed_frame0());
  NVE_CHECK(cold.visual_tree().find(2) != nullptr);
  NVE_CHECK(cold.visual_tree().find(2)->text == "state=43");

  // Loading dylib restores state=43 seamlessly via restore_state!
  NVE_CHECK(cold.load_dylib(NVE_WIN_TEST_STATEFUL_DYLIB, cfg));
  NVE_CHECK(cold.is_valid() && !cold.has_replayed_frame0());
  NVE_CHECK(cold.visual_tree().find(2)->text == "state=43");
  fs::remove(snap_path, ec);

  const std::string studio_env = get_env_string("NVE_TEST_STUDIO_DYLIB");
  if (!studio_env.empty()) {
    const fs::path studio_snap =
        fs::temp_directory_path() / "nve_win_studio_snapshot_test.nvesnap";
    fs::remove(studio_snap, ec);
    const NveSurfaceConfig studio_cfg{
        .surface_id = 1, .width = 960.0f, .height = 640.0f, .scale_factor = 1.0f};

    auto contains_text = [](const nve::win::NveRnwComponentHost& h, std::string_view needle) {
      for (const auto& flat : h.visual_tree().build_draw_order()) {
        const auto* node = h.visual_tree().find(flat.node_id);
        if (node != nullptr && node->is_text_leaf &&
            node->text.find(needle) != std::string::npos) {
          return true;
        }
      }
      return false;
    };

    {
      nve::win::NveRnwComponentHost studio_writer;
      NVE_CHECK(studio_writer.load_dylib(studio_env, studio_cfg));
      NVE_CHECK(studio_writer.update_props_json(R"({"accent":"violet","view":"metrics"})"));
      NVE_CHECK(contains_text(studio_writer, "Live Metrics"));
      NVE_CHECK(studio_writer.save_snapshot(studio_snap.string(), 1u));
    }

    nve::win::NveRnwComponentHost studio_cold;
    uint64_t studio_restore_us = 0u;
    NVE_CHECK(studio_cold.replay_frame0_snapshot(
        studio_snap.string(), 1u, studio_cfg, &studio_restore_us));
    std::cout << "[windows_shell_test] Cold mmap Frame-0 restore (studio-workspace): "
              << studio_restore_us << " us\n";
    NVE_CHECK(!studio_cold.is_valid() && studio_cold.has_replayed_frame0());
    NVE_CHECK(contains_text(studio_cold, "Live Metrics"));

    NVE_CHECK(studio_cold.load_dylib(studio_env, studio_cfg));
    NVE_CHECK(contains_text(studio_cold, "Live Metrics"));
    fs::remove(studio_snap, ec);
  }
}

struct MockWinImageScrollComponent {
  static inline MockWinImageScrollComponent* last_instance = nullptr;
  uint32_t surface_id{1};
  std::string image_src{"data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+X2dIAAAAASUVORK5CYII="};
  std::string resize_mode{"cover"};
  int counter{0};
  int scroll_events{0};
  uint32_t last_scroll_node{0};
  float last_scroll_x{0.0f};
  float last_scroll_y{0.0f};
  float last_scroll_content_h{0.0f};
  std::vector<uint8_t> cmd_buf;

  static NveAppHandle create(uint32_t sid) {
    auto* self = new MockWinImageScrollComponent();
    self->surface_id = sid;
    last_instance = self;
    return self;
  }
  static void destroy(NveAppHandle h) {
    auto* self = static_cast<MockWinImageScrollComponent*>(h);
    if (last_instance == self) last_instance = nullptr;
    delete self;
  }

  template <typename T>
  void push(T v) {
    const auto* p = reinterpret_cast<const uint8_t*>(&v);
    cmd_buf.insert(cmd_buf.end(), p, p + sizeof(T));
  }

  static NveBufferSlice render(NveAppHandle h, uint64_t frame_id) {
    auto* self = static_cast<MockWinImageScrollComponent*>(h);
    self->cmd_buf.clear();
    uint32_t ops = 0u;
    self->push<uint32_t>(NVE_ABI_MAGIC);
    self->push<uint16_t>(NVE_ABI_VERSION);
    self->push<uint16_t>(0u);
    self->push<uint32_t>(self->surface_id);
    self->push<uint64_t>(frame_id);
    self->push<uint32_t>(0u);

    auto create_node = [&](uint32_t id, uint16_t kind) {
      self->push<uint8_t>(NVE_OP_CREATE_NODE);
      self->push<uint32_t>(id);
      self->push<uint16_t>(kind);
      self->push<uint16_t>(0u);
      ++ops;
    };
    auto set_children = [&](uint32_t parent, std::initializer_list<uint32_t> kids) {
      self->push<uint8_t>(NVE_OP_SET_CHILDREN);
      self->push<uint32_t>(parent);
      self->push<uint32_t>(static_cast<uint32_t>(kids.size()));
      for (uint32_t k : kids) self->push<uint32_t>(k);
      ++ops;
    };
    auto set_text = [&](uint32_t id, const std::string& s) {
      self->push<uint8_t>(NVE_OP_SET_TEXT);
      self->push<uint32_t>(id);
      self->push<uint32_t>(static_cast<uint32_t>(s.size()));
      self->cmd_buf.insert(self->cmd_buf.end(), s.begin(), s.end());
      ++ops;
    };
    auto set_box = [&](uint32_t id, float w, float h) {
      self->push<uint8_t>(NVE_OP_SET_LAYOUT);
      self->push<uint32_t>(id);
      self->push<uint8_t>(1u); self->push<float>(w);
      self->push<uint8_t>(1u); self->push<float>(h);
      for (int i = 0; i < 4; ++i) { self->push<uint8_t>(0u); self->push<float>(0.0f); }
      self->push<uint8_t>(0u); self->push<uint8_t>(0u); self->push<uint8_t>(0u);
      self->push<float>(0.0f); self->push<float>(0.0f); self->push<float>(0.0f);
      for (int i = 0; i < 8; ++i) self->push<float>(0.0f);
      ++ops;
    };
    auto set_raw_str = [&](uint32_t id, std::string_view k, std::string_view v) {
      self->push<uint8_t>(NVE_OP_SET_RAW_PROP);
      self->push<uint32_t>(id);
      self->push<uint16_t>(static_cast<uint16_t>(k.size()));
      self->cmd_buf.insert(self->cmd_buf.end(), k.begin(), k.end());
      self->push<uint8_t>(4u);
      self->push<uint32_t>(static_cast<uint32_t>(v.size()));
      self->cmd_buf.insert(self->cmd_buf.end(), v.begin(), v.end());
      ++ops;
    };

    create_node(2u, NVE_COMP_VIEW);
    create_node(3u, NVE_COMP_IMAGE);
    create_node(4u, NVE_COMP_SCROLL_VIEW);
    create_node(5u, NVE_COMP_PRESSABLE);
    create_node(6u, NVE_COMP_TEXT);
    create_node(7u, NVE_COMP_TEXT);

    set_children(NVE_ROOT_NODE_ID, {2u});
    set_children(2u, {3u, 4u, 6u, 7u});
    set_children(4u, {5u});

    set_box(2u, 320.0f, 260.0f);
    set_box(3u, 64.0f, 64.0f);
    set_raw_str(3u, "src", self->image_src);
    set_raw_str(3u, "resizeMode", self->resize_mode);

    // ScrollView 4: 160x60 containing tall pressable 5 (160x220)
    set_box(4u, 160.0f, 60.0f);
    set_box(5u, 160.0f, 220.0f);

    set_box(6u, 140.0f, 24.0f);
    set_text(6u, "WinCount:" + std::to_string(self->counter));

    set_box(7u, 140.0f, 24.0f);
    set_text(7u, "UnchangedSibling");

    self->push<uint8_t>(NVE_OP_COMMIT);
    self->push<uint8_t>(1u);
    ++ops;
    std::memcpy(self->cmd_buf.data() + 20, &ops, sizeof(uint32_t));
    return NveBufferSlice{self->cmd_buf.data(), self->cmd_buf.size()};
  }

  static uint32_t dispatch_events_direct(NveAppHandle h, NveEventBatch batch) {
    auto* self = static_cast<MockWinImageScrollComponent*>(h);
    if (batch.ptr == nullptr) return 0u;
    for (uint32_t i = 0; i < batch.len; ++i) {
      const NveDirectEvent& ev = batch.ptr[i];
      if (ev.kind == static_cast<uint8_t>(nve::EventKind::Scroll)) {
        ++self->scroll_events;
        self->last_scroll_node = ev.node_id;
        self->last_scroll_x = ev.x;
        self->last_scroll_y = ev.y;
        self->last_scroll_content_h = ev.height;
      } else if (ev.kind == static_cast<uint8_t>(nve::EventKind::Press) && ev.node_id == 5u) {
        ++self->counter;
        return 1u;
      }
    }
    return 0u;
  }

  static uint32_t dispatch_events(NveAppHandle h, NveBufferSlice events) {
    auto* self = static_cast<MockWinImageScrollComponent*>(h);
    std::vector<nve::DecodedEvent> decoded;
    if (!nve::decode_event_ring(events, decoded)) return 0u;
    for (const auto& ev : decoded) {
      if (ev.kind == nve::EventKind::Press && ev.node_id == 5u) {
        ++self->counter;
        return 1u;
      }
    }
    return 0u;
  }

  static uint32_t set_props(NveAppHandle h, NveBufferSlice props) {
    auto* self = static_cast<MockWinImageScrollComponent*>(h);
    const std::string json(reinterpret_cast<const char*>(props.ptr), props.len);
    if (json.find("\"contain\"") != std::string::npos) self->resize_mode = "contain";
    if (json.find("\"stretch\"") != std::string::npos) self->resize_mode = "stretch";
    if (json.find("\"center\"") != std::string::npos) self->resize_mode = "center";
    if (json.find("\"file\"") != std::string::npos) self->image_src = "file:///tmp/avatar.png";
    return 1u;
  }

  static const void* get_interface(uint32_t iface) {
    static const NveSurfaceApi kSurface{&create, &destroy, &render, &dispatch_events, &dispatch_events_direct};
    static const NveComponentApi kComp{&set_props, nullptr, nullptr};
    if (iface == NVE_IFACE_SURFACE) return &kSurface;
    if (iface == NVE_IFACE_COMPONENT) return &kComp;
    return nullptr;
  }
};

void test_image_scroll_and_incremental_scene_view() {
  nve::win::NveRnwComponentHost host;
  const NveSurfaceConfig cfg{.surface_id = 17u, .width = 320.0f, .height = 260.0f, .scale_factor = 1.0f};
  NVE_CHECK(host.attach_interface(&MockWinImageScrollComponent::get_interface, cfg));
  NVE_CHECK(MockWinImageScrollComponent::last_instance != nullptr);
  auto* mock = MockWinImageScrollComponent::last_instance;

  // 1. Verify portable Image visual-tree `image_src` and `resize_mode` tracking!
  const auto* img = host.visual_tree().find(3u);
  NVE_CHECK(img != nullptr);
  NVE_CHECK(img->kind == NVE_NODE_KIND_IMAGE);
  NVE_CHECK(img->image_src.find("data:image/png;base64,") == 0u);
  NVE_CHECK(img->resize_mode == nve::win::ImageResizeMode::Cover);

  NVE_CHECK(host.update_props_json(R"({"resizeMode":"contain"})"));
  NVE_CHECK(host.visual_tree().find(3u)->resize_mode == nve::win::ImageResizeMode::Contain);
  NVE_CHECK(host.update_props_json(R"({"resizeMode":"stretch"})"));
  NVE_CHECK(host.visual_tree().find(3u)->resize_mode == nve::win::ImageResizeMode::Stretch);
  NVE_CHECK(host.update_props_json(R"({"resizeMode":"center","src":"file"})"));
  NVE_CHECK(host.visual_tree().find(3u)->resize_mode == nve::win::ImageResizeMode::Center);
  NVE_CHECK(host.visual_tree().find(3u)->image_src == "file:///tmp/avatar.png");

  // 2. Verify ScrollView bounds, clamping, wheel scroll offset, and zero-copy EventKind::Scroll!
  const auto* scroll = host.visual_tree().find(4u);
  NVE_CHECK(scroll != nullptr && scroll->kind == NVE_NODE_KIND_SCROLL_VIEW);
  NVE_CHECK(scroll->clip_bounds);
  NVE_CHECK(scroll->content_height >= 220.0f);

  const auto scroll_rect = host.visual_tree().absolute_rect(4u);
  NVE_CHECK(scroll_rect.has_value());
  const float sx = scroll_rect->x + 20.0f;
  const float sy = scroll_rect->y + 20.0f;
  NVE_CHECK(host.visual_tree().hit_test_scrollable(sx, sy) == std::optional<uint32_t>{4u});

  const auto child5_before = host.visual_tree().absolute_rect(5u);
  NVE_CHECK(child5_before.has_value());

  (void)host.dispatch_wheel_at(sx, sy, 0.0f, 45.0f);
  NVE_CHECK(mock->scroll_events == 1);
  NVE_CHECK(mock->last_scroll_node == 4u);
  NVE_CHECK(approx_eq(mock->last_scroll_y, 45.0f, 0.5f));
  NVE_CHECK(mock->last_scroll_content_h >= 220.0f);

  const auto off = host.visual_tree().scroll_offset(4u);
  NVE_CHECK(off.has_value() && approx_eq(off->second, 45.0f, 0.5f));

  const auto child5_after = host.visual_tree().absolute_rect(5u);
  NVE_CHECK(child5_after.has_value());
  NVE_CHECK(approx_eq(child5_after->y, child5_before->y - 45.0f, 0.5f));

  // Overscroll clamps at max_y = 220 - 60 = 160
  (void)host.dispatch_wheel_at(sx, sy, 0.0f, 500.0f);
  NVE_CHECK(mock->scroll_events == 2);
  NVE_CHECK(approx_eq(host.visual_tree().scroll_offset(4u)->second, 160.0f, 0.5f));
  NVE_CHECK(approx_eq(mock->last_scroll_y, 160.0f, 0.5f));

  // Direct dispatch_scroll delivers exact payload
  (void)host.dispatch_scroll(4u, 12.0f, 34.0f, 160.0f, 220.0f);
  NVE_CHECK(mock->scroll_events == 3);
  NVE_CHECK(approx_eq(mock->last_scroll_x, 12.0f, 0.5f));
  NVE_CHECK(approx_eq(mock->last_scroll_y, 34.0f, 0.5f));

  // 3. Verify O(changed) NveSceneView reconciliation on single-node press mutation!
  const uint64_t updates_before = host.visual_tree().updated_count();
  NVE_CHECK(host.dispatch_press(5u, 10.0f, 10.0f) == 1u);
  NVE_CHECK(host.visual_tree().find(6u)->text == "WinCount:1");
  NVE_CHECK(host.visual_tree().updated_count() - updates_before == 1u);
}

struct MockWinCanvasComponent {
  static inline MockWinCanvasComponent* last_instance = nullptr;
  uint32_t surface_id{1};
  std::string draw_list_attr;
  bool emit_corrupt_raw{false};
  std::string corrupt_raw;
  int tick{0};
  std::vector<uint8_t> cmd_buf;

  static NveAppHandle create(uint32_t sid) {
    auto* self = new MockWinCanvasComponent();
    self->surface_id = sid;
    last_instance = self;
    return self;
  }
  static void destroy(NveAppHandle h) {
    auto* self = static_cast<MockWinCanvasComponent*>(h);
    if (last_instance == self) last_instance = nullptr;
    delete self;
  }
  template <typename T>
  void push(T v) {
    const auto* p = reinterpret_cast<const uint8_t*>(&v);
    cmd_buf.insert(cmd_buf.end(), p, p + sizeof(T));
  }

  static NveBufferSlice render(NveAppHandle h, uint64_t frame_id) {
    auto* self = static_cast<MockWinCanvasComponent*>(h);
    self->cmd_buf.clear();
    uint32_t ops = 0u;
    self->push<uint32_t>(NVE_ABI_MAGIC);
    self->push<uint16_t>(NVE_ABI_VERSION);
    self->push<uint16_t>(0u);
    self->push<uint32_t>(self->surface_id);
    self->push<uint64_t>(frame_id);
    self->push<uint32_t>(0u);

    auto create_node = [&](uint32_t id, uint16_t kind) {
      self->push<uint8_t>(NVE_OP_CREATE_NODE);
      self->push<uint32_t>(id);
      self->push<uint16_t>(kind);
      self->push<uint16_t>(0u);
      ++ops;
    };
    auto set_children = [&](uint32_t parent, std::initializer_list<uint32_t> kids) {
      self->push<uint8_t>(NVE_OP_SET_CHILDREN);
      self->push<uint32_t>(parent);
      self->push<uint32_t>(static_cast<uint32_t>(kids.size()));
      for (uint32_t k : kids) self->push<uint32_t>(k);
      ++ops;
    };
    auto set_text = [&](uint32_t id, const std::string& s) {
      self->push<uint8_t>(NVE_OP_SET_TEXT);
      self->push<uint32_t>(id);
      self->push<uint32_t>(static_cast<uint32_t>(s.size()));
      self->cmd_buf.insert(self->cmd_buf.end(), s.begin(), s.end());
      ++ops;
    };
    auto set_box = [&](uint32_t id, float w, float h) {
      self->push<uint8_t>(NVE_OP_SET_LAYOUT);
      self->push<uint32_t>(id);
      self->push<uint8_t>(1u); self->push<float>(w);
      self->push<uint8_t>(1u); self->push<float>(h);
      for (int i = 0; i < 4; ++i) { self->push<uint8_t>(0u); self->push<float>(0.0f); }
      self->push<uint8_t>(0u); self->push<uint8_t>(0u); self->push<uint8_t>(0u);
      self->push<float>(0.0f); self->push<float>(0.0f); self->push<float>(0.0f);
      for (int i = 0; i < 8; ++i) self->push<float>(0.0f);
      ++ops;
    };
    auto set_raw_str = [&](uint32_t id, std::string_view k, std::string_view v) {
      self->push<uint8_t>(NVE_OP_SET_RAW_PROP);
      self->push<uint32_t>(id);
      self->push<uint16_t>(static_cast<uint16_t>(k.size()));
      self->cmd_buf.insert(self->cmd_buf.end(), k.begin(), k.end());
      self->push<uint8_t>(4u);
      self->push<uint32_t>(static_cast<uint32_t>(v.size()));
      self->cmd_buf.insert(self->cmd_buf.end(), v.begin(), v.end());
      ++ops;
    };

    create_node(2u, NVE_COMP_VIEW);
    create_node(3u, NVE_COMP_SKIA_CANVAS);
    create_node(4u, NVE_COMP_TEXT);

    set_children(NVE_ROOT_NODE_ID, {2u});
    set_children(2u, {3u, 4u});

    set_box(2u, 240.0f, 140.0f);
    set_box(3u, 200.0f, 80.0f);
    if (self->emit_corrupt_raw) {
      set_raw_str(3u, "drawList", "");
      set_text(3u, self->corrupt_raw);
    } else {
      set_raw_str(3u, "drawList", self->draw_list_attr);
    }

    set_box(4u, 120.0f, 24.0f);
    set_text(4u, "WinTick:" + std::to_string(self->tick));

    self->push<uint8_t>(NVE_OP_COMMIT);
    self->push<uint8_t>(1u);
    ++ops;
    std::memcpy(self->cmd_buf.data() + 20, &ops, sizeof(uint32_t));
    return NveBufferSlice{self->cmd_buf.data(), self->cmd_buf.size()};
  }

  static uint32_t dispatch_events(NveAppHandle, NveBufferSlice) { return 0u; }

  static const void* get_interface(uint32_t iface) {
    static const NveSurfaceApi kSurface{&create, &destroy, &render, &dispatch_events, nullptr};
    return (iface == NVE_IFACE_SURFACE) ? &kSurface : nullptr;
  }
};

struct DrawListOpCounts {
  uint32_t clear_ops{0};
  uint32_t fill_rect_ops{0};
  uint32_t fill_rounded_rect_ops{0};
  uint32_t fill_circle_ops{0};
  uint32_t stroke_line_ops{0};
  uint32_t stroke_polyline_ops{0};
  uint32_t polyline_vertices{0};
  uint32_t clip_rect_ops{0};
  uint32_t restore_clip_ops{0};

  void on_clear(const nve::DrawClearOp&) noexcept { ++clear_ops; }
  void on_fill_rect(const nve::DrawFillRectOp&) noexcept { ++fill_rect_ops; }
  void on_fill_rounded_rect(const nve::DrawFillRoundedRectOp&) noexcept { ++fill_rounded_rect_ops; }
  void on_fill_circle(const nve::DrawFillCircleOp&) noexcept { ++fill_circle_ops; }
  void on_stroke_line(const nve::DrawStrokeLineOp&) noexcept { ++stroke_line_ops; }
  void on_stroke_polyline(const nve::DrawStrokePolylineOp& op) noexcept {
    ++stroke_polyline_ops;
    polyline_vertices += static_cast<uint32_t>(op.points.size());
  }
  void on_clip_rect(const nve::DrawClipRectOp&) noexcept { ++clip_rect_ops; }
  void on_restore_clip(const nve::DrawRestoreClipOp&) noexcept { ++restore_clip_ops; }
};

void test_draw_list_visual_tree_extraction_and_corrupt_survival() {
  const std::vector<float> samples{12.0f, 25.0f, 18.0f, 44.0f, 39.0f, 68.0f};
  const nve::DrawColorRgba bg_col = nve::DrawColorRgba::rgb(15u, 23u, 42u);
  const nve::DrawColorRgba stroke_col = nve::DrawColorRgba::rgb(56u, 189u, 248u);

  nve::DrawListEncoder spark_enc;
  spark_enc.sparkline(samples, 200.0f, 80.0f, stroke_col, bg_col);
  const std::vector<uint8_t> spark_bytes = spark_enc.finish();

  nve::win::NveRnwComponentHost host;
  const NveSurfaceConfig cfg{.surface_id = 23u, .width = 240.0f, .height = 140.0f, .scale_factor = 1.0f};
  NVE_CHECK(host.attach_interface(&MockWinCanvasComponent::get_interface, cfg));
  NVE_CHECK(MockWinCanvasComponent::last_instance != nullptr);
  auto* mock = MockWinCanvasComponent::last_instance;

  const auto* canvas0 = host.visual_tree().find(3u);
  NVE_CHECK(canvas0 != nullptr && canvas0->kind == NVE_NODE_KIND_SKIA_CANVAS);
  NVE_CHECK(canvas0->draw_list.empty());

  // 1. Publish NVE_DRAW1 sparkline through text_slab and verify incremental extraction + decode!
  const uint64_t updates_before_draw = host.visual_tree().updated_count();
  mock->draw_list_attr = nve::encode_draw_list_attr_base64(spark_bytes);
  NVE_CHECK(host.render_frame() == 0);
  NVE_CHECK(host.visual_tree().updated_count() == updates_before_draw + 1u);

  const auto* canvas1 = host.visual_tree().find(3u);
  NVE_CHECK(canvas1 != nullptr);
  NVE_CHECK(canvas1->draw_list == spark_bytes);
  NVE_CHECK(canvas1->text.empty() && canvas1->image_src.empty());

  DrawListOpCounts counts{};
  const auto decoded = nve::decode_draw_list(canvas1->draw_list, counts);
  NVE_CHECK(decoded.ok());
  NVE_CHECK(counts.clear_ops == 1u);
  NVE_CHECK(counts.fill_rounded_rect_ops == 1u);
  NVE_CHECK(counts.clip_rect_ops == 1u);
  NVE_CHECK(counts.stroke_line_ops == 3u);
  NVE_CHECK(counts.fill_rect_ops == 1u);
  NVE_CHECK(counts.stroke_polyline_ops == 1u);
  NVE_CHECK(counts.polyline_vertices == 6u);
  NVE_CHECK(counts.fill_circle_ops == 1u);
  NVE_CHECK(counts.restore_clip_ops == 1u);

  // 2. Sibling-only mutation updates ONLY 1 node (text 4), leaving canvas draw_list untouched!
  const uint64_t updates_before_sibling = host.visual_tree().updated_count();
  mock->tick = 1;
  NVE_CHECK(host.render_frame() == 0);
  NVE_CHECK(host.visual_tree().updated_count() == updates_before_sibling + 1u);
  NVE_CHECK(host.visual_tree().find(4u)->text == "WinTick:1");
  NVE_CHECK(host.visual_tree().find(3u)->draw_list == spark_bytes);

  // 3. Updating sparkline bytes incrementally updates ONLY canvas node 3!
  const std::vector<float> next_samples{5.0f, 50.0f, 95.0f};
  nve::DrawListEncoder next_enc;
  next_enc.sparkline(next_samples, 200.0f, 80.0f, nve::DrawColorRgba::rgb(244u, 63u, 94u), bg_col);
  const std::vector<uint8_t> next_bytes = next_enc.finish();
  const uint64_t updates_before_next = host.visual_tree().updated_count();
  mock->draw_list_attr = nve::encode_draw_list_attr_base64(next_bytes);
  NVE_CHECK(host.render_frame() == 0);
  NVE_CHECK(host.visual_tree().updated_count() == updates_before_next + 1u);
  NVE_CHECK(host.visual_tree().find(3u)->draw_list == next_bytes);

  // 4. Corrupt NVE_DRAW1 buffers (truncated, bad magic, trailing bytes) survive extraction & decode!
  mock->emit_corrupt_raw = true;
  mock->corrupt_raw.assign(reinterpret_cast<const char*>(spark_bytes.data()), 18u);
  NVE_CHECK(host.render_frame() == 0);
  NVE_CHECK(host.visual_tree().find(3u)->draw_list.size() == 18u);
  NVE_CHECK(!nve::validate_draw_list(host.visual_tree().find(3u)->draw_list).ok());

  mock->corrupt_raw = "BAD_MAGIC_HEADER_DATA";
  NVE_CHECK(host.render_frame() == 0);
  NVE_CHECK(nve::validate_draw_list(host.visual_tree().find(3u)->draw_list).error ==
            nve::DrawListError::BadMagic);

  std::vector<uint8_t> with_trailing = spark_bytes;
  with_trailing.push_back(0xEEu);
  mock->corrupt_raw.assign(reinterpret_cast<const char*>(with_trailing.data()), with_trailing.size());
  NVE_CHECK(host.render_frame() == 0);
  NVE_CHECK(nve::validate_draw_list(host.visual_tree().find(3u)->draw_list).error ==
            nve::DrawListError::TrailingBytes);
}

#if defined(_WIN32)
using WinTestSock = SOCKET;
using WinTestSockLen = int;
inline bool win_test_sock_valid(WinTestSock s) noexcept { return s != INVALID_SOCKET; }
inline void win_test_close(WinTestSock s) noexcept { if (win_test_sock_valid(s)) ::closesocket(s); }
inline int win_test_poll_read(WinTestSock s, int ms) noexcept {
  WSAPOLLFD pfd{};
  pfd.fd = s;
  pfd.events = POLLIN;
  return ::WSAPoll(&pfd, 1u, ms);
}
inline ptrdiff_t win_test_send(WinTestSock s, const uint8_t* p, size_t n) noexcept {
  return static_cast<ptrdiff_t>(::send(s, reinterpret_cast<const char*>(p), static_cast<int>(n), 0));
}
inline ptrdiff_t win_test_recv(WinTestSock s, uint8_t* p, size_t n) noexcept {
  return static_cast<ptrdiff_t>(::recv(s, reinterpret_cast<char*>(p), static_cast<int>(n), 0));
}
#else
using WinTestSock = int;
using WinTestSockLen = socklen_t;
inline bool win_test_sock_valid(WinTestSock s) noexcept { return s >= 0; }
inline void win_test_close(WinTestSock s) noexcept { if (win_test_sock_valid(s)) ::close(s); }
inline int win_test_poll_read(WinTestSock s, int ms) noexcept {
  pollfd pfd{s, POLLIN, 0};
  return ::poll(&pfd, 1u, ms);
}
inline ptrdiff_t win_test_send(WinTestSock s, const uint8_t* p, size_t n) noexcept {
  return static_cast<ptrdiff_t>(::send(s, p, n, 0));
}
inline ptrdiff_t win_test_recv(WinTestSock s, uint8_t* p, size_t n) noexcept {
  return static_cast<ptrdiff_t>(::recv(s, p, n, 0));
}
#endif

bool win_pair_send_all(WinTestSock fd, const uint8_t* p, size_t n) {
  size_t off = 0u;
  while (off < n) {
    const ptrdiff_t rc = win_test_send(fd, p + off, n - off);
    if (rc <= 0) return false;
    off += static_cast<size_t>(rc);
  }
  return true;
}

bool win_pair_recv_exact(WinTestSock fd, uint8_t* p, size_t n, int timeout_ms = 2000) {
  size_t off = 0u;
  const auto deadline = std::chrono::steady_clock::now() + std::chrono::milliseconds(timeout_ms);
  while (off < n) {
    const auto now = std::chrono::steady_clock::now();
    if (now >= deadline) return false;
    const int rem = static_cast<int>(
        std::chrono::duration_cast<std::chrono::milliseconds>(deadline - now).count());
    if (win_test_poll_read(fd, std::max(1, std::min(rem, 50))) <= 0) continue;
    const ptrdiff_t rc = win_test_recv(fd, p + off, n - off);
    if (rc <= 0) return false;
    off += static_cast<size_t>(rc);
  }
  return true;
}

bool win_pair_write_tcp(WinTestSock fd, std::span<const uint8_t> payload) {
  uint8_t hdr[4]{
      static_cast<uint8_t>(payload.size() & 0xFFu),
      static_cast<uint8_t>((payload.size() >> 8u) & 0xFFu),
      static_cast<uint8_t>((payload.size() >> 16u) & 0xFFu),
      static_cast<uint8_t>((payload.size() >> 24u) & 0xFFu),
  };
  return win_pair_send_all(fd, hdr, 4u) && win_pair_send_all(fd, payload.data(), payload.size());
}

bool win_pair_read_tcp(WinTestSock fd, std::vector<uint8_t>& out, int timeout_ms = 2000) {
  uint8_t hdr[4]{};
  if (!win_pair_recv_exact(fd, hdr, 4u, timeout_ms)) return false;
  const uint32_t len = static_cast<uint32_t>(hdr[0]) | (static_cast<uint32_t>(hdr[1]) << 8u) |
                       (static_cast<uint32_t>(hdr[2]) << 16u) |
                       (static_cast<uint32_t>(hdr[3]) << 24u);
  if (len == 0u || len > 4u * 1024u * 1024u) return false;
  out.resize(len);
  return win_pair_recv_exact(fd, out.data(), len, timeout_ms);
}

std::vector<uint8_t> build_win_pair_frame(uint64_t frame_id, const std::string& label_text) {
  nve::CommandBufferEncoder enc(1u, frame_id);
  enc.create_node(2u, NVE_COMP_VIEW);
  enc.create_node(3u, NVE_COMP_TEXT);
  enc.create_node(4u, NVE_COMP_PRESSABLE);
  enc.create_node(5u, NVE_COMP_TEXT);
  enc.set_children(NVE_ROOT_NODE_ID, {2u});
  enc.set_children(2u, {3u, 4u});
  enc.set_children(4u, {5u});

  nve::LayoutProps root_box{};
  root_box.width = {1u, 360.0f};
  root_box.height = {1u, 220.0f};
  root_box.padding = {12.0f, 12.0f, 12.0f, 12.0f};
  root_box.gap = 8.0f;
  enc.set_layout(2u, root_box);

  nve::LayoutProps btn{};
  btn.width = {1u, 180.0f};
  btn.height = {1u, 48.0f};
  enc.set_layout(4u, btn);

  enc.set_text(3u, label_text);
  enc.set_text(5u, "Paired Button");

  nve::StyleProps st{};
  st.background_rgba = 0xFF1E293Bu;
  st.text_color_rgba = 0xFFFFFFFFu;
  st.font_size = 18.0f;
  enc.set_style(2u, st);
  enc.set_style(4u, st);
  enc.set_event_mask(4u, nve::kEventMaskPress);
  enc.commit(true);
  const NveBufferSlice slice = enc.finish();
  return std::vector<uint8_t>(slice.ptr, slice.ptr + slice.len);
}

void test_paired_thin_client_loopback_roundtrip() {
#if defined(_WIN32)
  WSADATA wsa_data{};
  NVE_CHECK(::WSAStartup(MAKEWORD(2, 2), &wsa_data) == 0);
#endif
  const std::string token = "nve-win-pair-token";
  const auto frame1 = build_win_pair_frame(1u, "Windows Paired Frame 1");
  const auto frame2 = build_win_pair_frame(2u, "Windows Paired Frame 2");

  WinTestSock listen_fd = ::socket(AF_INET, SOCK_STREAM, 0);
  NVE_CHECK(win_test_sock_valid(listen_fd));
  const int reuse = 1;
#if defined(_WIN32)
  (void)::setsockopt(
      listen_fd, SOL_SOCKET, SO_REUSEADDR, reinterpret_cast<const char*>(&reuse), sizeof(reuse));
#else
  (void)::setsockopt(listen_fd, SOL_SOCKET, SO_REUSEADDR, &reuse, sizeof(reuse));
#endif
  sockaddr_in addr{};
  addr.sin_family = AF_INET;
  addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
  addr.sin_port = 0;
  NVE_CHECK(::bind(listen_fd, reinterpret_cast<sockaddr*>(&addr), sizeof(addr)) == 0);
  WinTestSockLen alen = static_cast<WinTestSockLen>(sizeof(addr));
  NVE_CHECK(::getsockname(listen_fd, reinterpret_cast<sockaddr*>(&addr), &alen) == 0);
  const uint16_t port = ntohs(addr.sin_port);
  NVE_CHECK(::listen(listen_fd, 2) == 0);

  std::atomic<bool> saw_input{false};
  std::vector<uint8_t> rx_ev;
  std::mutex ev_mtx;

  auto send_raw = [](WinTestSock fd, std::span<const uint8_t> bytes) -> bool {
    size_t off = 0;
    while (off < bytes.size()) {
#if defined(_WIN32)
      const int n = ::send(fd, reinterpret_cast<const char*>(bytes.data() + off),
                           static_cast<int>(bytes.size() - off), 0);
#else
      const ssize_t n = ::send(fd, bytes.data() + off, bytes.size() - off, 0);
#endif
      if (n <= 0) return false;
      off += static_cast<size_t>(n);
    }
    return true;
  };
  auto recv_raw = [](WinTestSock fd, uint8_t* out, size_t len) -> bool {
    size_t off = 0;
    while (off < len) {
#if defined(_WIN32)
      const int n = ::recv(fd, reinterpret_cast<char*>(out + off), static_cast<int>(len - off), 0);
#else
      const ssize_t n = ::recv(fd, out + off, len - off, 0);
#endif
      if (n <= 0) return false;
      off += static_cast<size_t>(n);
    }
    return true;
  };
  auto read_v2 = [&](WinTestSock fd, std::vector<uint8_t>& buf, nve::WireFrameView& view) -> bool {
    std::array<uint8_t, nve::kWireHeaderSize> h{};
    if (!recv_raw(fd, h.data(), h.size())) return false;
    nve::WireHeader hdr{};
    if (!nve::wire_v2::decode_header(h, hdr)) return false;
    buf.resize(hdr.total_wire_len());
    std::memcpy(buf.data(), h.data(), h.size());
    if (buf.size() > h.size() && !recv_raw(fd, buf.data() + h.size(), buf.size() - h.size())) {
      return false;
    }
    return nve::wire_v2::decode_frame(buf, view);
  };

  std::thread srv([&]() {
    WinTestSock cfd = ::accept(listen_fd, nullptr, nullptr);
    NVE_CHECK(win_test_sock_valid(cfd));
    std::vector<uint8_t> msg;
    nve::WireFrameView view{};
    NVE_CHECK(read_v2(cfd, msg, view) && view.header.kind == nve::WireKind::Hello);
    nve::WireHello hp{};
    NVE_CHECK(nve::wire_v2::decode_hello(view.payload, hp));
    NVE_CHECK(nve::wire_v2::constant_time_eq(hp.bearer_token, token));

    const auto ack = nve::wire_v2::encode_hello_ack({42u, nve::kDefaultWireSchemaVersion, 0x0Fu});
    NVE_CHECK(send_raw(
        cfd, nve::wire_v2::encode_frame(nve::WireChannel::Control, nve::WireKind::HelloAck, 1u, ack)));
    NVE_CHECK(send_raw(
        cfd, nve::wire_v2::encode_frame(nve::WireChannel::UiScene, nve::WireKind::CommandDelta, 1u, frame1)));
    NVE_CHECK(send_raw(
        cfd, nve::wire_v2::encode_frame(nve::WireChannel::UiScene, nve::WireKind::CommandDelta, 2u, frame2)));

    while (read_v2(cfd, msg, view)) {
      if (view.header.kind == nve::WireKind::EventBatch) {
        std::lock_guard<std::mutex> lock(ev_mtx);
        rx_ev.assign(view.payload.begin(), view.payload.end());
        saw_input.store(true);
        break;
      }
      if (view.header.kind == nve::WireKind::Bye) break;
    }
    win_test_close(cfd);
    win_test_close(listen_fd);
  });

  nve::win::NveRnwComponentHost host;
  std::atomic<uint32_t> wake_hits{0u};
  std::atomic<uint32_t> tree_changes{0u};
  host.set_on_visual_tree_changed([&]() { tree_changes.fetch_add(1u, std::memory_order_relaxed); });

  const NveSurfaceConfig scfg{.surface_id = 1u, .width = 400.0f, .height = 280.0f, .scale_factor = 1.0f};
  std::string err;
  const std::string endpoint = "127.0.0.1:" + std::to_string(port);
  NVE_CHECK(host.connect_paired(
      endpoint,
      token,
      "auto",
      scfg,
      [&]() { wake_hits.fetch_add(1u, std::memory_order_relaxed); },
      &err));
  NVE_CHECK(host.is_paired() && host.is_paired_connected());
  NVE_CHECK(!host.is_valid()); // Zero local dylib!

  const auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(3);
  while (host.paired_frame_count() < 2u && std::chrono::steady_clock::now() < deadline) {
    (void)host.poll_paired_frames();
    std::this_thread::sleep_for(std::chrono::milliseconds(5));
  }
  NVE_CHECK(host.paired_frame_count() == 2u);
  NVE_CHECK(wake_hits.load() >= 1u);
  NVE_CHECK(tree_changes.load() >= 1u);

  const auto* label = host.visual_tree().find(3u);
  NVE_CHECK(label != nullptr && label->text == "Windows Paired Frame 2");
  const auto btn_rect = host.visual_tree().absolute_rect(4u);
  NVE_CHECK(btn_rect.has_value() && btn_rect->width > 10.0f && btn_rect->height > 10.0f);

  NVE_CHECK(host.dispatch_click_at(
                btn_rect->x + btn_rect->width * 0.5f, btn_rect->y + btn_rect->height * 0.5f) == 1u);

  const auto ev_deadline = std::chrono::steady_clock::now() + std::chrono::seconds(3);
  while (!saw_input.load() && std::chrono::steady_clock::now() < ev_deadline) {
    std::this_thread::sleep_for(std::chrono::milliseconds(5));
  }
  NVE_CHECK(saw_input.load());

  std::vector<nve::DecodedEvent> decoded;
  {
    std::lock_guard<std::mutex> lock(ev_mtx);
    NVE_CHECK(nve::decode_event_ring(NveBufferSlice{rx_ev.data(), rx_ev.size()}, decoded));
  }
  NVE_CHECK(decoded.size() == 1u);
  NVE_CHECK(decoded[0].kind == nve::EventKind::Press && decoded[0].node_id == 4u);

  host.disconnect_paired();
  NVE_CHECK(!host.is_paired());
  NVE_CHECK(host.visual_tree().empty());
  srv.join();
#if defined(_WIN32)
  ::WSACleanup();
#endif
}

void test_multi_window_manager_lifecycle_modal_veto_and_cascade() {
  nve::win::NveWinWindowManager mgr;
  bool allow_primary = false;
  std::vector<uint32_t> closed_order;
  int all_closed_calls = 0;

  mgr.set_should_close_callback([&](uint32_t sid) { return sid == 1u ? allow_primary : true; });
  mgr.set_on_window_closed_callback([&](uint32_t sid) { closed_order.push_back(sid); });
  mgr.set_on_all_windows_closed_callback([&]() { ++all_closed_calls; });

  auto* h1 = mgr.open_headless_window(
      {.surface_id = 1u, .title = "Primary", .width = 640, .height = 420});
  auto* h2 = mgr.open_headless_window(
      {.surface_id = 2u, .title = "Inspector", .width = 360, .height = 300});
  NVE_CHECK(h1 != nullptr && h2 != nullptr && h1 != h2);
  NVE_CHECK(mgr.window_count() == 2u);
  NVE_CHECK(mgr.focused_surface_id() == 2u);
  NVE_CHECK(mgr.open_headless_window({.surface_id = 1u, .title = "Dup"}) == nullptr);

  NVE_CHECK(mgr.set_title(1u, "Primary — Renamed"));
  NVE_CHECK(mgr.spec_for_surface(1u)->title == "Primary — Renamed");
  NVE_CHECK(mgr.set_bounds(1u, 40, 50, 800, 500));
  NVE_CHECK(mgr.spec_for_surface(1u)->width == 800 && mgr.spec_for_surface(1u)->height == 500);

  auto* h3 = mgr.open_headless_window({
      .surface_id = 3u,
      .title = "Modal Dialog",
      .width = 320,
      .height = 180,
      .parent_surface_id = 1u,
      .modal = true,
      .cascade_close = true,
  });
  auto* h4 = mgr.open_headless_window({
      .surface_id = 4u,
      .title = "Popover",
      .width = 200,
      .height = 120,
      .parent_surface_id = 3u,
      .cascade_close = true,
  });
  NVE_CHECK(h3 != nullptr && h4 != nullptr);
  NVE_CHECK(mgr.active_modal_blocker(1u) == 3u);
  NVE_CHECK(!mgr.focus_window(1u));
  NVE_CHECK(mgr.focus_window(2u));
  NVE_CHECK(mgr.focused_surface_id() == 2u);

  // Vetoed close on primary keeps all 4 windows open and does not post quit.
  NVE_CHECK(!mgr.request_close(1u));
  NVE_CHECK(mgr.window_count() == 4u && !mgr.quit_posted());

  // Allowed close on primary cascades 4 -> 3 -> 1, keeping independent window 2 open.
  allow_primary = true;
  NVE_CHECK(mgr.request_close(1u));
  NVE_CHECK(mgr.window_count() == 1u);
  NVE_CHECK(!mgr.quit_posted() && all_closed_calls == 0);
  NVE_CHECK((closed_order == std::vector<uint32_t>{4u, 3u, 1u}));

  // Closing the final window posts quit once.
  NVE_CHECK(mgr.request_close(2u));
  NVE_CHECK(mgr.window_count() == 0u);
  NVE_CHECK(mgr.quit_posted() && all_closed_calls == 1);
}

} // namespace

int main() {
  test_directwrite_text_measurer();
  test_visual_tree_reconciler_and_hit_testing();
  test_rnw_component_host_and_dylib_lifecycle();
  test_live_counter_dylib_if_configured();
  test_frame0_mmap_snapshot_roundtrip();
  test_image_scroll_and_incremental_scene_view();
  test_draw_list_visual_tree_extraction_and_corrupt_survival();
  test_paired_thin_client_loopback_roundtrip();
  test_multi_window_manager_lifecycle_modal_veto_and_cascade();

  std::cout << "nve_windows_shell_test passed ("
            << (nve::fabric_host_has_yoga_layout() ? "Yoga+JSI" : "Standalone") << "+"
#if defined(_WIN32)
            << "DirectWrite+Direct2D"
#else
            << "Portable"
#endif
            << ")\n";
  return 0;
}
