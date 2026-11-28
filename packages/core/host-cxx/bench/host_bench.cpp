// C++20 host read-path benchmarks: what every C++ host (`hosts/*-shell`, `apple-common`) pays per
// frame on top of `libnve_runtime`. `SceneReader::open` validates the borrowed `NveSceneView` once
// (UTF-8 text slab, table views + slot uniqueness, dirty / moved index ranges, sorted scroll
// offsets and dirty resources); a retained host then reads the dirty rows, rects, and a dense and
// a sparse table field per node. Synthetic flat trees of N children under one root, shaped like a
// real frame: a multi-byte UTF-8 text slab, one dense and one sparse (paged row index) table,
// scroll offsets and dirty resources.
//
//   nve_host_cxx_bench [--json] [--warmup N] [--iters N] [--filter substring]
//
// `--json` prints `{"suite": ..., "benchmarks": [{name, p50_ns, p95_ns, mean_ns, ops_per_sec}]}`
// for `scripts/bench-regress.py --cxx-bin`.
#include <nve/host/scene_reader.hpp>

#include <algorithm>
#include <chrono>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <vector>

#if defined(_MSC_VER)
#include <intrin.h>
#endif

namespace {

using nve::host::Key64;
using nve::host::SceneReader;

constexpr uint32_t kTableWords = 4;
constexpr uint32_t kDenseSlot = 0;
constexpr uint32_t kSparseSlot = 37;  // a second `table_set` word
constexpr uint32_t kSparseWords = 2;
constexpr auto kOpacity = nve::host::field(0, 0, 8, NVE_FIELD_U32);
constexpr auto kSparseValue = nve::host::field(1, 0, 32, NVE_FIELD_U32);

/// One published frame: `nodes` children under root slot 1, every node in the dense table, every
/// eighth in the sparse one; each node owns a label in the text slab.
struct Frame {
  std::vector<NveNodeRow> rows;
  std::vector<NveLayoutRect> rects;
  std::vector<uint32_t> children;
  std::vector<uint32_t> dirty;
  std::vector<uint32_t> moved;
  std::string text;
  std::vector<uint32_t> table_rows;
  std::vector<uint32_t> sparse_rows;
  std::vector<uint32_t> sparse_owners;
  std::vector<uint32_t> sparse_dir;
  std::vector<uint32_t> sparse_pages;
  std::vector<NveScrollOffset> scroll;
  std::vector<uint32_t> dirty_resources;
  NveTableView tables[2]{};
  NveSceneView view{};

