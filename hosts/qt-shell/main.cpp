#include "NveQtWidget.hpp"

#include <QApplication>
#include <QString>

#include <cstring>
#include <initializer_list>
#include <iostream>
#include <string>
#include <vector>

namespace {

/// Minimal built-in interactive NVE demo used when `nve_qt_shell` is launched without `--dylib`.
class BuiltinQtDemoApp {
 public:
  explicit BuiltinQtDemoApp(uint32_t sid) : surface_id_(sid) {}

  static NveAppHandle create(uint32_t sid) { return new BuiltinQtDemoApp(sid); }
  static void destroy(NveAppHandle h) { delete static_cast<BuiltinQtDemoApp*>(h); }

  static NveBufferSlice render(NveAppHandle h, uint64_t frame_id) {
    auto* self = static_cast<BuiltinQtDemoApp*>(h);
    self->encode_frame(frame_id);
    return NveBufferSlice{self->cmd_buf_.data(), self->cmd_buf_.size()};
  }

  static uint32_t dispatch_events(NveAppHandle h, NveBufferSlice events) {
    auto* self = static_cast<BuiltinQtDemoApp*>(h);
    std::vector<nve::DecodedEvent> decoded;
    if (!nve::decode_event_ring(events, decoded)) {
      return 0;
    }
    for (const auto& ev : decoded) {
      if (ev.kind == nve::EventKind::Press && ev.node_id == 4) {
        ++self->count_;
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
    return (interface_id == NVE_IFACE_SURFACE) ? &kSurface : nullptr;
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
          push_dim(0, 0.0f);
          push_dim(0, 0.0f);
          push_dim(0, 0.0f);
          push_dim(0, 0.0f);
          push<uint8_t>(0); // Column
          push<uint8_t>(1); // Center
          push<uint8_t>(2); // Center
          push<float>(0.0f);
          push<float>(1.0f);
          push<float>(gap);
          push_insets(pad, pad, pad, pad);
          push_insets(0.0f, 0.0f, 0.0f, 0.0f);
          ++ops;
        };

    create_node(2, NVE_COMP_VIEW);
    create_node(3, NVE_COMP_TEXT);
    create_node(4, NVE_COMP_PRESSABLE);
    create_node(5, NVE_COMP_TEXT);

    set_children(NVE_ROOT_NODE_ID, {2});
    set_children(2, {3, 4});
    set_children(4, {5});

    set_layout(2, 2, 100.0f, 2, 100.0f, 24.0f, 16.0f);
    set_style(2, 0xFF2A170Fu, 0xFFFFFFFFu, 0.0f, 16.0f); // #0f172a

    set_text(3, "NVE Qt 6 Host Shell — Count: " + std::to_string(count_));
    set_style(3, 0x00000000u, 0xFFFFFFFFu, 0.0f, 24.0f);

    set_layout(4, 1, 200.0f, 1, 48.0f, 12.0f, 0.0f);
    set_style(4, 0xFFEB6325u, 0xFFFFFFFFu, 12.0f, 16.0f); // #2563eb

    set_text(5, "Click to Increment");
    set_style(5, 0x00000000u, 0xFFFFFFFFu, 0.0f, 16.0f);

    push<uint8_t>(NVE_OP_COMMIT);
    push<uint8_t>(1);
    ++ops;

    std::memcpy(cmd_buf_.data() + 20, &ops, sizeof(uint32_t));
  }

  uint32_t surface_id_{1};
  int count_{0};
  std::vector<uint8_t> cmd_buf_;
};

} // namespace

int main(int argc, char* argv[]) {
  QApplication app(argc, argv);

  QString dylib_path;
  int width = 640;
  int height = 420;
  for (int i = 1; i + 1 < argc; ++i) {
    if (std::strcmp(argv[i], "--dylib") == 0) {
      dylib_path = QString::fromUtf8(argv[i + 1]);
      ++i;
    } else if (std::strcmp(argv[i], "--width") == 0) {
      width = std::max(120, std::atoi(argv[i + 1]));
      ++i;
    } else if (std::strcmp(argv[i], "--height") == 0) {
      height = std::max(120, std::atoi(argv[i + 1]));
      ++i;
    }
  }

  nve::qt::NveQtWidget window;
  window.setWindowTitle(QStringLiteral("NVE Qt 6 Host Shell"));
  window.resize(width, height);

  if (!dylib_path.isEmpty()) {
    int err_code = 0;
    QString err_msg;
    if (!window.loadDylib(dylib_path, 1, &err_code, &err_msg)) {
      std::cerr << "Failed to load NVE dylib '" << dylib_path.toStdString()
                << "' (code " << err_code << "): " << err_msg.toStdString() << "\n";
      return 1;
    }
  } else {
    if (!window.attachInterface(&BuiltinQtDemoApp::get_interface, 1)) {
      std::cerr << "Failed to attach built-in NVE Qt demo app\n";
      return 1;
    }
  }

  window.show();
  return app.exec();
}
