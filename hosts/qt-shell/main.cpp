#include "NveQtWidget.hpp"

#ifdef NVE_QT_HAS_QUICK
#include "NveQtQuickItem.hpp"
#include <QQmlComponent>
#include <QQuickView>
#include <QUrl>
#endif

#include <QApplication>
#include <QFileInfo>
#include <QObject>
#include <QString>

#include <algorithm>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <initializer_list>
#include <iostream>
#include <string>
#include <vector>

namespace {

/// Minimal built-in interactive NVE demo used when `nve_qt_shell` is launched without a dylib.
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

  static uint32_t dispatch_events_direct(NveAppHandle h, NveEventBatch batch) {
    auto* self = static_cast<BuiltinQtDemoApp*>(h);
    if (batch.ptr == nullptr) {
      return 0;
    }
    for (uint32_t i = 0; i < batch.len; ++i) {
      const NveDirectEvent& ev = batch.ptr[i];
      if (ev.kind == static_cast<uint8_t>(nve::EventKind::Press) && ev.node_id == 4) {
        ++self->count_;
        return 1;
      }
    }
    return 0;
  }

  static NveBufferSlice snapshot_state(NveAppHandle h) {
    auto* self = static_cast<BuiltinQtDemoApp*>(h);
    self->state_buf_ = std::to_string(self->count_);
    return NveBufferSlice{
        reinterpret_cast<const uint8_t*>(self->state_buf_.data()),
        self->state_buf_.size(),
    };
  }

  static void restore_state(NveAppHandle h, NveBufferSlice state) {
    auto* self = static_cast<BuiltinQtDemoApp*>(h);
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
        .dispatch_events_direct = &dispatch_events_direct,
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
  std::string state_buf_;
};

#ifdef NVE_QT_HAS_QUICK
int runQuickShell(
    QApplication& app,
    const QString& dylib_path,
    const QString& pair_connect,
    const QString& pair_token,
    const QString& pair_transport,
    const QString& snapshot_path,
    uint32_t schema_version,
    int width,
    int height) {
  nve::qt::NveQtQuickItem::registerQmlType();

  QQuickView view;
  view.setTitle(QStringLiteral("NVE Qt Quick GPU Host Shell"));
  view.setResizeMode(QQuickView::SizeRootObjectToView);
  view.resize(width, height);

  const QUrl qml_url;
  QQmlComponent component(view.engine());
  component.setData(QByteArrayLiteral("import Nve 1.0\nNveSurface {}\n"), qml_url);
  QObject* root_obj = component.create(view.rootContext());
  auto* surface = qobject_cast<nve::qt::NveQtQuickItem*>(root_obj);
  if (surface == nullptr) {
    std::cerr << "[nve-qt-shell] Failed to create QML NveSurface: "
              << component.errorString().toStdString() << "\n";
    delete root_obj;
    return 1;
  }

  view.setContent(qml_url, &component, root_obj);
  surface->setSize(QSizeF(width, height));

  if (!snapshot_path.isEmpty() && QFileInfo::exists(snapshot_path)) {
    uint64_t restore_us = 0u;
    if (surface->replaySnapshot(snapshot_path, schema_version, 1u, &restore_us)) {
      std::cout << "[nve-qt-shell] Instant-boot Frame-0 mmap restore (Quick GPU) from '"
                << snapshot_path.toStdString() << "' in " << restore_us << " us\n";
    }
  }

  view.show();

  if (!pair_connect.isEmpty()) {
    QString err_msg;
    if (!surface->connectPaired(pair_connect, pair_token, pair_transport, 1u, &err_msg)) {
      std::cerr << "Failed to connect paired client to '" << pair_connect.toStdString()
                << "': " << err_msg.toStdString() << "\n";
      return 1;
    }
    view.setTitle(QStringLiteral("NVE Qt Quick GPU Host Shell (Paired: %1)").arg(pair_connect));
  } else if (!dylib_path.isEmpty()) {
    int err_code = 0;
    QString err_msg;
    if (!surface->loadDylib(dylib_path, 1, &err_code, &err_msg)) {
      std::cerr << "Failed to load NVE dylib '" << dylib_path.toStdString()
                << "' (code " << err_code << "): " << err_msg.toStdString() << "\n";
      return 1;
    }
  } else {
    if (!surface->attachInterface(&BuiltinQtDemoApp::get_interface, 1)) {
      std::cerr << "Failed to attach built-in NVE Qt demo app\n";
      return 1;
    }
  }

  if (!snapshot_path.isEmpty()) {
    QObject::connect(
        &app,
        &QApplication::aboutToQuit,
        surface,
        [surface, snapshot_path, schema_version]() {
          if (surface->saveSnapshot(snapshot_path, schema_version)) {
            std::cout << "[nve-qt-shell] Saved Frame-0 + state snapshot to '"
                      << snapshot_path.toStdString() << "' (schema v" << schema_version << ")\n";
          } else {
            std::cerr << "[nve-qt-shell] Warning: failed to save snapshot to '"
                      << snapshot_path.toStdString() << "'\n";
          }
        });
  }

  return app.exec();
}
#endif

} // namespace

