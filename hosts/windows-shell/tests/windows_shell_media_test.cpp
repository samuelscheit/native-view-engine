#include "NveRnwComponentView.hpp"
#include "NveWinHostWindow.hpp"
#include "windows_test_helpers.hpp"

#include <cstring>
#include <filesystem>
#include <initializer_list>
#include <iostream>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

namespace {

using nve::win_test::approx_eq;
using nve::win_test::get_env_string;
using nve::win_test::nve_test_patch;

void test_live_counter_dylib_if_configured() {
  const std::string dylib_env = get_env_string("NVE_TEST_KITCHEN_SINK_DYLIB");
  if (dylib_env.empty()) return;

  nve::win::NveRnwComponentHost host;
  const NveSurfaceConfig cfg{.surface_id = 1, .width = 1280.0f, .height = 800.0f, .scale_factor = 1.0f};
  int err_code = 0;
  std::string err_msg;
  NVE_CHECK(host.load_dylib(dylib_env, cfg, &err_code, &err_msg));
  NVE_CHECK(err_code == 0);

  auto find_leaf_by_text = [&](std::string_view expected) -> const nve::win::NveWinNodeVisual* {
    for (const auto& flat : host.visual_tree().build_draw_order()) {
      const auto* node = host.visual_tree().find(flat.node_id);
      if (node != nullptr && node->is_text_leaf && node->text == expected) return node;
    }
    return nullptr;
  };

  auto find_pressable_ancestor = [&](uint32_t node_id) -> const nve::win::NveWinNodeVisual* {
    const auto* cur = host.visual_tree().find(node_id);
    uint32_t hops = 0;
    while (cur != nullptr && cur->parent_id != 0 && cur->parent_id != NVE_ROOT_NODE_ID && hops < 64) {
      const auto* parent = host.visual_tree().find(cur->parent_id);
      if (parent == nullptr) break;
      if (parent->interactive) return parent;
      cur = parent;
      ++hops;
    }
    return nullptr;
  };

  const auto* lbl_title = find_leaf_by_text("NVE Kitchen Sink");
  const auto* lbl_count = find_leaf_by_text("Count: 0");
  const auto* lbl_inc = find_leaf_by_text("Increment +");
  const auto* lbl_reset = find_leaf_by_text("Reset");
  NVE_CHECK(lbl_title && lbl_count && lbl_inc && lbl_reset);

  for (const auto* lbl : {lbl_title, lbl_count, lbl_inc, lbl_reset}) {
    NVE_CHECK(lbl->layout.width > 20.0f && lbl->layout.height >= 14.0f);
    NVE_CHECK(nve::unpack_rgba(lbl->text_color_rgba).a8 == 255);
  }
  NVE_CHECK(approx_eq(lbl_count->font_size, 28.0f, 0.5f));

  const auto* inc_btn = find_pressable_ancestor(lbl_inc->node_id);
  const auto* reset_btn = find_pressable_ancestor(lbl_reset->node_id);
  NVE_CHECK(inc_btn != nullptr && reset_btn != nullptr);

  if (nve::fabric_host_has_yoga_layout()) {
    bool saw_top_card = false;
    for (const auto& flat : host.visual_tree().build_draw_order()) {
      const auto* node = host.visual_tree().find(flat.node_id);
      if (node != nullptr && node->kind == NVE_COMP_VIEW && node->border_radius >= 15.0f) {
        if (nve::unpack_rgba(node->background_rgba).a8 == 255 &&
            node->layout.width > 100.0f && node->layout.height > 40.0f) {
          saw_top_card = true;
        }
      }
    }
    NVE_CHECK(saw_top_card);

    auto assert_centered = [&](const nve::win::NveWinNodeVisual* btn, const nve::win::NveWinNodeVisual* lbl) {
      const auto btn_rect = host.visual_tree().absolute_rect(btn->node_id);
      const auto lbl_rect = host.visual_tree().absolute_rect(lbl->node_id);
      NVE_CHECK(btn_rect.has_value() && lbl_rect.has_value());
      NVE_CHECK(approx_eq(lbl_rect->x + lbl_rect->width * 0.5f, btn_rect->x + btn_rect->width * 0.5f, 3.0f));
      NVE_CHECK(approx_eq(lbl_rect->y + lbl_rect->height * 0.5f, btn_rect->y + btn_rect->height * 0.5f, 3.0f));
    };
    assert_centered(inc_btn, lbl_inc);
    assert_centered(reset_btn, lbl_reset);

    const auto inc_rect = host.visual_tree().absolute_rect(inc_btn->node_id);
    const auto reset_rect = host.visual_tree().absolute_rect(reset_btn->node_id);
    NVE_CHECK(inc_rect.has_value() && reset_rect.has_value());

    NVE_CHECK(host.dispatch_click_at(inc_rect->x + inc_rect->width * 0.5f, inc_rect->y + inc_rect->height * 0.5f) != 0);
    NVE_CHECK(find_leaf_by_text("Count: 1") != nullptr);
    NVE_CHECK(host.dispatch_click_at(reset_rect->x + reset_rect->width * 0.5f, reset_rect->y + reset_rect->height * 0.5f) != 0);
    NVE_CHECK(find_leaf_by_text("Count: 0") != nullptr);
    NVE_CHECK(host.dispatch_click_at(inc_rect->x + inc_rect->width * 0.5f, inc_rect->y + inc_rect->height * 0.5f) != 0);
    NVE_CHECK(host.dispatch_click_at(inc_rect->x + inc_rect->width * 0.5f, inc_rect->y + inc_rect->height * 0.5f) != 0);
    NVE_CHECK(find_leaf_by_text("Count: 2") != nullptr);
  } else {
    NVE_CHECK(host.dispatch_press(inc_btn->node_id, 10.0f, 10.0f) != 0);
    NVE_CHECK(find_leaf_by_text("Count: 1") != nullptr);
    NVE_CHECK(host.dispatch_press(reset_btn->node_id, 10.0f, 10.0f) != 0);
    NVE_CHECK(find_leaf_by_text("Count: 0") != nullptr);
    NVE_CHECK(host.dispatch_press(inc_btn->node_id, 10.0f, 10.0f) != 0);
    NVE_CHECK(host.dispatch_press(inc_btn->node_id, 10.0f, 10.0f) != 0);
    NVE_CHECK(find_leaf_by_text("Count: 2") != nullptr);
  }

  NVE_CHECK(host.reload_dylib(dylib_env, &err_code, &err_msg));
  NVE_CHECK(err_code == 0 && find_leaf_by_text("Count: 2") != nullptr);

  const std::filesystem::path snap_path =
      std::filesystem::temp_directory_path() / "nve_win_counter_test.nvesnap";
  std::error_code ec;
  std::filesystem::remove(snap_path, ec);
  NVE_CHECK(host.save_snapshot(snap_path.string(), 1u));
  NVE_CHECK(host.update_props(nve::PropMap{{"initialCount", int64_t{0}}}));

  nve::win::NveRnwComponentHost cold_counter;
  uint64_t counter_restore_us = 0u;
  NVE_CHECK(cold_counter.replay_frame0_snapshot(snap_path.string(), 1u, cfg, &counter_restore_us));
  std::cout << "[windows_shell_test] Cold mmap Frame-0 restore (counter dylib): "
            << counter_restore_us << " us\n";
  NVE_CHECK(!cold_counter.is_valid() && cold_counter.has_replayed_frame0());

  auto has_text_in = [](const nve::win::NveRnwComponentHost& h, std::string_view expected) {
    for (const auto& flat : h.visual_tree().build_draw_order()) {
      const auto* node = h.visual_tree().find(flat.node_id);
      if (node != nullptr && node->is_text_leaf && node->text == expected) return true;
    }
    return false;
  };
  NVE_CHECK(has_text_in(cold_counter, "Count: 2"));
  NVE_CHECK(cold_counter.load_dylib(dylib_env, cfg, &err_code, &err_msg));
  NVE_CHECK(err_code == 0 && has_text_in(cold_counter, "Count: 2"));
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

  NVE_CHECK(!cold.is_valid() && cold.has_replayed_frame0());
  NVE_CHECK(cold.visual_tree().find(2) != nullptr && cold.visual_tree().find(2)->text == "state=43");

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
        if (node != nullptr && node->is_text_leaf && node->text.find(needle) != std::string::npos) {
          return true;
        }
      }
      return false;
    };

    {
      nve::win::NveRnwComponentHost studio_writer;
      NVE_CHECK(studio_writer.load_dylib(studio_env, studio_cfg));
      NVE_CHECK(studio_writer.update_props(
          nve::PropMap{{"accent", std::string("violet")}, {"view", std::string("metrics")}}));
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

  static uint32_t dispatch_events(NveAppHandle h, NveEventBatch batch) {
    auto* self = static_cast<MockWinImageScrollComponent*>(h);
    if (batch.ptr == nullptr) return 0u;
    for (uint32_t i = 0u; i < batch.len; ++i) {
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

  static uint32_t set_props(NveAppHandle h, const NveMapEntry* entries, uint32_t count) {
    auto* self = static_cast<MockWinImageScrollComponent*>(h);
    for (uint32_t i = 0u; i < count; ++i) {
      const auto k = nve::str_slice_view(entries[i].key);
      if (k == "resizeMode" && entries[i].value.tag == NVE_VAL_STR) {
        const auto v = nve::str_slice_view(entries[i].value.data.str_val);
        self->resize_mode.assign(v.data(), v.size());
      } else if (k == "src" && entries[i].value.tag == NVE_VAL_STR) {
        const auto v = nve::str_slice_view(entries[i].value.data.str_val);
        if (v == "file") self->image_src = "file:///tmp/avatar.png";
        else self->image_src.assign(v.data(), v.size());
      }
    }
    return 1u;
  }

  static const void* get_interface(uint32_t iface) {
    static const NveSurfaceApi kSurface{&create, &destroy, &render, &dispatch_events};
    static const NveComponentApi kComp{&set_props, nullptr};
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

  const auto* img = host.visual_tree().find(3u);
  NVE_CHECK(img != nullptr && img->kind == NVE_NODE_KIND_IMAGE);
  NVE_CHECK(img->image_src.find("data:image/png;base64,") == 0u);
  NVE_CHECK(img->resize_mode == nve::win::ImageResizeMode::Cover);

  NVE_CHECK(host.update_props(nve::PropMap{{"resizeMode", std::string("contain")}}));
  NVE_CHECK(host.visual_tree().find(3u)->resize_mode == nve::win::ImageResizeMode::Contain);
  NVE_CHECK(host.update_props(nve::PropMap{{"resizeMode", std::string("stretch")}}));
  NVE_CHECK(host.visual_tree().find(3u)->resize_mode == nve::win::ImageResizeMode::Stretch);
  NVE_CHECK(host.update_props(
      nve::PropMap{{"resizeMode", std::string("center")}, {"src", std::string("file")}}));
  NVE_CHECK(host.visual_tree().find(3u)->resize_mode == nve::win::ImageResizeMode::Center);
  NVE_CHECK(host.visual_tree().find(3u)->image_src == "file:///tmp/avatar.png");

  const auto* scroll = host.visual_tree().find(4u);
  NVE_CHECK(scroll != nullptr && scroll->kind == NVE_NODE_KIND_SCROLL_VIEW);
  NVE_CHECK(scroll->clip_bounds && scroll->content_height >= 220.0f);

  const auto scroll_rect = host.visual_tree().absolute_rect(4u);
  NVE_CHECK(scroll_rect.has_value());
  const float sx = scroll_rect->x + 20.0f;
  const float sy = scroll_rect->y + 20.0f;
  NVE_CHECK(host.visual_tree().hit_test_scrollable(sx, sy) == std::optional<uint32_t>{4u});

  const auto child5_before = host.visual_tree().absolute_rect(5u);
  NVE_CHECK(child5_before.has_value());

  (void)host.dispatch_wheel_at(sx, sy, 0.0f, 45.0f);
  NVE_CHECK(mock->scroll_events == 1 && mock->last_scroll_node == 4u);
  NVE_CHECK(approx_eq(mock->last_scroll_y, 45.0f, 0.5f) && mock->last_scroll_content_h >= 220.0f);

  const auto off = host.visual_tree().scroll_offset(4u);
  NVE_CHECK(off.has_value() && approx_eq(off->second, 45.0f, 0.5f));

  const auto child5_after = host.visual_tree().absolute_rect(5u);
  NVE_CHECK(child5_after.has_value() && approx_eq(child5_after->y, child5_before->y - 45.0f, 0.5f));

  (void)host.dispatch_wheel_at(sx, sy, 0.0f, 500.0f);
  NVE_CHECK(mock->scroll_events == 2);
  NVE_CHECK(approx_eq(host.visual_tree().scroll_offset(4u)->second, 160.0f, 0.5f));
  NVE_CHECK(approx_eq(mock->last_scroll_y, 160.0f, 0.5f));

  (void)host.dispatch_scroll(4u, 12.0f, 34.0f, 160.0f, 220.0f);
  NVE_CHECK(mock->scroll_events == 3);
  NVE_CHECK(approx_eq(mock->last_scroll_x, 12.0f, 0.5f) && approx_eq(mock->last_scroll_y, 34.0f, 0.5f));

  const uint64_t updates_before = host.visual_tree().updated_count();
  NVE_CHECK(host.dispatch_press(5u, 10.0f, 10.0f) == 1u);
  NVE_CHECK(host.visual_tree().find(6u)->text == "WinCount:1");
  NVE_CHECK(host.visual_tree().updated_count() - updates_before == 1u);
}

} // namespace

void nve_test_win_live_snapshot_and_media() {
  test_live_counter_dylib_if_configured();
  test_frame0_mmap_snapshot_roundtrip();
  test_image_scroll_and_incremental_scene_view();
}
