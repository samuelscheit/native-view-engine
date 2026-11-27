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

/// The measurer sees the node's whole typography: family, style, letter spacing, line height,
/// white-space, and max lines change Pango's answer exactly as they change the painted label.
void test_measure_typography(const NveGtkHost& host) {
  using nve::host::str_slice;
  using nve::host::text_style;
  const auto with = [](auto edit) {
    NveRuntimeTextStyle s = text_style(16.0f);
    edit(s);
    return s;
  };
  constexpr std::string_view kNarrow = "iiiiiiiiiiiiiiii";
  const NveRuntimeTextStyle sans = with([](NveRuntimeTextStyle& s) { s.font_family = str_slice("sans-serif"); });
  const NveRuntimeTextStyle mono = with([](NveRuntimeTextStyle& s) { s.font_family = str_slice("monospace"); });
  // Monospace gives `i` a full cell: far wider than a proportional `i`.
  NVE_CHECK(host.measure_text(kNarrow, mono).width > host.measure_text(kNarrow, sans).width * 1.5f);
  // Painter and measurer share one normalized list: entries trimmed of spaces / quotes, unsafe
  // bytes dropped, empties skipped.
  NVE_CHECK(gtk_internal::font_family_list(" 'Fira Code' , \"x;}\" ,, monospace ") == "Fira Code,x,monospace");
  NVE_CHECK(gtk_internal::font_family_list("").empty() && gtk_internal::font_family_list(" , ").empty());
  // A family list falls back past unknown names exactly like the label's CSS.
  const NveRuntimeTextStyle listed =
      with([](NveRuntimeTextStyle& s) { s.font_family = str_slice("NoSuchFontNve, monospace"); });
  NVE_CHECK(approx(host.measure_text(kNarrow, listed).width, host.measure_text(kNarrow, mono).width, 0.5));

  const std::string_view sentence = "Hello wrapping text that is long";
  const NveIntrinsicSize one_line = host.measure_text(sentence, sans);
  // `nowrap` / `pre` ignore the width (no extra lines); `normal` / `pre-wrap` wrap.
  for (const uint8_t ws : {uint8_t{NVE_WHITE_SPACE_NO_WRAP}, uint8_t{NVE_WHITE_SPACE_PRE}}) {
    NveRuntimeTextStyle s = sans;
    s.white_space = ws;
    const NveIntrinsicSize m = host.measure_text(sentence, s, 60.0f);
    NVE_CHECK(m.width == one_line.width && m.height == one_line.height);
    NVE_CHECK(host.measure_text(sentence, s, 0.0f).width == one_line.width);  // min-content too
  }
  const NveIntrinsicSize wrapped = host.measure_text(sentence, sans, 60.0f);
  NVE_CHECK(wrapped.width <= 61.0f && wrapped.height > one_line.height * 2.0f);
  const NveIntrinsicSize min_content = host.measure_text(sentence, sans, 0.0f);
  NVE_CHECK(min_content.width > 0.0f && min_content.width < one_line.width && min_content.height > wrapped.height);

  // Letter spacing widens by about one spacing per character.
  const NveRuntimeTextStyle spaced = with([](NveRuntimeTextStyle& s) { s.letter_spacing = 3.0f; });
  NVE_CHECK(approx(host.measure_text(kNarrow, spaced).width - host.measure_text(kNarrow, text_style(16.0f)).width,
                   3.0 * kNarrow.size(), 4.0));
  // Line height: points and percent (a multiplier) set the line box; `max_lines` caps it.
  const NveRuntimeTextStyle tall = with([](NveRuntimeTextStyle& s) {
    s.line_height = 40.0f;
    s.line_height_unit = NVE_LINE_HEIGHT_POINTS;
  });
  const NveRuntimeTextStyle tall_pct = with([](NveRuntimeTextStyle& s) {
    s.line_height = 250.0f;
    s.line_height_unit = NVE_LINE_HEIGHT_PERCENT;
  });
  NVE_CHECK(approx(host.measure_text("a\nb", tall).height, 80.0, 2.0));
  NVE_CHECK(approx(host.measure_text("a\nb", tall_pct).height, 80.0, 2.0));
  const NveRuntimeTextStyle two_lines = with([](NveRuntimeTextStyle& s) {
    s.line_height = 40.0f;
    s.line_height_unit = NVE_LINE_HEIGHT_POINTS;
    s.max_lines = 2u;
  });
  NVE_CHECK(approx(host.measure_text(sentence, two_lines, 60.0f).height, 80.0, 2.0));
  NVE_CHECK(host.measure_text(sentence, two_lines, 60.0f).width <= 61.0f);
  // Italic shapes the same text without collapsing it.
  NveRuntimeTextStyle italic = sans;
  italic.font_style = NVE_FONT_STYLE_ITALIC;
  NVE_CHECK(host.measure_text(sentence, italic).width > one_line.width * 0.8f);
}

