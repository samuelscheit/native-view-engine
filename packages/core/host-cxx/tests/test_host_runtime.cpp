// Live `libnve_runtime` tests of the retained-view host stack: `Nve2Writer` frames through the real
// runtime decoder, `RetainedTree` + `SceneTables` decoding (kinds, styles, inherited typography,
// resources, removal, slot reuse, relayout), `HostSession` with the built-in counter app, app
// `cdylib` loading / hot reload / state files, and the core resource codecs.
#include <cmath>
#include <cstdio>
#include <filesystem>
#include <map>
#include <string>
#include <vector>

#include "check.h"
#include "nve/host/controlled_text.hpp"
#include "nve/host/core_resources.hpp"
#include "nve/host/demo_counter_app.hpp"
#include "nve/host/host_session.hpp"
#include "nve/host/nve2_writer.hpp"
#include "nve/host/retained_tree.hpp"
#include "nve/host/testing/recorder.hpp"

namespace {

using namespace nve::host;
using testing::Recorder;
using Harness = testing::TreeHarness<Recorder>;

void test_writer_round_trip_and_retained_tree() {
  Harness h;
  CHECK(h.host != nullptr);
  if (!h.host) return;
  Nve2Writer& w = h.w;
  w.create(1u);
  for (uint32_t id = 2u; id <= 6u; ++id) w.create(id);
  // 2: column with inherited typography; 3: text; 4: image; 5: text input; 6: scroll.
  w.set(2u, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_WIDTH, length_points(200.0f));
  w.set(2u, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, rgba(1, 2, 3));
  // overflow_x / overflow_y share word 6: both must survive.
  w.set(2u, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_OVERFLOW_X, NVE_OVERFLOW_HIDDEN);
  w.set(2u, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_OVERFLOW_Y, NVE_OVERFLOW_SCROLL);
  w.set(2u, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_SET, NVE_TYPOGRAPHY_SET_FONT_SIZE | NVE_TYPOGRAPHY_SET_COLOR);
  w.set_f32(2u, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_FONT_SIZE, 20.0f);
  w.set(2u, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_COLOR, rgba(9, 9, 9));
  w.set_str(3u, NVE_TABLE_TEXT_DESC, NVE_TABLE_TEXT_TEXT, "héllo");
  w.set(3u, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_HEIGHT, length_points(30.0f));
  w.set(3u, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_SET, NVE_TYPOGRAPHY_SET_FONT_WEIGHT);
  w.set(3u, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_FONT_WEIGHT, 700u);
  const std::vector<uint8_t> src = encode_image_url("file:///tmp/a.png");
  w.define_resource(7u, kImageSrcKind, src);
  w.set(4u, NVE_TABLE_IMAGE_DESC, NVE_TABLE_IMAGE_SRC, 7u);
  w.set(4u, NVE_TABLE_IMAGE_DESC, NVE_TABLE_IMAGE_FIT, NVE_IMAGE_FIT_CONTAIN);
  w.set(4u, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_HEIGHT, length_points(40.0f));
  w.set_str(5u, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, "typed");
  w.set_str(5u, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_LABEL, "Name");
  w.attach(6u, NVE_TABLE_SCROLL_DESC);  // defaults-only row
  CHECK(!w.set_str(3u, NVE_TABLE_TEXT_DESC, NVE_TABLE_TEXT_TEXT, std::string_view("\xFF", 1)));  // invalid UTF-8
  CHECK(!w.set(3u, NVE_TABLE_TEXT_DESC, NVE_TABLE_TEXT_TEXT, 1u));  // `Str` needs set_str
  const uint32_t kids[] = {3u, 4u, 5u, 6u};
  w.set_children(2u, kids);
  const uint32_t root_kids[] = {2u};
  w.set_children(1u, root_kids);
  CHECK(h.submit());
  CHECK(h.sync() == 6u);  // root + 5 views
  Recorder& r = h.rec;
  CHECK(r.views.size() == 5u && r.creates == 5 && r.root_updates == 1);
  CHECK(r.root_children == std::vector<uint32_t>{2u});
  CHECK((r.views[2].children == std::vector<uint32_t>{3u, 4u, 5u, 6u}));
  CHECK(r.views[2].background == rgba(1, 2, 3));
  CHECK(r.views[3].kind == NodeKind::Text && r.views[3].text == "héllo");
  CHECK(r.views[3].style.font_size == 20.0f && r.views[3].style.color == rgba(9, 9, 9));  // inherited
  CHECK(r.views[3].style.font_weight == 700u);                                             // own
  CHECK(r.views[3].local.x == 0.0f && r.views[3].local.width == 200.0f);
  CHECK(r.views[4].kind == NodeKind::Image && r.views[4].image_src == 7u);
  CHECK(r.views[4].local.y == 30.0f);  // parent-relative
  CHECK(r.views[5].kind == NodeKind::TextInput && r.views[5].value == "typed" && r.views[5].label == "Name");
  CHECK(r.views[6].kind == NodeKind::Scroll);
  {
    auto guard = h.host->runtime().acquire_scene();
    CHECK(guard.has_value());
    if (guard) {
      const NodeData col = read_node(guard->reader(), h.tables, 2u, TextStyle{});
      CHECK(col.visual.overflow_x == NVE_OVERFLOW_HIDDEN && col.visual.overflow_y == NVE_OVERFLOW_SCROLL);
      CHECK(col.scrollable() && !col.wants(NVE_EVENT_PRESS));
      const NodeData img = read_node(guard->reader(), h.tables, 4u, TextStyle{});
      CHECK(img.image.fit == NVE_IMAGE_FIT_CONTAIN);
      const auto res = guard->resource(img.image.src);
      CHECK(res && res->kind == kImageSrcKind);
      const auto decoded = res ? decode_image_src(res->bytes) : std::nullopt;
      CHECK(decoded && decoded->is_url() && decoded->url == "file:///tmp/a.png");
    }
  }

  // Clean frame: nothing to do.
  CHECK(h.sync() == 0u);

  // Parent typography change cascades to the (untouched) text child only.
  const int text_updates = r.views[3].updates;
  const int image_updates = r.views[4].updates;
  w.set_f32(2u, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_FONT_SIZE, 24.0f);
  CHECK(h.submit());
  CHECK(h.sync() >= 2u);
  CHECK(r.views[3].style.font_size == 24.0f && r.views[3].updates == text_updates + 1);
  CHECK(r.views[4].style.font_size == 24.0f);  // every descendant inherits

  // Relayout moves the untouched image when the text above grows.
  w.set(3u, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_HEIGHT, length_points(50.0f));
  CHECK(h.submit());
  (void)h.sync();
  CHECK(r.views[4].local.y == 50.0f && r.views[4].updates > image_updates);

  // Reorder + remove + kind change (text -> image recreates the view).
  w.remove(6u);
  const uint32_t reordered[] = {5u, 3u, 4u};
  w.set_children(2u, reordered);
  w.define_resource(8u, kImageSrcKind, encode_image_url("data:image/png;base64,AA=="));
  w.set(3u, NVE_TABLE_IMAGE_DESC, NVE_TABLE_IMAGE_SRC, 8u);
  CHECK(h.submit());
  const int destroys = r.destroys;
  (void)h.sync();
  CHECK(r.views.count(6u) == 0u);
  CHECK(r.destroys == destroys + 2);  // removed scroll + recreated text->image
  CHECK(r.views[3].kind == NodeKind::Image);
  CHECK((r.views[2].children == std::vector<uint32_t>{5u, 3u, 4u}));

  // A recycled slot (new generation) recreates its view.
  w.create(make_node_id(6u, 1u));
  w.set_str(make_node_id(6u, 1u), NVE_TABLE_TEXT_DESC, NVE_TABLE_TEXT_TEXT, "reborn");
  const uint32_t with_new[] = {5u, 3u, 4u, make_node_id(6u, 1u)};
  w.set_children(2u, with_new);
  if (h.submit()) {
    (void)h.sync();
    CHECK(r.views.count(make_node_id(6u, 1u)) == 1u && r.views[make_node_id(6u, 1u)].text == "reborn");
    CHECK(h.tree.find(6u) == nullptr && h.tree.find(make_node_id(6u, 1u)) != nullptr);
  }

  // Kind change under a CLEAN parent: the recreated view is relinked into it.
  w.define_resource(9u, kImageSrcKind, encode_image_url("file:///tmp/b.png"));
  w.set(5u, NVE_TABLE_IMAGE_DESC, NVE_TABLE_IMAGE_SRC, 9u);
  CHECK(h.submit());
  (void)h.sync();
  CHECK(r.views[5].kind == NodeKind::Image && r.views[5].image_src == 9u);
  CHECK(r.views[5].attached_to == 2u);  // the NEW view object was linked
  CHECK((r.views[2].children == std::vector<uint32_t>{5u, 3u, 4u, make_node_id(6u, 1u)}));
  w.detach(5u, NVE_TABLE_IMAGE_DESC);

  // Detach a table row: the text input loses its value.
  w.detach(5u, NVE_TABLE_TEXT_INPUT_DESC);
  CHECK(h.submit());
  (void)h.sync();
  CHECK(r.views[5].kind == NodeKind::View && r.views[5].value.empty());

  h.tree.clear(h.rec);
  CHECK(r.views.empty() && h.tree.size() == 0u);
}

std::string scene_text(HostSession& host, uint32_t id) {
  auto guard = host.runtime().acquire_scene();
  if (!guard) return {};
  SceneTables t;
  t.bind(guard->reader());
  return std::string(read_node(guard->reader(), t, node_index(id), TextStyle{}).text);
}

/// The tree must never keep a view into the scene's text slab across frames: growing the slab
/// (re)allocates it, and a stale `font_family` compare would read freed memory (ASan).
void test_font_family_survives_text_slab_growth() {
  Harness h;
  if (!h.host) return;
  Nve2Writer& w = h.w;
  const std::string family = "Inter Display Variable Family";  // beyond any SSO buffer
  w.create(1u);
  w.create(2u);
  w.create(3u);
  w.set(2u, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_SET, NVE_TYPOGRAPHY_SET_FONT_FAMILY);
  CHECK(w.set_str(2u, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_FONT_FAMILY, family));
  CHECK(w.set_str(3u, NVE_TABLE_TEXT_DESC, NVE_TABLE_TEXT_TEXT, "child"));
  w.set_children(2u, std::vector<uint32_t>{3u});
  w.set_children(1u, std::vector<uint32_t>{2u});
  CHECK(h.submit());
  CHECK(h.sync() >= 2u);
  CHECK(h.rec.views[3].family == family);
  const auto slab = [&] {
    auto g = h.host->runtime().acquire_scene();
    return g ? static_cast<const void*>(g->reader().text_slab().data()) : nullptr;
  };
  const void* const slab_before = slab();
  // Grow the slab far past its capacity with unrelated text, then rewrite the same family.
  const std::string filler(4096, 'x');
  for (uint32_t id = 10u; id < 74u; ++id) {
    w.create(id);
    CHECK(w.set_str(id, NVE_TABLE_TEXT_DESC, NVE_TABLE_TEXT_TEXT, filler + std::to_string(id)));
  }
  CHECK(h.submit());
  CHECK(slab() != slab_before);  // the scenario under test: the slab really moved
  (void)h.sync();
  CHECK(w.set_str(2u, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_FONT_FAMILY, family));
  CHECK(h.submit());
  (void)h.sync();
  CHECK(h.rec.views[3].family == family);
  // A real change still cascades to the untouched child.
  CHECK(w.set_str(2u, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_FONT_FAMILY, "Mono"));
  CHECK(h.submit());
  CHECK(h.sync() >= 2u);
  CHECK(h.rec.views[3].family == "Mono");
}

/// Attaching another app replaces the scene without removals: the session's source generation
/// makes the next sync a full pass that destroys the old app's extra views and refreshes the rest.
void test_source_change_runs_a_full_pass() {
  using nve::host::demo::CounterApp;
  Harness h;
  if (!h.host) return;
  Nve2Writer& w = h.w;
  for (uint32_t id = 1u; id <= 3u; ++id) w.create(id);
  w.create(40u);
  CHECK(w.set_str(3u, NVE_TABLE_TEXT_DESC, NVE_TABLE_TEXT_TEXT, "old"));
  w.set_children(2u, std::vector<uint32_t>{3u, 40u});
  w.set_children(1u, std::vector<uint32_t>{2u});
  CHECK(h.submit());
  CHECK(h.sync() == 4u);  // root + 3 views
  CHECK(h.rec.views.count(40u) == 1u && h.rec.views[3].text == "old");
  const uint64_t before = h.host->source_generation();
  CHECK(h.host->attach(&CounterApp::get_interface));
  CHECK(h.host->source_generation() == before + 1u);
  (void)h.host->pump();
  CHECK(h.sync() > 0u);
  CHECK(h.rec.views.count(40u) == 0u);  // only the old app had node 40
  CHECK(h.rec.views[CounterApp::kTitle].text == "NVE native host");
  CHECK(h.rec.views[CounterApp::kTitle].attached_to == CounterApp::kColumn);  // relinked
  // A failed attach leaves the running app (and the generation) untouched.
  CHECK(!h.host->attach(nullptr));
  CHECK(h.host->source_generation() == before + 1u && h.host->has_app());
}

/// A dense table (`nve.visual`) stores a zeroed row at every index: nodes without the table must
/// read the table defaults (opacity 1), not that row (opacity 0 hid every text node).
void test_dense_table_rows_follow_table_set() {
  Harness h;
  if (!h.host) return;
  Nve2Writer& w = h.w;
  for (uint32_t id = 1u; id <= 3u; ++id) w.create(id);
  w.set(2u, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, rgba(1, 2, 3));
  w.set_f32(2u, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_OPACITY, 0.25f);
  CHECK(w.set_str(3u, NVE_TABLE_TEXT_DESC, NVE_TABLE_TEXT_TEXT, "no visual row"));
  w.set_children(1u, std::vector<uint32_t>{2u, 3u});
  CHECK(h.submit());
  auto guard = h.host->runtime().acquire_scene();
  CHECK(guard.has_value());
  SceneTables t;
  t.bind(guard->reader());
  CHECK(t.visual.has(2u) && t.visual.at(2u).present());
  CHECK(!t.visual.has(3u) && !t.visual.at(3u).present());
  CHECK(t.visual.at(3u).f32(NVE_TABLE_VISUAL_OPACITY) == 1.0f);
  const NodeData with = read_node(guard->reader(), t, 2u, TextStyle{});
  const NodeData without = read_node(guard->reader(), t, 3u, TextStyle{});
  CHECK(with.visual.opacity == 0.25f && with.visual.background == rgba(1, 2, 3));
  CHECK(without.visual.opacity == 1.0f && without.visual.background == 0u && !without.hidden());
}

/// `ControlledText`: first publish writes, edit echoes are absorbed, app rewrites win, host
/// writes are not edits.
void test_controlled_text() {
  ControlledText t;
  CHECK(t.app_published("a", ""));
  CHECK(t.user_edited("ab") && !t.user_edited("ab"));
  CHECK(!t.app_published("ab", "ab"));   // echo of the edit
  CHECK(t.user_edited("abc"));
  CHECK(!t.app_published("ab", "abc"));  // stale value while typing: keep the user's text
  CHECK(t.app_published("ABC", "abc"));  // the app transformed the value
  {
    const auto write = t.host_write();
    CHECK(t.writing() && !t.user_edited("ABC"));
  }
  CHECK(!t.writing() && !t.app_published("ABC", "ABC"));
  CHECK(t.app_published("", "ABC"));     // the app clears the field
  CHECK(t.user_edited("x") && !t.app_published("x", "x"));
}

void test_host_session_counter_and_state(const char* counter_module, const char* no_surface,
                                         const char* no_entry) {
  using demo::CounterApp;
  auto host = HostSession::create(RuntimeSession::default_config(3u, 320.0f, 240.0f));
  CHECK(host != nullptr);
  if (!host) return;
  CHECK(!host->has_app());
  std::vector<int64_t> emitted;
  host->set_event_sink([&emitted](std::span<const NveDirectEvent> evs) {
    for (const NveDirectEvent& ev : evs) {
      const std::string_view name(reinterpret_cast<const char*>(ev.name.ptr), ev.name.len);
      if (ev.kind == NVE_EVENT_CUSTOM && name == CounterApp::kCountEventName && ev.value.tag == NVE_VAL_I64) {
        emitted.push_back(ev.value.data.i64_val);
      }
    }
  });
  CHECK(host->attach(&CounterApp::get_interface));
  CHECK(host->has_app() && !host->has_module());
  (void)host->pump();
  CHECK(scene_text(*host, CounterApp::kCount) == "Count: 0");
  CHECK(host->dispatch(host->events().press(CounterApp::kIncrement, 1.0f, 1.0f)));
  CHECK(host->dispatch(host->events().press(CounterApp::kIncrement, 1.0f, 1.0f)));
  CHECK(scene_text(*host, CounterApp::kCount) == "Count: 2");
  CHECK((emitted == std::vector<int64_t>{1, 2}));  // outbound component events reach the sink
  CHECK(!host->dispatch(host->events().press(CounterApp::kTitle, 0.0f, 0.0f)));  // not a button
  const std::string title = "Retitled";
  NveMapEntry prop{};
  prop.key = NveStrSlice{reinterpret_cast<const uint8_t*>("title"), 5u};
  prop.value.tag = NVE_VAL_STR;
  prop.value.data.str_val = NveStrSlice{reinterpret_cast<const uint8_t*>(title.data()), title.size()};
  CHECK(host->set_props(std::span<const NveMapEntry>(&prop, 1u)));
  CHECK(scene_text(*host, CounterApp::kTitle) == "Retitled");
  CHECK(host->events().press(9u, 0, 0).surface_id == 3u);

  const std::string path = (std::filesystem::temp_directory_path() / "nve_host_state_test.bin").string();
  CHECK(host->save_state(path, 7u));
  CHECK(host->dispatch(host->events().press(CounterApp::kReset, 0.0f, 0.0f)));
  CHECK(scene_text(*host, CounterApp::kCount) == "Count: 0");
  CHECK(!host->restore_state_file(path, 8u));  // schema mismatch
  CHECK(host->restore_state_file(path, 7u));
  (void)host->pump();
  CHECK(scene_text(*host, CounterApp::kCount) == "Count: 2");
  {
    std::FILE* f = std::fopen(path.c_str(), "ab");  // trailing garbage
    if (f != nullptr) {
      std::fputc(1, f);
      std::fclose(f);
    }
  }
  CHECK(!host->restore_state_file(path, 7u));
  CHECK(!host->restore_state_file(path + ".missing", 7u));
  std::filesystem::remove(path);

  // `cdylib` modules: validation errors, load, and state-preserving hot reload.
  ModuleError err = ModuleError::None;
  std::string msg;
  CHECK(!host->load_module("/nonexistent/libnope.so", &err, &msg) && err == ModuleError::Open && !msg.empty());
  CHECK(!host->load_module(no_entry, &err, &msg) && err == ModuleError::MissingEntry);
  CHECK(!host->load_module(no_surface, &err, &msg) && err == ModuleError::MissingSurface);
  CHECK(scene_text(*host, CounterApp::kCount) == "Count: 2");  // failed loads keep the running app
  CHECK(host->load_module(counter_module, &err, &msg) && host->has_module());
  (void)host->pump();
  CHECK(scene_text(*host, CounterApp::kCount) == "Count: 0");  // fresh module instance
  CHECK(host->dispatch(host->events().press(CounterApp::kIncrement, 0.0f, 0.0f)));
  CHECK(host->reload_module(counter_module, &err, &msg));
  (void)host->pump();
  CHECK(scene_text(*host, CounterApp::kCount) == "Count: 1");  // state carried across reload
  CHECK(host->attach(&CounterApp::get_interface) && !host->has_module());

  // The wake poster receives runtime wakes (a state restore schedules a render) and, once
  // cleared, never again.
  int wakes = 0;
  host->set_wake_poster([&wakes] { ++wakes; });
  const std::vector<uint8_t> snap = host->snapshot_state();
  CHECK(!snap.empty() && host->runtime().restore_state(snap) == NVE_RUNTIME_OK);
  CHECK(wakes >= 1);
  (void)host->pump();
  const int seen = wakes;
  host->set_wake_poster({});
  CHECK(host->runtime().restore_state(snap) == NVE_RUNTIME_OK);
  CHECK(wakes == seen);
}

void set_env(const char* name, const char* value) {
#if defined(_WIN32)
  _putenv_s(name, value != nullptr ? value : "");
#else
  if (value != nullptr) {
    setenv(name, value, 1);
  } else {
    unsetenv(name);
  }
#endif
}

void test_app_state_paths() {
  namespace fs = std::filesystem;
  set_env("NVE_STATE_DISABLE", nullptr);
  set_env("NVE_STATE_PATH", nullptr);
  set_env("NVE_RESET_STATE", nullptr);
  const std::string a = app_state::resolve_path("/apps/My Cool App!.dylib");
  CHECK(a.find("my-cool-app-") != std::string::npos && a.ends_with(".nvestate"));
  CHECK(a == app_state::resolve_path("/apps/My Cool App!.dylib"));  // stable
  CHECK(a != app_state::resolve_path("/other/My Cool App!.dylib"));  // identity-scoped
  CHECK(app_state::slug("") == "nve-app" && app_state::slug("???") == "nve-app");

  const std::string explicit_path = (fs::temp_directory_path() / "nve_app_state_test" / "s.nvestate").string();
  set_env("NVE_STATE_PATH", explicit_path.c_str());
  CHECK(app_state::resolve_path("x") == explicit_path);
  const uint8_t payload[] = {1, 2, 3};
  CHECK(app_state::write_file(explicit_path, 9u, payload));  // creates the directory
  CHECK(!fs::exists(explicit_path + ".tmp"));
  CHECK(app_state::read_file(explicit_path, 9u) == std::vector<uint8_t>({1, 2, 3}));
  CHECK(!app_state::read_file(explicit_path, 10u));
  set_env("NVE_RESET_STATE", "yes");
  CHECK(app_state::resolve_path("x") == explicit_path && !fs::exists(explicit_path));
  set_env("NVE_RESET_STATE", nullptr);
  set_env("NVE_STATE_DISABLE", "TRUE");
  CHECK(app_state::resolve_path("x").empty());
  CHECK(!app_state::write_file("", 1u, payload));
  set_env("NVE_STATE_DISABLE", nullptr);
  set_env("NVE_STATE_PATH", nullptr);
  fs::remove_all(fs::temp_directory_path() / "nve_app_state_test");
}

void test_resource_codecs() {
  const std::vector<uint8_t> url = encode_image_url("https://x/y.png");
  CHECK(decode_image_src(url)->url == "https://x/y.png");
  const uint8_t png[] = {0x89, 'P', 'N', 'G'};
  const std::vector<uint8_t> bytes = encode_image_bytes(png);
  const auto img = decode_image_src(bytes);
  CHECK(img && !img->is_url() && img->encoded.size() == 4u && img->encoded[1] == 'P');
  for (size_t cut = 0; cut < url.size(); ++cut) {
    CHECK(!decode_image_src(std::span<const uint8_t>(url.data(), cut)));  // every truncation
  }
  std::vector<uint8_t> trailing = url;
  trailing.push_back(0u);
  CHECK(!decode_image_src(trailing));
  std::vector<uint8_t> bad_tag = url;
  bad_tag[0] = 2u;
  CHECK(!decode_image_src(bad_tag));
  std::vector<uint8_t> bad_utf8 = encode_image_url("ab");
  bad_utf8.back() = 0xFFu;
  CHECK(!decode_image_src(bad_utf8));

  const BoxShadow shadows[] = {{1.0f, 2.0f, 3.0f, 4.0f, rgba(1, 2, 3, 4), false}, {-1.0f, 0.0f, 8.0f, 0.0f, 7u, true}};
  const std::vector<uint8_t> enc = encode_box_shadows(shadows);
  CHECK(enc.size() == 4u + 2u * 21u);
  const auto dec = decode_box_shadows(enc);
  CHECK(dec && dec->size() == 2u && (*dec)[0] == shadows[0] && (*dec)[1] == shadows[1]);
  for (size_t cut = 0; cut < enc.size(); ++cut) CHECK(!decode_box_shadows(std::span<const uint8_t>(enc.data(), cut)));
  std::vector<uint8_t> bad_inset = enc;
  bad_inset.back() = 2u;
  CHECK(!decode_box_shadows(bad_inset));
  const uint8_t bomb[] = {0x01, 0x10, 0x00, 0x00};  // 4097 shadows
  CHECK(!decode_box_shadows(bomb));
}

}  // namespace

