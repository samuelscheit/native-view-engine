#include "NveGtkHost.hpp"

#include <cmath>
#include <cstdlib>
#include <cstring>
#include <functional>
#include <iostream>
#include <string>
#include <string_view>
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

struct MockGtkRustComponent {
  uint32_t surface_id{1u};
  int count{0};
  bool text_as_wrapper{false};
  std::string title{"Initial"};
  std::string input_value{"hello"};
  std::vector<uint8_t> cmd_buf;
  nve::EventBatchBuilder pending_outbound;
  nve::EventBatchBuilder drained_outbound;

  static NveAppHandle create(uint32_t sid) {
    auto* self = new MockGtkRustComponent();
    self->surface_id = sid;
    return self;
  }

  static void destroy(NveAppHandle h) { delete static_cast<MockGtkRustComponent*>(h); }

  void emit_custom(uint32_t node_id, uint64_t ts, std::string name, std::string payload) {
    (void)pending_outbound.push_custom(
        surface_id,
        node_id,
        ts,
        name,
        payload.empty() ? nullptr : reinterpret_cast<const uint8_t*>(payload.data()),
        payload.size());
  }

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
    auto set_layout = [&](uint32_t id, uint8_t dir, uint8_t wtag, float w, uint8_t htag, float h,
                          float grow, float pad, float gap) {
      self->push<uint8_t>(NVE_OP_SET_LAYOUT);
      self->push<uint32_t>(id);
      self->push_dim(wtag, w);
      self->push_dim(htag, h);
      self->push_dim(0u, 0.0f);
      self->push_dim(0u, 0.0f);
      self->push_dim(0u, 0.0f);
      self->push_dim(0u, 0.0f);
      self->push<uint8_t>(dir);
      self->push<uint8_t>(0u);
      self->push<uint8_t>(0u);
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

    set_layout(2u, 1u, 2u, 100.0f, 2u, 100.0f, 0.0f, 20.0f, 10.0f);
    set_style(2u, 0xFF202020u, 0xFFFFFFFFu, 8.0f, 16.0f);

    set_layout(3u, 0u, 1u, 120.0f, 1u, 40.0f, 0.0f, 0.0f, 0.0f);
    set_style(3u, 0x00000000u, 0xFFFFFFFFu, 0.0f, 18.0f);

    set_layout(4u, 0u, 0u, 0.0f, 1u, 40.0f, 1.0f, 0.0f, 0.0f);
    set_style(4u, 0xFFFF5500u, 0xFFFFFFFFu, 12.0f, 16.0f);

    set_layout(5u, 0u, 1u, 80.0f, 1u, 40.0f, 0.0f, 0.0f, 0.0f);
    set_style(5u, 0xFF101828u, 0xFFFFFFFFu, 4.0f, 14.0f);
    set_layout(6u, 0u, 1u, 140.0f, 1u, 80.0f, 0.0f, 0.0f, 0.0f);
    set_text(6u, "Scrollable");

    set_layout(7u, 0u, 1u, 90.0f, 1u, 32.0f, 0.0f, 0.0f, 0.0f);
    set_text(7u, self->input_value);
    set_style(7u, 0xFF0F172Au, 0xFFFFFFFFu, 6.0f, 14.0f);

    self->push<uint8_t>(NVE_OP_COMMIT);
    self->push<uint8_t>(1u);
    ++ops;

    std::memcpy(self->cmd_buf.data() + 20, &ops, sizeof(uint32_t));
    return NveBufferSlice{self->cmd_buf.data(), self->cmd_buf.size()};
  }

  static uint32_t dispatch_events(NveAppHandle h, NveEventBatch batch) {
    auto* self = static_cast<MockGtkRustComponent*>(h);
    if (batch.events == nullptr) return 0u;
    for (uint32_t i = 0u; i < batch.event_count; ++i) {
      const NveEvent& ev = batch.events[i];
      if (ev.table == NVE_TABLE_INTERACTION && ev.event == NVE_INTERACTION_EVENT_PRESS &&
          ev.node == 4u) {
        ++self->count;
        self->emit_custom(
            4u, ev.timestamp, "onIncrement", "{\"count\":" + std::to_string(self->count) + "}");
        return 1u;
      }
    }
    return 0u;
  }