/// GTK's own natural size of `label` (its CSS, wrap, lines, ellipsize) at `for_width`, with the
/// host's size request lifted for the measurement.
std::pair<int, int> painted_size(GtkWidget* label, int for_width) {
  int req_w = -1, req_h = -1;
  gtk_widget_get_size_request(label, &req_w, &req_h);
  gtk_widget_set_size_request(label, -1, -1);
  int nat_w = 0, nat_h = 0;
  gtk_widget_measure(label, GTK_ORIENTATION_HORIZONTAL, -1, nullptr, &nat_w, nullptr, nullptr);
  gtk_widget_measure(label, GTK_ORIENTATION_VERTICAL, for_width, nullptr, &nat_h, nullptr, nullptr);
  gtk_widget_set_size_request(label, req_w, req_h);
  return {nat_w, nat_h};
}

/// End to end, measure == paint: the runtime hands each label's resolved typography to Pango,
/// and GTK, measuring the label it painted (its CSS, wrap, lines), agrees with the box the
/// runtime laid out: a `nowrap` label in a narrow column keeps one line at full width, a
/// monospace / letter-spaced / line-height label gets its own metrics, wrapping text wraps
/// inside the column, and `max_lines` caps it.
void test_runtime_measures_labels_with_their_typography() {
  NveGtkHost host(GtkHostConfig{.width = 400.0f, .height = 600.0f});
  GtkWidget* window = present(host, 400, 600);  // CSS applies to realized, styled widgets
  ScriptedApp app;
  constexpr uint32_t kColumn = 2u, kNoWrap = 3u, kMono = 4u, kSpaced = 5u, kTall = 6u, kWraps = 7u, kCapped = 8u;
  constexpr std::string_view kText = "Hello wrapping text that is long";
  namespace h = nve::host;
  auto& w = app.w;
  w.create(1u);
  w.create(kColumn);
  w.set(kColumn, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_WIDTH, h::length_points(90.0f));
  w.set(kColumn, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_ALIGN_ITEMS, NVE_ALIGN_ITEMS_FLEX_START);
  for (const uint32_t id : {kNoWrap, kMono, kSpaced, kTall, kWraps, kCapped}) text_node(w, id, kText);
  const auto typo = [&](uint32_t id, uint32_t set) { w.set(id, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_SET, set); };
  const auto field = [&](uint32_t id, NveFieldSpec f, uint32_t v) { w.set(id, NVE_TABLE_TYPOGRAPHY_DESC, f, v); };
  typo(kNoWrap, NVE_TYPOGRAPHY_SET_WHITE_SPACE);
  field(kNoWrap, NVE_TABLE_TYPOGRAPHY_WHITE_SPACE, NVE_WHITE_SPACE_NO_WRAP);
  typo(kMono, NVE_TYPOGRAPHY_SET_WHITE_SPACE | NVE_TYPOGRAPHY_SET_FONT_FAMILY | NVE_TYPOGRAPHY_SET_FONT_STYLE);
  field(kMono, NVE_TABLE_TYPOGRAPHY_WHITE_SPACE, NVE_WHITE_SPACE_PRE);
  field(kMono, NVE_TABLE_TYPOGRAPHY_FONT_STYLE, NVE_FONT_STYLE_ITALIC);
  (void)w.set_str(kMono, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_FONT_FAMILY, "NoSuchFontNve, monospace");
  typo(kSpaced, NVE_TYPOGRAPHY_SET_WHITE_SPACE | NVE_TYPOGRAPHY_SET_LETTER_SPACING);
  field(kSpaced, NVE_TABLE_TYPOGRAPHY_WHITE_SPACE, NVE_WHITE_SPACE_NO_WRAP);
  (void)w.set_f32(kSpaced, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_LETTER_SPACING, 2.5f);
  typo(kTall, NVE_TYPOGRAPHY_SET_LINE_HEIGHT);
  field(kTall, NVE_TABLE_TYPOGRAPHY_LINE_HEIGHT, h::length_percent(180.0f));
  typo(kCapped, NVE_TYPOGRAPHY_SET_MAX_LINES | NVE_TYPOGRAPHY_SET_LINE_HEIGHT);
  field(kCapped, NVE_TABLE_TYPOGRAPHY_MAX_LINES, 2u);
  field(kCapped, NVE_TABLE_TYPOGRAPHY_LINE_HEIGHT, h::length_points(30.0f));
  const uint32_t kids[] = {kNoWrap, kMono, kSpaced, kTall, kWraps, kCapped};
  w.set_children(kColumn, kids);
  const uint32_t root_kids[] = {kColumn};
  w.set_children(1u, root_kids);
  NVE_CHECK(host.attach_interface(&ScriptedApp::get_interface));
  flush(host);
  spin();

  for (const uint32_t id : {kNoWrap, kMono, kSpaced, kTall, kWraps, kCapped}) {
    GtkWidget* label = host.widget_for_node_id(id);
    const int laid_w = request_width(label), laid_h = request_height(label);
    const auto [painted_w, painted_h] = painted_size(label, laid_w);
    if (!approx(laid_h, painted_h, 1.0) || (id != kWraps && id != kCapped && id != kTall && !approx(laid_w, painted_w, 1.0))) {
      std::cerr << "node " << id << ": laid out " << laid_w << "x" << laid_h << ", painted " << painted_w << "x"
                << painted_h << "\n";
      NVE_CHECK(false);
    }
  }
  const int line = request_height(host.widget_for_node_id(kNoWrap));
  NVE_CHECK(request_width(host.widget_for_node_id(kNoWrap)) > 90);  // one line, overflowing the column
  NVE_CHECK(request_width(host.widget_for_node_id(kMono)) > request_width(host.widget_for_node_id(kNoWrap)));
  NVE_CHECK(request_width(host.widget_for_node_id(kSpaced)) > request_width(host.widget_for_node_id(kNoWrap)));
  NVE_CHECK(request_width(host.widget_for_node_id(kWraps)) <= 90 && request_height(host.widget_for_node_id(kWraps)) > 2 * line);
  NVE_CHECK(approx(request_height(host.widget_for_node_id(kCapped)), 60.0, 1.0));  // 2 lines × 30pt
  NVE_CHECK(request_height(host.widget_for_node_id(kTall)) > request_height(host.widget_for_node_id(kWraps)));
  gtk_window_destroy(GTK_WINDOW(window));
}

