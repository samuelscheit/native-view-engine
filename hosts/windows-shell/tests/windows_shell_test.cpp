#include "NveRnwComponentView.hpp"
#include "NveWinHostWindow.hpp"
#include "NveWinTextMeasurer.hpp"
#include "mock_jsi_recorder.hpp"

#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <initializer_list>
#include <iostream>
#include <string>
#include <vector>

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

  static uint32_t apply_hot_patch(NveAppHandle h, NveBufferSlice patch_json) {
    auto* self = static_cast<MockWinRustComponent*>(h);
    self->title.assign(reinterpret_cast<const char*>(patch_json.ptr), patch_json.len);
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

  tree.sync({root, btn_top, btn_label, btn_under, btn_hidden});
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

  // Re-sync without tag/membership change reuses views (created_count unchanged).
  const uint64_t created_before = tree.created_count();
  const uint64_t updated_before = tree.updated_count();
  btn_label.text = "TopUpdated";
  tree.sync({root, btn_top, btn_label, btn_under, btn_hidden});
  NVE_CHECK(tree.created_count() == created_before);
  NVE_CHECK(tree.updated_count() == updated_before + 4);
  NVE_CHECK(tree.find(11)->text == "TopUpdated");

  // Transition node 11 from leaf NVE_COMP_TEXT to wrapper NVE_COMP_TEXT (children = {12}).
  // RetainedViewReconciler must detect view_tag_for_snapshot change and recreate node 11!
  nve::RenderedNodeSnapshot nested_leaf{};
  nested_leaf.node_id = 12;
  nested_leaf.kind = NVE_COMP_TEXT;
  nested_leaf.text = "NestedLeaf";
  nested_leaf.layout = {0.0f, 0.0f, 50.0f, 18.0f};
  btn_label.text.clear();
  btn_label.children = {12};

  const uint64_t destroyed_before = tree.destroyed_count();
  tree.sync({root, btn_top, btn_label, nested_leaf, btn_under, btn_hidden});
  NVE_CHECK(tree.size() == 5);
  NVE_CHECK(tree.destroyed_count() == destroyed_before + 1); // node 11 recreated
  NVE_CHECK(tree.created_count() == created_before + 2);     // node 11 + node 12
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

  NVE_CHECK(reload_host.apply_hot_patch_json("77"));
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

#if defined(_WIN32)
  const std::string shot_env = get_env_string("NVE_TEST_SCREENSHOT_PATH");
  if (!shot_env.empty()) {
    std::string bmp_err;
    NVE_CHECK(nve::win::render_visual_tree_to_bmp(
        host.visual_tree(), *host.text_measurer(), 640, 420, 1.0f, shot_env, &bmp_err));
  }
#endif
}

} // namespace

int main() {
  test_directwrite_text_measurer();
  test_visual_tree_reconciler_and_hit_testing();
  test_rnw_component_host_and_dylib_lifecycle();
  test_live_counter_dylib_if_configured();

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
