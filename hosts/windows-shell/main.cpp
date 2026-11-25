#include "NveRnwComponentView.hpp"
#include "NveWinHostWindow.hpp"

#include <cmath>
#include <cstdlib>
#include <cstring>
#include <initializer_list>
#include <iostream>
#include <string>
#include <vector>

namespace {

// Minimal built-in demo NVE app when launched without `--dylib <path.dll>`.
class BuiltinWinDemoApp {
 public:
  explicit BuiltinWinDemoApp(uint32_t sid) : surface_id_(sid) {}

  static NveAppHandle create(uint32_t sid) { return new BuiltinWinDemoApp(sid); }
  static void destroy(NveAppHandle h) { delete static_cast<BuiltinWinDemoApp*>(h); }

  static NveBufferSlice render(NveAppHandle h, uint64_t frame_id) {
    auto* self = static_cast<BuiltinWinDemoApp*>(h);
    self->encode_frame(frame_id);
    return NveBufferSlice{self->cmd_buf_.data(), self->cmd_buf_.size()};
  }

  static uint32_t dispatch_events(NveAppHandle h, NveBufferSlice events) {
    auto* self = static_cast<BuiltinWinDemoApp*>(h);
    std::vector<nve::DecodedEvent> decoded;
    if (!nve::decode_event_ring(events, decoded)) {
      return 0;
    }
    for (const auto& ev : decoded) {
      if (ev.kind == nve::EventKind::Press && ev.node_id == 4) {
        ++self->count_;
        return 1;
      }
      if (ev.kind == nve::EventKind::Press && ev.node_id == 6) {
        self->count_ = 0;
        return 1;
      }
    }
    return 0;
  }

  static const void* get_interface(uint32_t interface_id) {
    static const NveSurfaceApi kSurface{
        .create = &create,
        .destroy = &destroy,
        .render = &render,
        .dispatch_events = &dispatch_events,
        .dispatch_events_direct = nullptr,
    };
    if (interface_id == NVE_IFACE_SURFACE) {
      return &kSurface;
    }
    return nullptr;
  }

 private:
  template <typename T>
  void push(T v) {
    const auto* p = reinterpret_cast<const uint8_t*>(&v);
    cmd_buf_.insert(cmd_buf_.end(), p, p + sizeof(T));
  }

  void push_str(const std::string& s) { cmd_buf_.insert(cmd_buf_.end(), s.begin(), s.end()); }

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
    uint32_t ops = 0;

    push<uint32_t>(NVE_ABI_MAGIC);
    push<uint16_t>(NVE_ABI_VERSION);
    push<uint16_t>(0);
    push<uint32_t>(surface_id_);
    push<uint64_t>(frame_id);
    push<uint32_t>(0);

    auto create_node = [&](uint32_t id, uint16_t kind) {
      push<uint8_t>(NVE_OP_CREATE_NODE);
      push<uint32_t>(id);
      push<uint16_t>(kind);
      push<uint16_t>(0);
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
      push_str(txt);
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

    auto set_layout = [&](uint32_t id,
                          uint8_t dir,
                          uint8_t justify,
                          uint8_t align,
                          uint8_t wtag,
                          float w,
                          uint8_t htag,
                          float h,
                          float pad,
                          float gap) {
      push<uint8_t>(NVE_OP_SET_LAYOUT);
      push<uint32_t>(id);
      push_dim(wtag, w);
      push_dim(htag, h);
      push_dim(0, 0.0f);
      push_dim(0, 0.0f);
      push_dim(0, 0.0f);
      push_dim(0, 0.0f);
      push<uint8_t>(dir);
      push<uint8_t>(justify);
      push<uint8_t>(align);
      push<float>(0.0f);
      push<float>(1.0f);
      push<float>(gap);
      push_insets(pad, pad, pad, pad);
      push_insets(0, 0, 0, 0);
      ++ops;
    };

    create_node(2, NVE_COMP_VIEW);
    create_node(3, NVE_COMP_TEXT);
    create_node(8, NVE_COMP_VIEW);
    create_node(4, NVE_COMP_PRESSABLE);
    create_node(5, NVE_COMP_TEXT);
    create_node(6, NVE_COMP_PRESSABLE);
    create_node(7, NVE_COMP_TEXT);

    set_children(NVE_ROOT_NODE_ID, {2});
    set_children(2, {3, 8});
    set_children(8, {4, 6});
    set_children(4, {5});
    set_children(6, {7});

    // Background slate canvas: #0f172a -> little-endian RGBA8888 0xFF2A170F
    set_layout(2, 0, 1, 2, 2, 100.0f, 2, 100.0f, 24.0f, 16.0f);
    set_style(2, 0xFF2A170Fu, 0xFFFFFFFFu, 0.0f, 16.0f);

    set_text(3, "NVE Windows Host Shell - Count: " + std::to_string(count_));
    set_style(3, 0x00000000u, 0xFFFFFFFFu, 0.0f, 24.0f);

    // Row for buttons
    set_layout(8, 1, 1, 2, 0, 0.0f, 0, 0.0f, 0.0f, 12.0f);
    set_style(8, 0x00000000u, 0xFFFFFFFFu, 0.0f, 16.0f);

    // Increment button: #2563eb -> 0xFFEB6325
    set_layout(4, 0, 1, 2, 1, 160.0f, 1, 48.0f, 0.0f, 0.0f);
    set_style(4, 0xFFEB6325u, 0xFFFFFFFFu, 12.0f, 16.0f);

    set_text(5, "Increment +");
    set_style(5, 0x00000000u, 0xFFFFFFFFu, 0.0f, 16.0f);

    // Reset button: #334155 -> 0xFF554133
    set_layout(6, 0, 1, 2, 1, 120.0f, 1, 48.0f, 0.0f, 0.0f);
    set_style(6, 0xFF554133u, 0xFFFFFFFFu, 12.0f, 16.0f);

    set_text(7, "Reset");
    set_style(7, 0x00000000u, 0xFFF0E8E2u, 0.0f, 16.0f);

    push<uint8_t>(NVE_OP_COMMIT);
    push<uint8_t>(1);
    ++ops;

    std::memcpy(cmd_buf_.data() + 20, &ops, sizeof(uint32_t));
  }

