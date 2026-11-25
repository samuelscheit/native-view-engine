#include "NveGtkHost.hpp"
#include "command_encoder.hpp"
#include "mock_jsi_recorder.hpp"
#include "nve_pair_client.hpp"

#include <arpa/inet.h>
#include <netinet/in.h>
#include <poll.h>
#include <sys/socket.h>
#include <unistd.h>

#include <atomic>
#include <chrono>
#include <thread>

#include <cmath>
#include <cstdlib>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <functional>
#include <iostream>
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

namespace {

bool approx_eq(double a, double b, double eps = 1.5) {
  return std::fabs(a - b) <= eps;
}

// Rich mock Rust component supporting:
// - Row container (2), leaf text (3), pressable button (4), scroll view (5) with child (6),
//   text input (7)
// - Dynamic swap of node 3 between leaf text and wrapper text (wrapping child 8) via props JSON
// - Outbound EventRing events (`onPropsApplied`, `onIncrement`)
struct MockGtkRustComponent {
  uint32_t surface_id{1u};
  int count{0};
  bool text_as_wrapper{false};
  std::string title{"Initial"};
  std::string input_value{"hello"};
  std::vector<uint8_t> cmd_buf;
  nve::EventRingWriter outbound;
  std::vector<uint8_t> outbound_snap;

  static NveAppHandle create(uint32_t sid) {
    auto* self = new MockGtkRustComponent();
    self->surface_id = sid;
    self->outbound.clear();
    return self;
  }

  static void destroy(NveAppHandle h) { delete static_cast<MockGtkRustComponent*>(h); }

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
    auto* self = static_cast<MockGtkRustComponent*>(h);
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

    auto remove_node = [&](uint32_t id) {
      self->push<uint8_t>(NVE_OP_REMOVE_NODE);
      self->push<uint32_t>(id);
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
      self->push_dim(0u, 0.0f);
      self->push_dim(0u, 0.0f);
      self->push_dim(0u, 0.0f);
      self->push_dim(0u, 0.0f);
      self->push<uint8_t>(dir);
      self->push<uint8_t>(0u); /* FlexStart */
      self->push<uint8_t>(0u); /* Stretch */
      self->push<float>(grow);
      self->push<float>(1.0f);
      self->push<float>(gap);
      self->push_insets(pad, pad, pad, pad);
      self->push_insets(0.0f, 0.0f, 0.0f, 0.0f);
      ++ops;
    };

    auto set_style = [&](uint32_t id, uint32_t bg, uint32_t fg, float radius, float font_size) {
      self->push<uint8_t>(NVE_OP_SET_STYLE);
      self->push<uint32_t>(id);
      self->push<uint32_t>(bg);
      self->push<uint32_t>(fg);
      self->push<uint32_t>(0xFF0000FFu);
      self->push<float>(1.5f);
      self->push<float>(radius);
      self->push<float>(0.90f);
      self->push<float>(font_size);
      self->push<int32_t>(0);
      ++ops;
    };

    create_node(2u, NVE_COMP_VIEW);
    create_node(3u, NVE_COMP_TEXT);
    create_node(4u, NVE_COMP_PRESSABLE);
    create_node(5u, NVE_COMP_SCROLL_VIEW);
    create_node(6u, NVE_COMP_TEXT);
    create_node(7u, NVE_COMP_TEXT_INPUT);

    set_children(NVE_ROOT_NODE_ID, {2u});
    set_children(2u, {3u, 4u, 5u, 7u});
    set_children(5u, {6u});

    if (self->text_as_wrapper) {
      create_node(8u, NVE_COMP_TEXT);
      set_children(3u, {8u});
      set_text(3u, "");
      set_text(8u, "Wrapped:" + self->title + ":" + std::to_string(self->count));
      set_layout(8u, 0u, 1u, 110.0f, 1u, 32.0f, 0.0f, 0.0f, 0.0f);
    } else {
      set_children(3u, {});
      remove_node(8u);
      set_text(3u, self->title + ":" + std::to_string(self->count));
    }

    // Row container: 100% x 100%, pad=20, gap=10
    set_layout(2u, 1u, 2u, 100.0f, 2u, 100.0f, 0.0f, 20.0f, 10.0f);
    set_style(2u, 0xFF202020u, 0xFFFFFFFFu, 8.0f, 16.0f);

    // Label or wrapper text: fixed 120x40
    set_layout(3u, 0u, 1u, 120.0f, 1u, 40.0f, 0.0f, 0.0f, 0.0f);
    set_style(3u, 0x00000000u, 0xFFFFFFFFu, 0.0f, 18.0f);

    // Pressable button: flex_grow=1, height=40
    set_layout(4u, 0u, 0u, 0.0f, 1u, 40.0f, 1.0f, 0.0f, 0.0f);
    set_style(4u, 0xFFFF5500u, 0xFFFFFFFFu, 12.0f, 16.0f);

    // ScrollView: fixed 80x40 with child 6
    set_layout(5u, 0u, 1u, 80.0f, 1u, 40.0f, 0.0f, 0.0f, 0.0f);
    set_style(5u, 0xFF101828u, 0xFFFFFFFFu, 4.0f, 14.0f);
    set_layout(6u, 0u, 1u, 140.0f, 1u, 80.0f, 0.0f, 0.0f, 0.0f);
    set_text(6u, "Scrollable");

    // TextInput: fixed 90x32
    set_layout(7u, 0u, 1u, 90.0f, 1u, 32.0f, 0.0f, 0.0f, 0.0f);
    set_text(7u, self->input_value);
    set_style(7u, 0xFF0F172Au, 0xFFFFFFFFu, 6.0f, 14.0f);

    self->push<uint8_t>(NVE_OP_COMMIT);
    self->push<uint8_t>(1u);
    ++ops;

    std::memcpy(self->cmd_buf.data() + 20, &ops, sizeof(uint32_t));
    return NveBufferSlice{self->cmd_buf.data(), self->cmd_buf.size()};
  }

  static uint32_t dispatch_events(NveAppHandle h, NveBufferSlice events) {
    auto* self = static_cast<MockGtkRustComponent*>(h);
    std::vector<nve::DecodedEvent> decoded;
    if (!nve::decode_event_ring(events, decoded)) {
      return 0u;
    }
    for (const auto& ev : decoded) {
      if (ev.kind == nve::EventKind::Press && ev.node_id == 4u) {
        ++self->count;
        const std::string payload = "{\"count\":" + std::to_string(self->count) + "}";
        self->outbound.push_custom(
            self->surface_id,
            4u,
            ev.timestamp_us,
            "onIncrement",
            reinterpret_cast<const uint8_t*>(payload.data()),
            payload.size());
        return 1u;
      }
    }
    return 0u;
  }

  static uint32_t set_props(NveAppHandle h, NveBufferSlice props) {
    auto* self = static_cast<MockGtkRustComponent*>(h);
    const std::string json(reinterpret_cast<const char*>(props.ptr), props.len);
    if (json.find("\"Updated\"") != std::string::npos) {
      self->title = "Updated";
    }
    if (json.find("\"wrapper\":true") != std::string::npos) {
      self->text_as_wrapper = true;
    } else if (json.find("\"wrapper\":false") != std::string::npos) {
      self->text_as_wrapper = false;
    }
    if (json.find("\"input\":\"typed\"") != std::string::npos) {
      self->input_value = "typed";
    }
    const std::string payload = "{\"ok\":true}";
    self->outbound.push_custom(
        self->surface_id,
        NVE_ROOT_NODE_ID,
        nve::current_timestamp_us(),
        "onPropsApplied",
        reinterpret_cast<const uint8_t*>(payload.data()),
        payload.size());
    return 1u;
  }

  static NveBufferSlice drain_emitted_events(NveAppHandle h) {
    auto* self = static_cast<MockGtkRustComponent*>(h);
    self->outbound_snap = self->outbound.finalize();
    self->outbound.clear();
    return NveBufferSlice{self->outbound_snap.data(), self->outbound_snap.size()};
  }