void test_counter_widgets_measurement_and_events() {
  NveGtkHost host(GtkHostConfig{.surface_id = 7u, .width = 480.0f, .height = 360.0f});
  NVE_CHECK(!host.is_loaded() && host.view_count() == 0u);
  NVE_CHECK(host.root_widget() != nullptr && GTK_IS_FIXED(host.root_widget()));
  NVE_CHECK(GTK_IS_SCROLLED_WINDOW(host.host_widget()));

  // Pango measurement: monotonic in size and weight, wraps under a width constraint.
  using nve::host::text_style;
  const NveIntrinsicSize small = host.measure_text("Hello NVE", text_style(16.0f));
  const NveIntrinsicSize big = host.measure_text("Hello NVE", text_style(32.0f));
  const NveIntrinsicSize bold = host.measure_text("Hello NVE", text_style(16.0f, 800u));
  const NveIntrinsicSize wrapped = host.measure_text("Hello NVE wrapping text", text_style(16.0f), 40.0f);
  NVE_CHECK(small.width > 0.0f && small.height > 0.0f);
  NVE_CHECK(big.width > small.width * 1.5f && big.height > small.height);
  NVE_CHECK(bold.width >= small.width);
  NVE_CHECK(wrapped.width <= 41.0f && wrapped.height > small.height * 1.5f);
  NVE_CHECK(host.measure_text("", text_style(16.0f)).width == 0.0f);
  test_measure_typography(host);

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
  const NveIntrinsicSize title_size = host.measure_text("NVE native host", text_style(22.0f));
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
  test_runtime_measures_labels_with_their_typography();
  test_modules_hot_reload_and_state();
  test_live_kitchen_sink_if_configured();
  nve_test_gtk_a11y_and_media();
  nve_test_gtk_canvas_scroll_and_pair();
  nve_test_gtk_window_manager();
  nve_test_gtk_reconcile();
  std::cout << "nve_gtk_shell_test: ok\n";
  return 0;
}
