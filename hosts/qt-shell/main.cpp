// nve-qt-shell: runs an NVE app (built-in counter demo, a `cdylib`, or a paired `nve dev` server)
// in a native Qt 6 window (`QWidget` host, or `--quick` for the Qt Quick scene-graph host),
// persisting the app's state across launches (window close, SIGINT / SIGTERM, shutdown).
#include "NveQtWidget.hpp"

#ifdef NVE_QT_HAS_QUICK
#include "NveQtQuickItem.hpp"

#include <QQmlComponent>
#include <QQuickView>
#endif

#include <nve/host/demo_counter_app.hpp>
#include <nve/host/launch_options.hpp>

#include <QApplication>
#include <QEvent>
#include <QObject>
#include <QSocketNotifier>
#include <QWindow>

#if defined(__unix__) || defined(__APPLE__)
#include <csignal>
#include <sys/socket.h>
#include <unistd.h>
#endif

#include <iostream>
#include <memory>
#include <string>

namespace {

using nve::qt::QtHost;

/// The shared command line (`launch_options.hpp`) plus `--quick` (the Qt Quick surface).
struct LaunchOptions : nve::host::LaunchOptions {
  bool quick = false;
};

/// Saves the app state when the window closes, else at shutdown (a quit without a close: signals,
/// `QApplication::quit`).
class StatePersister final : public QObject {
 public:
  StatePersister(QtHost& host, const LaunchOptions& options) : host_(host), options_(options) {}

  void persist(const char* trigger) {
    if (options_.no_state || options_.state_path.empty() || host_.is_paired()) return;
    if (host_.save_state(options_.state_path, options_.schema_version)) {
      saved_ = true;
      std::cerr << "[nve-qt-shell] saved app state to " << options_.state_path << " (" << trigger << ")\n";
    }
  }
  void persist_unless_saved(const char* trigger) {
    if (!saved_) persist(trigger);
  }

 protected:
  bool eventFilter(QObject* watched, QEvent* event) override {
    if (event->type() == QEvent::Close) persist("window-close");
    return QObject::eventFilter(watched, event);
  }