/// `event_bit` matches Rust `EventMask::of` for every kind (composition kinds share a bit).
void test_event_bits() {
  static_assert(event_bit(NVE_EVENT_PRESS) == 1u);
  CHECK(event_bit(NVE_EVENT_COMPOSITION_START) == (1u << 8u));
  CHECK(event_bit(NVE_EVENT_COMPOSITION_UPDATE) == event_bit(NVE_EVENT_COMPOSITION_END));
  CHECK(event_bit(NVE_EVENT_KEY_DOWN) == (1u << 9u) && event_bit(NVE_EVENT_SUBMIT) == (1u << 11u));
  CHECK(event_bit(NVE_EVENT_WHEEL) == (1u << 17u) && event_bit(NVE_EVENT_TABLE_EVENT) == (1u << 18u));
  CHECK(event_bit(static_cast<NveEventKind>(0)) == 0u && event_bit(static_cast<NveEventKind>(22)) == 0u);
  uint32_t all = 0u;
  for (uint32_t k = 1u; k <= 21u; ++k) {
    const uint32_t bit = event_bit(static_cast<NveEventKind>(k));
    CHECK(bit != 0u && (bit & (bit - 1u)) == 0u);
    all |= bit;
  }
  CHECK(all == (1u << 19u) - 1u);  // 21 kinds cover exactly the 19 mask bits
}

int main() {
  test_event_bits();
  test_resource_codecs();
  test_app_state_paths();
  test_writer_round_trip_and_retained_tree();
  test_font_family_survives_text_slab_growth();
  test_source_change_runs_a_full_pass();
  test_dense_table_rows_follow_table_set();
  test_controlled_text();
  test_host_session_counter_and_state(NVE_TEST_COUNTER_MODULE, NVE_TEST_NO_SURFACE_MODULE, NVE_TEST_NO_ENTRY_MODULE);
  if (g_failures != 0) {
    fprintf(stderr, "nve_host_runtime_test: %d failure(s)\n", g_failures);
    return 1;
  }
  printf("nve_host_runtime_test: ok\n");
  return 0;
}
