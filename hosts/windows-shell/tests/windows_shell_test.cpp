#include "NveRnwComponentView.hpp"
#include "NveWinHostWindow.hpp"
#include "NveWinTextMeasurer.hpp"
#include "nve_scene_builder.hpp"
#include "windows_test_helpers.hpp"

#include <algorithm>
#include <cstdio>
#include <cstring>
#include <fstream>
#include <initializer_list>
#include <iostream>
#include <string>
#include <vector>

namespace {

using nve::win_test::approx_eq;
using nve::win_test::DirectOutboundQueue;
using nve::win_test::nve_test_patch;

struct MockWinRustComponent {
  uint32_t surface_id{1};
  int count{0};
  std::string title{"Initial"};
  bool wrap_text_node{false};
  std::vector<uint8_t> cmd_buf;
  DirectOutboundQueue outbound;

  static NveAppHandle create(uint32_t sid) {
    auto* self = new MockWinRustComponent();
    self->surface_id = sid;
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

    auto set_style = [&](uint32_t id, uint32_t bg, uint32_t fg, uint32_t border, float bw,
                         float radius, float opacity, float font_size, int32_t z_index) {
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
    if (self->wrap_text_node) create_node(5, NVE_COMP_TEXT);

    set_children(NVE_ROOT_NODE_ID, {2});
    set_children(2, {3, 4});
    if (self->wrap_text_node) set_children(3, {5});
    else set_children(3, {});

    set_layout(2, 1, 2, 100.0f, 2, 100.0f, 0.0f, 20.0f, 10.0f);
    set_style(2, 0xFF202020u, 0xFFFFFFFFu, 0xFF404040u, 1.0f, 8.0f, 1.0f, 16.0f, 0);

    set_layout(3, 0, 1, 120.0f, 1, 40.0f, 0.0f, 0.0f, 0.0f);
    set_style(3, 0x00000000u, 0xFFFFFFFFu, 0u, 0.0f, 0.0f, 1.0f, 18.0f, 1);
    if (self->wrap_text_node) set_text(5, self->title + ":" + std::to_string(self->count));
    else set_text(3, self->title + ":" + std::to_string(self->count));

    set_layout(4, 0, 0, 0.0f, 1, 40.0f, 1.0f, 0.0f, 0.0f);
    set_style(4, 0xFFEB6325u, 0xFFFFFFFFu, 0xFFFFFFFFu, 1.5f, 12.0f, 0.9f, 16.0f, 2);

    self->push<uint8_t>(NVE_OP_COMMIT);
    self->push<uint8_t>(1);
    ++ops;

    std::memcpy(self->cmd_buf.data() + 20, &ops, sizeof(uint32_t));
    return NveBufferSlice{self->cmd_buf.data(), self->cmd_buf.size()};
  }

  static uint32_t dispatch_events(NveAppHandle h, NveEventBatch batch) {
    auto* self = static_cast<MockWinRustComponent*>(h);
    if (batch.events == nullptr) return 0u;
    for (uint32_t i = 0u; i < batch.event_count; ++i) {
      const NveEvent& ev = batch.events[i];
      if (ev.table == NVE_TABLE_INTERACTION && ev.event == NVE_INTERACTION_EVENT_PRESS &&
          ev.node == 4u) {
        ++self->count;
        self->outbound.push_custom(
            self->surface_id,
            4u,
            ev.timestamp,
            "onIncrement",
            "{\"count\":" + std::to_string(self->count) + "}");
        return 1u;
      }
    }
    return 0u;
  }

  static NveIntrinsicSize measure(NveAppHandle, NveSizeConstraints c) {
    const float w = std::clamp(400.0f, c.min_w, c.max_w > 0.0f ? c.max_w : 400.0f);
    const float h = std::clamp(160.0f, c.min_h, c.max_h > 0.0f ? c.max_h : 160.0f);
    return NveIntrinsicSize{w, h};
  }

  static uint8_t hit_test(NveAppHandle, float, float, uint8_t) {
    return static_cast<uint8_t>(NVE_GESTURE_CLAIM);
  }

  static uint32_t set_props(NveAppHandle h, const NveMapEntry* entries, uint32_t count) {
    auto* self = static_cast<MockWinRustComponent*>(h);
    for (uint32_t i = 0u; i < count; ++i) {
      const auto k = nve::str_slice_view(entries[i].key);
      if (k == "title" && entries[i].value.tag == NVE_VAL_STR) {
        const auto v = nve::str_slice_view(entries[i].value.data.str_val);
        self->title.assign(v.data(), v.size());
      } else if (k == "wrap" && entries[i].value.tag == NVE_VAL_BOOL) {
        self->wrap_text_node = entries[i].value.data.bool_val != 0u;
      }
    }
    self->outbound.push_custom(
        self->surface_id, NVE_ROOT_NODE_ID, 500u, "onPropsApplied", "{\"ok\":true}");
    return 1u;
  }

  static NveEventBatch drain_emitted_events(NveAppHandle h) {
    return static_cast<MockWinRustComponent*>(h)->outbound.drain();
  }

  static uint32_t apply_hot_patch(NveAppHandle h, NveBufferSlice patch) {
    auto* self = static_cast<MockWinRustComponent*>(h);
    self->title.assign(reinterpret_cast<const char*>(patch.ptr), patch.len);
    return 1u;
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
      case NVE_IFACE_SURFACE: return &kSurface;
      case NVE_IFACE_LAYOUT: return &kLayout;
      case NVE_IFACE_COMPONENT: return &kComponent;
      case NVE_IFACE_DEV: return &kDev;
      default: return nullptr;
    }
  }
};

void test_directwrite_text_measurer() {
  nve::win::DirectWriteTextMeasurer measurer;
  NVE_CHECK(measurer.is_valid());

  const NveIntrinsicSize empty = measurer.measure("", 16.0f, 0.0f);
  NVE_CHECK(empty.width == 0.0f && empty.height == 0.0f);

  const NveIntrinsicSize title = measurer.measure("NATIVE VIEW ENGINE", 14.0f, 0.0f);
  NVE_CHECK(title.width > 40.0f && title.height >= 14.0f);

  const NveIntrinsicSize small_count = measurer.measure("Count: 0", 14.0f, 0.0f);
  const NveIntrinsicSize large_count = measurer.measure("Count: 0", 28.0f, 0.0f);
  NVE_CHECK(large_count.height > small_count.height && large_count.width > small_count.width);

  const std::string paragraph =
      "The quick brown fox jumps over the lazy dog inside Native View Engine";
  const NveIntrinsicSize unwrapped = measurer.measure(paragraph, 16.0f, 0.0f);
  const NveIntrinsicSize wrapped = measurer.measure(paragraph, 16.0f, 90.0f);
  NVE_CHECK(unwrapped.width > 120.0f);
  NVE_CHECK(wrapped.width > 0.0f && wrapped.width <= 90.5f);
  NVE_CHECK(wrapped.height > unwrapped.height);

  const NveIntrinsicSize fallback = measurer.measure("Fallback", 0.0f, 0.0f);
  NVE_CHECK(fallback.width > 10.0f && fallback.height >= 14.0f);

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

  nve::RenderedNodeSnapshot btn_top{};
  btn_top.node_id = 10;
  btn_top.kind = NVE_COMP_PRESSABLE;
  btn_top.layout = {20.0f, 20.0f, 100.0f, 50.0f};
  btn_top.z_index = 5;
  btn_top.children = {11};

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

  nve::RenderedNodeSnapshot btn_hidden{};
  btn_hidden.node_id = 30;
  btn_hidden.kind = NVE_COMP_PRESSABLE;
  btn_hidden.layout = {20.0f, 20.0f, 100.0f, 50.0f};
  btn_hidden.z_index = 10;
  btn_hidden.opacity = 0.0f;

  builder.sync(1u, {root, btn_top, btn_label, btn_under, btn_hidden});
  NVE_CHECK(tree.sync(builder.view()));
  NVE_CHECK(tree.size() == 4 && tree.created_count() == 4 && tree.destroyed_count() == 0);
  NVE_CHECK(tree.find(11) != nullptr && tree.find(11)->is_text_leaf);

  const auto order = tree.build_draw_order();
  NVE_CHECK(order.size() == 4);
  NVE_CHECK(order[0].node_id == 20 && order[1].node_id == 10 && order[2].node_id == 11 && order[3].node_id == 30);

  const auto hit = tree.hit_test_interactive(45.0f, 35.0f);
  NVE_CHECK(hit.has_value() && hit->node_id == 10);
  NVE_CHECK(approx_eq(hit->local_x, 25.0f) && approx_eq(hit->local_y, 15.0f));

  const auto abs_label = tree.absolute_rect(11);
  NVE_CHECK(abs_label.has_value() && approx_eq(abs_label->x, 30.0f) && approx_eq(abs_label->y, 30.0f));

  const uint64_t created_before = tree.created_count();
  const uint64_t updated_before = tree.updated_count();
  btn_label.text = "TopUpdated";
  builder.sync(2u, {root, btn_top, btn_label, btn_under, btn_hidden});
  NVE_CHECK(tree.sync(builder.view()));
  NVE_CHECK(tree.created_count() == created_before);
  NVE_CHECK(tree.updated_count() == updated_before + 1);
  NVE_CHECK(tree.find(11)->text == "TopUpdated");

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
  NVE_CHECK(tree.size() == 5 && tree.destroyed_count() == destroyed_before + 1);
  NVE_CHECK(tree.created_count() == created_before + 2);
  NVE_CHECK(!tree.find(11)->is_text_leaf && (tree.find(11)->view_tag & 0x10000u) != 0u);
  NVE_CHECK(tree.find(12)->is_text_leaf && (tree.find(12)->view_tag & 0x10000u) == 0u);
}

void test_rnw_component_host_and_dylib_lifecycle() {
  nve::win::NveRnwComponentHost host;
  std::vector<nve::DecodedEvent> received_events;
  int visual_notifications = 0;

  host.set_event_callback([&](const nve::DecodedEvent& ev) { received_events.push_back(ev); });
  host.set_on_visual_tree_changed([&]() { ++visual_notifications; });

  const NveSurfaceConfig cfg{.surface_id = 9, .width = 400.0f, .height = 160.0f, .scale_factor = 1.0f};
  NVE_CHECK(host.attach_interface(&MockWinRustComponent::get_interface, cfg));
  NVE_CHECK(host.is_valid() && host.surface_id() == 9 && host.node_count() == 4);
  NVE_CHECK(visual_notifications >= 1);

  const auto* container = host.visual_tree().find(2);
  const auto* label = host.visual_tree().find(3);
  const auto* button = host.visual_tree().find(4);
  NVE_CHECK(container != nullptr && label != nullptr && button != nullptr);
  NVE_CHECK(label->is_text_leaf && label->text == "Initial:0" && button->interactive);
  NVE_CHECK(approx_eq(container->layout.width, 400.0f));
  NVE_CHECK(approx_eq(label->layout.x, 20.0f) && approx_eq(label->layout.width, 120.0f));
  NVE_CHECK(approx_eq(button->layout.x, 150.0f) && approx_eq(button->layout.width, 230.0f));

  const auto measured = host.measure(NveSizeConstraints{0.0f, 0.0f, 640.0f, 480.0f});
  NVE_CHECK(measured.has_value() && approx_eq(measured->width, 400.0f) && approx_eq(measured->height, 160.0f));

  NVE_CHECK(host.update_props(nve::PropMap{{"title", std::string("Updated")}}));
  NVE_CHECK(host.visual_tree().find(3)->text == "Updated:0");
  NVE_CHECK(received_events.size() == 1 && received_events[0].name == "onPropsApplied");

  NVE_CHECK(host.dispatch_click_at(200.0f, 35.0f) != 0);
  NVE_CHECK(host.visual_tree().find(3)->text == "Updated:1");
  NVE_CHECK(received_events.size() == 2 && received_events[1].name == "onIncrement");
  NVE_CHECK(received_events[1].payload_as_string() == "{\"count\":1}");

  host.set_surface_size(500.0f, 160.0f, 1.5f);
  NVE_CHECK(approx_eq(host.visual_tree().find(4)->layout.width, 330.0f));

  NVE_CHECK(host.update_props(nve::PropMap{{"wrap", true}}));
  NVE_CHECK(!host.visual_tree().find(3)->is_text_leaf);
  NVE_CHECK(host.visual_tree().find(5) != nullptr && host.visual_tree().find(5)->is_text_leaf);
  NVE_CHECK(host.visual_tree().find(5)->text == "Updated:1");

  int err_code = 0;
  std::string err_msg;

  NVE_CHECK(!host.load_dylib("/nonexistent/missing_nve_app.dll", cfg, &err_code, &err_msg));
  NVE_CHECK(err_code == 1 && !err_msg.empty());
  NVE_CHECK(!host.load_dylib("", cfg, &err_code, &err_msg));
  NVE_CHECK(err_code == 1 && !err_msg.empty());
  NVE_CHECK(!host.load_dylib(NVE_WIN_TEST_NO_ENTRY_DYLIB, cfg, &err_code, &err_msg));
  NVE_CHECK(err_code == 2 && !err_msg.empty());
  NVE_CHECK(!host.load_dylib(NVE_WIN_TEST_NO_SURFACE_DYLIB, cfg, &err_code, &err_msg));
  NVE_CHECK(err_code == 3 && !err_msg.empty());

  NVE_CHECK(host.is_valid() && host.visual_tree().find(5)->text == "Updated:1");

  nve::win::NveRnwComponentHost reload_host;
  NVE_CHECK(!reload_host.is_valid());
  NVE_CHECK(reload_host.load_dylib(NVE_WIN_TEST_STATEFUL_DYLIB, cfg, &err_code, &err_msg));
  NVE_CHECK(err_code == 0 && err_msg.empty() && reload_host.visual_tree().find(2)->text == "state=0");

  NVE_CHECK(reload_host.apply_hot_patch(nve_test_patch("77")));
  NVE_CHECK(reload_host.visual_tree().find(2)->text == "state=77");

  NVE_CHECK(!reload_host.reload_dylib("/nonexistent/fail.dll", &err_code, &err_msg));
  NVE_CHECK(err_code == 1 && reload_host.visual_tree().find(2)->text == "state=77");
  NVE_CHECK(!reload_host.reload_dylib(NVE_WIN_TEST_NO_ENTRY_DYLIB, &err_code, &err_msg));
  NVE_CHECK(err_code == 2 && reload_host.visual_tree().find(2)->text == "state=77");
  NVE_CHECK(!reload_host.reload_dylib(NVE_WIN_TEST_NO_SURFACE_DYLIB, &err_code, &err_msg));
  NVE_CHECK(err_code == 3 && reload_host.visual_tree().find(2)->text == "state=77");

  NVE_CHECK(reload_host.reload_dylib(NVE_WIN_TEST_STATEFUL_DYLIB, &err_code, &err_msg));
  NVE_CHECK(err_code == 0 && reload_host.visual_tree().find(2)->text == "state=77");

  NVE_CHECK(reload_host.dispatch_press(3, 5.0f, 5.0f) != 0);
  NVE_CHECK(reload_host.visual_tree().find(2)->text == "state=78");
  NVE_CHECK(reload_host.update_props(nve::PropMap{{"prefix", std::string("Updated")}}));
  NVE_CHECK(reload_host.visual_tree().find(2)->text == "Updated=78");

  NVE_CHECK(reload_host.load_dylib(NVE_WIN_TEST_STATEFUL_DYLIB, cfg, &err_code, &err_msg));
  NVE_CHECK(err_code == 0 && reload_host.visual_tree().find(2)->text == "state=0");

#if defined(_WIN32)
  const std::string bmp_path = "nve_windows_shell_unit_test.bmp";
  std::string bmp_err;
  NVE_CHECK(nve::win::render_visual_tree_to_bmp(
      host.visual_tree(), *host.text_measurer(), 400, 160, 1.0f, bmp_path, &bmp_err));
  {
    std::ifstream in(bmp_path, std::ios::binary | std::ios::ate);
    NVE_CHECK(in.is_open() && in.tellg() == static_cast<std::streamoff>(54 + 400 * 160 * 4));
    in.seekg(0);
    char magic[2]{};
    in.read(magic, 2);
    NVE_CHECK(magic[0] == 'B' && magic[1] == 'M');
  }
  (void)std::remove(bmp_path.c_str());
#endif
}

} // namespace

void nve_test_win_live_snapshot_and_media();
void nve_test_win_canvas_pair_and_wm();

int main() {
  test_directwrite_text_measurer();
  test_visual_tree_reconciler_and_hit_testing();
  test_rnw_component_host_and_dylib_lifecycle();
  nve_test_win_live_snapshot_and_media();
  nve_test_win_canvas_pair_and_wm();

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