  uint32_t surface_id_{1};
  int count_{0};
  std::vector<uint8_t> cmd_buf_;
};

struct CliOptions {
  std::string dylib_path;
  std::string screenshot_path;
  int width{640};
  int height{420};
  float scale{1.0f};
};

CliOptions parse_cli(int argc, char** argv) {
  CliOptions opts{};
  for (int i = 1; i + 1 < argc; ++i) {
    if (std::strcmp(argv[i], "--dylib") == 0) {
      opts.dylib_path = argv[++i];
    } else if (std::strcmp(argv[i], "--screenshot") == 0) {
      opts.screenshot_path = argv[++i];
    } else if (std::strcmp(argv[i], "--width") == 0) {
      const int v = std::atoi(argv[++i]);
      if (v > 0) opts.width = v;
    } else if (std::strcmp(argv[i], "--height") == 0) {
      const int v = std::atoi(argv[++i]);
      if (v > 0) opts.height = v;
    } else if (std::strcmp(argv[i], "--scale") == 0) {
      const float v = std::strtof(argv[++i], nullptr);
      if (std::isfinite(v) && v > 0.0f) opts.scale = v;
    }
  }
  return opts;
}

} // namespace

int main(int argc, char** argv) {
  const CliOptions opts = parse_cli(argc, argv);

#if defined(_WIN32)
  ::SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

  // Headless offscreen Direct2D/DirectWrite screenshot mode for automated testing & CI.
  if (!opts.screenshot_path.empty()) {
    nve::win::NveRnwComponentHost host;
    const NveSurfaceConfig cfg{
        .surface_id = 1,
        .width = static_cast<float>(opts.width),
        .height = static_cast<float>(opts.height),
        .scale_factor = opts.scale,
    };

    if (!opts.dylib_path.empty()) {
      int err_code = 0;
      std::string err_msg;
      if (!host.load_dylib(opts.dylib_path, cfg, &err_code, &err_msg)) {
        std::cerr << "Failed to load NVE dylib '" << opts.dylib_path << "' (code " << err_code
                  << "): " << err_msg << "\n";
        return 1;
      }
    } else {
      if (!host.attach_interface(&BuiltinWinDemoApp::get_interface, cfg)) {
        std::cerr << "Failed to attach built-in demo interface\n";
        return 1;
      }
    }

    const uint32_t px_w =
        static_cast<uint32_t>(std::max(1.0f, std::round(cfg.width * cfg.scale_factor)));
    const uint32_t px_h =
        static_cast<uint32_t>(std::max(1.0f, std::round(cfg.height * cfg.scale_factor)));
    std::string bmp_err;
    if (!nve::win::render_visual_tree_to_bmp(
            host.visual_tree(),
            *host.text_measurer(),
            px_w,
            px_h,
            cfg.scale_factor,
            opts.screenshot_path,
            &bmp_err)) {
      std::cerr << "Failed to write BMP screenshot '" << opts.screenshot_path << "': " << bmp_err
                << "\n";
      return 1;
    }
    std::cout << "Rendered " << px_w << "x" << px_h << " screenshot to " << opts.screenshot_path
              << "\n";
    return 0;
  }

  nve::win::NveWinHostWindow window;
  nve::win::WinHostWindowOptions win_opts{};
  win_opts.title = "NVE Windows Host Shell";
  win_opts.width = opts.width;
  win_opts.height = opts.height;
  win_opts.surface_id = 1;

  if (!window.create(::GetModuleHandleW(nullptr), win_opts)) {
    std::cerr << "Failed to create Win32 host window\n";
    return 1;
  }

  if (!opts.dylib_path.empty()) {
    int err_code = 0;
    std::string err_msg;
    if (!window.load_dylib(opts.dylib_path, &err_code, &err_msg)) {
      std::cerr << "Failed to load NVE dylib '" << opts.dylib_path << "' (code " << err_code
                << "): " << err_msg << "\n";
      return 1;
    }
  } else {
    if (!window.attach_interface(&BuiltinWinDemoApp::get_interface)) {
      std::cerr << "Failed to attach built-in demo interface\n";
      return 1;
    }
  }

  window.show(SW_SHOWDEFAULT);
  return window.run_message_loop();
#else
  // Non-Windows headless smoke check for portable host controller verification.
  nve::win::NveRnwComponentHost host;
  const NveSurfaceConfig cfg{
      .surface_id = 1,
      .width = static_cast<float>(opts.width),
      .height = static_cast<float>(opts.height),
      .scale_factor = opts.scale,
  };
  if (!opts.dylib_path.empty()) {
    int err_code = 0;
    std::string err_msg;
    if (!host.load_dylib(opts.dylib_path, cfg, &err_code, &err_msg)) {
      std::cerr << "Failed to load NVE dylib '" << opts.dylib_path << "' (code " << err_code
                << "): " << err_msg << "\n";
      return 1;
    }
  } else if (!host.attach_interface(&BuiltinWinDemoApp::get_interface, cfg)) {
    std::cerr << "Failed to attach built-in demo interface\n";
    return 1;
  }
  std::cout << "nve_windows_shell headless host ready (nodes=" << host.node_count()
            << "; Win32/Direct2D window requires Windows)\n";
  return 0;
#endif
}
