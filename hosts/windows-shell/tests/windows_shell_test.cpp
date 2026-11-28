// Core Windows host suite (portable: runs on every OS against libnve_runtime): the built-in counter
// app mirrored into the retained visual tree, O(changed) reconciliation, surface-point presses,
// outbound events, props, resize, `cdylib` loading errors, state-preserving hot reload, app-state
// files, and the live kitchen-sink `cdylib` (`NVE_TEST_KITCHEN_SINK_DYLIB`).
#include "windows_test_helpers.hpp"

#include <nve/host/demo_counter_app.hpp>

#include <cstdlib>
#include <filesystem>
#include <vector>

namespace {

using namespace nve::win;
using namespace nve::win::test;
using nve::host::demo::CounterApp;
namespace h = nve::host;

std::string text_of(const NveWinHost& host, uint32_t id) { return node(host, id).text; }

void test_counter_model_and_events() {
  const auto host_owner = make_host(WinHostConfig{.surface_id = 7u, .width = 480.0f, .height = 360.0f});
  NveWinHost& host = *host_owner;
  NVE_CHECK(!host.is_loaded() && host.view_count() == 0u && host.surface_id() == 7u);

  std::vector<int64_t> counts;
  host.set_event_callback([&counts](std::span<const NveDirectEvent> evs) {
    for (const NveDirectEvent& ev : evs) {
      if (ev.kind == NVE_EVENT_CUSTOM && ev.value.tag == NVE_VAL_I64) counts.push_back(ev.value.data.i64_val);
    }
  });
  int changes = 0;
  host.set_on_change([&changes] { ++changes; });

  NVE_CHECK(host.attach_interface(&CounterApp::get_interface));
  NVE_CHECK(host.is_loaded() && host.view_count() == 5u && changes >= 1);
  const NveWinVisualTree& tree = host.visual_tree();
  NVE_CHECK((std::vector<uint32_t>(tree.root_children().begin(), tree.root_children().end()) ==
             std::vector<uint32_t>{CounterApp::kColumn}));
  NVE_CHECK((node(host, CounterApp::kColumn).children ==
             std::vector<uint32_t>{CounterApp::kTitle, CounterApp::kIncrement, CounterApp::kCount, CounterApp::kReset}));
  NVE_CHECK((paint_order(host) == std::vector<uint32_t>{CounterApp::kColumn, CounterApp::kTitle, CounterApp::kIncrement,
                                                        CounterApp::kCount, CounterApp::kReset}));
  NVE_CHECK(text_of(host, CounterApp::kTitle) == "NVE native host" && text_of(host, CounterApp::kCount) == "Count: 0");
  NVE_CHECK(tree.find(1u) == nullptr && tree.find(99u) == nullptr && tree.root_id() != 0u);

  // Layout: the column fills the surface, its padding offsets the first child.
  const Rect column = abs_rect(host, CounterApp::kColumn);
  NVE_CHECK(approx(column.x, 0.0) && approx(column.width, 480.0, 0.5));
  const Rect title = abs_rect(host, CounterApp::kTitle);
  NVE_CHECK(approx(title.x, 24.0, 0.5) && approx(title.y, 24.0, 0.5) && title.width > 40.0f);
  NVE_CHECK(abs_rect(host, CounterApp::kCount).y > title.y + 40.0f);

  // Styles, typography, interaction.
  const WinNodeVisual& inc = node(host, CounterApp::kIncrement);
  NVE_CHECK(inc.visual.background == h::rgba(37, 99, 235) && approx(inc.visual.border_radius, 8.0, 0.01));
  NVE_CHECK(inc.text_style.color == h::rgba(255, 255, 255) && approx(inc.text_style.font_size, 16.0, 0.01));
  NVE_CHECK(approx(node(host, CounterApp::kTitle).text_style.font_size, 22.0, 0.01));
  NVE_CHECK(node(host, CounterApp::kColumn).visual.background == h::rgba(248, 250, 252));
  NVE_CHECK(inc.wants(NVE_EVENT_PRESS) && inc.interaction.focusable && !node(host, CounterApp::kTitle).wants(NVE_EVENT_PRESS));

  // Surface-point press -> app -> new frame: only the changed label is reconciled, views retained.
  const WinNodeVisual* count_view = tree.find(CounterApp::kCount);
  const uint64_t before = tree.updated_count();
  const int changes_before = changes;
  const auto [ix, iy] = center(host, CounterApp::kIncrement);
  const PressResult pressed = host.press_at(ix, iy);
  NVE_CHECK(pressed.node_id == CounterApp::kIncrement && pressed.dispatched && pressed.changed && pressed.href.empty());
  NVE_CHECK(text_of(host, CounterApp::kCount) == "Count: 1" && tree.find(CounterApp::kCount) == count_view);
  const uint64_t delta = tree.updated_count() - before;
  NVE_CHECK(delta >= 1u && delta <= 3u && changes > changes_before);
  const auto [tx, ty] = center(host, CounterApp::kTitle);
  NVE_CHECK(host.press_at(tx, ty).node_id == 0u);    // not pressable
  NVE_CHECK(host.press_at(479.0f, 359.0f).node_id == 0u);  // column background
  NVE_CHECK(host.press_at(-5.0f, -5.0f).node_id == 0u);
  NVE_CHECK(host.simulate_press(CounterApp::kIncrement));
  NVE_CHECK(!host.simulate_press(CounterApp::kTitle));
  NVE_CHECK(text_of(host, CounterApp::kCount) == "Count: 2");
  NVE_CHECK((counts == std::vector<int64_t>{1, 2}));
  NVE_CHECK(host.pump() == 0u);  // idle frame: nothing to reconcile
  NVE_CHECK(host.cursor_at(ix, iy) == NVE_CURSOR_KIND_AUTO);

  // Component props; a hot patch the demo cannot apply is a no-op.
  const std::string retitle = "Retitled";
  NveMapEntry prop{};
  prop.key = NveStrSlice{reinterpret_cast<const uint8_t*>("title"), 5u};
  prop.value.tag = NVE_VAL_STR;
  prop.value.data.str_val = NveStrSlice{reinterpret_cast<const uint8_t*>(retitle.data()), retitle.size()};
  NVE_CHECK(host.apply_props(std::span<const NveMapEntry>(&prop, 1u)));
  NVE_CHECK(text_of(host, CounterApp::kTitle) == "Retitled");
  const uint8_t bogus_patch[] = {1, 2, 3};
  NVE_CHECK(!host.apply_hot_patch(bogus_patch));
  NVE_CHECK(text_of(host, CounterApp::kCount) == "Count: 2");

  // Resize relayouts retained views in place; invalid sizes are ignored.
  const WinNodeVisual* column_view = tree.find(CounterApp::kColumn);
  host.set_surface_size(600.0f, 300.0f, 2.0f);
  NVE_CHECK(approx(abs_rect(host, CounterApp::kColumn).width, 600.0, 0.5) && tree.find(CounterApp::kColumn) == column_view);
  NVE_CHECK(host.config().scale_factor == 2.0f && approx(tree.root_rect().width, 600.0, 0.5));
  host.set_surface_size(-1.0f, NAN);
  NVE_CHECK(host.config().width == 600.0f);
}

void test_modules_hot_reload_and_state() {
  const auto host_owner = make_host();
  NveWinHost& host = *host_owner;
  h::ModuleError err = h::ModuleError::None;
  std::string msg;
  NVE_CHECK(!host.load_dylib("", &err, &msg) && err == h::ModuleError::Open);
  NVE_CHECK(!host.load_dylib("/nonexistent/libnope.dll", &err, &msg) && err == h::ModuleError::Open);
  NVE_CHECK(!host.load_dylib(NVE_TEST_NO_ENTRY_MODULE, &err, &msg) && err == h::ModuleError::MissingEntry);
  NVE_CHECK(!host.load_dylib(NVE_TEST_NO_SURFACE_MODULE, &err, &msg) && err == h::ModuleError::MissingSurface);
  NVE_CHECK(!host.load_dylib(NVE_TEST_PARTIAL_SURFACE_MODULE, &err, &msg) && err == h::ModuleError::MissingSurface);
  NVE_CHECK(!host.is_loaded() && host.view_count() == 0u);

  // The dev loop: one app path, rebuilt in place between reloads (the runtime loads a private
  // copy, so the rebuilt file is neither cached by the loader nor locked against the rebuild).
  const ScratchDir scratch("nve_win_app_files_test");
  const std::filesystem::path& dir = scratch.path();
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
  NVE_CHECK(text_of(host, CounterApp::kCount) == "Count: 2");

  // A failed reload keeps the running app.
  NVE_CHECK(!host.reload_dylib(NVE_TEST_NO_SURFACE_MODULE, &err, &msg));
  NVE_CHECK(host.simulate_press(CounterApp::kIncrement));
  NVE_CHECK(text_of(host, CounterApp::kCount) == "Count: 3");
  // Reloading the rebuilt file runs its NEW code (a press counts 2) and carries the state.
  const WinNodeVisual* count_view = host.visual_tree().find(CounterApp::kCount);
  build(NVE_TEST_COUNTER_DOUBLE_MODULE);
  NVE_CHECK(host.reload_dylib(app_path, &err, &msg));
  NVE_CHECK(text_of(host, CounterApp::kCount) == "Count: 3" && host.view_count() == 5u);
  NVE_CHECK(host.visual_tree().find(CounterApp::kCount) == count_view);  // reconciled in place
  NVE_CHECK(host.simulate_press(CounterApp::kIncrement));
  NVE_CHECK(text_of(host, CounterApp::kCount) == "Count: 5");

  // App-state file (runtime `StateFile`): save, mutate, restore; other schema / missing refuse;
  // clearing is a cold start.
  const std::string path = (dir / "state.nvestate").string();
  NVE_CHECK(host.save_state(path, 4u));
  NVE_CHECK(host.simulate_press(CounterApp::kReset));
  NVE_CHECK(text_of(host, CounterApp::kCount) == "Count: 0");
  NVE_CHECK(!host.restore_state(path, 3u));  // newer file: kept
  NVE_CHECK(!host.restore_state(path + ".missing", 4u));
  NVE_CHECK(host.restore_state(path, 4u));
  NVE_CHECK(text_of(host, CounterApp::kCount) == "Count: 5");
  NVE_CHECK(host.clear_state(path) && !std::filesystem::exists(path) && !host.restore_state(path, 4u));

  // Switching apps drops the previous app's views.
  ScriptedApp app;
  app.w.create(1u);
  text_node(app.w, 2u, "scripted");
  const uint32_t kids[] = {2u};
  app.w.set_children(1u, kids);
  NVE_CHECK(host.attach_interface(&ScriptedApp::get_interface));
  NVE_CHECK(host.view_count() == 1u && text_of(host, 2u) == "scripted");
  NVE_CHECK(host.visual_tree().find(CounterApp::kCount) == nullptr);
}

/// Live Rust kitchen-sink `cdylib` (`NVE_TEST_KITCHEN_SINK_DYLIB`, required by check-all): its
/// texts are laid out (non-empty rects), surface-point presses on its buttons count, and the
/// count survives a hot reload.
void test_live_kitchen_sink_if_configured() {
  const char* path = std::getenv("NVE_TEST_KITCHEN_SINK_DYLIB");
  if (path == nullptr || *path == '\0') return;
  const auto host_owner = make_host(WinHostConfig{.width = 1280.0f, .height = 800.0f});
  NveWinHost& host = *host_owner;
  h::ModuleError err = h::ModuleError::None;
  std::string msg;
  NVE_CHECK(host.load_dylib(path, &err, &msg) && err == h::ModuleError::None);
  NVE_CHECK(host.is_loaded() && host.view_count() > 10u);
  struct Scan {
    std::vector<const WinNodeVisual*> texts;
    size_t laid_out = 0u;
    bool enter(const WinNodeVisual& n, const PaintFrame& f) {
      if (!n.text.empty() && f.visible) {
        texts.push_back(&n);
        laid_out += f.rect.width > 0.0f && f.rect.height >= 10.0f ? 1u : 0u;
      }
      return true;
    }
    void leave(const WinNodeVisual&, const PaintFrame&, bool) {}
  };
  const auto find_text = [&host](std::string_view text) -> const WinNodeVisual* {
    Scan scan;
    host.visual_tree().walk(scan);
    for (const WinNodeVisual* t : scan.texts) {
      if (t->text == text) return t;
    }
    return nullptr;
  };
  Scan scan;
  host.visual_tree().walk(scan);
  NVE_CHECK(scan.laid_out > 10u && scan.laid_out == scan.texts.size());  // measured layout everywhere
  NVE_CHECK(find_text("NVE Kitchen Sink") != nullptr && find_text("Count: 0") != nullptr);
  const auto button = [&](std::string_view label) {
    const WinNodeVisual* t = find_text(label);
    const WinNodeVisual* b = t != nullptr ? host.visual_tree().nearest(t, [](const WinNodeVisual& v) {
      return v.wants(NVE_EVENT_PRESS);
    }) : nullptr;
    return b != nullptr ? b->id : 0u;
  };
  const uint32_t inc = button("Increment +");
  const uint32_t reset = button("Reset");
  NVE_CHECK(inc != 0u && reset != 0u && inc != reset);
  const auto tap = [&host](uint32_t id) {
    const auto [x, y] = center(host, id);
    return host.press_at(x, y).dispatched;
  };
  NVE_CHECK(tap(inc) && find_text("Count: 1") != nullptr);
  NVE_CHECK(tap(inc) && find_text("Count: 2") != nullptr);
  NVE_CHECK(tap(reset) && find_text("Count: 0") != nullptr);
  for (int i = 0; i < 3; ++i) NVE_CHECK(tap(inc));
  NVE_CHECK(find_text("Count: 3") != nullptr);
  const size_t views = host.view_count();
  NVE_CHECK(host.reload_dylib(path, &err, &msg));
  NVE_CHECK(find_text("Count: 3") != nullptr && host.view_count() == views);
}

/// Damage follows descendants: when a container moves (or scrolls), its children's painted
/// bounds move with it, so damage covers exactly their old and new areas (no full-surface
/// repaint), and a later change of a child damages where it is painted now, never its stale
/// pre-move area. A removed child damages its last painted area.
void test_damage_follows_moved_and_removed_children() {
  constexpr uint32_t kBox = 2u, kChild = 3u, kOther = 4u;
  ScriptedApp app;
  h::Nve2Writer& w = app.w;
  w.create(1u);
  w.set(1u, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_FLEX_DIRECTION, NVE_FLEX_DIRECTION_ROW);
  w.create(kBox);
  sized(w, kBox, 40.0f, 40.0f);
  w.create(kChild);
  sized(w, kChild, 20.0f, 60.0f);  // taller than the box: it scrolls later
  w.set(kChild, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, h::rgba(255, 0, 0));
  w.create(kOther);
  sized(w, kOther, 10.0f, 10.0f);
  const uint32_t child[] = {kChild};
  w.set_children(kBox, child);
  const uint32_t top[] = {kBox, kOther};
  w.set_children(1u, top);
  const auto host_owner = make_host(WinHostConfig{.width = 400.0f, .height = 300.0f});
  NveWinHost& host = *host_owner;
  NVE_CHECK(host.attach_interface(&ScriptedApp::get_interface));
  (void)host.take_damage();
  const Rect before = abs_rect(host, kChild);

  // The box moves 200px right (geometry only): damage = the child's old + new areas.
  w.set(kBox, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_MARGIN_LEFT, h::length_points(200.0f));
  flush(host);
  const Rect after = abs_rect(host, kChild);
  NVE_CHECK(approx(after.x, before.x + 200.0f, 0.5));
  const Rect moved = host.take_damage();
  NVE_CHECK(moved.x <= before.x && moved.right() >= after.right() && moved.bottom() < 300.0f);
  NVE_CHECK(moved.height <= 61.0f);  // not the surface: the row only

  // A paint-only change of the child: only where it is painted now.
  w.set(kChild, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, h::rgba(0, 0, 255));
  flush(host);
  const Rect repaint = host.take_damage();
  NVE_CHECK(approx(repaint.x, after.x, 0.5) && approx(repaint.width, after.width, 0.5));

  // A runtime-driven scroll of the box shifts the child's painted bounds too.
  w.set(kBox, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_OVERFLOW_Y, NVE_OVERFLOW_SCROLL);
  flush(host);
  (void)host.take_damage();
  const auto [bx, by] = center(host, kBox);
  NVE_CHECK(host.wheel_at(bx, by, 0.0f, 10.0f));
  NVE_CHECK(approx(node(host, kBox).scroll_y, 10.0, 0.01) && approx(abs_rect(host, kChild).y, after.y - 10.0f, 0.5));
  (void)host.take_damage();
  w.set(kChild, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, h::rgba(0, 255, 0));
  flush(host);
  const Rect scrolled_child = abs_rect(host, kChild);
  const Rect scrolled_repaint = host.take_damage();
  NVE_CHECK(approx(scrolled_repaint.y, scrolled_child.y, 0.5) && approx(scrolled_repaint.height, scrolled_child.height, 0.5));

  // Removal: the child's last painted area.
  w.remove(kChild);
  w.set_children(kBox, std::span<const uint32_t>());
  flush(host);
  const Rect removed = host.take_damage();
  NVE_CHECK(removed.x <= scrolled_child.x && removed.right() >= scrolled_child.right());
  NVE_CHECK(removed.y <= scrolled_child.y && removed.bottom() >= scrolled_child.bottom());
}

}  // namespace

void nve_test_win_media_and_input();
void nve_test_win_canvas_scroll_and_windows();
#if defined(_WIN32)
void nve_test_win_direct2d();
#endif

int main() {
  test_counter_model_and_events();
  test_modules_hot_reload_and_state();
  test_live_kitchen_sink_if_configured();
  test_damage_follows_moved_and_removed_children();
  nve_test_win_media_and_input();
  nve_test_win_canvas_scroll_and_windows();
#if defined(_WIN32)
  nve_test_win_direct2d();
#endif
  std::cout << "nve_windows_shell_test: ok\n";
  return 0;
}