  static uint32_t set_props(NveAppHandle h, const NveMapEntry* entries, uint32_t count) {
    auto* self = static_cast<MockGtkRustComponent*>(h);
    for (uint32_t i = 0u; i < count; ++i) {
      const std::string_view key = nve::str_slice_view(entries[i].key);
      const NveValue& val = entries[i].value;
      if (key == "title" && val.tag == NVE_VAL_STR) {
        self->title = std::string(nve::str_slice_view(val.data.str_val));
      } else if (key == "wrapper" && val.tag == NVE_VAL_BOOL) {
        self->text_as_wrapper = val.data.bool_val != 0u;
      } else if (key == "input" && val.tag == NVE_VAL_STR) {
        self->input_value = std::string(nve::str_slice_view(val.data.str_val));
      }
    }
    self->emit_custom(NVE_ROOT_NODE_ID, nve::current_timestamp_us(), "onPropsApplied", "{\"ok\":true}");
    return 1u;
  }

  static NveEventBatch drain_emitted_events(NveAppHandle h) {
    auto* self = static_cast<MockGtkRustComponent*>(h);
    self->drained_outbound = std::move(self->pending_outbound);
    self->pending_outbound.clear();
    return self->drained_outbound.as_batch();
  }

  static const void* get_interface(uint32_t interface_id) {
    static const NveSurfaceApi kSurface{
        .create = &create,
        .destroy = &destroy,
        .render = &render,
        .dispatch_events = &dispatch_events,
    };
    static const NveComponentApi kComponent{
        .set_props = &set_props,
        .drain_emitted_events = &drain_emitted_events,
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
    if (w == nullptr) return;
    if (GTK_IS_LABEL(w)) out.push_back(GTK_LABEL(w));
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
  for (const auto& s : snaps) by_id[s.node_id] = &s;

  std::function<bool(uint32_t)> contains_text = [&](uint32_t id) -> bool {
    auto it = by_id.find(id);
    if (it == by_id.end()) return false;
    if (it->second->text == expected_text) return true;
    for (uint32_t cid : it->second->children) {
      if (contains_text(cid)) return true;
    }
    return false;
  };

  for (const auto& s : snaps) {
    if (s.kind == NVE_COMP_PRESSABLE && contains_text(s.node_id)) return s.node_id;
  }
  return 0u;
}

void test_attached_widgets_pango_and_swaps() {
  NveSurfaceConfig cfg{.surface_id = 9u, .width = 500.0f, .height = 200.0f, .scale_factor = 1.0f};
  nve::NveGtkHost host(cfg);
  NVE_CHECK(host.root_widget() != nullptr && GTK_IS_FIXED(host.root_widget()));
  NVE_CHECK(host.host_widget() != nullptr && GTK_IS_SCROLLED_WINDOW(host.host_widget()));
  NVE_CHECK(!host.is_loaded() && host.node_count() == 0u);
  NVE_CHECK(host.widget_for_node_id(NVE_ROOT_NODE_ID) == host.root_widget());

  const NveIntrinsicSize empty_sz = host.measure_text("", 16.0f, 0.0f);
  NVE_CHECK(empty_sz.width == 0.0f && empty_sz.height == 0.0f);

  const NveIntrinsicSize small_sz = host.measure_text("Native View Engine", 12.0f, 0.0f);
  const NveIntrinsicSize large_sz = host.measure_text("Native View Engine", 28.0f, 0.0f);
  NVE_CHECK(small_sz.width > 0.0f && small_sz.height > 0.0f);
  NVE_CHECK(large_sz.width > small_sz.width && large_sz.height > small_sz.height);

  const std::string long_text =
      "The quick brown fox jumps over the lazy dog across multiple wrapped Pango lines";
  const NveIntrinsicSize wide_sz = host.measure_text(long_text, 16.0f, 0.0f);
  const NveIntrinsicSize wrapped_sz = host.measure_text(long_text, 16.0f, 120.0f);
  NVE_CHECK(wide_sz.width > wrapped_sz.width && wrapped_sz.width <= 122.0f);
  NVE_CHECK(wrapped_sz.height > wide_sz.height);

  std::vector<nve::DecodedEvent> received_events;
  host.set_event_callback([&](const nve::DecodedEvent& ev) { received_events.push_back(ev); });
  NVE_CHECK(host.attach_interface(&MockGtkRustComponent::get_interface, 9u));
  NVE_CHECK(host.is_loaded() && host.surface_id() == 9u && host.node_count() == 7u);
  {
    host.set_surface_size(960.0f, 640.0f, 1.0f);
    int min_w = 0, nat_w = 0, min_h = 0, nat_h = 0;
    gtk_widget_measure(host.host_widget(), GTK_ORIENTATION_HORIZONTAL, -1, &min_w, &nat_w, nullptr, nullptr);
    gtk_widget_measure(host.host_widget(), GTK_ORIENTATION_VERTICAL, -1, &min_h, &nat_h, nullptr, nullptr);
    NVE_CHECK(min_w <= 16 && min_h <= 16);
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

  NVE_CHECK(gtk_label_get_wrap(GTK_LABEL(label_widget)) == TRUE);
  NVE_CHECK(approx_eq(gtk_label_get_xalign(GTK_LABEL(label_widget)), 0.0, 1e-4));
  NVE_CHECK(approx_eq(gtk_label_get_yalign(GTK_LABEL(label_widget)), 0.0, 1e-4));
  NVE_CHECK(gtk_widget_get_can_target(label_widget) == FALSE);
  NVE_CHECK(std::string(gtk_label_get_text(GTK_LABEL(label_widget))) == "Initial:0");

  NVE_CHECK(gtk_widget_get_overflow(container) == GTK_OVERFLOW_HIDDEN);
  NVE_CHECK(gtk_widget_get_overflow(pressable) == GTK_OVERFLOW_HIDDEN);
  NVE_CHECK(approx_eq(gtk_widget_get_opacity(container), 0.90, 0.02));
  NVE_CHECK(std::string(gtk_editable_get_text(GTK_EDITABLE(text_input))) == "hello");

  NVE_CHECK(gtk_widget_get_parent(container) == host.root_widget());
  NVE_CHECK(gtk_widget_get_parent(label_widget) == container);
  NVE_CHECK(gtk_widget_get_parent(pressable) == container);
  NVE_CHECK(gtk_widget_get_parent(scroll_view) == container);
  NVE_CHECK(gtk_widget_get_parent(text_input) == container);
  NVE_CHECK(gtk_widget_get_parent(scroll_child) != nullptr && GTK_IS_FIXED(gtk_widget_get_parent(scroll_child)));

  GskTransform* label_transform = gtk_fixed_get_child_transform(GTK_FIXED(container), label_widget);
  NVE_CHECK(label_transform != nullptr);
  float lx = 0.0f, ly = 0.0f;
  gsk_transform_to_translate(label_transform, &lx, &ly);
  NVE_CHECK(approx_eq(lx, 20.0) && approx_eq(ly, 20.0));

  int lw = 0, lh = 0;
  gtk_widget_get_size_request(label_widget, &lw, &lh);
  NVE_CHECK(lw == 120 && lh == 40);

  NVE_CHECK(gtk_widget_has_css_class(pressable, "nve-node-4") == TRUE);
  const std::string expected_selector =
      ".nve-host-" + std::to_string(host.host_instance_id()) + "-node-4";
  NVE_CHECK(host.active_css().find(expected_selector) != std::string::npos);
  NVE_CHECK(host.active_css().find("border-radius: 12px;") != std::string::npos);

  // Direct apply_props updates text, entry, and emits outbound event.
  NVE_CHECK(host.apply_props(nve::PropMap{{"title", std::string("Updated")}, {"input", std::string("typed")}}));
  NVE_CHECK(std::string(gtk_label_get_text(GTK_LABEL(label_widget))) == "Updated:0");
  NVE_CHECK(std::string(gtk_editable_get_text(GTK_EDITABLE(text_input))) == "typed");
  NVE_CHECK(received_events.size() == 1u);
  NVE_CHECK(received_events[0].name == "onPropsApplied");

  // simulate_press dispatches press to node 4, re-renders, and emits onIncrement.
  NVE_CHECK(host.simulate_press(4u, 10.0f, 12.0f) == 1u);
  NVE_CHECK(std::string(gtk_label_get_text(GTK_LABEL(label_widget))) == "Updated:1");
  NVE_CHECK(received_events.size() == 2u);
  NVE_CHECK(received_events[1].name == "onIncrement");
  NVE_CHECK(received_events[1].payload_as_string() == "{\"count\":1}");

  // Text leaf -> wrapper -> leaf tag swap.
  NVE_CHECK(host.apply_props(nve::PropMap{{"wrapper", true}}));
  GtkWidget* wrapper_widget = host.widget_for_node_id(3u);
  GtkWidget* nested_leaf = host.widget_for_node_id(8u);
  NVE_CHECK(wrapper_widget != nullptr && GTK_IS_FIXED(wrapper_widget));
  NVE_CHECK(nested_leaf != nullptr && GTK_IS_LABEL(nested_leaf));
  NVE_CHECK(gtk_widget_get_parent(nested_leaf) == wrapper_widget);
  NVE_CHECK(std::string(gtk_label_get_text(GTK_LABEL(nested_leaf))) == "Wrapped:Updated:1");

  NVE_CHECK(host.apply_props(nve::PropMap{{"wrapper", false}}));
  GtkWidget* restored_leaf = host.widget_for_node_id(3u);
  NVE_CHECK(restored_leaf != nullptr && GTK_IS_LABEL(restored_leaf));
  NVE_CHECK(host.widget_for_node_id(8u) == nullptr);
  NVE_CHECK(std::string(gtk_label_get_text(GTK_LABEL(restored_leaf))) == "Updated:1");

  int bw_before = 0, bh_before = 0;
  gtk_widget_get_size_request(pressable, &bw_before, &bh_before);
  host.set_surface_size(620.0f, 200.0f, 2.0f);
  int bw_after = 0, bh_after = 0;
  gtk_widget_get_size_request(pressable, &bw_after, &bh_after);
  NVE_CHECK(bw_after > bw_before);

  NveFrameStats stats{};
  NVE_CHECK(host.render_frame(&stats) == 0 && stats.op_count > 0u);
}

void test_dylib_error_codes_and_hot_reload() {
  NveSurfaceConfig cfg{.surface_id = 3u, .width = 360.0f, .height = 240.0f, .scale_factor = 1.0f};
  nve::NveGtkHost host(cfg);

  int err_code = 0;
  std::string err_msg;

  NVE_CHECK(!host.load_dylib("", &err_code, &err_msg) && err_code == 1 && !err_msg.empty());
  err_code = 0; err_msg.clear();
  NVE_CHECK(!host.load_dylib("/nonexistent/libmissing_nve.so", &err_code, &err_msg) && err_code == 1 && !err_msg.empty());
  err_code = 0; err_msg.clear();
  NVE_CHECK(!host.load_dylib(NVE_TEST_NO_ENTRY_DYLIB, &err_code, &err_msg) && err_code == 2 && !err_msg.empty());
  err_code = 0; err_msg.clear();
  NVE_CHECK(!host.load_dylib(NVE_TEST_NO_SURFACE_DYLIB, &err_code, &err_msg) && err_code == 3 && !err_msg.empty());
  NVE_CHECK(!host.is_loaded());

  err_code = -1; err_msg = "stale";
  NVE_CHECK(host.reload_dylib(NVE_TEST_STATEFUL_DYLIB, &err_code, &err_msg));
  NVE_CHECK(err_code == 0 && err_msg.empty() && host.is_loaded() && host.node_count() == 3u);

  GtkWidget* state_label = host.widget_for_node_id(2u);
  NVE_CHECK(state_label != nullptr && GTK_IS_LABEL(state_label));
  NVE_CHECK(std::string(gtk_label_get_text(GTK_LABEL(state_label))) == "state=0");

  NVE_CHECK(host.simulate_press(3u, 5.0f, 5.0f) == 1u);
  NVE_CHECK(host.simulate_press(3u, 5.0f, 5.0f) == 1u);
  NVE_CHECK(std::string(gtk_label_get_text(GTK_LABEL(state_label))) == "state=2");

  NVE_CHECK(!host.reload_dylib("/nonexistent/fail.so", &err_code, &err_msg) && err_code == 1);
  NVE_CHECK(!host.reload_dylib(NVE_TEST_NO_ENTRY_DYLIB, &err_code, &err_msg) && err_code == 2);
  NVE_CHECK(!host.reload_dylib(NVE_TEST_NO_SURFACE_DYLIB, &err_code, &err_msg) && err_code == 3);
  NVE_CHECK(host.is_loaded());
  NVE_CHECK(std::string(gtk_label_get_text(GTK_LABEL(host.widget_for_node_id(2u)))) == "state=2");

  NVE_CHECK(host.reload_dylib(NVE_TEST_STATEFUL_DYLIB, &err_code, &err_msg) && err_code == 0 && err_msg.empty());
  NVE_CHECK(std::string(gtk_label_get_text(GTK_LABEL(host.widget_for_node_id(2u)))) == "state=2");

  NVE_CHECK(host.load_dylib(NVE_TEST_STATEFUL_DYLIB, &err_code, &err_msg) && err_code == 0 && err_msg.empty());
  NVE_CHECK(std::string(gtk_label_get_text(GTK_LABEL(host.widget_for_node_id(2u)))) == "state=0");
}

void test_live_counter_dylib_if_present() {
  const char* dylib_env = std::getenv("NVE_TEST_KITCHEN_SINK_DYLIB");
  if (dylib_env == nullptr || dylib_env[0] == '\0') return;

  NveSurfaceConfig cfg{.surface_id = 1u, .width = 1280.0f, .height = 800.0f, .scale_factor = 1.0f};
  nve::NveGtkHost host(cfg);

  int err_code = -1;
  std::string err_msg;
  NVE_CHECK(host.load_dylib(dylib_env, &err_code, &err_msg) && err_code == 0 && err_msg.empty());
  NVE_CHECK(host.is_loaded());

  GtkWidget* top_container = gtk_widget_get_first_child(host.root_widget());
  NVE_CHECK(top_container != nullptr && GTK_IS_FIXED(top_container));

  const auto has_label_text = [&](std::string_view needle) {
    for (GtkLabel* lbl : collect_labels(host.root_widget())) {
      const char* txt = gtk_label_get_text(lbl);
      if (txt != nullptr && needle == txt) return true;
    }
    return false;
  };

  const std::vector<GtkLabel*> labels = collect_labels(host.root_widget());
  NVE_CHECK(labels.size() >= 4u);
  NVE_CHECK(has_label_text("NVE Kitchen Sink") && has_label_text("Count: 0") &&
            has_label_text("Increment +") && has_label_text("Reset"));

  for (GtkLabel* lbl : labels) {
    int w = 0, h = 0;
    gtk_widget_get_size_request(GTK_WIDGET(lbl), &w, &h);
    NVE_CHECK(w > 0 && h >= 10 && gtk_widget_get_can_target(GTK_WIDGET(lbl)) == FALSE);
  }

  const uint32_t inc_node = find_pressable_node_for_label_text(host, "Increment +");
  const uint32_t reset_node = find_pressable_node_for_label_text(host, "Reset");
  NVE_CHECK(inc_node != 0u && reset_node != 0u && inc_node != reset_node);

  NVE_CHECK(host.simulate_press(inc_node, 20.0f, 20.0f) != 0u && has_label_text("Count: 1"));
  NVE_CHECK(host.simulate_press(inc_node, 20.0f, 20.0f) != 0u && has_label_text("Count: 2"));
  NVE_CHECK(host.simulate_press(reset_node, 20.0f, 20.0f) != 0u && has_label_text("Count: 0"));

  NVE_CHECK(host.simulate_press(inc_node, 20.0f, 20.0f) != 0u);
  NVE_CHECK(host.simulate_press(inc_node, 20.0f, 20.0f) != 0u);
  NVE_CHECK(host.simulate_press(inc_node, 20.0f, 20.0f) != 0u && has_label_text("Count: 3"));

  err_code = -1;
  NVE_CHECK(host.reload_dylib(dylib_env, &err_code, &err_msg) && err_code == 0 && err_msg.empty());
  NVE_CHECK(has_label_text("Count: 3"));
}

} // namespace

void nve_test_gtk_snapshot_a11y_and_media();
void nve_test_gtk_canvas_and_pair();
void nve_test_gtk_window_manager();

int main() {
  if (!gtk_init_check()) {
    std::cout << "nve_gtk_shell_test skipped: GTK4 display unavailable "
                 "(run under xvfb-run on headless Linux)\n";
    return 0;
  }

  test_attached_widgets_pango_and_swaps();
  test_dylib_error_codes_and_hot_reload();
  test_live_counter_dylib_if_present();
  nve_test_gtk_snapshot_a11y_and_media();
  nve_test_gtk_canvas_and_pair();
  nve_test_gtk_window_manager();

  std::cout << "nve_gtk_shell_test passed ("
            << (nve::fabric_host_has_yoga_layout() ? "Yoga+JSI+GTK4" : "Standalone+GTK4")
            << ")\n";
  return 0;
}
