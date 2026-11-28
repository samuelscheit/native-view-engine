// Live `libnve_runtime` tests of `RetainedTree`'s O(changed) contract: geometry-only changes reach
// `move` (not `update`), adapters without `move` fall back to `update`, parent-relative origins stay
// exact across parent moves and reparents, sparse rows resolve correctly across swap-remove
// compaction and slot reuse, cascade-recreated views are relinked, skipped frames and new scene
// epochs resync fully, republished resources re-apply exactly their clean users, and
// a steady-state sync of a 10k-node tree costs O(changed) work with zero heap allocations.
#include <atomic>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <map>
#include <new>
#include <string>
#include <utility>
#include <vector>

#include "check.h"
#include "nve/host/core_resources.hpp"
#include "nve/host/demo_counter_app.hpp"
#include "nve/host/testing/recorder.hpp"

// Counts every C++ heap allocation of this binary (the Rust runtime uses its own allocator).
namespace {
std::atomic<size_t> g_allocs{0u};
}  // namespace

void* operator new(size_t n) {
  g_allocs.fetch_add(1u, std::memory_order_relaxed);
  if (void* p = std::malloc(n != 0u ? n : 1u)) return p;
  throw std::bad_alloc();
}
void operator delete(void* p) noexcept { std::free(p); }
void operator delete(void* p, size_t) noexcept { std::free(p); }

