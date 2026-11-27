// Live `libnve_runtime` tests of `RetainedTree`'s O(changed) contract: geometry-only changes reach
// `move` (not `update`), adapters without `move` fall back to `update`, parent-relative origins stay
// exact across parent moves and reparents, sparse rows resolve correctly across swap-remove
// compaction and slot reuse, cascade-recreated views are relinked, skipped frames resync fully, and
// a steady-state sync of a 10k-node tree costs O(changed) work with zero heap allocations.
#include <atomic>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <map>
#include <new>
#include <string>
#include <vector>

#include "check.h"
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
  w.set_children(4u, std::vector<uint32_t>{5u});
  CHECK(h.submit());
  (void)h.sync();
  CHECK(v[5].attached_to == 4u && v[3].children.empty() && v[4].children == std::vector<uint32_t>{5u});
  CHECK(v[5].local.y == 0.0f && v[5].moves == 1);
}

/// Sparse rows swap-remove: removing a middle row moves the last row into its place, and freed
/// slots are reused by new generations. Every live node must still read its own row.
void test_sparse_rows_survive_compaction_and_slot_reuse() {
  Harness<MovingRecorder> h;
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
  if (!h.host) return;
  Nve2Writer& w = h.w;
  for (uint32_t id = 1u; id <= 4u; ++id) w.create(id);  // 2 styled parent, 3 cascaded child, 4 grandchild
  w.set(2u, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_SET, NVE_TYPOGRAPHY_SET_FONT_SIZE);
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
  if (!h.host) return;
  mount_column(h);
  CHECK(h.w.set_str(3u, NVE_TABLE_TEXT_DESC, NVE_TABLE_TEXT_TEXT, "first"));
  CHECK(h.submit());
  CHECK(h.w.set_str(4u, NVE_TABLE_TEXT_DESC, NVE_TABLE_TEXT_TEXT, "second"));
  CHECK(h.submit());
  (void)h.sync();
  CHECK(h.rec.views[3].text == "first" && h.rec.views[4].text == "second");
}

/// Adapter that only counts calls (no allocation of its own), for cost and allocation checks.
struct Counter {
  using Handle = uint32_t;
  size_t updates = 0u, moves = 0u;
  uint32_t tag(const NodeData& n) { return static_cast<uint32_t>(n.kind); }
  Handle create(const NodeData& n) { return n.id; }
  void update(Handle&, const NodeData&) { ++updates; }
  void move(Handle&, const NodeData&) { ++moves; }
  void update_root(const NodeData&) {}
  void destroy(uint32_t, Handle&) {}
  void set_children(const NodeData&, Handle*, std::span<Handle* const>) {}
};

/// 10k nodes: ROOT -> [filler(3) of 8996 text leaves, list(2) of 1000 rows]. A paint edit of one
/// node and a 1000-row shift must cost O(changed) adapter calls, and a warm sync allocates nothing.
void test_ten_thousand_nodes_sync_is_o_changed() {
  Harness<Counter> h;
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
    if (!guard) continue;
    const size_t before = g_allocs.load();
    t0 = Clock::now();
    const size_t updated = h.tree.sync(guard->reader(), h.tables, h.rec, h.host->source_generation());
    paint_us = us(Clock::now() - t0);
    paint_allocs = g_allocs.load() - before;
    CHECK(updated == 1u && h.tree.moved() == 0u);
  }
  CHECK(paint_allocs == 0u);

  // Shift the 1000 rows by toggling the first row's height: 1 update + 999 moves (+ list resize).
  double shift_us = 0.0;
  size_t shift_allocs = 0u, shift_updates = 0u, shift_moves = 0u;
  for (uint32_t round = 0u; round < 6u; ++round) {
    set_height(w, kFirstRow, round % 2u == 0u ? 20.0f : 10.0f);
    CHECK(h.submit());
    auto guard = h.host->runtime().acquire_scene();
    if (!guard) continue;
    const size_t before = g_allocs.load();
    t0 = Clock::now();
    shift_updates = h.tree.sync(guard->reader(), h.tables, h.rec, h.host->source_generation());
    shift_us = us(Clock::now() - t0);
    shift_allocs = g_allocs.load() - before;
    shift_moves = h.tree.moved();
  }
  CHECK(shift_updates == 1u);
  CHECK(shift_moves == kRows);  // 999 shifted rows + the auto-height list
  CHECK(shift_allocs == 0u);
  printf("retained_tree 10k nodes: full sync %.0f us; 1 paint edit %.1f us (%zu allocs); "
         "1000-row shift %.1f us (%zu updates, %zu moves, %zu allocs)\n",
         full_us, paint_us, paint_allocs, shift_us, shift_updates, shift_moves, shift_allocs);
}

}  // namespace

int main() {
  test_moved_only_nodes_get_move_not_update();
  test_adapters_without_move_fall_back_to_update();
  test_parent_origin_move_refreshes_child_local();
  test_reparent_with_unchanged_absolute_rect();
  test_sparse_rows_survive_compaction_and_slot_reuse();
  test_cascade_recreated_view_is_relinked();
  test_skipped_frame_resyncs_fully();
  test_ten_thousand_nodes_sync_is_o_changed();
  if (g_failures != 0) {
    fprintf(stderr, "nve_host_retained_tree_test: %d failure(s)\n", g_failures);
    return 1;
  }
  printf("nve_host_retained_tree_test: ok\n");
  return 0;
}