  static const void* get_interface(uint32_t interface_id) {
    static const NveSurfaceApi kSurface{
        .create = &create,
        .destroy = &destroy,
        .render = &render,
        .dispatch_events = &dispatch_events,
        .dispatch_events_direct = nullptr,
    };
    static const NveComponentApi kComponent{
        .set_props = &set_props,
        .drain_emitted_events = &drain_emitted_events,
        .set_props_direct = nullptr,
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
};

std::vector<GtkLabel*> collect_labels(GtkWidget* root) {
  std::vector<GtkLabel*> out;
  std::function<void(GtkWidget*)> walk = [&](GtkWidget* w) {
    if (w == nullptr) {
      return;
    }
    if (GTK_IS_LABEL(w)) {
      out.push_back(GTK_LABEL(w));
    }
    for (GtkWidget* child = gtk_widget_get_first_child(w); child != nullptr;
         child = gtk_widget_get_next_sibling(child)) {
      walk(child);
    }
  };
  walk(root);
  return out;
}

uint32_t find_pressable_node_for_label_text(
    const nve::NveGtkHost& host,
    std::string_view expected_text) {
  const auto snaps = host.session().snapshot_nodes();
  std::unordered_map<uint32_t, const nve::RenderedNodeSnapshot*> by_id;
  for (const auto& s : snaps) {
    by_id[s.node_id] = &s;
  }

  // DFS to check if a subtree under `node_id` contains a text node with `expected_text`.
  std::function<bool(uint32_t)> contains_text = [&](uint32_t id) -> bool {
    auto it = by_id.find(id);
    if (it == by_id.end()) {
      return false;
    }
    if (it->second->text == expected_text) {
      return true;
    }
    for (uint32_t cid : it->second->children) {
      if (contains_text(cid)) {
        return true;
      }
    }
    return false;
  };

  for (const auto& s : snaps) {
    if (s.kind == NVE_COMP_PRESSABLE && contains_text(s.node_id)) {
      return s.node_id;
    }
  }
  return 0u;
}

void test_attached_widgets_pango_and_swaps() {
  NveSurfaceConfig cfg{.surface_id = 9u, .width = 500.0f, .height = 200.0f, .scale_factor = 1.0f};
  nve::NveGtkHost host(cfg);
  NVE_CHECK(host.root_widget() != nullptr);
  NVE_CHECK(GTK_IS_FIXED(host.root_widget()));
  NVE_CHECK(host.host_widget() != nullptr);
  NVE_CHECK(GTK_IS_SCROLLED_WINDOW(host.host_widget()));
  NVE_CHECK(!host.is_loaded());
  NVE_CHECK(host.node_count() == 0u);
  NVE_CHECK(host.widget_for_node_id(NVE_ROOT_NODE_ID) == host.root_widget());

  // 1. PangoLayout native text measurer checks
  const NveIntrinsicSize empty_sz = host.measure_text("", 16.0f, 0.0f);
  NVE_CHECK(empty_sz.width == 0.0f && empty_sz.height == 0.0f);

  const NveIntrinsicSize small_sz = host.measure_text("Native View Engine", 12.0f, 0.0f);
  const NveIntrinsicSize large_sz = host.measure_text("Native View Engine", 28.0f, 0.0f);
  NVE_CHECK(small_sz.width > 0.0f && small_sz.height > 0.0f);
  NVE_CHECK(large_sz.width > small_sz.width);
  NVE_CHECK(large_sz.height > small_sz.height);

  const std::string long_text =
      "The quick brown fox jumps over the lazy dog across multiple wrapped Pango lines";
  const NveIntrinsicSize wide_sz = host.measure_text(long_text, 16.0f, 0.0f);
  const NveIntrinsicSize wrapped_sz = host.measure_text(long_text, 16.0f, 120.0f);
  NVE_CHECK(wide_sz.width > wrapped_sz.width);
  NVE_CHECK(wrapped_sz.width <= 122.0f);
  NVE_CHECK(wrapped_sz.height > wide_sz.height);

  // 2. Attach in-process interface + event callback
  std::vector<nve::DecodedEvent> received_events;
  host.set_event_callback([&](const nve::DecodedEvent& ev) { received_events.push_back(ev); });
  NVE_CHECK(host.attach_interface(&MockGtkRustComponent::get_interface, 9u));
  NVE_CHECK(host.is_loaded());
  NVE_CHECK(host.surface_id() == 9u);
  NVE_CHECK(host.node_count() == 7u);
  {
    // Verify host_widget() never locks the hosting GtkWindow minimum size when grown large.
    host.set_surface_size(960.0f, 640.0f, 1.0f);
    int min_w = 0, nat_w = 0, min_h = 0, nat_h = 0;
    gtk_widget_measure(host.host_widget(), GTK_ORIENTATION_HORIZONTAL, -1, &min_w, &nat_w, nullptr, nullptr);
    gtk_widget_measure(host.host_widget(), GTK_ORIENTATION_VERTICAL, -1, &min_h, &nat_h, nullptr, nullptr);
    NVE_CHECK(min_w <= 16);
    NVE_CHECK(min_h <= 16);
    host.set_surface_size(500.0f, 200.0f, 1.0f);
  }

  GtkWidget* container = host.widget_for_node_id(2u);
  GtkWidget* label_widget = host.widget_for_node_id(3u);
  GtkWidget* pressable = host.widget_for_node_id(4u);
  GtkWidget* scroll_view = host.widget_for_node_id(5u);
  GtkWidget* scroll_child = host.widget_for_node_id(6u);
  GtkWidget* text_input = host.widget_for_node_id(7u);

  NVE_CHECK(container != nullptr && GTK_IS_FIXED(container));
  NVE_CHECK(label_widget != nullptr && GTK_IS_LABEL(label_widget));
  NVE_CHECK(pressable != nullptr && GTK_IS_FIXED(pressable));
  NVE_CHECK(scroll_view != nullptr && GTK_IS_SCROLLED_WINDOW(scroll_view));
  NVE_CHECK(scroll_child != nullptr && GTK_IS_LABEL(scroll_child));
  NVE_CHECK(text_input != nullptr && GTK_IS_ENTRY(text_input));

  // Verify label properties (wrap, alignment, can_target=FALSE, text)
  NVE_CHECK(gtk_label_get_wrap(GTK_LABEL(label_widget)) == TRUE);
  NVE_CHECK(approx_eq(gtk_label_get_xalign(GTK_LABEL(label_widget)), 0.0, 1e-4));
  NVE_CHECK(approx_eq(gtk_label_get_yalign(GTK_LABEL(label_widget)), 0.0, 1e-4));
  NVE_CHECK(gtk_widget_get_can_target(label_widget) == FALSE);
  NVE_CHECK(std::string(gtk_label_get_text(GTK_LABEL(label_widget))) == "Initial:0");

  // Verify overflow clipping for rounded corners & opacity
  NVE_CHECK(gtk_widget_get_overflow(container) == GTK_OVERFLOW_HIDDEN);
  NVE_CHECK(gtk_widget_get_overflow(pressable) == GTK_OVERFLOW_HIDDEN);
  NVE_CHECK(approx_eq(gtk_widget_get_opacity(container), 0.90, 0.02));

  // Verify entry text
  NVE_CHECK(std::string(gtk_editable_get_text(GTK_EDITABLE(text_input))) == "hello");

  // Verify parent-child mounting + GtkFixed positions
  NVE_CHECK(gtk_widget_get_parent(container) == host.root_widget());
  NVE_CHECK(gtk_widget_get_parent(label_widget) == container);
  NVE_CHECK(gtk_widget_get_parent(pressable) == container);
  NVE_CHECK(gtk_widget_get_parent(scroll_view) == container);
  NVE_CHECK(gtk_widget_get_parent(text_input) == container);
  NVE_CHECK(gtk_widget_get_parent(scroll_child) != nullptr);
  NVE_CHECK(GTK_IS_FIXED(gtk_widget_get_parent(scroll_child)));

  GskTransform* label_transform =
      gtk_fixed_get_child_transform(GTK_FIXED(container), label_widget);
  NVE_CHECK(label_transform != nullptr);
  float lx = 0.0f, ly = 0.0f;
  gsk_transform_to_translate(label_transform, &lx, &ly);
  NVE_CHECK(approx_eq(lx, 20.0));
  NVE_CHECK(approx_eq(ly, 20.0));

  int lw = 0, lh = 0;
  gtk_widget_get_size_request(label_widget, &lw, &lh);
  NVE_CHECK(lw == 120 && lh == 40);

  // Verify scoped CSS rules exist with host-specific isolation
  NVE_CHECK(gtk_widget_has_css_class(pressable, "nve-node-4") == TRUE);
  const std::string expected_selector =
      ".nve-host-" + std::to_string(host.host_instance_id()) + "-node-4";
  NVE_CHECK(host.active_css().find(expected_selector) != std::string::npos);
  NVE_CHECK(host.active_css().find("border-radius: 12px;") != std::string::npos);

  // 3. apply_props_json updates text, entry, and emits outbound event
  NVE_CHECK(host.apply_props_json("{\"title\":\"Updated\",\"input\":\"typed\"}"));
  NVE_CHECK(std::string(gtk_label_get_text(GTK_LABEL(label_widget))) == "Updated:0");
  NVE_CHECK(std::string(gtk_editable_get_text(GTK_EDITABLE(text_input))) == "typed");
  NVE_CHECK(received_events.size() == 1u);
  NVE_CHECK(received_events[0].name == "onPropsApplied");

  // 4. simulate_press dispatches press to node 4, re-renders, and emits onIncrement
  NVE_CHECK(host.simulate_press(4u, 10.0f, 12.0f) == 1u);
  NVE_CHECK(std::string(gtk_label_get_text(GTK_LABEL(label_widget))) == "Updated:1");
  NVE_CHECK(received_events.size() == 2u);
  NVE_CHECK(received_events[1].name == "onIncrement");
  NVE_CHECK(received_events[1].payload_as_string() == "{\"count\":1}");

  // 5. Text leaf -> wrapper -> leaf tag swap via RetainedViewReconciler
  NVE_CHECK(host.apply_props_json("{\"wrapper\":true}"));
  GtkWidget* wrapper_widget = host.widget_for_node_id(3u);
  GtkWidget* nested_leaf = host.widget_for_node_id(8u);
  NVE_CHECK(wrapper_widget != nullptr && GTK_IS_FIXED(wrapper_widget));
  NVE_CHECK(nested_leaf != nullptr && GTK_IS_LABEL(nested_leaf));
  NVE_CHECK(gtk_widget_get_parent(nested_leaf) == wrapper_widget);
  NVE_CHECK(std::string(gtk_label_get_text(GTK_LABEL(nested_leaf))) == "Wrapped:Updated:1");

  // Swap back to leaf text -> node 8 pruned, node 3 becomes GtkLabel again
  NVE_CHECK(host.apply_props_json("{\"wrapper\":false}"));
  GtkWidget* restored_leaf = host.widget_for_node_id(3u);
  NVE_CHECK(restored_leaf != nullptr && GTK_IS_LABEL(restored_leaf));
  NVE_CHECK(host.widget_for_node_id(8u) == nullptr);
  NVE_CHECK(std::string(gtk_label_get_text(GTK_LABEL(restored_leaf))) == "Updated:1");

  // 6. Dynamic resize recalculates container & flex-grow button width
  int bw_before = 0, bh_before = 0;
  gtk_widget_get_size_request(pressable, &bw_before, &bh_before);
  host.set_surface_size(620.0f, 200.0f, 2.0f);
  int bw_after = 0, bh_after = 0;
  gtk_widget_get_size_request(pressable, &bw_after, &bh_after);
  NVE_CHECK(bw_after > bw_before);

  // 7. Explicit render_frame returns 0 and populates stats
  NveFrameStats stats{};
  NVE_CHECK(host.render_frame(&stats) == 0);
  NVE_CHECK(stats.op_count > 0u);
}

void test_dylib_error_codes_and_hot_reload() {
  NveSurfaceConfig cfg{.surface_id = 3u, .width = 360.0f, .height = 240.0f, .scale_factor = 1.0f};
  nve::NveGtkHost host(cfg);

  int err_code = 0;
  std::string err_msg;

  // Error code 1: empty path & nonexistent path
  NVE_CHECK(!host.load_dylib("", &err_code, &err_msg));
  NVE_CHECK(err_code == 1 && !err_msg.empty());

  err_code = 0;
  err_msg.clear();
  NVE_CHECK(!host.load_dylib("/nonexistent/libmissing_nve.so", &err_code, &err_msg));
  NVE_CHECK(err_code == 1 && !err_msg.empty());

  // Error code 2: missing nve_get_interface
  err_code = 0;
  err_msg.clear();
  NVE_CHECK(!host.load_dylib(NVE_TEST_NO_ENTRY_DYLIB, &err_code, &err_msg));
  NVE_CHECK(err_code == 2 && !err_msg.empty());

  // Error code 3: exports nve_get_interface but missing NVE_IFACE_SURFACE
  err_code = 0;
  err_msg.clear();
  NVE_CHECK(!host.load_dylib(NVE_TEST_NO_SURFACE_DYLIB, &err_code, &err_msg));
  NVE_CHECK(err_code == 3 && !err_msg.empty());
  NVE_CHECK(!host.is_loaded());

  // Cold reload with stateful fixture succeeds
  err_code = -1;
  err_msg = "stale";
  NVE_CHECK(host.reload_dylib(NVE_TEST_STATEFUL_DYLIB, &err_code, &err_msg));
  NVE_CHECK(err_code == 0 && err_msg.empty());
  NVE_CHECK(host.is_loaded());
  NVE_CHECK(host.node_count() == 3u);

  GtkWidget* state_label = host.widget_for_node_id(2u);
  NVE_CHECK(state_label != nullptr && GTK_IS_LABEL(state_label));
  NVE_CHECK(std::string(gtk_label_get_text(GTK_LABEL(state_label))) == "state=0");

  // Mutate state via presses and hot patch
  NVE_CHECK(host.simulate_press(3u, 5.0f, 5.0f) == 1u);
  NVE_CHECK(host.simulate_press(3u, 5.0f, 5.0f) == 1u);
  NVE_CHECK(std::string(gtk_label_get_text(GTK_LABEL(state_label))) == "state=2");

  // Failed reloads (codes 1/2/3) must leave the live app, widgets, and state intact
  NVE_CHECK(!host.reload_dylib("/nonexistent/fail.so", &err_code, &err_msg));
  NVE_CHECK(err_code == 1);
  NVE_CHECK(!host.reload_dylib(NVE_TEST_NO_ENTRY_DYLIB, &err_code, &err_msg));
  NVE_CHECK(err_code == 2);
  NVE_CHECK(!host.reload_dylib(NVE_TEST_NO_SURFACE_DYLIB, &err_code, &err_msg));
  NVE_CHECK(err_code == 3);
  NVE_CHECK(host.is_loaded());
  NVE_CHECK(std::string(gtk_label_get_text(GTK_LABEL(host.widget_for_node_id(2u)))) == "state=2");

  // Warm reload preserves state=2
  NVE_CHECK(host.reload_dylib(NVE_TEST_STATEFUL_DYLIB, &err_code, &err_msg));
  NVE_CHECK(err_code == 0 && err_msg.empty());
  NVE_CHECK(std::string(gtk_label_get_text(GTK_LABEL(host.widget_for_node_id(2u)))) == "state=2");

  // Fresh load resets state back to 0
  NVE_CHECK(host.load_dylib(NVE_TEST_STATEFUL_DYLIB, &err_code, &err_msg));
  NVE_CHECK(err_code == 0 && err_msg.empty());
  NVE_CHECK(std::string(gtk_label_get_text(GTK_LABEL(host.widget_for_node_id(2u)))) == "state=0");
}

void test_live_counter_dylib_if_present() {
  const char* dylib_env = std::getenv("NVE_TEST_KITCHEN_SINK_DYLIB");
  if (dylib_env == nullptr || dylib_env[0] == '\0') {
    return;
  }

  NveSurfaceConfig cfg{.surface_id = 1u, .width = 1280.0f, .height = 800.0f, .scale_factor = 1.0f};
  nve::NveGtkHost host(cfg);

  int err_code = -1;
  std::string err_msg;
  NVE_CHECK(host.load_dylib(dylib_env, &err_code, &err_msg));
  NVE_CHECK(err_code == 0 && err_msg.empty());
  NVE_CHECK(host.is_loaded());

  // Verify root container is mounted under root_widget()
  GtkWidget* top_container = gtk_widget_get_first_child(host.root_widget());
  NVE_CHECK(top_container != nullptr && GTK_IS_FIXED(top_container));

  // Verify core Kitchen Sink labels:
  // "NVE Kitchen Sink", "Count: 0", "Increment +", "Reset"
  const auto has_label_text = [&](std::string_view needle) {
    for (GtkLabel* lbl : collect_labels(host.root_widget())) {
      const char* txt = gtk_label_get_text(lbl);
      if (txt != nullptr && needle == txt) {
        return true;
      }
    }
    return false;
  };

  const std::vector<GtkLabel*> labels = collect_labels(host.root_widget());
  NVE_CHECK(labels.size() >= 4u);
  NVE_CHECK(has_label_text("NVE Kitchen Sink"));
  NVE_CHECK(has_label_text("Count: 0"));
  NVE_CHECK(has_label_text("Increment +"));
  NVE_CHECK(has_label_text("Reset"));

  // Every label must have non-zero Pango-measured layout dimensions and can_target=FALSE
  for (GtkLabel* lbl : labels) {
    int w = 0, h = 0;
    gtk_widget_get_size_request(GTK_WIDGET(lbl), &w, &h);
    NVE_CHECK(w > 0 && h >= 10);
    NVE_CHECK(gtk_widget_get_can_target(GTK_WIDGET(lbl)) == FALSE);
  }

  // Click Increment + twice -> Count: 2
  const uint32_t inc_node = find_pressable_node_for_label_text(host, "Increment +");
  const uint32_t reset_node = find_pressable_node_for_label_text(host, "Reset");
  NVE_CHECK(inc_node != 0u && reset_node != 0u && inc_node != reset_node);

  NVE_CHECK(host.simulate_press(inc_node, 20.0f, 20.0f) != 0u);
  NVE_CHECK(has_label_text("Count: 1"));
  NVE_CHECK(host.simulate_press(inc_node, 20.0f, 20.0f) != 0u);
  NVE_CHECK(has_label_text("Count: 2"));

  // Click Reset -> Count: 0
  NVE_CHECK(host.simulate_press(reset_node, 20.0f, 20.0f) != 0u);
  NVE_CHECK(has_label_text("Count: 0"));

  // Advance to Count: 3 via clicks, then state-preserving reload_dylib keeps Count: 3
  NVE_CHECK(host.simulate_press(inc_node, 20.0f, 20.0f) != 0u);
  NVE_CHECK(host.simulate_press(inc_node, 20.0f, 20.0f) != 0u);
  NVE_CHECK(host.simulate_press(inc_node, 20.0f, 20.0f) != 0u);
  NVE_CHECK(has_label_text("Count: 3"));

  err_code = -1;
  NVE_CHECK(host.reload_dylib(dylib_env, &err_code, &err_msg));
  NVE_CHECK(err_code == 0 && err_msg.empty());
  NVE_CHECK(has_label_text("Count: 3"));
}

void test_snapshot_replay_state_restore_and_atomic_save() {
  struct TempSnapshotGuard {
    std::filesystem::path path;
    ~TempSnapshotGuard() {
      std::error_code ec;
      std::filesystem::remove(path, ec);
    }
  };

  const std::filesystem::path snap_path =
      std::filesystem::temp_directory_path() / "nve_gtk_shell_test_frame0.nvesnap";
  TempSnapshotGuard guard{snap_path};
  {
    std::error_code ec;
    std::filesystem::remove(snap_path, ec);
  }

  NveSurfaceConfig cfg{.surface_id = 5u, .width = 480.0f, .height = 300.0f, .scale_factor = 1.0f};

  // 1. Live host mutates stateful fixture dylib to state=5 and saves atomic snapshot.
  {
    nve::NveGtkHost writer_host(cfg);
    int err_code = -1;
    std::string err_msg;
    NVE_CHECK(writer_host.load_dylib(NVE_TEST_STATEFUL_DYLIB, &err_code, &err_msg, 5u));
    NVE_CHECK(err_code == 0 && err_msg.empty());
    for (int i = 0; i < 5; ++i) {
      NVE_CHECK(writer_host.simulate_press(3u, 8.0f, 8.0f) == 1u);
    }
    GtkWidget* lbl = writer_host.widget_for_node_id(2u);
    NVE_CHECK(lbl != nullptr && GTK_IS_LABEL(lbl));
    NVE_CHECK(std::string(gtk_label_get_text(GTK_LABEL(lbl))) == "state=5");

    nve::SnapshotStatus save_status = nve::SnapshotStatus::IoError;
    NVE_CHECK(writer_host.save_snapshot(snap_path.string(), /*schema_version=*/7u, "gtk4", &save_status));
    NVE_CHECK(save_status == nve::SnapshotStatus::Ok);
    NVE_CHECK(std::filesystem::exists(snap_path));
  }

  // 2. Fresh host replays Frame-0 snapshot BEFORE any dylib loads.
  {
    nve::NveGtkHost reader_host(cfg);
    NVE_CHECK(!reader_host.is_loaded());
    NVE_CHECK(!reader_host.has_replayed_frame0());
    NVE_CHECK(reader_host.node_count() == 0u);

    // Schema mismatch must fail cleanly without mounting widgets.
    nve::SnapshotStatus mismatch_status = nve::SnapshotStatus::Ok;
    NVE_CHECK(!reader_host.replay_snapshot(
        snap_path.string(), /*expected_schema_version=*/99u, nullptr, &mismatch_status));
    NVE_CHECK(mismatch_status == nve::SnapshotStatus::SchemaVersionMismatch);
    NVE_CHECK(reader_host.node_count() == 0u);
    NVE_CHECK(!reader_host.has_replayed_frame0());

    // Matching schema replays Frame 0 immediately into live GTK4 widgets before dylib load.
    NveFrameStats replay_stats{};
    nve::SnapshotStatus replay_status = nve::SnapshotStatus::IoError;
    NVE_CHECK(reader_host.replay_snapshot(
        snap_path.string(), /*expected_schema_version=*/7u, &replay_stats, &replay_status));
    NVE_CHECK(replay_status == nve::SnapshotStatus::Ok);
    NVE_CHECK(replay_stats.op_count > 0u);
    NVE_CHECK(!reader_host.is_loaded());
    NVE_CHECK(reader_host.has_replayed_frame0());
    NVE_CHECK(reader_host.node_count() == 3u);

    GtkWidget* pre_dylib_label = reader_host.widget_for_node_id(2u);
    NVE_CHECK(pre_dylib_label != nullptr && GTK_IS_LABEL(pre_dylib_label));
    NVE_CHECK(gtk_widget_get_parent(pre_dylib_label) == reader_host.root_widget());
    NVE_CHECK(std::string(gtk_label_get_text(GTK_LABEL(pre_dylib_label))) == "state=5");

    // 3. Loading the stateful dylib automatically restores cached state=5 (not 0!).
    int err_code = -1;
    std::string err_msg;
    NVE_CHECK(reader_host.load_dylib(NVE_TEST_STATEFUL_DYLIB, &err_code, &err_msg, 5u));
    NVE_CHECK(err_code == 0 && err_msg.empty());
    NVE_CHECK(reader_host.is_loaded());
    NVE_CHECK(!reader_host.has_replayed_frame0());

    GtkWidget* live_label = reader_host.widget_for_node_id(2u);
    NVE_CHECK(live_label != nullptr && GTK_IS_LABEL(live_label));
    NVE_CHECK(std::string(gtk_label_get_text(GTK_LABEL(live_label))) == "state=5");

    // 4. Subsequent interaction increments from restored state (5 -> 6) and re-saves atomically.
    NVE_CHECK(reader_host.simulate_press(3u, 8.0f, 8.0f) == 1u);
    NVE_CHECK(std::string(gtk_label_get_text(GTK_LABEL(live_label))) == "state=6");
    NVE_CHECK(reader_host.save_snapshot(snap_path.string(), /*schema_version=*/7u));
  }

  // 5. Re-verify updated atomic snapshot reflects state=6.
  {
    nve::NveGtkHost verify_host(cfg);
    NVE_CHECK(verify_host.replay_snapshot(snap_path.string(), /*expected_schema_version=*/7u));
    GtkWidget* lbl = verify_host.widget_for_node_id(2u);
    NVE_CHECK(lbl != nullptr && GTK_IS_LABEL(lbl));
    NVE_CHECK(std::string(gtk_label_get_text(GTK_LABEL(lbl))) == "state=6");
  }
}

struct MockGtkA11yImeComponent {
  static inline MockGtkA11yImeComponent* last_instance = nullptr;
  uint32_t surface_id{1u};
  bool disabled_btn{false};
  bool secure_input{false};
  std::string input_text{"initial"};
  int text_change_calls{0};
  int submit_calls{0};
  std::string last_text;
  std::string last_submit;
  std::vector<uint8_t> cmd_buf;

  static NveAppHandle create(uint32_t sid) {
    auto* self = new MockGtkA11yImeComponent();
    self->surface_id = sid;
    last_instance = self;
    return self;
  }
  static void destroy(NveAppHandle h) {
    auto* self = static_cast<MockGtkA11yImeComponent*>(h);
    if (last_instance == self) last_instance = nullptr;
    delete self;
  }

  template <typename T>
  void push(T v) {
    const auto* p = reinterpret_cast<const uint8_t*>(&v);
    cmd_buf.insert(cmd_buf.end(), p, p + sizeof(T));
  }

  static NveBufferSlice render(NveAppHandle h, uint64_t frame_id) {
    auto* self = static_cast<MockGtkA11yImeComponent*>(h);
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
      self->push<float>(0.0f); self->push<float>(1.0f); self->push<float>(0.0f);
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
    auto set_raw_bool = [&](uint32_t id, std::string_view k, bool v) {
      self->push<uint8_t>(NVE_OP_SET_RAW_PROP);
      self->push<uint32_t>(id);
      self->push<uint16_t>(static_cast<uint16_t>(k.size()));
      self->cmd_buf.insert(self->cmd_buf.end(), k.begin(), k.end());
      self->push<uint8_t>(1u);
      self->push<uint8_t>(v ? 1u : 0u);
      ++ops;
    };

    create_node(2u, NVE_COMP_PRESSABLE);
    create_node(3u, NVE_COMP_TEXT);
    create_node(4u, NVE_COMP_VIEW);
    create_node(5u, NVE_COMP_TEXT_INPUT);

    set_children(NVE_ROOT_NODE_ID, {2u, 3u, 4u, 5u});
    set_box(2u, 140.0f, 36.0f);
    set_box(3u, 180.0f, 24.0f);
    set_box(4u, 100.0f, 24.0f);
    set_box(5u, 220.0f, 32.0f);

    set_raw_str(2u, "accessibilityLabel", "Publish Release");
    set_raw_str(2u, "accessibilityHint", "Deploys current build");
    set_raw_str(2u, "accessibilityValue", "ready");
    set_raw_str(2u, "testID", "gtk-publish-btn");
    set_raw_bool(2u, "accessibilityDisabled", self->disabled_btn);

    set_text(3u, "Dashboard");
    set_raw_str(3u, "role", "header");
    set_raw_str(3u, "data-testid", "gtk-header");

    set_raw_str(4u, "accessibilityRole", "checkbox");
    set_raw_str(4u, "aria-label", "Dark Theme");

    set_text(5u, self->input_text);
    set_raw_str(5u, "placeholder", "Type UTF-8 text...");
    set_raw_str(5u, "accessibilityLabel", "Release Note");
    set_raw_str(5u, "testID", "gtk-note-input");
    set_raw_bool(5u, "secureTextEntry", self->secure_input);

    self->push<uint8_t>(NVE_OP_COMMIT);
    self->push<uint8_t>(1u);
    ++ops;
    std::memcpy(self->cmd_buf.data() + 20, &ops, sizeof(uint32_t));
    return NveBufferSlice{self->cmd_buf.data(), self->cmd_buf.size()};
  }

  static uint32_t dispatch_events(NveAppHandle h, NveBufferSlice events) {
    auto* self = static_cast<MockGtkA11yImeComponent*>(h);
    std::vector<nve::DecodedEvent> decoded;
    if (!nve::decode_event_ring(events, decoded)) return 0u;
    for (const auto& ev : decoded) {
      if (ev.node_id == 5u && ev.kind == nve::EventKind::TextChange) {
        ++self->text_change_calls;
        self->last_text = ev.text;
        return 1u; // Force a re-render while inside `changed` to test loop/IME guard!
      }
      if (ev.node_id == 5u && ev.kind == nve::EventKind::Custom && ev.name == "onSubmit") {
        ++self->submit_calls;
        if (const auto v = nve::BinValueView::parse(ev.payload_bytes.data(), ev.payload_bytes.size());
            v && v->as_str()) {
          self->last_submit = std::string(*v->as_str());
        }
      }
    }
    return 0u;
  }

  static uint32_t dispatch_events_direct(NveAppHandle h, NveEventBatch batch) {
    auto* self = static_cast<MockGtkA11yImeComponent*>(h);
    if (batch.ptr == nullptr) return 0u;
    uint32_t dirty = 0u;
    for (uint32_t i = 0u; i < batch.len; ++i) {
      const NveDirectEvent& ev = batch.ptr[i];
      if (ev.node_id == 5u && ev.kind == static_cast<uint8_t>(nve::EventKind::TextChange)) {
        ++self->text_change_calls;
        const auto sv = nve::str_slice_view(ev.text);
        self->last_text.assign(sv.data(), sv.size());
        dirty = 1u;
      } else if (ev.node_id == 5u && ev.kind == static_cast<uint8_t>(nve::EventKind::Custom) &&
                 nve::str_slice_view(ev.name) == "onSubmit") {
        ++self->submit_calls;
        const auto sv = nve::str_slice_view(ev.text);
        self->last_submit.assign(sv.data(), sv.size());
      }
    }
    return dirty;
  }

  static uint32_t set_props(NveAppHandle h, NveBufferSlice props) {
    auto* self = static_cast<MockGtkA11yImeComponent*>(h);
    const std::string json(reinterpret_cast<const char*>(props.ptr), props.len);
    if (json.find("\"disabled\":true") != std::string::npos) self->disabled_btn = true;
    if (json.find("\"secure\":true") != std::string::npos) self->secure_input = true;
    if (json.find("\"text\":\"programmatic\"") != std::string::npos) self->input_text = "programmatic";
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

void test_accessibility_and_ime_text_input() {
  NveSurfaceConfig cfg{.surface_id = 12u, .width = 480.0f, .height = 300.0f, .scale_factor = 1.0f};
  nve::NveGtkHost host(cfg);
  NVE_CHECK(host.attach_interface(&MockGtkA11yImeComponent::get_interface, 12u));
  NVE_CHECK(MockGtkA11yImeComponent::last_instance != nullptr);
  auto* mock = MockGtkA11yImeComponent::last_instance;

  GtkWidget* btn = host.widget_for_node_id(2u);
  GtkWidget* hdr = host.widget_for_node_id(3u);
  GtkWidget* chk = host.widget_for_node_id(4u);
  GtkWidget* entry = host.widget_for_node_id(5u);
  NVE_CHECK(btn != nullptr && hdr != nullptr && chk != nullptr && entry != nullptr);
  NVE_CHECK(GTK_IS_ENTRY(entry));

  // Verify GTK4 accessible roles, properties, states, and widget names (`testID`)
  gtk_test_accessible_assert_role(btn, GTK_ACCESSIBLE_ROLE_BUTTON);
  gtk_test_accessible_assert_property(btn, GTK_ACCESSIBLE_PROPERTY_LABEL, "Publish Release");
  gtk_test_accessible_assert_property(btn, GTK_ACCESSIBLE_PROPERTY_DESCRIPTION, "Deploys current build");
  gtk_test_accessible_assert_property(btn, GTK_ACCESSIBLE_PROPERTY_VALUE_TEXT, "ready");
  gtk_test_accessible_assert_state(btn, GTK_ACCESSIBLE_STATE_DISABLED, FALSE);
  NVE_CHECK(gtk_widget_get_sensitive(btn) == TRUE);
  NVE_CHECK(std::string(gtk_widget_get_name(btn)) == "gtk-publish-btn");

  gtk_test_accessible_assert_role(hdr, GTK_ACCESSIBLE_ROLE_HEADING);
  gtk_test_accessible_assert_property(hdr, GTK_ACCESSIBLE_PROPERTY_LABEL, "Dashboard");
  NVE_CHECK(std::string(gtk_widget_get_name(hdr)) == "gtk-header");

  gtk_test_accessible_assert_role(chk, GTK_ACCESSIBLE_ROLE_CHECKBOX);
  gtk_test_accessible_assert_property(chk, GTK_ACCESSIBLE_PROPERTY_LABEL, "Dark Theme");

  gtk_test_accessible_assert_role(entry, GTK_ACCESSIBLE_ROLE_TEXT_BOX);
  gtk_test_accessible_assert_property(entry, GTK_ACCESSIBLE_PROPERTY_LABEL, "Release Note");
  NVE_CHECK(std::string(gtk_widget_get_name(entry)) == "gtk-note-input");

  // Verify GtkEntry placeholder-text, visibility, and zero feedback loop on initial mount
  const char* ph = gtk_entry_get_placeholder_text(GTK_ENTRY(entry));
  NVE_CHECK(ph != nullptr && std::string(ph) == "Type UTF-8 text...");
  NVE_CHECK(gtk_entry_get_visibility(GTK_ENTRY(entry)) == TRUE);
  NVE_CHECK(std::string(gtk_editable_get_text(GTK_EDITABLE(entry))) == "initial");
  NVE_CHECK(mock->text_change_calls == 0);

  // Programmatic snapshot text update must NOT trigger a feedback `TextChange` loop
  NVE_CHECK(host.apply_props_json(R"({"text":"programmatic"})"));
  NVE_CHECK(std::string(gtk_editable_get_text(GTK_EDITABLE(entry))) == "programmatic");
  NVE_CHECK(mock->text_change_calls == 0);

  // IME-style UTF-8 insertion fires `changed` -> dispatches exactly 1 TextChange and does NOT get
  // clobbered by the synchronous re-render whose snapshot still holds "programmatic"!
  {
    int pos = static_cast<int>(std::string("programmatic").size());
    gtk_editable_insert_text(GTK_EDITABLE(entry), " 日本語 café", -1, &pos);
  }
  NVE_CHECK(mock->text_change_calls == 1);
  NVE_CHECK(mock->last_text == "programmatic 日本語 café");
  NVE_CHECK(std::string(gtk_editable_get_text(GTK_EDITABLE(entry))) == "programmatic 日本語 café");

  // Full replacement via gtk_editable_set_text (delete+insert) ends cleanly on exact UTF-8 text
  gtk_editable_set_text(GTK_EDITABLE(entry), "日本語 café");
  const int calls_after_replace = mock->text_change_calls;
  NVE_CHECK(calls_after_replace >= 2);
  NVE_CHECK(mock->last_text == "日本語 café");
  NVE_CHECK(std::string(gtk_editable_get_text(GTK_EDITABLE(entry))) == "日本語 café");

  // Unrelated re-render with unchanged snapshot text still preserves user/IME buffer
  NVE_CHECK(host.render_frame() == 0);
  NVE_CHECK(std::string(gtk_editable_get_text(GTK_EDITABLE(entry))) == "日本語 café");
  NVE_CHECK(mock->text_change_calls == calls_after_replace);

  // `activate` signal -> dispatches `onSubmit` with exact UTF-8 text
  g_signal_emit_by_name(entry, "activate");
  NVE_CHECK(mock->submit_calls == 1);
  NVE_CHECK(mock->last_submit == "日本語 café");

  // Toggle disabled=true and secureTextEntry=true
  NVE_CHECK(host.apply_props_json(R"({"disabled":true,"secure":true})"));
  gtk_test_accessible_assert_state(btn, GTK_ACCESSIBLE_STATE_DISABLED, TRUE);
  NVE_CHECK(gtk_widget_get_sensitive(btn) == FALSE);
  NVE_CHECK(gtk_entry_get_visibility(GTK_ENTRY(entry)) == FALSE);
}

std::string make_8x8_png_data_uri(uint8_t r, uint8_t g, uint8_t b, uint8_t a) {
  std::vector<uint8_t> rgba(8u * 8u * 4u);
  for (size_t i = 0; i < rgba.size(); i += 4u) {
    rgba[i + 0u] = r;
    rgba[i + 1u] = g;
    rgba[i + 2u] = b;
    rgba[i + 3u] = a;
  }
  GBytes* raw = g_bytes_new(rgba.data(), rgba.size());
  GdkTexture* tex = gdk_memory_texture_new(8, 8, GDK_MEMORY_R8G8B8A8, raw, 8 * 4);
  g_bytes_unref(raw);
  NVE_CHECK(tex != nullptr);
  GBytes* png = gdk_texture_save_to_png_bytes(tex);
  g_object_unref(tex);
  NVE_CHECK(png != nullptr);
  gsize png_len = 0;
  const auto* png_data = static_cast<const guchar*>(g_bytes_get_data(png, &png_len));
  gchar* b64 = g_base64_encode(png_data, png_len);
  std::string out = "data:image/png;base64," + std::string(b64);
  g_free(b64);
  g_bytes_unref(png);
  return out;
}

struct MockGtkImageScrollComponent {
  static inline MockGtkImageScrollComponent* last_instance = nullptr;
  uint32_t surface_id{1u};
  std::string data_uri;
  std::string file_uri;
  std::string resize_mode{"cover"};
  int counter{0};
  int scroll_events{0};
  uint32_t last_scroll_node{0};
  float last_scroll_x{0.0f};
  float last_scroll_y{0.0f};
  float last_scroll_content_h{0.0f};
  std::vector<uint8_t> cmd_buf;

  static NveAppHandle create(uint32_t sid) {
    auto* self = new MockGtkImageScrollComponent();
    self->surface_id = sid;
    last_instance = self;
    return self;
  }
  static void destroy(NveAppHandle h) {
    auto* self = static_cast<MockGtkImageScrollComponent*>(h);
    if (last_instance == self) last_instance = nullptr;
    delete self;
  }

  template <typename T>
  void push(T v) {
    const auto* p = reinterpret_cast<const uint8_t*>(&v);
    cmd_buf.insert(cmd_buf.end(), p, p + sizeof(T));
  }

  static NveBufferSlice render(NveAppHandle h, uint64_t frame_id) {
    auto* self = static_cast<MockGtkImageScrollComponent*>(h);
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
    create_node(4u, NVE_COMP_IMAGE);
    create_node(5u, NVE_COMP_SCROLL_VIEW);
    create_node(6u, NVE_COMP_VIEW);
    create_node(7u, NVE_COMP_TEXT);
    create_node(8u, NVE_COMP_TEXT);

    set_children(NVE_ROOT_NODE_ID, {2u});
    set_children(2u, {3u, 4u, 5u, 7u, 8u});
    set_children(5u, {6u});

    set_box(2u, 360.0f, 300.0f);
    set_box(3u, 64.0f, 64.0f);
    set_raw_str(3u, "src", self->data_uri);
    set_raw_str(3u, "resizeMode", self->resize_mode);

    set_box(4u, 48.0f, 48.0f);
    set_raw_str(4u, "src", self->file_uri);
    set_raw_str(4u, "resizeMode", "contain");

    set_box(5u, 160.0f, 60.0f);
    set_box(6u, 220.0f, 240.0f);

    set_box(7u, 120.0f, 24.0f);
    set_text(7u, "Step:" + std::to_string(self->counter));

    set_box(8u, 120.0f, 24.0f);
    set_text(8u, "StaticLabel");

    self->push<uint8_t>(NVE_OP_COMMIT);
    self->push<uint8_t>(1u);
    ++ops;
    std::memcpy(self->cmd_buf.data() + 20, &ops, sizeof(uint32_t));
    return NveBufferSlice{self->cmd_buf.data(), self->cmd_buf.size()};
  }

  static uint32_t dispatch_events_direct(NveAppHandle h, NveEventBatch batch) {
    auto* self = static_cast<MockGtkImageScrollComponent*>(h);
    if (batch.ptr == nullptr) return 0u;
    for (uint32_t i = 0; i < batch.len; ++i) {
      const NveDirectEvent& ev = batch.ptr[i];
      if (ev.kind == static_cast<uint8_t>(nve::EventKind::Scroll)) {
        ++self->scroll_events;
        self->last_scroll_node = ev.node_id;
        self->last_scroll_x = ev.x;
        self->last_scroll_y = ev.y;
        self->last_scroll_content_h = ev.height;
      } else if (ev.kind == static_cast<uint8_t>(nve::EventKind::Press) && ev.node_id == 7u) {
        ++self->counter;
        return 1u;
      }
    }
    return 0u;
  }

  static uint32_t dispatch_events(NveAppHandle h, NveBufferSlice events) {
    auto* self = static_cast<MockGtkImageScrollComponent*>(h);
    std::vector<nve::DecodedEvent> decoded;
    if (!nve::decode_event_ring(events, decoded)) return 0u;
    for (const auto& ev : decoded) {
      if (ev.kind == nve::EventKind::Press && ev.node_id == 7u) {
        ++self->counter;
        return 1u;
      }
    }
    return 0u;
  }

  static uint32_t set_props(NveAppHandle h, NveBufferSlice props) {
    auto* self = static_cast<MockGtkImageScrollComponent*>(h);
    const std::string json(reinterpret_cast<const char*>(props.ptr), props.len);
    if (json.find("\"contain\"") != std::string::npos) self->resize_mode = "contain";
    if (json.find("\"stretch\"") != std::string::npos) self->resize_mode = "stretch";
    if (json.find("\"center\"") != std::string::npos) self->resize_mode = "center";
    if (json.find("\"cover\"") != std::string::npos) self->resize_mode = "cover";
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

void test_image_picture_scroll_and_incremental_reconcile() {
  const std::string data_uri = make_8x8_png_data_uri(220, 40, 60, 255);
  const std::filesystem::path tmp_png =
      std::filesystem::temp_directory_path() / "nve_gtk_img_8x8.png";
  {
    std::vector<uint8_t> rgba(8u * 8u * 4u, 200u);
    GBytes* raw = g_bytes_new(rgba.data(), rgba.size());
    GdkTexture* tex = gdk_memory_texture_new(8, 8, GDK_MEMORY_R8G8B8A8, raw, 8 * 4);
    g_bytes_unref(raw);
    NVE_CHECK(tex != nullptr);
    NVE_CHECK(gdk_texture_save_to_png(tex, tmp_png.c_str()) == TRUE);
    g_object_unref(tex);
  }
  gchar* uri_c = g_filename_to_uri(tmp_png.c_str(), nullptr, nullptr);
  NVE_CHECK(uri_c != nullptr);
  const std::string file_uri(uri_c);
  g_free(uri_c);

  NveSurfaceConfig cfg{.surface_id = 15u, .width = 360.0f, .height = 300.0f, .scale_factor = 1.0f};
  nve::NveGtkHost host(cfg);
  NVE_CHECK(host.attach_interface(&MockGtkImageScrollComponent::get_interface, 15u));
  NVE_CHECK(MockGtkImageScrollComponent::last_instance != nullptr);
  auto* mock = MockGtkImageScrollComponent::last_instance;
  mock->data_uri = data_uri;
  mock->file_uri = file_uri;
  NVE_CHECK(host.render_frame() == 0);

  GtkWidget* pic3 = host.widget_for_node_id(3u);
  GtkWidget* pic4 = host.widget_for_node_id(4u);
  GtkWidget* scroll5 = host.widget_for_node_id(5u);
  GtkWidget* lbl7 = host.widget_for_node_id(7u);
  NVE_CHECK(pic3 != nullptr && GTK_IS_PICTURE(pic3));
  NVE_CHECK(pic4 != nullptr && GTK_IS_PICTURE(pic4));
  NVE_CHECK(scroll5 != nullptr && GTK_IS_SCROLLED_WINDOW(scroll5));
  NVE_CHECK(lbl7 != nullptr && GTK_IS_LABEL(lbl7));

  // Verify GdkTexture decoded cleanly to 8x8 for both data:image/png;base64 and file://!
  GdkPaintable* p3 = gtk_picture_get_paintable(GTK_PICTURE(pic3));
  GdkPaintable* p4 = gtk_picture_get_paintable(GTK_PICTURE(pic4));
  NVE_CHECK(p3 != nullptr && GDK_IS_TEXTURE(p3));
  NVE_CHECK(p4 != nullptr && GDK_IS_TEXTURE(p4));
  NVE_CHECK(gdk_texture_get_width(GDK_TEXTURE(p3)) == 8);
  NVE_CHECK(gdk_texture_get_height(GDK_TEXTURE(p3)) == 8);
  NVE_CHECK(gdk_texture_get_width(GDK_TEXTURE(p4)) == 8);
  NVE_CHECK(gdk_texture_get_height(GDK_TEXTURE(p4)) == 8);
  NVE_CHECK(host.texture_cache().size() == 2u);

  // Verify GtkContentFit mapping across all 4 resizeModes!
#if GTK_CHECK_VERSION(4, 8, 0)
  NVE_CHECK(gtk_picture_get_content_fit(GTK_PICTURE(pic3)) == GTK_CONTENT_FIT_COVER);
  NVE_CHECK(gtk_picture_get_content_fit(GTK_PICTURE(pic4)) == GTK_CONTENT_FIT_CONTAIN);

  NVE_CHECK(host.apply_props_json(R"({"resizeMode":"contain"})"));
  NVE_CHECK(gtk_picture_get_content_fit(GTK_PICTURE(pic3)) == GTK_CONTENT_FIT_CONTAIN);

  NVE_CHECK(host.apply_props_json(R"({"resizeMode":"stretch"})"));
  NVE_CHECK(gtk_picture_get_content_fit(GTK_PICTURE(pic3)) == GTK_CONTENT_FIT_FILL);

  NVE_CHECK(host.apply_props_json(R"({"resizeMode":"center"})"));
  NVE_CHECK(gtk_picture_get_content_fit(GTK_PICTURE(pic3)) == GTK_CONTENT_FIT_SCALE_DOWN);
#endif

  // Verify GtkScrolledWindow vadjustment & hadjustment value-changed -> zero-copy EventKind::Scroll!
  GtkAdjustment* vadj = gtk_scrolled_window_get_vadjustment(GTK_SCROLLED_WINDOW(scroll5));
  GtkAdjustment* hadj = gtk_scrolled_window_get_hadjustment(GTK_SCROLLED_WINDOW(scroll5));
  NVE_CHECK(vadj != nullptr && hadj != nullptr);
  gtk_adjustment_set_value(vadj, 42.0);
  NVE_CHECK(mock->scroll_events >= 1);
  NVE_CHECK(mock->last_scroll_node == 5u);
  NVE_CHECK(approx_eq(mock->last_scroll_y, 42.0, 1.0));
  NVE_CHECK(mock->last_scroll_content_h >= 240.0f);

  const int events_before_h = mock->scroll_events;
  gtk_adjustment_set_value(hadj, 15.0);
  NVE_CHECK(mock->scroll_events > events_before_h);
  NVE_CHECK(approx_eq(mock->last_scroll_x, 15.0, 1.0));

  // Verify O(changed) IncrementalSceneReconciler updates ONLY node 7 on single-label mutation!
  const uint64_t updates_before = host.reconcile_update_count();
  NVE_CHECK(host.simulate_press(7u, 4.0f, 4.0f) == 1u);
  NVE_CHECK(std::string(gtk_label_get_text(GTK_LABEL(lbl7))) == "Step:1");
  NVE_CHECK(host.reconcile_update_count() - updates_before == 1u);

  std::error_code ec;
  std::filesystem::remove(tmp_png, ec);
}

struct MockGtkCanvasComponent {
  static inline MockGtkCanvasComponent* last_instance = nullptr;
  uint32_t surface_id{1u};
  std::string draw_list_attr;
  bool emit_raw_corrupt{false};
  std::string raw_corrupt;
  int tick{0};
  std::vector<uint8_t> cmd_buf;

  static NveAppHandle create(uint32_t sid) {
    auto* self = new MockGtkCanvasComponent();
    self->surface_id = sid;
    last_instance = self;
    return self;
  }
  static void destroy(NveAppHandle h) {
    auto* self = static_cast<MockGtkCanvasComponent*>(h);
    if (last_instance == self) last_instance = nullptr;
    delete self;
  }
  template <typename T>
  void push(T v) {
    const auto* p = reinterpret_cast<const uint8_t*>(&v);
    cmd_buf.insert(cmd_buf.end(), p, p + sizeof(T));
  }

  static NveBufferSlice render(NveAppHandle h, uint64_t frame_id) {
    auto* self = static_cast<MockGtkCanvasComponent*>(h);
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
    if (self->emit_raw_corrupt) {
      set_raw_str(3u, "drawList", "");
      set_text(3u, self->raw_corrupt);
    } else {
      set_raw_str(3u, "drawList", self->draw_list_attr);
    }

    set_box(4u, 120.0f, 24.0f);
    set_text(4u, "Tick:" + std::to_string(self->tick));

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

void test_cairo_draw_list_sparkline_and_drawing_area() {
  const std::vector<float> samples{14.0f, 28.0f, 19.0f, 47.0f, 38.0f, 70.0f};
  const nve::DrawColorRgba bg_col = nve::DrawColorRgba::rgb(15u, 23u, 42u);
  const nve::DrawColorRgba stroke_col = nve::DrawColorRgba::rgb(56u, 189u, 248u);

  nve::DrawListEncoder spark_enc;
  spark_enc.sparkline(samples, 200.0f, 80.0f, stroke_col, bg_col);
  const std::vector<uint8_t> spark_bytes = spark_enc.finish();

  // 1. Rasterize sparkline onto offscreen cairo_image_surface_t and assert exact pixels!
  {
    cairo_surface_t* surf = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, 200, 80);
    cairo_t* cr = cairo_create(surf);
    const auto res = nve::render_draw_list_to_cairo(cr, 200.0, 80.0, spark_bytes);
    NVE_CHECK(res.ok());
    NVE_CHECK(res.ops_decoded >= 8u);
    NVE_CHECK(cairo_status(cr) == CAIRO_STATUS_SUCCESS);
    cairo_surface_flush(surf);

    const unsigned char* data = cairo_image_surface_get_data(surf);
    const int stride = cairo_image_surface_get_stride(surf);
    NVE_CHECK(data != nullptr && stride >= 200 * 4);

    int bg_pixels = 0;
    int stroke_pixels = 0;
    for (int y = 0; y < 80; ++y) {
      const auto* row = reinterpret_cast<const uint32_t*>(data + y * stride);
      for (int x = 0; x < 200; ++x) {
        const uint32_t px = row[x]; // Native-endian premultiplied ARGB32
        const uint8_t a = static_cast<uint8_t>((px >> 24u) & 0xFFu);
        const uint8_t r = static_cast<uint8_t>((px >> 16u) & 0xFFu);
        const uint8_t g = static_cast<uint8_t>((px >> 8u) & 0xFFu);
        const uint8_t b = static_cast<uint8_t>(px & 0xFFu);
        if (a == 255u && std::abs(static_cast<int>(r) - 15) <= 2 &&
            std::abs(static_cast<int>(g) - 23) <= 2 &&
            std::abs(static_cast<int>(b) - 42) <= 2) {
          ++bg_pixels;
        }
        if (a == 255u && b > 210u && g > 150u && r < 90u) {
          ++stroke_pixels;
        }
      }
    }
    NVE_CHECK(bg_pixels > 5000);
    NVE_CHECK(stroke_pixels > 40);

    // 2. Unbalanced ClipRect + extra RestoreClip must leave Cairo clip extents intact!
    double cx0 = 0.0, cy0 = 0.0, cx1 = 0.0, cy1 = 0.0;
    cairo_clip_extents(cr, &cx0, &cy0, &cx1, &cy1);

    nve::DrawListEncoder unbal;
    unbal.clip_rect(10.0f, 10.0f, 30.0f, 30.0f)
        .clip_rect(15.0f, 15.0f, 20.0f, 20.0f)
        .fill_rect(0.0f, 0.0f, 200.0f, 80.0f, nve::DrawColorRgba::rgb(240u, 30u, 50u));
    NVE_CHECK(nve::render_draw_list_to_cairo(cr, 200.0, 80.0, unbal.finish()).ok());

    double ax0 = 0.0, ay0 = 0.0, ax1 = 0.0, ay1 = 0.0;
    cairo_clip_extents(cr, &ax0, &ay0, &ax1, &ay1);
    NVE_CHECK(approx_eq(ax0, cx0) && approx_eq(ay0, cy0) &&
              approx_eq(ax1, cx1) && approx_eq(ay1, cy1));

    nve::DrawListEncoder extra_pop;
    extra_pop.restore_clip().restore_clip().fill_circle(
        50.0f, 40.0f, 6.0f, nve::DrawColorRgba::rgb(34u, 197u, 94u));
    NVE_CHECK(nve::render_draw_list_to_cairo(cr, 200.0, 80.0, extra_pop.finish()).ok());
    NVE_CHECK(cairo_status(cr) == CAIRO_STATUS_SUCCESS);

    // 3. Corrupt buffers (bad magic, truncated, trailing bytes) fail cleanly without corrupting Cairo!
    const std::vector<uint8_t> bad_magic{'N', 'O', 'P', 'E', '0', '0', '0', '0', '0', 1, 0, 0, 0, 0, 0};
    NVE_CHECK(nve::render_draw_list_to_cairo(cr, 200.0, 80.0, bad_magic).error ==
              nve::DrawListError::BadMagic);

    std::vector<uint8_t> truncated = spark_bytes;
    truncated.resize(20u);
    NVE_CHECK(!nve::render_draw_list_to_cairo(cr, 200.0, 80.0, truncated).ok());

    std::vector<uint8_t> trailing = spark_bytes;
    trailing.push_back(0xFFu);
    NVE_CHECK(nve::render_draw_list_to_cairo(cr, 200.0, 80.0, trailing).error ==
              nve::DrawListError::TrailingBytes);
    NVE_CHECK(cairo_status(cr) == CAIRO_STATUS_SUCCESS);

    cairo_destroy(cr);
    cairo_surface_destroy(surf);
  }

  // 4. End-to-end NveGtkHost -> GtkDrawingArea mounting, byte caching, and redraw deduplication!
  NveSurfaceConfig cfg{.surface_id = 21u, .width = 240.0f, .height = 140.0f, .scale_factor = 1.0f};
  nve::NveGtkHost host(cfg);
  NVE_CHECK(host.attach_interface(&MockGtkCanvasComponent::get_interface, 21u));
  NVE_CHECK(MockGtkCanvasComponent::last_instance != nullptr);
  auto* mock = MockGtkCanvasComponent::last_instance;

  GtkWidget* canvas = host.widget_for_node_id(3u);
  NVE_CHECK(canvas != nullptr && GTK_IS_DRAWING_AREA(canvas));
  NVE_CHECK(host.canvas_draw_list_for_node_id(3u).empty());

  mock->draw_list_attr = nve::encode_draw_list_attr_base64(spark_bytes);
  NVE_CHECK(host.render_frame() == 0);
  const auto cached = host.canvas_draw_list_for_node_id(3u);
  NVE_CHECK(cached.size() == spark_bytes.size());
  NVE_CHECK(std::memcmp(cached.data(), spark_bytes.data(), spark_bytes.size()) == 0);
  NVE_CHECK(host.canvas_redraw_count_for_node_id(3u) == 1u);

  // Sibling-only mutation with identical canvas bytes must NOT increment redraw count!
  mock->tick = 1;
  NVE_CHECK(host.render_frame() == 0);
  NVE_CHECK(host.canvas_redraw_count_for_node_id(3u) == 1u);

  // Updating drawList bytes triggers exactly 1 new queue_draw!
  nve::DrawListEncoder alt_enc;
  alt_enc.sparkline(samples, 200.0f, 80.0f, nve::DrawColorRgba::rgb(236u, 72u, 153u), bg_col);
  mock->draw_list_attr = alt_enc.to_attr_string();
  NVE_CHECK(host.render_frame() == 0);
  NVE_CHECK(host.canvas_redraw_count_for_node_id(3u) == 2u);

  // Corrupt raw bytes survive host update cleanly.
  mock->emit_raw_corrupt = true;
  mock->raw_corrupt.assign(reinterpret_cast<const char*>(spark_bytes.data()), 18u);
  NVE_CHECK(host.render_frame() == 0);
  NVE_CHECK(host.canvas_redraw_count_for_node_id(3u) == 3u);
}

void test_paired_thin_client_gtk_host() {
  auto build_frame = [](uint64_t fid, const std::string& label_text) {
    nve::CommandBufferEncoder enc(1u, fid);
    enc.create_node(2u, NVE_COMP_VIEW);
    enc.create_node(3u, NVE_COMP_TEXT);
    enc.create_node(4u, NVE_COMP_PRESSABLE);
    enc.set_children(NVE_ROOT_NODE_ID, {2u});
    enc.set_children(2u, {3u, 4u});
    nve::LayoutProps box{};
    box.width = {1u, 300.0f};
    box.height = {1u, 180.0f};
    enc.set_layout(2u, box);
    nve::LayoutProps btn{};
    btn.width = {1u, 140.0f};
    btn.height = {1u, 40.0f};
    enc.set_layout(4u, btn);
    enc.set_text(3u, label_text);
    enc.set_event_mask(4u, nve::kEventMaskPress);
    enc.commit(true);
    const NveBufferSlice s = enc.finish();
    return std::vector<uint8_t>(s.ptr, s.ptr + s.len);
  };

  const std::string token = "gtk-pair-token";
  const auto f1 = build_frame(1u, "GTK Paired V1");
  const auto f2 = build_frame(2u, "GTK Paired V2");

  int lfd = ::socket(AF_INET, SOCK_STREAM, 0);
  NVE_CHECK(lfd >= 0);
  int one = 1;
  (void)::setsockopt(lfd, SOL_SOCKET, SO_REUSEADDR, &one, sizeof(one));
  sockaddr_in addr{};
  addr.sin_family = AF_INET;
  addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
  addr.sin_port = 0;
  NVE_CHECK(::bind(lfd, reinterpret_cast<sockaddr*>(&addr), sizeof(addr)) == 0);
  socklen_t alen = sizeof(addr);
  NVE_CHECK(::getsockname(lfd, reinterpret_cast<sockaddr*>(&addr), &alen) == 0);
  const uint16_t port = ntohs(addr.sin_port);
  NVE_CHECK(::listen(lfd, 2) == 0);

  std::atomic<bool> saw_press{false};
  std::thread srv([&]() {
    auto read_exact = [](int fd, uint8_t* buf, size_t len) -> bool {
      size_t off = 0u;
      const auto dl = std::chrono::steady_clock::now() + std::chrono::seconds(3);
      while (off < len) {
        if (std::chrono::steady_clock::now() >= dl) return false;
        pollfd pfd{fd, POLLIN, 0};
        if (::poll(&pfd, 1u, 25) <= 0) continue;
        const ssize_t n = ::recv(fd, buf + off, len - off, 0);
        if (n <= 0) return false;
        off += static_cast<size_t>(n);
      }
      return true;
    };
    auto read_msg = [&](int fd, std::vector<uint8_t>& out) -> bool {
      uint8_t hdr[4]{};
      if (!read_exact(fd, hdr, 4u)) return false;
      const uint32_t len = static_cast<uint32_t>(hdr[0]) | (static_cast<uint32_t>(hdr[1]) << 8u) |
                           (static_cast<uint32_t>(hdr[2]) << 16u) |
                           (static_cast<uint32_t>(hdr[3]) << 24u);
      out.resize(len);
      return read_exact(fd, out.data(), len);
    };
    auto write_msg = [](int fd, std::span<const uint8_t> p) -> bool {
      uint8_t hdr[4]{
          static_cast<uint8_t>(p.size() & 0xFFu),
          static_cast<uint8_t>((p.size() >> 8u) & 0xFFu),
          static_cast<uint8_t>((p.size() >> 16u) & 0xFFu),
          static_cast<uint8_t>((p.size() >> 24u) & 0xFFu),
      };
      return ::send(fd, hdr, 4u, 0) == 4 &&
             ::send(fd, p.data(), p.size(), 0) == static_cast<ssize_t>(p.size());
    };

    int cfd = ::accept(lfd, nullptr, nullptr);
    NVE_CHECK(cfd >= 0);
    std::vector<uint8_t> msg;
    NVE_CHECK(read_msg(cfd, msg));
    nve::TransportEnvelopeView env{};
    NVE_CHECK(nve::pair_wire::parse_envelope(msg, env) &&
              env.kind == nve::TransportMsgKind::Hello);
    nve::HelloPayload hp{};
    NVE_CHECK(nve::pair_wire::decode_hello_payload(env.payload, hp));
    const uint64_t sn = 0x0F1E2D3C4B5A6978ULL;
    const uint32_t sid = nve::pair_wire::derive_session_id(token, 1u, hp.client_nonce, sn);
    nve::HelloAckPayload ack{};
    ack.schema_version = 1u;
    ack.server_max_window = 64u;
    ack.server_nonce = sn;
    const auto ack_b = nve::pair_wire::encode_hello_ack_payload(ack);
    NVE_CHECK(write_msg(
        cfd, nve::pair_wire::encode_envelope(nve::TransportMsgKind::HelloAck, sid, ack_b)));

    const auto kf1 = nve::pair_wire::encode_nvd_packet(
        nve::DeltaPacketKind::Keyframe, 0u, 1u, 0u, 0u, f1);
    NVE_CHECK(write_msg(
        cfd, nve::pair_wire::encode_envelope(nve::TransportMsgKind::DeltaSingle, sid, kf1)));

    const auto d2 = nve::pair_wire::encode_command_op_delta(f1, f2);
    NVE_CHECK(d2.has_value());
    const auto p2 = nve::pair_wire::encode_nvd_packet(
        nve::DeltaPacketKind::CommandOpDelta, 0u, 2u, 1u, 0u, *d2);
    NVE_CHECK(write_msg(
        cfd, nve::pair_wire::encode_envelope(nve::TransportMsgKind::DeltaSingle, sid, p2)));

    while (read_msg(cfd, msg)) {
      nve::TransportEnvelopeView in{};
      if (nve::pair_wire::parse_envelope(msg, in) && in.session_id == sid &&
          in.kind == nve::TransportMsgKind::InputEvents) {
        std::vector<nve::DecodedEvent> dec;
        if (nve::decode_event_ring(NveBufferSlice{in.payload.data(), in.payload.size()}, dec) &&
            dec.size() == 1u && dec[0].kind == nve::EventKind::Press && dec[0].node_id == 4u) {
          saw_press.store(true);
          break;
        }
      }
    }
    ::close(cfd);
    ::close(lfd);
  });

  NveSurfaceConfig cfg{.surface_id = 1u, .width = 320.0f, .height = 200.0f, .scale_factor = 1.0f};
  nve::NveGtkHost host(cfg);
  std::string err;
  NVE_CHECK(host.connect_paired_endpoint(
      "127.0.0.1:" + std::to_string(port), token, "tcp", &err, 1u));
  NVE_CHECK(host.is_paired() && host.is_paired_connected() && host.is_loaded());

  const auto dl = std::chrono::steady_clock::now() + std::chrono::seconds(3);
  while (std::chrono::steady_clock::now() < dl) {
    while (g_main_context_iteration(nullptr, FALSE)) {}
    GtkWidget* lbl = host.widget_for_node_id(3u);
    if (lbl != nullptr && GTK_IS_LABEL(lbl) &&
        std::string(gtk_label_get_text(GTK_LABEL(lbl))) == "GTK Paired V2") {
      break;
    }
    std::this_thread::sleep_for(std::chrono::milliseconds(5));
  }
  GtkWidget* lbl = host.widget_for_node_id(3u);
  NVE_CHECK(lbl != nullptr && GTK_IS_LABEL(lbl));
  NVE_CHECK(std::string(gtk_label_get_text(GTK_LABEL(lbl))) == "GTK Paired V2");

  NVE_CHECK(host.simulate_press(4u, 6.0f, 8.0f) == 1u);
  const auto ev_dl = std::chrono::steady_clock::now() + std::chrono::seconds(3);
  while (!saw_press.load() && std::chrono::steady_clock::now() < ev_dl) {
    std::this_thread::sleep_for(std::chrono::milliseconds(5));
  }
  NVE_CHECK(saw_press.load());

  host.disconnect_paired();
  NVE_CHECK(!host.is_paired());
  srv.join();
}

} // namespace

void nve_test_gtk_window_manager();

int main() {
  if (!gtk_init_check()) {
    std::cout << "nve_gtk_shell_test skipped: GTK4 display unavailable "
                 "(run under xvfb-run on headless Linux)\n";
    return 0;
  }

  test_attached_widgets_pango_and_swaps();
  test_dylib_error_codes_and_hot_reload();
  test_snapshot_replay_state_restore_and_atomic_save();
  test_live_counter_dylib_if_present();
  test_accessibility_and_ime_text_input();
  test_image_picture_scroll_and_incremental_reconcile();
  test_cairo_draw_list_sparkline_and_drawing_area();
  test_paired_thin_client_gtk_host();
  nve_test_gtk_window_manager();

  std::cout << "nve_gtk_shell_test passed ("
            << (nve::fabric_host_has_yoga_layout() ? "Yoga+JSI+GTK4" : "Standalone+GTK4")
            << ")\n";
  return 0;
}