namespace {

using namespace nve::host;
using testing::MovingRecorder;
using testing::Recorder;
template <class A>
using Harness = testing::TreeHarness<A>;

void set_height(Nve2Writer& w, uint32_t id, float h) {
  w.set(id, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_HEIGHT, length_points(h));
}

/// ROOT(1) -> column(2)[a(3) h=30, b(4) h=40, c(5) h=50].
template <class A>
void mount_column(Harness<A>& h) {
  Nve2Writer& w = h.w;
  for (uint32_t id = 1u; id <= 5u; ++id) w.create(id);
  w.set(2u, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_WIDTH, length_points(100.0f));
  set_height(w, 3u, 30.0f);
  set_height(w, 4u, 40.0f);
  set_height(w, 5u, 50.0f);
  w.set_children(2u, std::vector<uint32_t>{3u, 4u, 5u});
  w.set_children(1u, std::vector<uint32_t>{2u});
  CHECK(h.submit());
  CHECK(h.sync() == 5u);
}

void test_moved_only_nodes_get_move_not_update() {
  Harness<MovingRecorder> h;
  CHECK(h.host != nullptr);
  if (!h.host) return;
  mount_column(h);
  auto& v = h.rec.views;
  const int a_updates = v[3].updates, b_updates = v[4].updates, c_updates = v[5].updates;
  set_height(h.w, 3u, 60.0f);  // `a` grows: b and c shift, the auto-height column grows
  CHECK(h.submit());
  CHECK(h.sync() == 1u);         // only `a` is content-dirty
  CHECK(h.tree.moved() == 3u);   // column, b, c
  CHECK(v[3].updates == a_updates + 1);
  CHECK(v[4].updates == b_updates && v[4].moves == 1 && v[4].local.y == 60.0f);
  CHECK(v[5].updates == c_updates && v[5].moves == 1 && v[5].local.y == 100.0f);
  CHECK(v[2].moves == 1 && v[2].local.height == 150.0f);

  // Content + move in one frame: a full `update` (with the new geometry), never a `move` too.
  set_height(h.w, 3u, 30.0f);
  h.w.set(4u, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, rgba(4, 5, 6));
  CHECK(h.submit());
  (void)h.sync();
  CHECK(v[4].updates == b_updates + 1 && v[4].moves == 1);
  CHECK(v[4].local.y == 30.0f && v[4].background == rgba(4, 5, 6));
  CHECK(v[5].moves == 2 && v[5].local.y == 70.0f);
}

void test_adapters_without_move_fall_back_to_update() {
  Harness<Recorder> h;
  CHECK(h.host != nullptr);
  if (!h.host) return;
  mount_column(h);
  auto& v = h.rec.views;
  const int b_updates = v[4].updates;
  set_height(h.w, 3u, 60.0f);
  CHECK(h.submit());
  CHECK(h.sync() == 4u);  // a + the three moved nodes, all through `update`
  CHECK(h.tree.moved() == 0u);
  CHECK(v[4].updates == b_updates + 1 && v[4].local.y == 60.0f);
}

/// A parent whose origin moves while its child keeps its absolute rect: the child's parent-relative
/// `local` must still follow (spacer shrinks by 10 while the parent gains 10 of top padding).
void test_parent_origin_move_refreshes_child_local() {
  Harness<MovingRecorder> h;
  CHECK(h.host != nullptr);
  if (!h.host) return;
  Nve2Writer& w = h.w;
  for (uint32_t id = 1u; id <= 5u; ++id) w.create(id);  // 2 column, 3 spacer, 4 p, 5 x
  w.set(2u, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_WIDTH, length_points(100.0f));
  set_height(w, 3u, 20.0f);
  set_height(w, 5u, 10.0f);
  w.set_children(4u, std::vector<uint32_t>{5u});
  w.set_children(2u, std::vector<uint32_t>{3u, 4u});
  w.set_children(1u, std::vector<uint32_t>{2u});
  CHECK(h.submit());
  (void)h.sync();
  auto& v = h.rec.views;
  CHECK(v[4].local.y == 20.0f && v[5].local.y == 0.0f);
  set_height(w, 3u, 10.0f);
  w.set(4u, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_PADDING_TOP, length_points(10.0f));
  CHECK(h.submit());
  (void)h.sync();
  CHECK(v[4].local.y == 10.0f);
  CHECK(v[5].local.y == 10.0f && v[5].moves == 1);  // absolute y unchanged (20), local 0 -> 10
}

/// Reparenting with an unchanged absolute rect: p1 (padding-top 20) holds x at y=20; p2 starts at
/// y=20, so x moved into p2 keeps y=20 but its origin inside its parent drops from 20 to 0.
void test_reparent_with_unchanged_absolute_rect() {
  Harness<MovingRecorder> h;
  CHECK(h.host != nullptr);
  if (!h.host) return;
  Nve2Writer& w = h.w;
  for (uint32_t id = 1u; id <= 5u; ++id) w.create(id);  // 2 column, 3 p1, 4 p2, 5 x
  w.set(2u, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_WIDTH, length_points(100.0f));
  set_height(w, 3u, 20.0f);
  w.set(3u, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_PADDING_TOP, length_points(20.0f));
  set_height(w, 4u, 20.0f);
  set_height(w, 5u, 10.0f);
  w.set_children(3u, std::vector<uint32_t>{5u});
  w.set_children(2u, std::vector<uint32_t>{3u, 4u});
  w.set_children(1u, std::vector<uint32_t>{2u});
  CHECK(h.submit());
  (void)h.sync();
  auto& v = h.rec.views;
  CHECK(v[5].local.y == 20.0f && v[5].attached_to == 3u);
  const int updates = v[5].updates;
  w.set_children(4u, std::vector<uint32_t>{5u});
  CHECK(h.submit());
  (void)h.sync();
  CHECK(v[5].attached_to == 4u && v[3].children.empty() && v[4].children == std::vector<uint32_t>{5u});
  CHECK(v[5].local.y == 0.0f && v[5].updates == updates + 1 && v[5].moves == 0);  // reparented: dirty
}

/// A reparent that changes no geometry at all (both parents at one origin) still relinks the view
/// and re-inherits typography from its new parent.
void test_reparent_between_styled_parents_at_one_origin() {
  Harness<MovingRecorder> h;
  CHECK(h.host != nullptr);
  if (!h.host) return;
  Nve2Writer& w = h.w;  // ROOT(1) -> column(2)[p1(3)[p2(4), x(5) "x"]]: p2 empty at p1's origin
  for (uint32_t id = 1u; id <= 5u; ++id) w.create(id);
  w.set(2u, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_WIDTH, length_points(100.0f));
  for (const auto& [id, size] : {std::pair{3u, 20.0f}, std::pair{4u, 30.0f}}) {
    w.set_f32(id, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_FONT_SIZE, size);
  }
  CHECK(w.set_str(5u, NVE_TABLE_TEXT_DESC, NVE_TABLE_TEXT_TEXT, "x"));
  set_height(w, 5u, 10.0f);
  w.set_children(3u, std::vector<uint32_t>{4u, 5u});
  w.set_children(2u, std::vector<uint32_t>{3u});
  w.set_children(1u, std::vector<uint32_t>{2u});
  CHECK(h.submit());
  (void)h.sync();
  auto& v = h.rec.views;
  CHECK(v[5].attached_to == 3u && v[5].style.font_size == 20.0f);
  const NveLayoutRect local = v[5].local;
  w.set_children(4u, std::vector<uint32_t>{5u});
  CHECK(h.submit());
  (void)h.sync();
  CHECK(v[5].attached_to == 4u && v[4].children == std::vector<uint32_t>{5u});
  CHECK(v[3].children == std::vector<uint32_t>{4u});
  CHECK(v[5].style.font_size == 30.0f);
  CHECK(v[5].local.x == local.x && v[5].local.y == local.y && v[5].local.width == local.width);
}

/// Sparse rows swap-remove: removing a middle row moves the last row into its place, and freed
/// slots are reused by new generations. Every live node must still read its own row.
void test_sparse_rows_survive_compaction_and_slot_reuse() {
  Harness<MovingRecorder> h;
  CHECK(h.host != nullptr);
  if (!h.host) return;
  Nve2Writer& w = h.w;
  std::map<uint32_t, std::string> expect;  // packed id -> text
  w.create(1u);
  std::vector<uint32_t> kids;
  for (uint32_t id = 2u; id <= 9u; ++id) {
    w.create(id);
    expect[id] = "t" + std::to_string(id);
    CHECK(w.set_str(id, NVE_TABLE_TEXT_DESC, NVE_TABLE_TEXT_TEXT, expect[id]));
    kids.push_back(id);
  }
  w.set_children(1u, kids);
  CHECK(h.submit());
  (void)h.sync();
  const auto verify = [&](const char* step) {
    auto guard = h.host->runtime().acquire_scene();
    CHECK(guard.has_value());
    if (!guard) return;
    SceneTables t;
    t.bind(guard->reader());
    for (const auto& [id, text] : expect) {
      const uint32_t i = node_index(id);
      const std::string got(read_node(guard->reader(), t, i, TextStyle{}).text);
      if (got != text) fprintf(stderr, "%s: node %u reads \"%s\", expected \"%s\"\n", step, id, got.c_str(), text.c_str());
      CHECK(got == text);
      CHECK(h.rec.views.count(id) == 1u && h.rec.views[id].text == text);
    }
  };
  verify("mount");
  // Remove two middle rows and detach a third: the tail rows move into their places.
  w.remove(4u);
  w.remove(6u);
  w.detach(3u, NVE_TABLE_TEXT_DESC);
  expect.erase(4u);
  expect.erase(6u);
  expect[3u] = "";
  std::erase(kids, 4u);
  std::erase(kids, 6u);
  w.set_children(1u, kids);
  CHECK(h.submit());
  (void)h.sync();
  verify("compaction");
  // Reuse the freed slots with new generations, add fresh slots, and edit a moved row.
  for (const uint32_t id : {make_node_id(4u, 1u), make_node_id(6u, 1u), 10u, 11u}) {
    w.create(id);
    expect[id] = "n" + std::to_string(id);
    CHECK(w.set_str(id, NVE_TABLE_TEXT_DESC, NVE_TABLE_TEXT_TEXT, expect[id]));
    kids.push_back(id);
  }
  expect[9u] = "edited";
  CHECK(w.set_str(9u, NVE_TABLE_TEXT_DESC, NVE_TABLE_TEXT_TEXT, "edited"));
  w.set_children(1u, kids);
  CHECK(h.submit());
  (void)h.sync();
  verify("reuse");
}

/// Recreates a view whenever its inherited font size crosses 30: a cascade (no own change) can
/// replace a view, which must then be relinked into its parent and re-adopt its own children.
struct SizeTaggedRecorder : MovingRecorder {
  uint32_t tag(const NodeData& n) { return static_cast<uint32_t>(n.kind) | (n.text_style.font_size > 30.0f ? 256u : 0u); }
};

void test_cascade_recreated_view_is_relinked() {
  Harness<SizeTaggedRecorder> h;
  CHECK(h.host != nullptr);
  if (!h.host) return;
  Nve2Writer& w = h.w;
  for (uint32_t id = 1u; id <= 4u; ++id) w.create(id);  // 2 styled parent, 3 cascaded child, 4 grandchild
  w.set_f32(2u, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_FONT_SIZE, 16.0f);
  w.set_children(3u, std::vector<uint32_t>{4u});
  w.set_children(2u, std::vector<uint32_t>{3u});
  w.set_children(1u, std::vector<uint32_t>{2u});
  CHECK(h.submit());
  (void)h.sync();
  const int destroys = h.rec.destroys;
  w.set_f32(2u, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_FONT_SIZE, 40.0f);
  CHECK(h.submit());
  (void)h.sync();
  auto& v = h.rec.views;
  CHECK(h.rec.destroys == destroys + 3);  // 2 (own change) plus 3 and 4 through the cascade
  CHECK(h.rec.root_children == std::vector<uint32_t>{2u});
  CHECK(v[3].attached_to == 2u && v[2].children == std::vector<uint32_t>{3u});
  CHECK(v[4].attached_to == 3u && v[3].children == std::vector<uint32_t>{4u});
}

/// Two frames published between syncs: the skipped frame's change sets are gone, so the tree must
/// resync fully instead of applying only the latest frame's sets.
void test_skipped_frame_resyncs_fully() {
  Harness<MovingRecorder> h;
  CHECK(h.host != nullptr);
  if (!h.host) return;
  mount_column(h);
  CHECK(h.w.set_str(3u, NVE_TABLE_TEXT_DESC, NVE_TABLE_TEXT_TEXT, "first"));
  CHECK(h.submit());
  CHECK(h.w.set_str(4u, NVE_TABLE_TEXT_DESC, NVE_TABLE_TEXT_TEXT, "second"));
  CHECK(h.submit());
  (void)h.sync();
  CHECK(h.rec.views[3].text == "first" && h.rec.views[4].text == "second");
  // The skipped frame's `dirty_resources` were never seen: no id-keyed cache may be kept.
  const auto guard = h.host->runtime().acquire_scene();
  CHECK(guard.has_value());
  if (guard) CHECK(!h.tree.resource_unchanged(guard->reader(), 0u, 0u));
}

/// Moving a container moves only its view: views are positioned relative to their parent, so its
/// nested subtree (absolute rects shifted, parent-relative boxes unchanged) is not touched.
void test_container_move_leaves_its_subtree_alone() {
  Harness<MovingRecorder> h;
  CHECK(h.host != nullptr);
  if (!h.host) return;
  Nve2Writer& w = h.w;  // ROOT(1) -> column(2)[spacer(3), box(4)[row(5)[leaf(6), leaf(7)]]]
  for (uint32_t id = 1u; id <= 7u; ++id) w.create(id);
  w.set(2u, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_WIDTH, length_points(100.0f));
  set_height(w, 3u, 10.0f);
  set_height(w, 6u, 5.0f);
  set_height(w, 7u, 5.0f);
  w.set_children(5u, std::vector<uint32_t>{6u, 7u});
  w.set_children(4u, std::vector<uint32_t>{5u});
  w.set_children(2u, std::vector<uint32_t>{3u, 4u});
  w.set_children(1u, std::vector<uint32_t>{2u});
  CHECK(h.submit());
  CHECK(h.sync() == 7u);
  auto& v = h.rec.views;
  const NveLayoutRect leaf = v[7].local;
  set_height(w, 3u, 30.0f);
  CHECK(h.submit());
  CHECK(h.sync() == 1u && v[3].updates == 2);  // the spacer
  CHECK(h.rec.moves == 2 && v[2].moves == 1 && v[4].moves == 1);  // column (grew) and box (shifted)
  CHECK(v[4].local.y == 30.0f);
  for (const uint32_t id : {5u, 6u, 7u}) CHECK(v[id].moves == 0 && v[id].updates == 1);
  CHECK(v[7].local.y == leaf.y && v[7].local.height == leaf.height);
}

/// A resource republished under the same id re-runs `update` on exactly the clean views that read
/// it (image source, box shadows, a module-attached id), tracking re-pointed and destroyed users.
void test_dirty_resource_updates_its_clean_users() {
  Harness<testing::ModuleRecorder> h;
  CHECK(h.host != nullptr);
  if (!h.host) return;
  h.rec.module[5u] = 7u;  // node 5 also draws resource 7 (as a canvas would its draw list)
  Nve2Writer& w = h.w;
  const auto url = [](const char* u) { return encode_image_url(u); };
  w.define_resource(7u, kImageSrcKind, url("file:///tmp/a.png"));
  w.define_resource(8u, kBoxShadowsKind, encode_box_shadows(std::vector<BoxShadow>{BoxShadow{}}));
  w.set(3u, NVE_TABLE_IMAGE_DESC, NVE_TABLE_IMAGE_SRC, 7u);
  w.set(4u, NVE_TABLE_IMAGE_DESC, NVE_TABLE_IMAGE_SRC, 7u);
  w.set(5u, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BOX_SHADOWS, 8u);
  mount_column(h);
  auto& v = h.rec.views;
  const auto updates = [&] { return std::vector<int>{v[2].updates, v[3].updates, v[4].updates, v[5].updates}; };

  w.define_resource(7u, kImageSrcKind, url("file:///tmp/b.png"));  // same id, new bytes
  CHECK(h.submit());
  std::vector<int> before = updates();
  CHECK(h.sync() == 3u);
  CHECK((updates() == std::vector<int>{before[0], before[1] + 1, before[2] + 1, before[3] + 1}));
  {
    // The adapter re-decode contract: same id, incremental sync, and not redefined this frame.
    const auto guard = h.host->runtime().acquire_scene();
    CHECK(guard.has_value());
    if (!guard) return;
    const SceneReader& r = guard->reader();
    CHECK(r.resource_dirty(7u) && !r.resource_dirty(8u) && !r.resource_dirty(0u) && !r.resource_dirty(UINT32_MAX));
    CHECK(!h.tree.resource_unchanged(r, 7u, 7u));  // redefined
    CHECK(h.tree.resource_unchanged(r, 8u, 8u));   // kept its bytes
    CHECK(!h.tree.resource_unchanged(r, 8u, 7u));  // another id
  }

  w.define_resource(8u, kBoxShadowsKind, encode_box_shadows(std::vector<BoxShadow>{BoxShadow{}, BoxShadow{}}));
  CHECK(h.submit());
  before = updates();
  CHECK(h.sync() == 1u && v[5].updates == before[3] + 1);

  // A background fill (`nve.res.background_fill`, solid: u32 tag 0 + u32 RGBA) read by the column.
  const Key64 kFillKind = Key64::of("nve.res.background_fill");
  w.define_resource(12u, kFillKind, std::vector<uint8_t>{0u, 0u, 0u, 0u, 1u, 2u, 3u, 4u});
  w.set(2u, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND_FILL, 12u);
  CHECK(h.submit());
  (void)h.sync();
  w.define_resource(12u, kFillKind, std::vector<uint8_t>{0u, 0u, 0u, 0u, 9u, 8u, 7u, 6u});
  CHECK(h.submit());
  before = updates();
  CHECK(h.sync() == 1u);
  CHECK((updates() == std::vector<int>{before[0] + 1, before[1], before[2], before[3]}));

  // Node 3 re-points to 9: a later republish of 7 reaches only nodes 4 and 5; one of 9 only 3.
  w.define_resource(9u, kImageSrcKind, url("file:///tmp/c.png"));
  w.set(3u, NVE_TABLE_IMAGE_DESC, NVE_TABLE_IMAGE_SRC, 9u);
  CHECK(h.submit());
  (void)h.sync();
  w.define_resource(7u, kImageSrcKind, url("file:///tmp/d.png"));
  CHECK(h.submit());
  before = updates();
  CHECK(h.sync() == 2u);
  CHECK((updates() == std::vector<int>{before[0], before[1], before[2] + 1, before[3] + 1}));
  w.define_resource(9u, kImageSrcKind, url("file:///tmp/e.png"));
  CHECK(h.submit());
  before = updates();
  CHECK(h.sync() == 1u && v[3].updates == before[1] + 1);

  // Destroyed users are forgotten: with node 3 gone, a republish of 9 updates nothing.
  w.remove(3u);
  w.set_children(2u, std::vector<uint32_t>{4u, 5u});
  w.define_resource(9u, kImageSrcKind, url("file:///tmp/f.png"));
  CHECK(h.submit());
  (void)h.sync();
  w.define_resource(9u, kImageSrcKind, url("file:///tmp/g.png"));
  CHECK(h.submit());
  CHECK(h.sync() == 0u);
}

/// A new `scene_epoch` (the runtime discarded its scene) forces a full pass even when `frame_id`
/// is the next one and no removals were published.
void test_new_scene_epoch_runs_a_full_pass() {
  Harness<MovingRecorder> h;
  CHECK(h.host != nullptr);
  if (!h.host) return;
  using demo::CounterApp;
  mount_column(h);
  h.w.create(40u);  // a node the next app never has
  h.w.set_children(1u, std::vector<uint32_t>{2u, 40u});
  CHECK(h.submit());
  (void)h.sync();
  CHECK(h.rec.views.count(40u) == 1u);
  const int column_updates = h.rec.views[CounterApp::kColumn].updates;
  const int creates = h.rec.creates;
  const uint32_t epoch = testing::published_scene_epoch(*h.host);
  CHECK(h.host->attach(&CounterApp::get_interface));
  CHECK(testing::published_scene_epoch(*h.host) != epoch);
  CHECK(h.sync() > 0u);
  CHECK(h.rec.views.count(40u) == 0u);  // no removal was published: only the full pass drops it
  CHECK(h.rec.views[CounterApp::kTitle].text == "NVE native host");
  CHECK(h.rec.views[CounterApp::kColumn].updates > column_updates);  // same node: kept, re-applied
  CHECK(h.rec.creates < creates + 6);
  CHECK(h.rec.views[CounterApp::kCount].text == CounterApp::count_label(0));
  CHECK(h.rec.root_children == std::vector<uint32_t>{CounterApp::kColumn});
}

/// Adapter that only counts calls (no allocation of its own), for cost and allocation checks.
struct Counter {
  using Handle = uint32_t;
  size_t updates = 0u, moves = 0u;
  uint32_t tag(const NodeData& n) { return static_cast<uint32_t>(n.kind); }
  Handle create(const NodeData& n) { return n.id; }
  void update(Handle&, const NodeData&) { ++updates; }
  void move(Handle&, const NodeGeometry&) { ++moves; }
  void update_root(const NodeData&) {}
  void destroy(uint32_t, Handle&) {}
  void set_children(const NodeGeometry&, Handle*, std::span<Handle* const>) {}
};

/// 10k nodes: ROOT -> [filler(3) of 8996 text leaves, list(2) of 1000 rows]. A paint edit of one
/// node and a 1000-row shift must cost O(changed) adapter calls, and a warm sync allocates nothing.
void test_ten_thousand_nodes_sync_is_o_changed() {
  Harness<Counter> h;
  CHECK(h.host != nullptr);
  if (!h.host) return;
  Nve2Writer& w = h.w;
  constexpr uint32_t kRows = 1000u, kNodes = 10000u, kFirstRow = 4u, kFirstLeaf = kFirstRow + kRows;
  for (uint32_t id = 1u; id < kFirstLeaf + (kNodes - kFirstLeaf); ++id) w.create(id);
  std::vector<uint32_t> rows, leaves;
  for (uint32_t id = kFirstRow; id < kFirstLeaf; ++id) {
    set_height(w, id, 10.0f);
    rows.push_back(id);
  }
  for (uint32_t id = kFirstLeaf; id < kNodes; ++id) {
    CHECK(w.set_str(id, NVE_TABLE_TEXT_DESC, NVE_TABLE_TEXT_TEXT, "leaf"));
    set_height(w, id, 1.0f);
    leaves.push_back(id);
  }
  w.set_children(2u, rows);
  w.set_children(3u, leaves);
  w.set_children(1u, std::vector<uint32_t>{3u, 2u});  // the list last: shifting it moves nothing else
  // 100 leaves read one image resource (republished below).
  constexpr uint32_t kImage = 77u, kImageUsers = 100u;
  w.define_resource(kImage, kImageSrcKind, encode_image_url("file:///tmp/0.png"));
  for (uint32_t i = 0u; i < kImageUsers; ++i) w.set(kFirstLeaf + 100u + i, NVE_TABLE_IMAGE_DESC, NVE_TABLE_IMAGE_SRC, kImage);
  CHECK(h.submit());
  using Clock = std::chrono::steady_clock;
  const auto us = [](Clock::duration d) { return std::chrono::duration<double, std::micro>(d).count(); };
  auto t0 = Clock::now();
  CHECK(h.sync() == kNodes - 1u);  // first sync visits every live node
  const double full_us = us(Clock::now() - t0);

  // One paint edit, repeated: 1 update, 0 moves, zero allocations once warm.
  double paint_us = 0.0;
  size_t paint_allocs = 0u;
  for (uint32_t round = 0u; round < 8u; ++round) {
    w.set(kFirstLeaf + 7u, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, rgba(static_cast<uint8_t>(round), 1, 2));
    CHECK(h.submit());
    auto guard = h.host->runtime().acquire_scene();
    CHECK(guard.has_value());
    if (!guard) continue;
    const size_t updates = h.rec.updates, moves = h.rec.moves;
    const size_t before = g_allocs.load();
    t0 = Clock::now();
    const size_t updated = h.tree.sync(guard->reader(), h.tables, h.rec);
    paint_us = us(Clock::now() - t0);
    paint_allocs = g_allocs.load() - before;
    CHECK(updated == 1u && h.tree.moved() == 0u);
    CHECK(h.rec.updates == updates + 1u && h.rec.moves == moves);
  }
  CHECK(paint_allocs == 0u);

  // Shift the 1000 rows by toggling the first row's height: 1 update + 999 moves (+ list resize).
  double shift_us = 0.0;
  size_t shift_allocs = 0u, shift_updates = 0u, shift_moves = 0u;
  for (uint32_t round = 0u; round < 6u; ++round) {
    set_height(w, kFirstRow, round % 2u == 0u ? 20.0f : 10.0f);
    CHECK(h.submit());
    auto guard = h.host->runtime().acquire_scene();
    CHECK(guard.has_value());
    if (!guard) continue;
    const size_t updates = h.rec.updates, moves = h.rec.moves;
    const size_t before = g_allocs.load();
    t0 = Clock::now();
    shift_updates = h.tree.sync(guard->reader(), h.tables, h.rec);
    shift_us = us(Clock::now() - t0);
    shift_allocs = g_allocs.load() - before;
    shift_moves = h.tree.moved();
    CHECK(h.rec.updates == updates + shift_updates && h.rec.moves == moves + shift_moves);
  }
  CHECK(shift_updates == 1u);
  CHECK(shift_moves == kRows);  // 999 shifted rows + the auto-height list
  CHECK(shift_allocs == 0u);

  // Republishing the image under its id: exactly its 100 users update, zero allocations.
  double resource_us = 0.0;
  size_t resource_allocs = 0u;
  for (uint32_t round = 0u; round < 6u; ++round) {
    const std::string url = "file:///tmp/" + std::to_string(round + 1u) + ".png";
    w.define_resource(kImage, kImageSrcKind, encode_image_url(url));
    CHECK(h.submit());
    auto guard = h.host->runtime().acquire_scene();
    CHECK(guard.has_value());
    if (!guard) continue;
    const size_t updates = h.rec.updates;
    const size_t before = g_allocs.load();
    t0 = Clock::now();
    const size_t updated = h.tree.sync(guard->reader(), h.tables, h.rec);
    resource_us = us(Clock::now() - t0);
    resource_allocs = g_allocs.load() - before;
    CHECK(updated == kImageUsers && h.rec.updates == updates + kImageUsers && h.tree.moved() == 0u);
  }
  CHECK(resource_allocs == 0u);
  printf("retained_tree 10k nodes: full sync %.0f us; 1 paint edit %.1f us (%zu allocs); "
         "1000-row shift %.1f us (%zu updates, %zu moves, %zu allocs); "
         "resource republish %.1f us (%u updates, %zu allocs)\n",
         full_us, paint_us, paint_allocs, shift_us, shift_updates, shift_moves, shift_allocs, resource_us,
         kImageUsers, resource_allocs);
}

}  // namespace

int main() {
  test_moved_only_nodes_get_move_not_update();
  test_adapters_without_move_fall_back_to_update();
  test_parent_origin_move_refreshes_child_local();
  test_reparent_with_unchanged_absolute_rect();
  test_reparent_between_styled_parents_at_one_origin();
  test_sparse_rows_survive_compaction_and_slot_reuse();
  test_cascade_recreated_view_is_relinked();
  test_skipped_frame_resyncs_fully();
  test_container_move_leaves_its_subtree_alone();
  test_dirty_resource_updates_its_clean_users();
  test_new_scene_epoch_runs_a_full_pass();
  test_ten_thousand_nodes_sync_is_o_changed();
  if (g_failures != 0) {
    fprintf(stderr, "nve_host_retained_tree_test: %d failure(s)\n", g_failures);
    return 1;
  }
  printf("nve_host_retained_tree_test: ok\n");
  return 0;
}
