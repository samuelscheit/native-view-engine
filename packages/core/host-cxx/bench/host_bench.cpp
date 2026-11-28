// C++20 host read-path benchmarks: what every C++ host (`hosts/*-shell`, `apple-common`) pays per
// frame on top of `libnve_runtime`. `SceneReader::open` validates the borrowed `NveSceneView` once
// (O(tables + scroll + dirty resources)); a retained host then reads the dirty rows, rects, and a
// table field per node. Synthetic flat trees of N children under one root, one dense table.
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
#include <functional>
#include <string>
#include <string_view>
#include <vector>

namespace {

using nve::host::Key64;
using nve::host::SceneReader;

constexpr uint32_t kTableWords = 4;
constexpr uint32_t kTableSlot = 0;
constexpr auto kOpacity = nve::host::field(0, 0, 8, NVE_FIELD_U32);

/// One published frame: `nodes` children under root slot 1, every node in one dense table.
struct Frame {
  std::vector<NveNodeRow> rows;
  std::vector<NveLayoutRect> rects;
  std::vector<uint32_t> children;
  std::vector<uint32_t> dirty;
  std::vector<uint32_t> table_rows;
  NveTableView table{};
  NveSceneView view{};

  Frame(uint32_t nodes, uint32_t dirty_count) {
    const uint32_t capacity = nodes + 2u;  // null slot 0 + root slot 1
    rows.resize(capacity);
    rects.resize(capacity);
    table_rows.assign(static_cast<size_t>(capacity) * kTableWords, 0u);
    for (uint32_t i = 1; i < capacity; ++i) {
      NveNodeRow& r = rows[i];
      r.table_set[0] = 1u << kTableSlot;
      r.parent = i == 1u ? 0u : 1u;
      r.generation = 1u;
      r.primary_slot = static_cast<uint8_t>(kTableSlot);
      r.flags = static_cast<uint8_t>(NVE_NODE_ROW_FLAG_LIVE);
      rects[i] = NveLayoutRect{0.0f, static_cast<float>(i) * 20.0f, 320.0f, 20.0f};
      table_rows[static_cast<size_t>(i) * kTableWords] = i & 0xFFu;
      if (i > 1u) children.push_back((1u << 24) | i);
    }
    rows[1].child_off = 0u;
    rows[1].child_len = nodes;
    for (uint32_t d = 0; d < dirty_count; ++d) dirty.push_back(2u + d * (nodes / std::max(dirty_count, 1u)));
    const Key64 key = Key64::of("bench.visual");
    table = NveTableView{key.lo(), key.hi(), kTableSlot, kTableWords, capacity, 0u, 0u, 0u,
                         table_rows.data(), nullptr, nullptr, nullptr, nullptr};
    view.schema_hash = NVE_RUNTIME_SCHEMA_HASH;
    view.frame_id = 1u;
    view.node_capacity = capacity;
    view.rows = rows.data();
    view.rects = rects.data();
    view.children_slab = children.data();
    view.children_slab_len = static_cast<uint32_t>(children.size());
    view.dirty_indices = dirty.data();
    view.dirty_count = static_cast<uint32_t>(dirty.size());
    view.tables = &table;
    view.table_count = 1u;
  }
};

struct Result {
  std::string name;
  double p50_ns, p95_ns, mean_ns;
  int iters;
};

volatile uint64_t g_sink = 0;  // keeps reads observable

/// Per-call nanoseconds; each sample times `batch` calls so sub-timer-resolution work is measurable.
Result measure(std::string name, int warmup, int iters, int batch, const std::function<uint64_t()>& body) {
  for (int i = 0; i < warmup * batch; ++i) g_sink = g_sink + body();
  std::vector<double> samples(static_cast<size_t>(iters));
  for (double& s : samples) {
    const auto t0 = std::chrono::steady_clock::now();
    for (int b = 0; b < batch; ++b) g_sink = g_sink + body();
    s = std::chrono::duration<double, std::nano>(std::chrono::steady_clock::now() - t0).count() / batch;
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
  const auto run = [&](std::string name, int warmup, int iters, int batch, const std::function<uint64_t()>& body) {
    if (!filter.empty() && name.find(filter) == std::string::npos) return;
    results.push_back(measure(std::move(name), warmup_override >= 0 ? warmup_override : warmup,
                              std::max(1, iters_override > 0 ? iters_override : iters), batch, body));
  };

  for (const uint32_t nodes : {1000u, 10000u}) {
    const Frame frame(nodes, 10u);
    const std::string n = std::to_string(nodes);
    run("scene_reader_open_" + n, 10, 500, 200, [&] {
      return static_cast<uint64_t>(SceneReader::open(&frame.view)->node_capacity());
    });
    const SceneReader scene = *SceneReader::open(&frame.view);
    run("scene_read_dirty_10_of_" + n, 10, 500, 200, [&] { return read_nodes(scene, scene.dirty_indices()); });
    std::vector<uint32_t> all;  // every child slot, resolved from the root's child ids
    const std::span<const uint32_t> kids = scene.node_children(1).value();
    for (const uint32_t id : kids) all.push_back(id & 0xFFFFFFu);
    run("scene_read_all_" + n, 10, nodes >= 10000u ? 200 : 1000, 1, [&] { return read_nodes(scene, all); });
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