  Frame(uint32_t nodes, uint32_t dirty_count) {
    const uint32_t capacity = nodes + 2u;  // null slot 0 + root slot 1
    rows.resize(capacity);
    rects.resize(capacity);
    table_rows.assign(static_cast<size_t>(capacity) * kTableWords, 0u);
    const uint32_t pages = (capacity + NVE_ROW_INDEX_PAGE_LEN - 1u) / NVE_ROW_INDEX_PAGE_LEN;
    sparse_dir.resize(pages);
    sparse_pages.assign(static_cast<size_t>(pages) * NVE_ROW_INDEX_PAGE_LEN, 0u);
    for (uint32_t p = 0; p < pages; ++p) sparse_dir[p] = p;
    for (uint32_t i = 1; i < capacity; ++i) {
      NveNodeRow& r = rows[i];
      r.table_set[0] = 1u << kDenseSlot;
      r.parent = i == 1u ? 0u : 1u;
      r.generation = 1u;
      r.primary_slot = static_cast<uint8_t>(kDenseSlot);
      r.flags = static_cast<uint8_t>(NVE_NODE_ROW_FLAG_LIVE);
      rects[i] = NveLayoutRect{0.0f, static_cast<float>(i) * 20.0f, 320.0f, 20.0f};
      table_rows[static_cast<size_t>(i) * kTableWords] = i & 0xFFu;
      if (i > 1u) children.push_back((1u << 24) | i);
      text += "Zeile ";
      text += std::to_string(i);
      text += " \xC3\xBC\xE2\x82\xAC\xF0\x9F\x9A\x80 ";  // "ü€🚀": 2-, 3- and 4-byte sequences
      if (i % 8u == 0u) {
        r.table_set[kSparseSlot >> 5u] |= 1u << (kSparseSlot & 31u);
        sparse_owners.push_back((1u << 24) | i);
        sparse_rows.push_back(0u);
        sparse_rows.push_back(i * 3u);
        sparse_pages[i] = static_cast<uint32_t>(sparse_owners.size());  // `row + 1`
      }
      if (i % 64u == 0u) scroll.push_back(NveScrollOffset{(1u << 24) | i, 0.0f, static_cast<float>(i)});
      // Steady-state frame: only a small bounded set of resources change per frame (matches `dirty_count`).
      if (i % 16u == 0u && dirty_resources.size() < dirty_count) dirty_resources.push_back(i);
    }
    rows[1].child_off = 0u;
    rows[1].child_len = nodes;
    for (uint32_t d = 0; d < dirty_count; ++d) {
      dirty.push_back(2u + d * (nodes / std::max(dirty_count, 1u)));
      moved.push_back(3u + d * (nodes / std::max(dirty_count, 1u)));
    }
    const Key64 dense = Key64::of("bench.visual");
    const Key64 sparse = Key64::of("bench.sparse");
    tables[0] = NveTableView{dense.lo(), dense.hi(), kDenseSlot, kTableWords, capacity, 0u, 0u, 0u,
                             table_rows.data(), nullptr, nullptr, nullptr, nullptr};
    tables[1] = NveTableView{sparse.lo(), sparse.hi(), kSparseSlot, kSparseWords,
                             static_cast<uint32_t>(sparse_owners.size()), 0u, pages, pages, sparse_rows.data(),
                             sparse_owners.data(), nullptr, sparse_dir.data(), sparse_pages.data()};
    view.schema_hash = NVE_RUNTIME_SCHEMA_HASH;
    view.frame_id = 1u;
    view.node_capacity = capacity;
    view.rows = rows.data();
    view.rects = rects.data();
    view.text_slab = reinterpret_cast<const uint8_t*>(text.data());
    view.text_slab_len = static_cast<uint32_t>(text.size());
    view.children_slab = children.data();
    view.children_slab_len = static_cast<uint32_t>(children.size());
    view.dirty_indices = dirty.data();
    view.dirty_count = static_cast<uint32_t>(dirty.size());
    view.moved_indices = moved.data();
    view.moved_count = static_cast<uint32_t>(moved.size());
    view.tables = tables;
    view.table_count = 2u;
    view.scroll_offsets = scroll.data();
    view.scroll_count = static_cast<uint32_t>(scroll.size());
    view.dirty_resources = dirty_resources.data();
    view.dirty_resource_count = static_cast<uint32_t>(dirty_resources.size());
  }
};

struct Result {
  std::string name;
  double p50_ns, p95_ns, mean_ns;
  int iters;
};

volatile uint64_t g_sink = 0;  // written once per batch: keeps the results observable

/// Compiler barrier: prevents hoisting pure `body()` loads or folding repeated batch iterations.
template <class T>
inline void do_not_optimize(T& value) noexcept {
#if defined(_MSC_VER)
  _ReadWriteBarrier();
  (void)value;
#else
  asm volatile("" : "+r"(value) :: "memory");
#endif
}

/// Per-call nanoseconds; each sample times `batch` calls so sub-timer-resolution work is
/// measurable. `body` is inlined (no type-erased call inside the timed loop); `do_not_optimize`
/// keeps each iteration live and results fold into `g_sink` once per batch.
template <class Body>
Result measure(std::string name, int warmup, int iters, int batch, Body&& body) {
  uint64_t acc = 0;
  for (int i = 0; i < warmup * batch; ++i) {
    uint64_t r = body();
    do_not_optimize(r);
    acc += r;
  }
  std::vector<double> samples(static_cast<size_t>(iters));
  for (double& s : samples) {
    const auto t0 = std::chrono::steady_clock::now();
    for (int b = 0; b < batch; ++b) {
      uint64_t r = body();
      do_not_optimize(r);
      acc += r;
    }
    s = std::chrono::duration<double, std::nano>(std::chrono::steady_clock::now() - t0).count() / batch;
    g_sink = acc;
  }
  std::sort(samples.begin(), samples.end());
  double sum = 0.0;
  for (const double s : samples) sum += s;
  const auto at = [&](double q) { return samples[static_cast<size_t>(q * static_cast<double>(samples.size() - 1u))]; };
  return Result{std::move(name), at(0.5), at(0.95), sum / static_cast<double>(samples.size()), iters};
}

/// Sum of a dense-table field and rect heights over `indices` (a retained host's per-node read).
uint64_t read_nodes(const SceneReader& scene, std::span<const uint32_t> indices) {
  const auto table = scene.table(0);
  uint64_t acc = 0;
  for (const uint32_t i : indices) {
    if (!scene.is_live(i)) continue;
    acc += scene.rect(i)->height > 0.0f ? 1u : 0u;
    acc += table->u32(table->row_of_node(i).value_or(0u), kOpacity).value_or(0u);
  }
  return acc;
}

/// Sum of a sparse-table field over `indices` (paged row index lookups, hits and misses).
uint64_t read_sparse(const SceneReader& scene, std::span<const uint32_t> indices) {
  const auto table = scene.table(1);
  uint64_t acc = 0;
  for (const uint32_t i : indices) {
    if (const auto row = table->row_of_node(i)) acc += table->u32(*row, kSparseValue).value_or(0u);
  }
  return acc;
}

}  // namespace

