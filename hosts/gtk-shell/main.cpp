// nve-gtk-shell: runs an NVE app (built-in counter demo, a `cdylib`, or a paired `nve dev`
// server) in a native GTK4 window, persisting the app's state across launches.
#include "NveGtkHost.hpp"

#include <nve/host/demo_counter_app.hpp>
#include <nve/host/launch_options.hpp>

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

struct LaunchState : nve::host::LaunchOptions {
  GtkApplication* app = nullptr;
  std::unique_ptr<NveGtkHost> host;
};

void persist_state(LaunchState* s, const char* trigger) {
  if (s->no_state || s->host == nullptr || s->host->is_paired()) return;
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

bool attach_builtin(NveGtkHost& host) { return host.attach_interface(&nve::host::demo::CounterApp::get_interface); }

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

  bool loaded = false;  // the requested source runs (not the built-in fallback)
  if (!s->pair_uri.empty()) {
    std::string error;
    loaded = s->host->connect_paired(s->pair_uri, s->pair_token, &error);
    if (loaded) {
      const std::string title = "NVE GTK4 Host Shell (paired: " + s->pair_uri + ")";
      gtk_window_set_title(GTK_WINDOW(window), title.c_str());
    } else {
      std::cerr << "[nve-gtk-shell] pairing with '" << s->pair_uri << "' failed: " << error << "\n";
    }
  } else if (!s->dylib_path.empty()) {
    nve::host::ModuleError code = nve::host::ModuleError::None;
    std::string error;
    loaded = s->host->load_dylib(s->dylib_path, &code, &error);
    if (!loaded) {
      std::cerr << "[nve-gtk-shell] loading '" << s->dylib_path << "' failed (" << static_cast<int>(code)
                << "): " << error << "\n";
    }
  } else {
    loaded = attach_builtin(*s->host);
  }
  if (!loaded) (void)attach_builtin(*s->host);

  switch (nve::host::state_action_after_load(*s, loaded)) {
    case nve::host::StateAction::Clear:
      if (!s->host->clear_state(s->state_path)) {
        std::cerr << "[nve-gtk-shell] clearing " << s->state_path << " failed: " << s->host->last_error() << "\n";
      }
      break;
    case nve::host::StateAction::Restore:
      if (s->host->restore_state(s->state_path, s->schema_version)) {
        std::cerr << "[nve-gtk-shell] restored app state from " << s->state_path << "\n";
      }
      break;
    case nve::host::StateAction::None: break;
  }
  gtk_window_present(GTK_WINDOW(window));
}

void print_usage(const char* argv0) {
  std::cout << "Usage: " << argv0 << " " << nve::host::kLaunchUsage << "\n" << nve::host::kLaunchEnvUsage << "\n";
}

}  // namespace

int main(int argc, char* argv[]) {
  LaunchState s;
  std::string error;
  switch (nve::host::parse_launch_args(argc, argv, s, &error)) {
    case nve::host::LaunchParse::Help: print_usage(argv[0]); return 0;
    case nve::host::LaunchParse::Error:
      std::cerr << "[nve-gtk-shell] " << error << "\n";
      print_usage(argv[0]);
      return 2;
    case nve::host::LaunchParse::Run: break;
  }
  nve::host::resolve_launch_state(s, "nve-gtk-shell-builtin");

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
