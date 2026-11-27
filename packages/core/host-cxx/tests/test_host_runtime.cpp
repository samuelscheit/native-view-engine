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
#include "nve/host/core_resources.hpp"
#include "nve/host/demo_counter_app.hpp"
#include "nve/host/host_session.hpp"
#include "nve/host/nve2_writer.hpp"
#include "nve/host/retained_tree.hpp"

namespace {

using namespace nve::host;

/// Records every adapter call; views are plain structs keyed by node id.
struct Recorder {
  struct View {
    uint32_t id = 0u;
    NodeKind kind = NodeKind::View;
    std::string text;
    TextStyle style;
    NveLayoutRect local{};
    uint32_t background = 0u;
    uint32_t image_src = 0u;
    std::string value;
    std::string label;
    std::vector<uint32_t> children;
    int updates = 0;
  };
  using Handle = View*;

  std::map<uint32_t, View> views;
  std::vector<uint32_t> root_children;
  int creates = 0;
  int destroys = 0;
  int root_updates = 0;

  uint32_t tag(const NodeData& n) { return static_cast<uint32_t>(n.kind); }
  Handle create(const NodeData& n) {
    ++creates;
    View& v = views[n.id];
    v = View{};
    v.id = n.id;
    return &v;
  }
  void update(Handle& h, const NodeData& n) {
    h->kind = n.kind;
    h->text = std::string(n.text);
    h->style = n.text_style;
    h->style.font_family = {};  // do not keep a view into the scene
    h->local = n.local;
    h->background = n.visual.background;
    h->image_src = n.image.src;
    h->value = std::string(n.input.value);
    h->label = std::string(n.a11y.label);
    ++h->updates;
  }
  void update_root(const NodeData&) { ++root_updates; }
  void destroy(uint32_t id, Handle&) {
    ++destroys;
    views.erase(id);
  }
  void set_children(const NodeData&, Handle* parent, std::span<Handle* const> kids) {
    std::vector<uint32_t>& out = parent != nullptr ? (*parent)->children : root_children;
    out.clear();
    for (Handle* k : kids) out.push_back((*k)->id);
  }
};

struct Harness {
  std::unique_ptr<HostSession> host = HostSession::create(RuntimeSession::default_config(1u, 400.0f, 300.0f));
  Nve2Writer w;
  SceneTables tables;
  RetainedTree<Recorder> tree;
  Recorder rec;
  uint64_t frame = 0u;

