#pragma once

#include "NveGtkHost.hpp"

#include <nve/host/testing/scripted_app.hpp>

#include <cmath>
#include <cstdlib>
#include <iostream>
#include <string>

#define NVE_CHECK(expr)                                                                         \
  do {                                                                                          \
    if (!(expr)) {                                                                              \
      std::cerr << "NVE_CHECK failed at " << __FILE__ << ":" << __LINE__ << ": " #expr << "\n"; \
      std::abort();                                                                             \
    }                                                                                           \
  } while (0)

namespace nve::gtk::test {

using host::testing::ScriptedApp;

inline bool approx(double a, double b, double eps = 1.5) { return std::fabs(a - b) <= eps; }

inline std::string label_text(GtkWidget* w) {
  return w != nullptr && GTK_IS_LABEL(w) ? std::string(gtk_label_get_text(GTK_LABEL(w))) : std::string();
}

inline int request_width(GtkWidget* w) {
  int width = -1;
  gtk_widget_get_size_request(w, &width, nullptr);
  return width;
}

inline int request_height(GtkWidget* w) {
  int height = -1;
  gtk_widget_get_size_request(w, nullptr, &height);
  return height;
}

/// Position of a `GtkFixed` child (its translation transform).
inline std::pair<float, float> child_position(GtkWidget* fixed, GtkWidget* child) {
  float x = 0.0f;
  float y = 0.0f;
  gsk_transform_to_translate(gtk_fixed_get_child_transform(GTK_FIXED(fixed), child), &x, &y);
  return {x, y};
}

/// Shows `host` in a toplevel and spins the main loop until it is allocated (bounded).
inline GtkWidget* present(NveGtkHost& host, int width, int height) {
  GtkWidget* window = gtk_window_new();
  gtk_window_set_default_size(GTK_WINDOW(window), width, height);
  gtk_window_set_child(GTK_WINDOW(window), host.host_widget());
  gtk_window_present(GTK_WINDOW(window));
  for (int i = 0; i < 400 && gtk_widget_get_width(host.host_widget()) <= 0; ++i) {
    g_main_context_iteration(nullptr, FALSE);
  }
  return window;
}

inline void spin(int iterations = 50) {
  for (int i = 0; i < iterations; ++i) g_main_context_iteration(nullptr, FALSE);
}

/// Renders the `ScriptedApp`'s pending ops and mirrors them into widgets.
inline void flush(NveGtkHost& host) {
  (void)host.session().dispatch(ScriptedApp::flush_event(host.surface_id()));
  (void)host.pump();
}

/// Text node helper for scripted scenes.
inline void text_node(host::Nve2Writer& w, uint32_t id, std::string_view text) {
  w.create(id);
  (void)w.set_str(id, NVE_TABLE_TEXT_DESC, NVE_TABLE_TEXT_TEXT, text);
}

}  // namespace nve::gtk::test