int main(int argc, char** argv) {
  bool json = false;
  int warmup_override = -1, iters_override = -1;
  std::string_view filter;
  for (int i = 1; i < argc; ++i) {
    const std::string_view arg(argv[i]);
    if (arg == "--json") json = true;
    else if (arg == "--warmup" && i + 1 < argc) warmup_override = std::atoi(argv[++i]);
    else if (arg == "--iters" && i + 1 < argc) iters_override = std::atoi(argv[++i]);
    else if (arg == "--filter" && i + 1 < argc) filter = argv[++i];
    else {
      std::fprintf(stderr, "usage: %s [--json] [--warmup N] [--iters N] [--filter substring]\n", argv[0]);
      return 2;
    }
  }

  std::vector<Result> results;
  const auto run = [&](std::string name, int warmup, int iters, int batch, auto&& body) {
    if (!filter.empty() && name.find(filter) == std::string::npos) return;
    results.push_back(measure(std::move(name), warmup_override >= 0 ? warmup_override : warmup,
                              std::max(1, iters_override > 0 ? iters_override : iters), batch, body));
  };

  for (const uint32_t nodes : {1000u, 10000u}) {
    const Frame frame(nodes, 10u);
    const std::string n = std::to_string(nodes);
    // A volatile view pointer: the compiler cannot hoist the validation out of the loop.
    const NveSceneView* volatile view = &frame.view;
    run("scene_reader_open_" + n, 10, 500, 20, [&] {
      return static_cast<uint64_t>(SceneReader::open(view)->node_capacity());
    });
    const SceneReader scene = *SceneReader::open(&frame.view);
    run("scene_read_dirty_10_of_" + n, 10, 500, 200, [&] { return read_nodes(scene, scene.dirty_indices()); });
    std::vector<uint32_t> all;  // every child slot, resolved from the root's child ids
    const std::span<const uint32_t> kids = scene.node_children(1).value();
    for (const uint32_t id : kids) all.push_back(id & 0xFFFFFFu);
    run("scene_read_all_" + n, 10, nodes >= 10000u ? 200 : 1000, 1, [&] { return read_nodes(scene, all); });
    run("scene_read_sparse_all_" + n, 10, nodes >= 10000u ? 200 : 1000, 1, [&] { return read_sparse(scene, all); });
  }

  if (json) {
    std::printf("{\n  \"suite\": \"nve_host_cxx_bench\",\n  \"benchmarks\": [\n");
    for (size_t i = 0; i < results.size(); ++i) {
      const Result& r = results[i];
      std::printf("    {\"name\": \"%s\", \"p50_ns\": %.1f, \"p95_ns\": %.1f, \"mean_ns\": %.1f, \"ops_per_sec\": %.1f, \"iters\": %d}%s\n",
                  r.name.c_str(), r.p50_ns, r.p95_ns, r.mean_ns, r.p50_ns > 0.0 ? 1e9 / r.p50_ns : 0.0, r.iters,
                  i + 1u < results.size() ? "," : "");
    }
    std::printf("  ]\n}\n");
  } else {
    for (const Result& r : results) {
      std::printf("%-32s p50 %10.1f ns  p95 %10.1f ns  mean %10.1f ns  (%d iters)\n", r.name.c_str(), r.p50_ns,
                  r.p95_ns, r.mean_ns, r.iters);
    }
  }
  return 0;
}
