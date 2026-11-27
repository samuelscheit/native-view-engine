// Core GTK host suite: the built-in counter app mirrored into widgets, Pango measurement feeding
// layout, O(changed) reconciliation, press dispatch, outbound events, props, resize, `cdylib`
// loading errors, state-preserving hot reload, app-state files, and (opt-in) the live Rust
// kitchen-sink `cdylib`.
#include "gtk_test_helpers.hpp"
#include "NveGtkInternal.hpp"

#include <nve/host/demo_counter_app.hpp>

#include <filesystem>
#include <vector>

namespace {

using namespace nve::gtk;
using namespace nve::gtk::test;
using nve::host::demo::CounterApp;

void test_counter_widgets_measurement_and_events() {
  NveGtkHost host(GtkHostConfig{.surface_id = 7u, .width = 480.0f, .height = 360.0f});
  NVE_CHECK(!host.is_loaded() && host.view_count() == 0u);
  NVE_CHECK(host.root_widget() != nullptr && GTK_IS_FIXED(host.root_widget()));
  NVE_CHECK(GTK_IS_SCROLLED_WINDOW(host.host_widget()));

  // Pango measurement: monotonic in size and weight, wraps under a width constraint.
  const NveIntrinsicSize small = host.measure_text("Hello NVE", 16.0f);
  const NveIntrinsicSize big = host.measure_text("Hello NVE", 32.0f);
  const NveIntrinsicSize bold = host.measure_text("Hello NVE", 16.0f, 800u);
  const NveIntrinsicSize wrapped = host.measure_text("Hello NVE wrapping text", 16.0f, 400u, 40.0f);
  NVE_CHECK(small.width > 0.0f && small.height > 0.0f);
  NVE_CHECK(big.width > small.width * 1.5f && big.height > small.height);
  NVE_CHECK(bold.width >= small.width);
  NVE_CHECK(wrapped.width <= 41.0f && wrapped.height > small.height * 1.5f);
  NVE_CHECK(host.measure_text("", 16.0f).width == 0.0f);

  std::vector<int64_t> counts;
  host.set_event_callback([&counts](std::span<const NveDirectEvent> evs) {
    for (const NveDirectEvent& ev : evs) {
      if (ev.kind == NVE_EVENT_CUSTOM && ev.value.tag == NVE_VAL_I64) counts.push_back(ev.value.data.i64_val);
    }
  });

  NVE_CHECK(host.attach_interface(&CounterApp::get_interface));
  NVE_CHECK(host.is_loaded() && host.view_count() == 5u);
  GtkWidget* column = host.widget_for_node_id(CounterApp::kColumn);
  GtkWidget* title = host.widget_for_node_id(CounterApp::kTitle);
  GtkWidget* inc = host.widget_for_node_id(CounterApp::kIncrement);
  GtkWidget* count = host.widget_for_node_id(CounterApp::kCount);
  NVE_CHECK(column != nullptr && GTK_IS_FIXED(column));
  NVE_CHECK(gtk_widget_get_parent(column) == host.root_widget());
  NVE_CHECK(gtk_widget_get_parent(title) == column && gtk_widget_get_parent(count) == column);
  NVE_CHECK(label_text(title) == "NVE native host" && label_text(count) == "Count: 0");
  NVE_CHECK(host.widget_for_node_id(1u) == host.root_widget());
  NVE_CHECK(host.widget_for_node_id(99u) == nullptr);

  // Children in scene order (paint order).
  std::vector<GtkWidget*> order;
  for (GtkWidget* c = gtk_widget_get_first_child(column); c != nullptr; c = gtk_widget_get_next_sibling(c)) {
    order.push_back(c);
  }
  NVE_CHECK((order == std::vector<GtkWidget*>{title, inc, count, host.widget_for_node_id(CounterApp::kReset)}));

  // The runtime laid out text with Pango: the title's width is its measured width.
  const NveIntrinsicSize title_size = host.measure_text("NVE native host", 22.0f);
  NVE_CHECK(approx(request_width(title), title_size.width, 2.0));
  // Column fills the surface; its padding offsets the first child.
  NVE_CHECK(request_width(column) == 480);
  const auto [tx, ty] = child_position(column, title);
  NVE_CHECK(approx(tx, 24.0, 0.5) && approx(ty, 24.0, 0.5));
  const auto [cx, cy] = child_position(column, count);
  NVE_CHECK(approx(cx, 24.0, 0.5) && cy > ty + 40.0f);  // below title + button

  // Styles: button background + inherited white text; a11y roles.
  NVE_CHECK(host.active_css().find("rgba(37, 99, 235, 1)") != std::string::npos);
  NVE_CHECK(gtk_accessible_get_accessible_role(GTK_ACCESSIBLE(inc)) == GTK_ACCESSIBLE_ROLE_BUTTON);
  NVE_CHECK(gtk_accessible_get_accessible_role(GTK_ACCESSIBLE(title)) == GTK_ACCESSIBLE_ROLE_HEADING);
  NVE_CHECK(gtk_widget_get_can_target(inc) && !gtk_widget_get_can_target(count));  // text buttons take clicks
  const auto inc_sem = host.accessibility_info_for_node_id(CounterApp::kIncrement);
  NVE_CHECK(inc_sem && inc_sem->label == "Increment" && inc_sem->role == GTK_ACCESSIBLE_ROLE_BUTTON);

  // Press -> app -> new frame: only the changed label is reconciled.
  const uint64_t before = host.reconcile_update_count();
  NVE_CHECK(host.simulate_press(CounterApp::kIncrement, 4.0f, 4.0f));
  NVE_CHECK(label_text(count) == "Count: 1");
  NVE_CHECK(host.widget_for_node_id(CounterApp::kCount) == count);  // retained, not recreated
  const uint64_t delta = host.reconcile_update_count() - before;
  NVE_CHECK(delta >= 1u && delta <= 3u);
  NVE_CHECK(host.simulate_press(CounterApp::kIncrement));
  NVE_CHECK(!host.simulate_press(CounterApp::kTitle));  // not subscribed: no change
  NVE_CHECK(label_text(count) == "Count: 2");
  NVE_CHECK((counts == std::vector<int64_t>{1, 2}));
  NVE_CHECK(host.pump() == 0u);  // idle frame: nothing to reconcile

  // Component props.
  const std::string retitle = "Retitled";
  NveMapEntry prop{};
  prop.key = NveStrSlice{reinterpret_cast<const uint8_t*>("title"), 5u};
  prop.value.tag = NVE_VAL_STR;
  prop.value.data.str_val = NveStrSlice{reinterpret_cast<const uint8_t*>(retitle.data()), retitle.size()};
  NVE_CHECK(host.apply_props(std::span<const NveMapEntry>(&prop, 1u)));
  NVE_CHECK(label_text(title) == "Retitled");
  // The demo has no templates: a hot patch is a no-op, not a failure mode.
  const uint8_t bogus_patch[] = {1, 2, 3};
  NVE_CHECK(!host.apply_hot_patch(bogus_patch));
  NVE_CHECK(label_text(count) == "Count: 2");

  // Resize relayouts retained widgets in place.
  host.set_surface_size(600.0f, 300.0f, 2.0f);
  NVE_CHECK(request_width(column) == 600 && host.widget_for_node_id(CounterApp::kColumn) == column);
}

void test_modules_hot_reload_and_state() {
  NveGtkHost host;
  nve::host::ModuleError err = nve::host::ModuleError::None;
  std::string msg;
  NVE_CHECK(!host.load_dylib("", &err, &msg) && err == nve::host::ModuleError::Open);
  NVE_CHECK(!host.load_dylib("/nonexistent/libnope.so", &err, &msg) && err == nve::host::ModuleError::Open);
  NVE_CHECK(!host.load_dylib(NVE_TEST_NO_ENTRY_MODULE, &err, &msg) && err == nve::host::ModuleError::MissingEntry);
  NVE_CHECK(!host.load_dylib(NVE_TEST_NO_SURFACE_MODULE, &err, &msg) && err == nve::host::ModuleError::MissingSurface);
  NVE_CHECK(!host.load_dylib(NVE_TEST_PARTIAL_SURFACE_MODULE, &err, &msg) &&
            err == nve::host::ModuleError::MissingSurface);
  NVE_CHECK(!host.is_loaded() && host.view_count() == 0u);

  // The dev loop: one app path, rebuilt in place between reloads.
  const std::filesystem::path dir = std::filesystem::temp_directory_path() / "nve_gtk_app_files_test";
  std::filesystem::remove_all(dir);
  std::filesystem::create_directories(dir);
  const std::string app_path =
      (dir / ("app" + std::filesystem::path(NVE_TEST_COUNTER_MODULE).extension().string())).string();
  const auto build = [&](const char* fixture) {
    std::filesystem::copy_file(fixture, app_path, std::filesystem::copy_options::overwrite_existing);
  };
  build(NVE_TEST_COUNTER_MODULE);
  NVE_CHECK(host.load_dylib(app_path, &err, &msg));
  NVE_CHECK(host.is_loaded() && host.view_count() == 5u);
  NVE_CHECK(host.simulate_press(CounterApp::kIncrement));
  NVE_CHECK(host.simulate_press(CounterApp::kIncrement));
  NVE_CHECK(label_text(host.widget_for_node_id(CounterApp::kCount)) == "Count: 2");

  // A failed reload keeps the running app.
  NVE_CHECK(!host.reload_dylib(NVE_TEST_NO_SURFACE_MODULE, &err, &msg));
  NVE_CHECK(host.simulate_press(CounterApp::kIncrement));
  NVE_CHECK(label_text(host.widget_for_node_id(CounterApp::kCount)) == "Count: 3");
  // Reloading the rebuilt file runs its NEW code (a press counts 2) and carries the state.
  GtkWidget* const count_label = host.widget_for_node_id(CounterApp::kCount);
  build(NVE_TEST_COUNTER_DOUBLE_MODULE);
  NVE_CHECK(host.reload_dylib(app_path, &err, &msg));
  NVE_CHECK(label_text(host.widget_for_node_id(CounterApp::kCount)) == "Count: 3");
  NVE_CHECK(host.widget_for_node_id(CounterApp::kCount) == count_label);  // widgets survive the reload
  NVE_CHECK(host.simulate_press(CounterApp::kIncrement));
  NVE_CHECK(label_text(host.widget_for_node_id(CounterApp::kCount)) == "Count: 5");
  NVE_CHECK(host.view_count() == 5u);

  // App-state file (runtime `StateFile`): save, mutate, restore; other schema / missing refuse;
  // clearing is a cold start.
  const std::string path = (dir / "state.nvestate").string();
  NVE_CHECK(host.save_state(path, 4u));
  NVE_CHECK(host.simulate_press(CounterApp::kReset));
  NVE_CHECK(label_text(host.widget_for_node_id(CounterApp::kCount)) == "Count: 0");
  NVE_CHECK(!host.restore_state(path, 3u));  // newer file: kept
  NVE_CHECK(!host.restore_state(path + ".missing", 4u));
  NVE_CHECK(host.restore_state(path, 4u));
  NVE_CHECK(label_text(host.widget_for_node_id(CounterApp::kCount)) == "Count: 5");
  NVE_CHECK(host.clear_state(path) && !std::filesystem::exists(path) && !host.restore_state(path, 4u));
  std::filesystem::remove_all(dir);

  // Switching apps drops the previous app's views.
  ScriptedApp app;
  app.w.create(1u);
  text_node(app.w, 2u, "scripted");
  const uint32_t kids[] = {2u};
  app.w.set_children(1u, kids);
  NVE_CHECK(host.attach_interface(&ScriptedApp::get_interface));
  NVE_CHECK(host.view_count() == 1u && label_text(host.widget_for_node_id(2u)) == "scripted");
  NVE_CHECK(host.widget_for_node_id(CounterApp::kCount) == nullptr);
}

/// Live Rust kitchen-sink `cdylib` (opt-in via `NVE_TEST_KITCHEN_SINK_DYLIB`): mirrors the real
/// app into widgets, measures every label, presses its counter buttons, and hot-reloads in place
/// keeping state.
void test_live_kitchen_sink_if_configured() {
  const char* path = std::getenv("NVE_TEST_KITCHEN_SINK_DYLIB");
  if (path == nullptr || *path == '\0') return;
  NveGtkHost host(GtkHostConfig{.surface_id = 1u, .width = 1280.0f, .height = 800.0f});
  nve::host::ModuleError err = nve::host::ModuleError::None;
  std::string msg;
  NVE_CHECK(host.load_dylib(path, &err, &msg) && err == nve::host::ModuleError::None);
  NVE_CHECK(host.is_loaded() && host.view_count() > 10u);

  std::vector<GtkWidget*> labels;
  const auto collect = [&](auto& self, GtkWidget* w) -> void {
    if (GTK_IS_LABEL(w)) labels.push_back(w);
    for (GtkWidget* c = gtk_widget_get_first_child(w); c != nullptr; c = gtk_widget_get_next_sibling(c)) self(self, c);
  };
  const auto find_label = [&](std::string_view text) -> GtkWidget* {
    labels.clear();
    collect(collect, host.root_widget());
    for (GtkWidget* l : labels) {
      if (label_text(l) == text) return l;
    }
    return nullptr;
  };
  // The nearest ancestor view (or the label itself) subscribed to presses.
  const auto pressable = [](GtkWidget* w) -> uint32_t {
    for (; w != nullptr; w = gtk_widget_get_parent(w)) {
      const auto* v = static_cast<const GtkNodeView*>(g_object_get_data(G_OBJECT(w), kViewKey));
      if (v != nullptr && v->wants_press) return v->id;
    }
    return 0u;
  };

  NVE_CHECK(find_label("NVE Kitchen Sink") != nullptr && find_label("Count: 0") != nullptr);
  size_t measured = 0u;
  for (GtkWidget* l : labels) {
    // Our text views only (not GTK-internal labels such as an entry's placeholder).
    if (g_object_get_data(G_OBJECT(l), kViewKey) == nullptr || !gtk_widget_get_visible(l) ||
        label_text(l).empty()) {
      continue;
    }
    NVE_CHECK(request_width(l) > 0 && request_height(l) >= 10);  // Pango-measured layout
    ++measured;
  }
  NVE_CHECK(measured > 10u);
  const uint32_t inc = pressable(find_label("Increment +"));
  const uint32_t reset = pressable(find_label("Reset"));
  NVE_CHECK(inc != 0u && reset != 0u && inc != reset);
  NVE_CHECK(host.simulate_press(inc) && find_label("Count: 1") != nullptr);
  NVE_CHECK(host.simulate_press(inc) && find_label("Count: 2") != nullptr);
  NVE_CHECK(host.simulate_press(reset) && find_label("Count: 0") != nullptr);
  for (int i = 0; i < 3; ++i) NVE_CHECK(host.simulate_press(inc));
  NVE_CHECK(find_label("Count: 3") != nullptr);

  const size_t views = host.view_count();
  NVE_CHECK(host.reload_dylib(path, &err, &msg));
  NVE_CHECK(find_label("Count: 3") != nullptr && host.view_count() == views);
}

}  // namespace

void nve_test_gtk_a11y_and_media();
void nve_test_gtk_canvas_scroll_and_pair();
void nve_test_gtk_window_manager();
void nve_test_gtk_reconcile();

int main() {
  gtk_init();
  test_counter_widgets_measurement_and_events();
  test_modules_hot_reload_and_state();
  test_live_kitchen_sink_if_configured();
  nve_test_gtk_a11y_and_media();
  nve_test_gtk_canvas_scroll_and_pair();
  nve_test_gtk_window_manager();
  nve_test_gtk_reconcile();
  std::cout << "nve_gtk_shell_test: ok\n";
  return 0;
}
