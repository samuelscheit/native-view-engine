#include "NveGtkHost.hpp"

#include <chrono>
#include <cstdlib>
#include <cstring>
#include <filesystem>
#include <iostream>
#include <memory>
#include <string>
#include <vector>

namespace {

// Minimal built-in demo NVE app when launched without `--dylib <path>`.
class BuiltinGtkDemoApp {
 public:
  explicit BuiltinGtkDemoApp(uint32_t sid) : surface_id_(sid) {}

  static NveAppHandle create(uint32_t sid) { return new BuiltinGtkDemoApp(sid); }
  static void destroy(NveAppHandle h) { delete static_cast<BuiltinGtkDemoApp*>(h); }

  static NveBufferSlice render(NveAppHandle h, uint64_t frame_id) {
    auto* self = static_cast<BuiltinGtkDemoApp*>(h);
    self->encode_frame(frame_id);
    return NveBufferSlice{self->cmd_buf_.data(), self->cmd_buf_.size()};
  }

  static uint32_t dispatch_events(NveAppHandle h, NveBufferSlice events) {
    auto* self = static_cast<BuiltinGtkDemoApp*>(h);
    std::vector<nve::DecodedEvent> decoded;
    if (!nve::decode_event_ring(events, decoded)) {
      return 0u;
    }
    for (const auto& ev : decoded) {
      if (ev.kind == nve::EventKind::Press && ev.node_id == 4u) {
        ++self->count_;
        return 1u;
      }
    }
    return 0u;
  }

  static NveBufferSlice snapshot_state(NveAppHandle h) {
    auto* self = static_cast<BuiltinGtkDemoApp*>(h);
    self->state_buf_ = std::to_string(self->count_);
    return NveBufferSlice{
        reinterpret_cast<const uint8_t*>(self->state_buf_.data()),
        self->state_buf_.size(),
    };
  }

  static void restore_state(NveAppHandle h, NveBufferSlice state) {
    auto* self = static_cast<BuiltinGtkDemoApp*>(h);
    if (state.ptr == nullptr || state.len == 0u) {
      return;
    }
    const std::string raw(reinterpret_cast<const char*>(state.ptr), state.len);
    self->count_ = std::atoi(raw.c_str());
  }

  static const void* get_interface(uint32_t interface_id) {
    static const NveSurfaceApi kSurface{
        .create = &create,
        .destroy = &destroy,
        .render = &render,
        .dispatch_events = &dispatch_events,
        .dispatch_events_direct = nullptr,
    };
    static const NveDevApi kDev{
        .apply_hot_patch = nullptr,
        .snapshot_state = &snapshot_state,
        .restore_state = &restore_state,
    };
    if (interface_id == NVE_IFACE_SURFACE) {
      return &kSurface;
    }
    if (interface_id == NVE_IFACE_DEV) {
      return &kDev;
    }
    return nullptr;
  }