 private:
  QtHost& host_;
  const LaunchOptions& options_;
  bool saved_ = false;
};

#if defined(__unix__) || defined(__APPLE__)
int g_signal_fds[2] = {-1, -1};

void on_termination_signal(int) {
  const char byte = 1;
  (void)!::write(g_signal_fds[1], &byte, 1);  // async-signal-safe: hand over to the event loop
}

/// SIGINT / SIGTERM: quit on the UI thread (self-pipe + `QSocketNotifier`); `run` then persists.
std::unique_ptr<QSocketNotifier> install_signal_handlers() {
  if (::socketpair(AF_UNIX, SOCK_STREAM, 0, g_signal_fds) != 0) return nullptr;
  auto notifier = std::make_unique<QSocketNotifier>(g_signal_fds[0], QSocketNotifier::Read);
  QObject::connect(notifier.get(), &QSocketNotifier::activated, notifier.get(), [n = notifier.get()] {
    n->setEnabled(false);
    char byte = 0;
    (void)!::read(g_signal_fds[0], &byte, 1);
    std::cerr << "[nve-qt-shell] terminating on signal\n";
    QApplication::quit();
  });
  struct sigaction sa {};
  sa.sa_handler = &on_termination_signal;
  sigemptyset(&sa.sa_mask);
  sa.sa_flags = SA_RESTART;
  ::sigaction(SIGINT, &sa, nullptr);
  ::sigaction(SIGTERM, &sa, nullptr);
  return notifier;
}
#endif

/// Starts the requested app source; on a pairing or load failure runs the built-in demo with
/// persistence disabled (the fallback must neither restore nor overwrite another app's state).
void start_app(QtHost& host, LaunchOptions& o, const std::function<void(const std::string&)>& set_title) {
  const auto builtin = [&] { return host.attach_interface(&nve::host::demo::CounterApp::get_interface); };
  bool loaded = false;  // the requested source runs (not the built-in fallback)
  if (!o.pair_uri.empty()) {
    std::string error;
    loaded = host.connect_paired(o.pair_uri, o.pair_token, &error);
    if (loaded) {
      set_title("paired: " + o.pair_uri);
    } else {
      std::cerr << "[nve-qt-shell] pairing with '" << o.pair_uri << "' failed: " << error << "\n";
    }
  } else if (!o.dylib_path.empty()) {
    nve::host::ModuleError code = nve::host::ModuleError::None;
    std::string error;
    loaded = host.load_dylib(o.dylib_path, &code, &error);
    if (!loaded) {
      std::cerr << "[nve-qt-shell] loading '" << o.dylib_path << "' failed (" << static_cast<int>(code)
                << "): " << error << "\n";
    }
  } else {
    loaded = builtin();
  }
  if (!loaded) (void)builtin();
  switch (nve::host::state_action_after_load(o, loaded)) {
    case nve::host::StateAction::Clear:
      if (!host.clear_state(o.state_path)) {
        std::cerr << "[nve-qt-shell] clearing " << o.state_path << " failed: " << host.last_error() << "\n";
      }
      break;
    case nve::host::StateAction::Restore:
      if (host.restore_state(o.state_path, o.schema_version)) {
        std::cerr << "[nve-qt-shell] restored app state from " << o.state_path << "\n";
      }
      break;
    case nve::host::StateAction::None: break;
  }
}

/// Runs the event loop for `host` shown in `window`, persisting state on close, signal, shutdown.
int run(QApplication& qapp, QtHost& host, QObject& window, LaunchOptions& o) {
  StatePersister persister(host, o);
  window.installEventFilter(&persister);
#if defined(__unix__) || defined(__APPLE__)
  const std::unique_ptr<QSocketNotifier> signal_notifier = install_signal_handlers();
#endif
  const int status = qapp.exec();
  persister.persist_unless_saved("shutdown");
  window.removeEventFilter(&persister);
  return status;
}

int run_widgets(QApplication& qapp, LaunchOptions& o) {
  const QString base = QStringLiteral("NVE Qt 6 Host Shell");
  nve::qt::NveQtWidget window(
      nve::qt::QtHostConfig{.width = static_cast<float>(o.width), .height = static_cast<float>(o.height)});
  window.setWindowTitle(base);
  start_app(window.host(), o, [&](const std::string& suffix) {
    window.setWindowTitle(base + QStringLiteral(" (") + QString::fromStdString(suffix) + QStringLiteral(")"));
  });
  window.show();
  return run(qapp, window.host(), window, o);
}

#ifdef NVE_QT_HAS_QUICK
int run_quick(QApplication& qapp, LaunchOptions& o) {
  nve::qt::NveQtQuickItem::registerQmlType();
  const QString base = QStringLiteral("NVE Qt Quick Host Shell");
  QQuickView view;
  view.setTitle(base);
  view.setResizeMode(QQuickView::SizeRootObjectToView);
  view.resize(o.width, o.height);
  QQmlComponent component(view.engine());
  component.setData(QByteArrayLiteral("import Nve 1.0\nNveSurface {}\n"), QUrl());
  std::unique_ptr<QObject> root(component.create(view.rootContext()));
  auto* surface = qobject_cast<nve::qt::NveQtQuickItem*>(root.get());
  if (surface == nullptr) {
    std::cerr << "[nve-qt-shell] creating the QML NveSurface failed: " << component.errorString().toStdString() << "\n";
    return 1;
  }
  view.setContent(QUrl(), &component, root.release());
  surface->setSize(QSizeF(o.width, o.height));
  start_app(surface->host(), o, [&](const std::string& suffix) {
    view.setTitle(base + QStringLiteral(" (") + QString::fromStdString(suffix) + QStringLiteral(")"));
  });
  view.show();
  return run(qapp, surface->host(), view, o);
}
#endif

void print_usage(const char* argv0) {
  std::cout << "Usage: " << argv0 << " [--quick] " << nve::host::kLaunchUsage << "\n" << nve::host::kLaunchEnvUsage
            << "\n";
}

}  // namespace

int main(int argc, char* argv[]) {
  QApplication qapp(argc, argv);  // consumes Qt's own arguments first
  LaunchOptions o;
  std::string error;
  const auto quick_flag = [&o](std::string_view flag, const char*) {
    if (flag != "--quick") return 0;
    o.quick = true;
    return 1;
  };
  switch (nve::host::parse_launch_args(argc, argv, o, &error, quick_flag)) {
    case nve::host::LaunchParse::Help: print_usage(argv[0]); return 0;
    case nve::host::LaunchParse::Error:
      std::cerr << "[nve-qt-shell] " << error << "\n";
      print_usage(argv[0]);
      return 2;
    case nve::host::LaunchParse::Run: break;
  }
  nve::host::resolve_launch_state(o, "nve-qt-shell-builtin");

  if (o.quick) {
#ifdef NVE_QT_HAS_QUICK
    return run_quick(qapp, o);
#else
    std::cerr << "[nve-qt-shell] --quick needs a build with Qt6::Quick\n";
    return 1;
#endif
  }
  return run_widgets(qapp, o);
}
