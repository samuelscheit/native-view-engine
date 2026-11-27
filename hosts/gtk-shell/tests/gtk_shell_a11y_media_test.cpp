#include "NveGtkHost.hpp"

#include <cmath>
#include <cstdlib>
#include <cstring>
#include <filesystem>
#include <iostream>
#include <string>
#include <string_view>
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
    NVE_CHECK(writer_host.save_snapshot(snap_path.string(), 7u, "gtk4", &save_status));
    NVE_CHECK(save_status == nve::SnapshotStatus::Ok && std::filesystem::exists(snap_path));
  }

  {
    nve::NveGtkHost reader_host(cfg);
    NVE_CHECK(!reader_host.is_loaded() && !reader_host.has_replayed_frame0() && reader_host.node_count() == 0u);

    nve::SnapshotStatus mismatch_status = nve::SnapshotStatus::Ok;
    NVE_CHECK(!reader_host.replay_snapshot(snap_path.string(), 99u, nullptr, &mismatch_status));
    NVE_CHECK(mismatch_status == nve::SnapshotStatus::SchemaVersionMismatch);
    NVE_CHECK(reader_host.node_count() == 0u && !reader_host.has_replayed_frame0());

    NveFrameStats replay_stats{};
    nve::SnapshotStatus replay_status = nve::SnapshotStatus::IoError;
    NVE_CHECK(reader_host.replay_snapshot(snap_path.string(), 7u, &replay_stats, &replay_status));
    NVE_CHECK(replay_status == nve::SnapshotStatus::Ok && replay_stats.op_count > 0u);
    NVE_CHECK(!reader_host.is_loaded() && reader_host.has_replayed_frame0() && reader_host.node_count() == 3u);

    GtkWidget* pre_dylib_label = reader_host.widget_for_node_id(2u);
    NVE_CHECK(pre_dylib_label != nullptr && GTK_IS_LABEL(pre_dylib_label));
    NVE_CHECK(gtk_widget_get_parent(pre_dylib_label) == reader_host.root_widget());
    NVE_CHECK(std::string(gtk_label_get_text(GTK_LABEL(pre_dylib_label))) == "state=5");

    int err_code = -1;
    std::string err_msg;
    NVE_CHECK(reader_host.load_dylib(NVE_TEST_STATEFUL_DYLIB, &err_code, &err_msg, 5u));
    NVE_CHECK(err_code == 0 && err_msg.empty() && reader_host.is_loaded() && !reader_host.has_replayed_frame0());

    GtkWidget* live_label = reader_host.widget_for_node_id(2u);
    NVE_CHECK(live_label != nullptr && GTK_IS_LABEL(live_label));
    NVE_CHECK(std::string(gtk_label_get_text(GTK_LABEL(live_label))) == "state=5");

    NVE_CHECK(reader_host.simulate_press(3u, 8.0f, 8.0f) == 1u);
    NVE_CHECK(std::string(gtk_label_get_text(GTK_LABEL(live_label))) == "state=6");
    NVE_CHECK(reader_host.save_snapshot(snap_path.string(), 7u));
  }

  {
    nve::NveGtkHost verify_host(cfg);
    NVE_CHECK(verify_host.replay_snapshot(snap_path.string(), 7u));
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

  static uint32_t dispatch_events(NveAppHandle h, NveEventBatch batch) {
    auto* self = static_cast<MockGtkA11yImeComponent*>(h);
    if (batch.events == nullptr) return 0u;
    auto read_text = [&](const NveEvent& ev) -> std::string {
      if (ev.bytes.len == 0u || batch.bytes == nullptr || ev.bytes.offset > batch.bytes_len ||
          ev.bytes.len > batch.bytes_len - ev.bytes.offset) {
        return {};
      }
      return std::string(
          reinterpret_cast<const char*>(batch.bytes + ev.bytes.offset), ev.bytes.len);
    };
    uint32_t dirty = 0u;
    for (uint32_t i = 0u; i < batch.event_count; ++i) {
      const NveEvent& ev = batch.events[i];
      if (ev.node == 5u && ev.table == NVE_TABLE_TEXT_INPUT &&
          ev.event == NVE_TEXT_INPUT_EVENT_CHANGE) {
        ++self->text_change_calls;
        self->last_text = read_text(ev);
        dirty = 1u;
      } else if (ev.node == 5u && ev.table == NVE_TABLE_TEXT_INPUT &&
                 ev.event == NVE_TEXT_INPUT_EVENT_SUBMIT) {
        ++self->submit_calls;
        self->last_submit = read_text(ev);
      }
    }
    return dirty;
  }

  static uint32_t set_props(NveAppHandle h, const NveMapEntry* entries, uint32_t count) {
    auto* self = static_cast<MockGtkA11yImeComponent*>(h);
    for (uint32_t i = 0u; i < count; ++i) {
      const std::string_view key = nve::str_slice_view(entries[i].key);
      const NveValue& val = entries[i].value;
      if (key == "disabled" && val.tag == NVE_VAL_BOOL) self->disabled_btn = val.data.bool_val != 0u;
      if (key == "secure" && val.tag == NVE_VAL_BOOL) self->secure_input = val.data.bool_val != 0u;
      if (key == "text" && val.tag == NVE_VAL_STR) {
        self->input_text = std::string(nve::str_slice_view(val.data.str_val));
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
  NVE_CHECK(btn != nullptr && hdr != nullptr && chk != nullptr && entry != nullptr && GTK_IS_ENTRY(entry));

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

  const char* ph = gtk_entry_get_placeholder_text(GTK_ENTRY(entry));
  NVE_CHECK(ph != nullptr && std::string(ph) == "Type UTF-8 text...");
  NVE_CHECK(gtk_entry_get_visibility(GTK_ENTRY(entry)) == TRUE);
  NVE_CHECK(std::string(gtk_editable_get_text(GTK_EDITABLE(entry))) == "initial");
  NVE_CHECK(mock->text_change_calls == 0);

  NVE_CHECK(host.apply_props(nve::PropMap{{"text", std::string("programmatic")}}));
  NVE_CHECK(std::string(gtk_editable_get_text(GTK_EDITABLE(entry))) == "programmatic");
  NVE_CHECK(mock->text_change_calls == 0);

  {
    int pos = static_cast<int>(std::string("programmatic").size());
    gtk_editable_insert_text(GTK_EDITABLE(entry), " 日本語 café", -1, &pos);
  }
  NVE_CHECK(mock->text_change_calls == 1);
  NVE_CHECK(mock->last_text == "programmatic 日本語 café");
  NVE_CHECK(std::string(gtk_editable_get_text(GTK_EDITABLE(entry))) == "programmatic 日本語 café");

  gtk_editable_set_text(GTK_EDITABLE(entry), "日本語 café");
  const int calls_after_replace = mock->text_change_calls;
  NVE_CHECK(calls_after_replace >= 2 && mock->last_text == "日本語 café");
  NVE_CHECK(std::string(gtk_editable_get_text(GTK_EDITABLE(entry))) == "日本語 café");

  NVE_CHECK(host.render_frame() == 0);
  NVE_CHECK(std::string(gtk_editable_get_text(GTK_EDITABLE(entry))) == "日本語 café");
  NVE_CHECK(mock->text_change_calls == calls_after_replace);

  g_signal_emit_by_name(entry, "activate");
  NVE_CHECK(mock->submit_calls == 1 && mock->last_submit == "日本語 café");

  NVE_CHECK(host.apply_props(nve::PropMap{{"disabled", true}, {"secure", true}}));
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

  static uint32_t dispatch_events(NveAppHandle h, NveEventBatch batch) {
    auto* self = static_cast<MockGtkImageScrollComponent*>(h);
    if (batch.events == nullptr) return 0u;
    for (uint32_t i = 0; i < batch.event_count; ++i) {
      const NveEvent& ev = batch.events[i];
      if (ev.table == NVE_TABLE_SCROLL && ev.event == NVE_SCROLL_EVENT_SCROLL) {
        ++self->scroll_events;
        self->last_scroll_node = ev.node;
        self->last_scroll_x = nve::event_payload_f32(ev, 0u);
        self->last_scroll_y = nve::event_payload_f32(ev, 1u);
        self->last_scroll_content_h = nve::event_payload_f32(ev, 3u);
      } else if (ev.table == NVE_TABLE_INTERACTION && ev.event == NVE_INTERACTION_EVENT_PRESS &&
                 ev.node == 7u) {
        ++self->counter;
        return 1u;
      }
    }
    return 0u;
  }

  static uint32_t set_props(NveAppHandle h, const NveMapEntry* entries, uint32_t count) {
    auto* self = static_cast<MockGtkImageScrollComponent*>(h);
    for (uint32_t i = 0u; i < count; ++i) {
      if (nve::str_slice_view(entries[i].key) == "resizeMode" && entries[i].value.tag == NVE_VAL_STR) {
        self->resize_mode = std::string(nve::str_slice_view(entries[i].value.data.str_val));
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

void test_image_picture_scroll_and_incremental_reconcile() {
  const std::string data_uri = make_8x8_png_data_uri(220, 40, 60, 255);
  const std::filesystem::path tmp_png =
      std::filesystem::temp_directory_path() / "nve_gtk_img_8x8.png";
  {
    std::vector<uint8_t> rgba(8u * 8u * 4u, 200u);
    GBytes* raw = g_bytes_new(rgba.data(), rgba.size());
    GdkTexture* tex = gdk_memory_texture_new(8, 8, GDK_MEMORY_R8G8B8A8, raw, 8 * 4);
    g_bytes_unref(raw);
    NVE_CHECK(tex != nullptr && gdk_texture_save_to_png(tex, tmp_png.c_str()) == TRUE);
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
  NVE_CHECK(pic3 != nullptr && GTK_IS_PICTURE(pic3) && pic4 != nullptr && GTK_IS_PICTURE(pic4));
  NVE_CHECK(scroll5 != nullptr && GTK_IS_SCROLLED_WINDOW(scroll5) && lbl7 != nullptr && GTK_IS_LABEL(lbl7));

  GdkPaintable* p3 = gtk_picture_get_paintable(GTK_PICTURE(pic3));
  GdkPaintable* p4 = gtk_picture_get_paintable(GTK_PICTURE(pic4));
  NVE_CHECK(p3 != nullptr && GDK_IS_TEXTURE(p3) && p4 != nullptr && GDK_IS_TEXTURE(p4));
  NVE_CHECK(gdk_texture_get_width(GDK_TEXTURE(p3)) == 8 && gdk_texture_get_height(GDK_TEXTURE(p3)) == 8);
  NVE_CHECK(gdk_texture_get_width(GDK_TEXTURE(p4)) == 8 && gdk_texture_get_height(GDK_TEXTURE(p4)) == 8);
  NVE_CHECK(host.texture_cache().size() == 2u);

#if GTK_CHECK_VERSION(4, 8, 0)
  NVE_CHECK(gtk_picture_get_content_fit(GTK_PICTURE(pic3)) == GTK_CONTENT_FIT_COVER);
  NVE_CHECK(gtk_picture_get_content_fit(GTK_PICTURE(pic4)) == GTK_CONTENT_FIT_CONTAIN);

  NVE_CHECK(host.apply_props(nve::PropMap{{"resizeMode", std::string("contain")}}));
  NVE_CHECK(gtk_picture_get_content_fit(GTK_PICTURE(pic3)) == GTK_CONTENT_FIT_CONTAIN);

  NVE_CHECK(host.apply_props(nve::PropMap{{"resizeMode", std::string("stretch")}}));
  NVE_CHECK(gtk_picture_get_content_fit(GTK_PICTURE(pic3)) == GTK_CONTENT_FIT_FILL);

  NVE_CHECK(host.apply_props(nve::PropMap{{"resizeMode", std::string("center")}}));
  NVE_CHECK(gtk_picture_get_content_fit(GTK_PICTURE(pic3)) == GTK_CONTENT_FIT_SCALE_DOWN);
#endif

  GtkAdjustment* vadj = gtk_scrolled_window_get_vadjustment(GTK_SCROLLED_WINDOW(scroll5));
  GtkAdjustment* hadj = gtk_scrolled_window_get_hadjustment(GTK_SCROLLED_WINDOW(scroll5));
  NVE_CHECK(vadj != nullptr && hadj != nullptr);
  gtk_adjustment_set_value(vadj, 42.0);
  NVE_CHECK(mock->scroll_events >= 1 && mock->last_scroll_node == 5u);
  NVE_CHECK(approx_eq(mock->last_scroll_y, 42.0, 1.0) && mock->last_scroll_content_h >= 240.0f);

  const int events_before_h = mock->scroll_events;
  gtk_adjustment_set_value(hadj, 15.0);
  NVE_CHECK(mock->scroll_events > events_before_h && approx_eq(mock->last_scroll_x, 15.0, 1.0));

  const uint64_t updates_before = host.reconcile_update_count();
  NVE_CHECK(host.simulate_press(7u, 4.0f, 4.0f) == 1u);
  NVE_CHECK(std::string(gtk_label_get_text(GTK_LABEL(lbl7))) == "Step:1");
  NVE_CHECK(host.reconcile_update_count() - updates_before == 1u);

  std::error_code ec;
  std::filesystem::remove(tmp_png, ec);
}

} // namespace

void nve_test_gtk_snapshot_a11y_and_media() {
  test_snapshot_replay_state_restore_and_atomic_save();
  test_accessibility_and_ime_text_input();
  test_image_picture_scroll_and_incremental_reconcile();
}