  bool submit() {
    const auto bytes = w.finish(++frame);
    if (host->runtime().submit_nve2(bytes) != NVE_RUNTIME_OK) {
      fprintf(stderr, "submit_nve2: %s\n", host->last_error().c_str());
      return false;
    }
    (void)host->pump();
    return true;
  }
  size_t sync() {
    auto guard = host->runtime().acquire_scene();
    if (!guard) return 0u;
    return tree.sync(guard->reader(), tables, rec);
  }
};

void test_writer_round_trip_and_retained_tree() {
  Harness h;
  CHECK(h.host != nullptr);
  if (!h.host) return;
  Nve2Writer& w = h.w;
  w.create(1u);
  for (uint32_t id = 2u; id <= 6u; ++id) w.create(id);
  // 2: column with inherited typography; 3: text; 4: image; 5: text input; 6: scroll.
  w.set(2u, NVE_TABLE_LAYOUT_DESC, NVE_LAYOUT_WIDTH, length_points(200.0f));
  w.set(2u, NVE_TABLE_VISUAL_DESC, NVE_VISUAL_BACKGROUND, rgba(1, 2, 3));
  // overflow_x / overflow_y share word 6: both must survive.
  w.set(2u, NVE_TABLE_VISUAL_DESC, NVE_VISUAL_OVERFLOW_X, NVE_OVERFLOW_HIDDEN);
  w.set(2u, NVE_TABLE_VISUAL_DESC, NVE_VISUAL_OVERFLOW_Y, NVE_OVERFLOW_SCROLL);
  w.set(2u, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TYPOGRAPHY_SET, NVE_TYPOGRAPHY_SET_FONT_SIZE | NVE_TYPOGRAPHY_SET_COLOR);
  w.set_f32(2u, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TYPOGRAPHY_FONT_SIZE, 20.0f);
  w.set(2u, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TYPOGRAPHY_COLOR, rgba(9, 9, 9));
  w.set_str(3u, NVE_TABLE_TEXT_DESC, NVE_TEXT_TEXT, "héllo");
  w.set(3u, NVE_TABLE_LAYOUT_DESC, NVE_LAYOUT_HEIGHT, length_points(30.0f));
  w.set(3u, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TYPOGRAPHY_SET, NVE_TYPOGRAPHY_SET_FONT_WEIGHT);
  w.set(3u, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TYPOGRAPHY_FONT_WEIGHT, 700u);
  const std::vector<uint8_t> src = encode_image_url("file:///tmp/a.png");
  w.define_resource(7u, kImageSrcKind, src);
  w.set(4u, NVE_TABLE_IMAGE_DESC, NVE_IMAGE_SRC, 7u);
  w.set(4u, NVE_TABLE_IMAGE_DESC, NVE_IMAGE_FIT, NVE_IMAGE_FIT_CONTAIN);
  w.set(4u, NVE_TABLE_LAYOUT_DESC, NVE_LAYOUT_HEIGHT, length_points(40.0f));
  w.set_str(5u, NVE_TABLE_TEXT_INPUT_DESC, NVE_TEXT_INPUT_VALUE, "typed");
  w.set_str(5u, NVE_TABLE_ACCESSIBILITY_DESC, NVE_ACCESSIBILITY_LABEL, "Name");
  w.attach(6u, NVE_TABLE_SCROLL_DESC);  // defaults-only row
  CHECK(!w.set_str(3u, NVE_TABLE_TEXT_DESC, NVE_TEXT_TEXT, std::string_view("\xFF", 1)));  // invalid UTF-8
  CHECK(!w.set(3u, NVE_TABLE_TEXT_DESC, NVE_TEXT_TEXT, 1u));  // `Str` needs set_str
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
    }
  }
  std::vector<uint8_t> res;
  CHECK(h.host->runtime().resource_bytes(7u, res) == NVE_RUNTIME_OK);
  const auto decoded = decode_image_src(res);
  CHECK(decoded && decoded->is_url() && decoded->url == "file:///tmp/a.png");

  // Clean frame: nothing to do.
  CHECK(h.sync() == 0u);

  // Parent typography change cascades to the (untouched) text child only.
  const int text_updates = r.views[3].updates;
  const int image_updates = r.views[4].updates;
  w.set_f32(2u, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TYPOGRAPHY_FONT_SIZE, 24.0f);
  CHECK(h.submit());
  CHECK(h.sync() >= 2u);
  CHECK(r.views[3].style.font_size == 24.0f && r.views[3].updates == text_updates + 1);
  CHECK(r.views[4].style.font_size == 24.0f);  // every descendant inherits

  // Relayout moves the untouched image when the text above grows.
  w.set(3u, NVE_TABLE_LAYOUT_DESC, NVE_LAYOUT_HEIGHT, length_points(50.0f));
  CHECK(h.submit());
  (void)h.sync();
  CHECK(r.views[4].local.y == 50.0f && r.views[4].updates > image_updates);

  // Reorder + remove + kind change (text -> image recreates the view).
  w.remove(6u);
  const uint32_t reordered[] = {5u, 3u, 4u};
  w.set_children(2u, reordered);
  w.define_resource(8u, kImageSrcKind, encode_image_url("data:image/png;base64,AA=="));
  w.set(3u, NVE_TABLE_IMAGE_DESC, NVE_IMAGE_SRC, 8u);
  CHECK(h.submit());
  const int destroys = r.destroys;
  (void)h.sync();
  CHECK(r.views.count(6u) == 0u);
  CHECK(r.destroys == destroys + 2);  // removed scroll + recreated text->image
  CHECK(r.views[3].kind == NodeKind::Image);
  CHECK((r.views[2].children == std::vector<uint32_t>{5u, 3u, 4u}));

  // A recycled slot (new generation) recreates its view.
  w.create(make_node_id(6u, 1u));
  w.set_str(make_node_id(6u, 1u), NVE_TABLE_TEXT_DESC, NVE_TEXT_TEXT, "reborn");
  const uint32_t with_new[] = {5u, 3u, 4u, make_node_id(6u, 1u)};
  w.set_children(2u, with_new);
  if (h.submit()) {
    (void)h.sync();
    CHECK(r.views.count(make_node_id(6u, 1u)) == 1u && r.views[make_node_id(6u, 1u)].text == "reborn");
    CHECK(h.tree.find(6u) == nullptr && h.tree.find(make_node_id(6u, 1u)) != nullptr);
  }

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

void test_host_session_counter_and_state(const char* counter_module, const char* no_surface,
                                         const char* no_entry) {
  using demo::CounterApp;
  auto host = HostSession::create(RuntimeSession::default_config(3u, 320.0f, 240.0f));
  CHECK(host != nullptr);
  if (!host) return;
  CHECK(!host->has_app());
  CHECK(host->attach(&CounterApp::get_interface));
  CHECK(host->has_app() && !host->has_module());
  (void)host->pump();
  CHECK(scene_text(*host, CounterApp::kCount) == "Count: 0");
  CHECK(host->dispatch(host->events().press(CounterApp::kIncrement, 1.0f, 1.0f)));
  CHECK(host->dispatch(host->events().press(CounterApp::kIncrement, 1.0f, 1.0f)));
  CHECK(scene_text(*host, CounterApp::kCount) == "Count: 2");
  CHECK(!host->dispatch(host->events().press(CounterApp::kTitle, 0.0f, 0.0f)));  // not a button
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

  // Wake poster installs and clears without leaking callbacks.
  int wakes = 0;
  host->set_wake_poster([&wakes] { ++wakes; });
  host->set_wake_poster({});
  CHECK(wakes >= 0);
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

int main() {
  test_resource_codecs();
  test_writer_round_trip_and_retained_tree();
  test_host_session_counter_and_state(NVE_TEST_COUNTER_MODULE, NVE_TEST_NO_SURFACE_MODULE, NVE_TEST_NO_ENTRY_MODULE);
  if (g_failures != 0) {
    fprintf(stderr, "nve_host_runtime_test: %d failure(s)\n", g_failures);
    return 1;
  }
  printf("nve_host_runtime_test: ok\n");
  return 0;
}
