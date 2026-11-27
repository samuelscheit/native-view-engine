// nve-gtk-shell: runs an NVE app (built-in counter demo, a `cdylib`, or a paired `nve dev`
// server) in a native GTK4 window, persisting the app's state across launches.
#include "NveGtkHost.hpp"

#include <nve/host/app_state.hpp>
#include <nve/host/demo_counter_app.hpp>

#if defined(__unix__) || defined(__APPLE__)
#include <glib-unix.h>
#include <csignal>
#endif

#include <cstdlib>
#include <cstring>
#include <iostream>
#include <memory>
#include <string>

namespace {

using nve::gtk::NveGtkHost;
namespace app_state = nve::host::app_state;

struct LaunchState {
  std::string dylib_path;
  std::string pair_uri;
  std::string pair_token;
  std::string state_path;
  uint32_t schema_version = 1u;
  bool no_state = false;
  bool reset_state = false;
  int width = 640;
  int height = 420;
  GtkApplication* app = nullptr;
  std::unique_ptr<NveGtkHost> host;
};

void persist_state(LaunchState* s, const char* trigger) {
  if (s->no_state || s->state_path.empty() || s->host == nullptr || s->host->is_paired()) return;
  if (s->host->save_state(s->state_path, s->schema_version)) {
    std::cerr << "[nve-gtk-shell] saved app state to " << s->state_path << " (" << trigger << ")\n";
  }
}

gboolean on_close_request(GtkWindow*, gpointer data) {
  persist_state(static_cast<LaunchState*>(data), "window-close");
  return FALSE;
}

#if defined(__unix__) || defined(__APPLE__)
gboolean on_termination_signal(gpointer data) {
  auto* s = static_cast<LaunchState*>(data);
  persist_state(s, "signal");
  g_application_quit(G_APPLICATION(s->app));
  return G_SOURCE_REMOVE;
}
#endif

void attach_builtin(NveGtkHost& host) { (void)host.attach_interface(&nve::host::demo::CounterApp::get_interface); }

void on_activate(GtkApplication* app, gpointer data) {
  auto* s = static_cast<LaunchState*>(data);
  GtkWidget* window = gtk_application_window_new(app);
  gtk_window_set_title(GTK_WINDOW(window), "NVE GTK4 Host Shell");
  gtk_window_set_default_size(GTK_WINDOW(window), s->width, s->height);
  s->host = std::make_unique<NveGtkHost>(nve::gtk::GtkHostConfig{
      .surface_id = 1u,
      .width = static_cast<float>(s->width),
      .height = static_cast<float>(s->height),
      .scale_factor = 1.0f,
  });
  s->host->attach_window_resize(GTK_WINDOW(window));
  g_signal_connect(window, "close-request", nve::gtk::gtk_detail::to_gcallback(&on_close_request), s);
  gtk_window_set_child(GTK_WINDOW(window), s->host->host_widget());

  if (!s->pair_uri.empty()) {
    std::string error;
    if (s->host->connect_paired(s->pair_uri, s->pair_token, &error)) {
      const std::string title = "NVE GTK4 Host Shell (paired: " + s->pair_uri + ")";
      gtk_window_set_title(GTK_WINDOW(window), title.c_str());
    } else {
      std::cerr << "[nve-gtk-shell] pairing with '" << s->pair_uri << "' failed: " << error << "\n";
      attach_builtin(*s->host);
      s->no_state = true;  // the fallback app must neither restore nor overwrite another app's state
    }
  } else if (!s->dylib_path.empty()) {
    nve::host::ModuleError code = nve::host::ModuleError::None;
    std::string error;
    if (!s->host->load_dylib(s->dylib_path, &code, &error)) {
      std::cerr << "[nve-gtk-shell] loading '" << s->dylib_path << "' failed (" << static_cast<int>(code)
                << "): " << error << "\n";
      attach_builtin(*s->host);
      s->no_state = true;  // the fallback app must neither restore nor overwrite the dylib's state
    }
  } else {
    attach_builtin(*s->host);
  }

  if (s->no_state) {
    // Nothing to restore or clear.
  } else if (s->reset_state) {
    if (!s->host->clear_state(s->state_path)) {
      std::cerr << "[nve-gtk-shell] clearing " << s->state_path << " failed: " << s->host->last_error() << "\n";
    }
  } else if (s->host->restore_state(s->state_path, s->schema_version)) {
    std::cerr << "[nve-gtk-shell] restored app state from " << s->state_path << "\n";
  }
  gtk_window_present(GTK_WINDOW(window));
}

void print_usage(const char* argv0) {
  std::cout << "Usage: " << argv0
            << " [--dylib <libapp.so|.dylib>] [--pair <nve://|quic://|wt://|ws(s)://...>] [--pair-token <token>]"
               " [--state <file>] [--no-state] [--reset-state] [--schema <ver>] [--width W] [--height H]\n"
               "Env: NVE_STATE_DISABLE=1, NVE_STATE_PATH=<file>, NVE_RESET_STATE=1\n";
}

}  // namespace

int main(int argc, char* argv[]) {
  LaunchState s;
  for (int i = 1; i < argc; ++i) {
    const auto is = [&](const char* flag) { return std::strcmp(argv[i], flag) == 0; };
    const bool has_value = i + 1 < argc;
    if (is("--help") || is("-h")) {
      print_usage(argv[0]);
      return 0;
    } else if (is("--no-state")) {
      s.no_state = true;
    } else if (is("--reset-state")) {
      s.reset_state = true;
    } else if (is("--dylib") && has_value) {
      s.dylib_path = argv[++i];
    } else if (is("--pair") && has_value) {
      s.pair_uri = argv[++i];
    } else if (is("--pair-token") && has_value) {
      s.pair_token = argv[++i];
    } else if (is("--state") && has_value) {
      s.state_path = argv[++i];
    } else if (is("--schema") && has_value) {
      s.schema_version = static_cast<uint32_t>(std::strtoul(argv[++i], nullptr, 10));
    } else if (is("--width") && has_value) {
      s.width = std::max(1, std::atoi(argv[++i]));
    } else if (is("--height") && has_value) {
      s.height = std::max(1, std::atoi(argv[++i]));
    } else {
      std::cerr << "[nve-gtk-shell] unknown argument '" << argv[i] << "'\n";
      print_usage(argv[0]);
      return 2;
    }
  }
  if (!s.no_state && s.state_path.empty() && s.pair_uri.empty()) {
    s.state_path = app_state::resolve_path(s.dylib_path.empty() ? "nve-gtk-shell-builtin" : s.dylib_path);
  }
  s.no_state = s.no_state || s.state_path.empty();
  s.reset_state = s.reset_state || app_state::reset_requested();

  GtkApplication* app = gtk_application_new("dev.nve.GtkShell", G_APPLICATION_NON_UNIQUE);
  s.app = app;
  g_signal_connect(app, "activate", nve::gtk::gtk_detail::to_gcallback(&on_activate), &s);
#if defined(__unix__) || defined(__APPLE__)
  const guint sigint_id = g_unix_signal_add(SIGINT, &on_termination_signal, &s);
  const guint sigterm_id = g_unix_signal_add(SIGTERM, &on_termination_signal, &s);
#endif
  const int status = g_application_run(G_APPLICATION(app), 0, nullptr);
#if defined(__unix__) || defined(__APPLE__)
  for (const guint id : {sigint_id, sigterm_id}) {  // a fired handler already removed itself
    if (g_main_context_find_source_by_id(nullptr, id) != nullptr) g_source_remove(id);
  }
#endif
  persist_state(&s, "shutdown");
  s.host.reset();
  g_object_unref(app);
  return status;
}