int main(int argc, char* argv[]) {
  QApplication app(argc, argv);

  bool use_quick = false;
  QString dylib_path;
  QString pair_connect;
  QString pair_token;
  QString pair_transport;
  QString snapshot_path;
  uint32_t schema_version = 1u;
  int width = 640;
  int height = 420;
  for (int i = 1; i < argc; ++i) {
    if (std::strcmp(argv[i], "--quick") == 0) {
      use_quick = true;
    } else if (std::strcmp(argv[i], "--dylib") == 0 && i + 1 < argc) {
      dylib_path = QString::fromUtf8(argv[++i]);
    } else if (std::strcmp(argv[i], "--pair-connect") == 0 && i + 1 < argc) {
      pair_connect = QString::fromUtf8(argv[++i]);
    } else if (std::strcmp(argv[i], "--pair-token") == 0 && i + 1 < argc) {
      pair_token = QString::fromUtf8(argv[++i]);
    } else if (std::strcmp(argv[i], "--pair-transport") == 0 && i + 1 < argc) {
      pair_transport = QString::fromUtf8(argv[++i]);
    } else if (std::strcmp(argv[i], "--snapshot") == 0 && i + 1 < argc) {
      snapshot_path = QString::fromUtf8(argv[++i]);
    } else if (std::strcmp(argv[i], "--schema") == 0 && i + 1 < argc) {
      const long parsed = std::strtol(argv[++i], nullptr, 10);
      if (parsed >= 0) {
        schema_version = static_cast<uint32_t>(parsed);
      }
    } else if (std::strcmp(argv[i], "--width") == 0 && i + 1 < argc) {
      width = std::max(120, std::atoi(argv[++i]));
    } else if (std::strcmp(argv[i], "--height") == 0 && i + 1 < argc) {
      height = std::max(120, std::atoi(argv[++i]));
    } else if (argv[i][0] != '-' && dylib_path.isEmpty()) {
      dylib_path = QString::fromUtf8(argv[i]);
    }
  }

  if (use_quick) {
#ifdef NVE_QT_HAS_QUICK
    return runQuickShell(
        app, dylib_path, pair_connect, pair_token, pair_transport, snapshot_path, schema_version,
        width, height);
#else
    std::cerr << "[nve-qt-shell] Error: --quick requested, but this binary was built without Qt6::Quick.\n";
    return 1;
#endif
  }

  nve::qt::NveQtWidget window;
  window.setWindowTitle(QStringLiteral("NVE Qt 6 Host Shell"));
  window.resize(width, height);

  if (!snapshot_path.isEmpty() && QFileInfo::exists(snapshot_path)) {
    uint64_t restore_us = 0u;
    if (window.replayFrame0Snapshot(snapshot_path, schema_version, 1u, &restore_us)) {
      std::cout << "[nve-qt-shell] Instant-boot Frame-0 mmap restore from '"
                << snapshot_path.toStdString() << "' in " << restore_us << " us\n";
    }
  }

  window.show();

  if (!pair_connect.isEmpty()) {
    QString err_msg;
    if (!window.connectPaired(pair_connect, pair_token, pair_transport, 1u, &err_msg)) {
      std::cerr << "Failed to connect paired client to '" << pair_connect.toStdString()
                << "': " << err_msg.toStdString() << "\n";
      return 1;
    }
    window.setWindowTitle(QStringLiteral("NVE Qt 6 Host Shell (Paired: %1)").arg(pair_connect));
  } else if (!dylib_path.isEmpty()) {
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

  if (!snapshot_path.isEmpty()) {
    QObject::connect(&app, &QApplication::aboutToQuit, &window, [&window, snapshot_path, schema_version]() {
      if (window.saveSnapshot(snapshot_path, schema_version)) {
        std::cout << "[nve-qt-shell] Saved Frame-0 + state snapshot to '"
                  << snapshot_path.toStdString() << "' (schema v" << schema_version << ")\n";
      } else {
        std::cerr << "[nve-qt-shell] Warning: failed to save snapshot to '"
                  << snapshot_path.toStdString() << "'\n";
      }
    });
  }

  return app.exec();
}