 private:
  template <typename T>
  void push(T v) {
    const auto* p = reinterpret_cast<const uint8_t*>(&v);
    cmd_buf_.insert(cmd_buf_.end(), p, p + sizeof(T));
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

  void encode_frame(uint64_t frame_id) {
    cmd_buf_.clear();
    uint32_t ops = 0u;
    push<uint32_t>(NVE_ABI_MAGIC);
    push<uint16_t>(NVE_ABI_VERSION);
    push<uint16_t>(0u);
    push<uint32_t>(surface_id_);
    push<uint64_t>(frame_id);
    push<uint32_t>(0u);

    auto create_node = [&](uint32_t id, uint16_t kind) {
      push<uint8_t>(NVE_OP_CREATE_NODE);
      push<uint32_t>(id);
      push<uint16_t>(kind);
      push<uint16_t>(0u);
      ++ops;
    };

    auto set_children = [&](uint32_t parent, std::initializer_list<uint32_t> kids) {
      push<uint8_t>(NVE_OP_SET_CHILDREN);
      push<uint32_t>(parent);
      push<uint32_t>(static_cast<uint32_t>(kids.size()));
      for (uint32_t k : kids) {
        push<uint32_t>(k);
      }
      ++ops;
    };

    auto set_text = [&](uint32_t id, const std::string& txt) {
      push<uint8_t>(NVE_OP_SET_TEXT);
      push<uint32_t>(id);
      push<uint32_t>(static_cast<uint32_t>(txt.size()));
      cmd_buf_.insert(cmd_buf_.end(), txt.begin(), txt.end());
      ++ops;
    };

    auto set_style = [&](uint32_t id, uint32_t bg, uint32_t fg, float radius, float font_size) {
      push<uint8_t>(NVE_OP_SET_STYLE);
      push<uint32_t>(id);
      push<uint32_t>(bg);
      push<uint32_t>(fg);
      push<uint32_t>(0u);
      push<float>(0.0f);
      push<float>(radius);
      push<float>(1.0f);
      push<float>(font_size);
      push<int32_t>(0);
      ++ops;
    };

    auto set_layout =
        [&](uint32_t id, uint8_t wtag, float w, uint8_t htag, float h, float pad, float gap) {
          push<uint8_t>(NVE_OP_SET_LAYOUT);
          push<uint32_t>(id);
          push_dim(wtag, w);
          push_dim(htag, h);
          push_dim(0u, 0.0f);
          push_dim(0u, 0.0f);
          push_dim(0u, 0.0f);
          push_dim(0u, 0.0f);
          push<uint8_t>(0u); /* Column */
          push<uint8_t>(1u); /* Center */
          push<uint8_t>(2u); /* Center */
          push<float>(0.0f);
          push<float>(1.0f);
          push<float>(gap);
          push_insets(pad, pad, pad, pad);
          push_insets(0.0f, 0.0f, 0.0f, 0.0f);
          ++ops;
        };

    create_node(2u, NVE_COMP_VIEW);
    create_node(3u, NVE_COMP_TEXT);
    create_node(4u, NVE_COMP_PRESSABLE);
    create_node(5u, NVE_COMP_TEXT);

    set_children(NVE_ROOT_NODE_ID, {2u});
    set_children(2u, {3u, 4u});
    set_children(4u, {5u});

    set_layout(2u, 2u, 100.0f, 2u, 100.0f, 24.0f, 16.0f);
    set_style(2u, 0xFF2A170Fu, 0xFFFFFFFFu, 0.0f, 16.0f);

    set_text(3u, "NVE GTK4 Host Shell — Count: " + std::to_string(count_));
    set_style(3u, 0x00000000u, 0xFFFFFFFFu, 0.0f, 22.0f);

    set_layout(4u, 1u, 200.0f, 1u, 44.0f, 10.0f, 0.0f);
    set_style(4u, 0xFFEB6325u, 0xFFFFFFFFu, 12.0f, 16.0f);

    set_text(5u, "Click to Increment");
    set_style(5u, 0x00000000u, 0xFFFFFFFFu, 0.0f, 15.0f);

    push<uint8_t>(NVE_OP_COMMIT);
    push<uint8_t>(1u);
    ++ops;

    std::memcpy(cmd_buf_.data() + 20, &ops, sizeof(uint32_t));
  }

  uint32_t surface_id_{1u};
  int count_{0};
  std::vector<uint8_t> cmd_buf_;
  std::string state_buf_;
};

struct AppLaunchState {
  std::string dylib_path;
  std::string snapshot_path;
  uint32_t schema_version{1u};
  bool snapshot_saved{false};
  int width{640};
  int height{420};
  std::unique_ptr<nve::NveGtkHost> host;
};

void persist_snapshot_if_configured(AppLaunchState* state) {
  if (state == nullptr || state->snapshot_saved || state->snapshot_path.empty() ||
      state->host == nullptr) {
    return;
  }
  state->snapshot_saved = true;
  if (state->host->save_snapshot(state->snapshot_path, state->schema_version)) {
    std::cerr << "[nve-gtk-shell] Saved Frame-0 + state snapshot to " << state->snapshot_path
              << " (schema v" << state->schema_version << ")\n";
  } else {
    std::cerr << "[nve-gtk-shell] Warning: failed to save snapshot to " << state->snapshot_path
              << "\n";
  }
}

gboolean on_window_close_request(GtkWindow* window, gpointer user_data) {
  (void)window;
  persist_snapshot_if_configured(static_cast<AppLaunchState*>(user_data));
  return FALSE;
}

void on_app_activate(GtkApplication* app, gpointer user_data) {
  auto* state = static_cast<AppLaunchState*>(user_data);

  GtkWidget* window = gtk_application_window_new(app);
  gtk_window_set_title(GTK_WINDOW(window), "NVE GTK4 Host Shell");
  gtk_window_set_default_size(GTK_WINDOW(window), state->width, state->height);

  NveSurfaceConfig cfg{
      .surface_id = 1u,
      .width = static_cast<float>(state->width),
      .height = static_cast<float>(state->height),
      .scale_factor = 1.0f,
  };
  state->host = std::make_unique<nve::NveGtkHost>(cfg);
  state->host->attach_window_resize(GTK_WINDOW(window));
  g_signal_connect(
      window,
      "close-request",
      nve::gtk_detail::to_gcallback(&on_window_close_request),
      state);

  gtk_window_set_child(GTK_WINDOW(window), state->host->host_widget());

  if (!state->snapshot_path.empty()) {
    std::error_code ec;
    if (std::filesystem::exists(state->snapshot_path, ec)) {
      const auto t0 = std::chrono::steady_clock::now();
      if (state->host->replay_snapshot(state->snapshot_path, state->schema_version)) {
        const auto elapsed_us = std::chrono::duration_cast<std::chrono::microseconds>(
                                    std::chrono::steady_clock::now() - t0)
                                    .count();
        std::cerr << "[nve-gtk-shell] Instant-boot Frame-0 mmap restore from "
                  << state->snapshot_path << " in " << elapsed_us << " us\n";
      }
    }
  }

  gtk_window_present(GTK_WINDOW(window));

  if (!state->dylib_path.empty()) {
    int err_code = 0;
    std::string err_msg;
    if (!state->host->load_dylib(state->dylib_path, &err_code, &err_msg)) {
      std::cerr << "Failed to load NVE dylib '" << state->dylib_path << "' (code " << err_code
                << "): " << err_msg << "\n";
      (void)state->host->attach_interface(&BuiltinGtkDemoApp::get_interface, 1u);
    }
  } else {
    (void)state->host->attach_interface(&BuiltinGtkDemoApp::get_interface, 1u);
  }
}

void print_usage(const char* argv0) {
  std::cout << "Usage: " << argv0
            << " [--dylib <path/to/libapp.so>] [--snapshot <path/to/frame0.nvesnap>]"
               " [--schema <ver>] [--width W] [--height H]\n";
}

} // namespace

int main(int argc, char* argv[]) {
  AppLaunchState state{};
  for (int i = 1; i < argc; ++i) {
    if (std::strcmp(argv[i], "--help") == 0 || std::strcmp(argv[i], "-h") == 0) {
      print_usage(argv[0]);
      return 0;
    }
    if (std::strcmp(argv[i], "--dylib") == 0 && i + 1 < argc) {
      state.dylib_path = argv[++i];
    } else if (std::strcmp(argv[i], "--snapshot") == 0 && i + 1 < argc) {
      state.snapshot_path = argv[++i];
    } else if (std::strcmp(argv[i], "--schema") == 0 && i + 1 < argc) {
      const long parsed = std::strtol(argv[++i], nullptr, 10);
      if (parsed >= 0) {
        state.schema_version = static_cast<uint32_t>(parsed);
      }
    } else if (std::strcmp(argv[i], "--width") == 0 && i + 1 < argc) {
      const int parsed = std::atoi(argv[++i]);
      if (parsed > 0) {
        state.width = parsed;
      }
    } else if (std::strcmp(argv[i], "--height") == 0 && i + 1 < argc) {
      const int parsed = std::atoi(argv[++i]);
      if (parsed > 0) {
        state.height = parsed;
      }
    }
  }

  GtkApplication* app = gtk_application_new("dev.nve.GtkShell", G_APPLICATION_NON_UNIQUE);
  g_signal_connect(app, "activate", nve::gtk_detail::to_gcallback(&on_app_activate), &state);
  const int status = g_application_run(G_APPLICATION(app), 0, nullptr);

  persist_snapshot_if_configured(&state);
  // Destroy host before releasing GtkApplication.
  state.host.reset();
  g_object_unref(app);
  return status;
}
