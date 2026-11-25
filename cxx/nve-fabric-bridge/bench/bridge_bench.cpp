#include "command_encoder.hpp"
#include "mock_jsi_recorder.hpp"
#include "nve_abi.h"
#include "nve_host_runtime.hpp"
#include "nve_rn_component.hpp"

#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <functional>
#include <iomanip>
#include <iostream>
#include <numeric>
#include <string>
#include <string_view>
#include <vector>

namespace {

using Clock = std::chrono::steady_clock;

volatile uint64_t g_bench_sink = 0;

template <typename T>
inline void do_not_optimize(const T& value) {
  uint64_t bits = 0;
  std::memcpy(&bits, &value, sizeof(T) < sizeof(bits) ? sizeof(T) : sizeof(bits));
  g_bench_sink ^= bits;
}

struct BenchConfig {
  bool json_output{false};
  int warmup_override{-1};
  int iters_override{-1};
  std::string filter;
};

struct BenchResult {
  std::string name;
  int warmup{0};
  int iterations{0};
  uint64_t ops_per_iter{1};
  double p50_ns{0.0};
  double p95_ns{0.0};
  double p99_ns{0.0};
  double mean_ns{0.0};
  double min_ns{0.0};
  double max_ns{0.0};
  double ns_per_op{0.0};
  double ops_per_sec{0.0};
};

double percentile(const std::vector<double>& sorted, double p) {
  if (sorted.empty()) return 0.0;
  if (sorted.size() == 1) return sorted.front();
  const double idx = (p / 100.0) * static_cast<double>(sorted.size() - 1);
  const size_t lo = static_cast<size_t>(std::floor(idx));
  const size_t hi = static_cast<size_t>(std::ceil(idx));
  const double frac = idx - static_cast<double>(lo);
  return sorted[lo] + (sorted[hi] - sorted[lo]) * frac;
}

template <typename SetupFn, typename BodyFn, typename TeardownFn>
BenchResult run_bench_with_hooks(
    std::string name,
    int default_warmup,
    int default_iters,
    uint64_t ops_per_iter,
    const BenchConfig& cfg,
    SetupFn&& setup,
    BodyFn&& body,
    TeardownFn&& teardown) {
  const int warmup = cfg.warmup_override >= 0 ? cfg.warmup_override : default_warmup;
  const int iters = cfg.iters_override > 0 ? cfg.iters_override : default_iters;

  for (int i = 0; i < warmup; ++i) {
    setup(i);
    body(i);
    teardown(i);
  }

  std::vector<double> samples;
  samples.reserve(static_cast<size_t>(iters));
  for (int i = 0; i < iters; ++i) {
    setup(warmup + i);
    const auto t0 = Clock::now();
    body(warmup + i);
    const auto t1 = Clock::now();
    teardown(warmup + i);
    const double elapsed_ns =
        std::chrono::duration<double, std::nano>(t1 - t0).count();
    samples.push_back(elapsed_ns);
  }

  std::sort(samples.begin(), samples.end());
  const double sum = std::accumulate(samples.begin(), samples.end(), 0.0);
  const double mean_ns = samples.empty() ? 0.0 : sum / static_cast<double>(samples.size());
  const double p50_ns = percentile(samples, 50.0);
  const double p95_ns = percentile(samples, 95.0);
  const double p99_ns = percentile(samples, 99.0);
  const double ns_per_op = p50_ns / static_cast<double>(ops_per_iter > 0 ? ops_per_iter : 1);
  const double ops_per_sec = ns_per_op > 0.0 ? (1e9 / ns_per_op) : 0.0;

  return BenchResult{
      .name = std::move(name),
      .warmup = warmup,
      .iterations = iters,
      .ops_per_iter = ops_per_iter,
      .p50_ns = p50_ns,
      .p95_ns = p95_ns,
      .p99_ns = p99_ns,
      .mean_ns = mean_ns,
      .min_ns = samples.empty() ? 0.0 : samples.front(),
      .max_ns = samples.empty() ? 0.0 : samples.back(),
      .ns_per_op = ns_per_op,
      .ops_per_sec = ops_per_sec,
  };
}

template <typename BodyFn>
BenchResult run_bench(
    std::string name,
    int default_warmup,
    int default_iters,
    uint64_t ops_per_iter,
    const BenchConfig& cfg,
    BodyFn&& body) {
  return run_bench_with_hooks(
      std::move(name),
      default_warmup,
      default_iters,
      ops_per_iter,
      cfg,
      [](int) {},
      std::forward<BodyFn>(body),
      [](int) {});
}

bool should_run(std::string_view name, const BenchConfig& cfg) {
  return cfg.filter.empty() || name.find(cfg.filter) != std::string_view::npos;
}

/// Build a binary CommandBuffer mounting `total_nodes` shadow nodes (excluding root):
/// root -> list container (id=2) -> `row_count` rows, each containing:
///   row container (View) -> [avatar (View), title (Text), subtitle (Text), action (Pressable)]
/// Total nodes created = 1 (list) + row_count * 5 (trimmed to exact `total_nodes`).
struct BuiltTreeBuffer {
  std::vector<uint8_t> bytes;
  std::vector<uint32_t> text_node_ids;
  std::vector<uint32_t> row_node_ids;
  uint32_t total_nodes{0};
};

BuiltTreeBuffer encode_tree_mount(uint32_t surface_id, uint64_t frame_id, uint32_t target_nodes) {
  BuiltTreeBuffer out;
  nve::CommandBufferEncoder enc(surface_id, frame_id);
  enc.reserve(static_cast<size_t>(target_nodes) * 160u);

  const uint32_t list_id = 2;
  enc.create_node(list_id, NVE_COMP_SCROLL_VIEW);
  nve::LayoutProps list_layout{};
  list_layout.width = {2, 100.0f};
  list_layout.height = {2, 100.0f};
  list_layout.flex_direction = 0; /* Column */
  list_layout.padding = {12.0f, 12.0f, 12.0f, 12.0f};
  list_layout.gap = 8.0f;
  enc.set_layout(list_id, list_layout);

  uint32_t next_id = 3;
  uint32_t created = 1; /* list_id */

  nve::LayoutProps row_layout{};
  row_layout.width = {2, 100.0f};
  row_layout.height = {1, 56.0f};
  row_layout.flex_direction = 1; /* Row */
  row_layout.align_items = 2;    /* Center */
  row_layout.padding = {8.0f, 12.0f, 8.0f, 12.0f};
  row_layout.gap = 10.0f;

  nve::LayoutProps avatar_layout{};
  avatar_layout.width = {1, 36.0f};
  avatar_layout.height = {1, 36.0f};

  nve::LayoutProps title_layout{};
  title_layout.flex_grow = 1.0f;

  nve::LayoutProps badge_layout{};
  badge_layout.width = {1, 64.0f};
  badge_layout.height = {1, 28.0f};

  nve::StyleProps row_style{};
  row_style.background_rgba = 0xFF1E222Au;
  row_style.border_radius = 8.0f;

  nve::StyleProps title_style{};
  title_style.text_color_rgba = 0xFFF2F5F9u;
  title_style.font_size = 15.0f;

  nve::StyleProps sub_style{};
  sub_style.text_color_rgba = 0xFFA0AEC0u;
  sub_style.font_size = 12.0f;

  int row_idx = 0;
  while (created < target_nodes) {
    const uint32_t row_id = next_id++;
    ++created;
    enc.create_node(row_id, NVE_COMP_VIEW);
    enc.set_layout(row_id, row_layout);
    enc.set_style(row_id, row_style);
    out.row_node_ids.push_back(row_id);

    std::vector<uint32_t> row_children;
    if (created < target_nodes) {
      const uint32_t avatar_id = next_id++;
      ++created;
      enc.create_node(avatar_id, NVE_COMP_VIEW);
      enc.set_layout(avatar_id, avatar_layout);
      row_children.push_back(avatar_id);
    }
    if (created < target_nodes) {
      const uint32_t title_id = next_id++;
      ++created;
      enc.create_node(title_id, NVE_COMP_TEXT);
      enc.set_layout(title_id, title_layout);
      enc.set_style(title_id, title_style);
      enc.set_text(title_id, "Benchmark item #" + std::to_string(row_idx) + " primary label");
      out.text_node_ids.push_back(title_id);
      row_children.push_back(title_id);
    }
    if (created < target_nodes) {
      const uint32_t sub_id = next_id++;
      ++created;
      enc.create_node(sub_id, NVE_COMP_TEXT);
      enc.set_style(sub_id, sub_style);
      enc.set_text(sub_id, "Subtitle status " + std::to_string(row_idx % 10));
      out.text_node_ids.push_back(sub_id);
      row_children.push_back(sub_id);
    }
    if (created < target_nodes) {
      const uint32_t btn_id = next_id++;
      ++created;
      enc.create_node(btn_id, NVE_COMP_PRESSABLE);
      enc.set_layout(btn_id, badge_layout);
      enc.set_event_mask(btn_id, 1ULL << 0); /* PRESS */
      row_children.push_back(btn_id);
    }
    if (!row_children.empty()) {
      enc.set_children(row_id, row_children);
    }
    ++row_idx;
  }

  enc.set_children(NVE_ROOT_NODE_ID, {list_id});
  enc.set_children(list_id, out.row_node_ids);
  enc.commit(true);
  const NveBufferSlice slice = enc.finish();
  out.bytes.assign(slice.ptr, slice.ptr + slice.len);
  out.total_nodes = created;
  return out;
}

/// Build a Yoga layout benchmark tree with `row_count` flexbox rows, each having a flex
/// badge + wrapping text node so Yoga exercises both flex distribution and text measurement.
BuiltTreeBuffer encode_flex_rows_tree(uint32_t surface_id, uint64_t frame_id, uint32_t row_count) {
  BuiltTreeBuffer out;
  nve::CommandBufferEncoder enc(surface_id, frame_id);
  enc.reserve(static_cast<size_t>(row_count) * 420u);

  const uint32_t list_id = 2;
  enc.create_node(list_id, NVE_COMP_VIEW);
  nve::LayoutProps list_layout{};
  list_layout.width = {2, 100.0f};
  list_layout.flex_direction = 0; /* Column */
  list_layout.padding = {8.0f, 8.0f, 8.0f, 8.0f};
  list_layout.gap = 6.0f;
  enc.set_layout(list_id, list_layout);

  nve::LayoutProps row_layout{};
  row_layout.width = {2, 100.0f};
  row_layout.flex_direction = 1; /* Row */
  row_layout.justify_content = 3; /* SpaceBetween */
  row_layout.align_items = 2; /* Center */
  row_layout.padding = {6.0f, 8.0f, 6.0f, 8.0f};
  row_layout.gap = 8.0f;

  nve::LayoutProps icon_layout{};
  icon_layout.width = {1, 32.0f};
  icon_layout.height = {1, 32.0f};

  nve::LayoutProps text_layout{};
  text_layout.flex_grow = 1.0f;
  text_layout.flex_shrink = 1.0f;

  nve::LayoutProps pill_layout{};
  pill_layout.width = {1, 48.0f};
  pill_layout.height = {1, 24.0f};

  nve::StyleProps text_style{};
  text_style.text_color_rgba = 0xFF111111u;
  text_style.font_size = 14.0f;

  uint32_t next_id = 3;
  for (uint32_t r = 0; r < row_count; ++r) {
    const uint32_t row_id = next_id++;
    const uint32_t icon_id = next_id++;
    const uint32_t text_id = next_id++;
    const uint32_t pill_id = next_id++;

    enc.create_node(row_id, NVE_COMP_VIEW);
    enc.set_layout(row_id, row_layout);

    enc.create_node(icon_id, NVE_COMP_VIEW);
    enc.set_layout(icon_id, icon_layout);

    enc.create_node(text_id, NVE_COMP_TEXT);
    enc.set_layout(text_id, text_layout);
    enc.set_style(text_id, text_style);
    enc.set_text(
        text_id,
        "Row " + std::to_string(r) +
            ": Native View Engine zero-JS Fabric Yoga flexbox measurement workload");

    enc.create_node(pill_id, NVE_COMP_VIEW);
    enc.set_layout(pill_id, pill_layout);

    enc.set_children(row_id, {icon_id, text_id, pill_id});
    out.row_node_ids.push_back(row_id);
    out.text_node_ids.push_back(text_id);
  }

  enc.set_children(NVE_ROOT_NODE_ID, {list_id});
  enc.set_children(list_id, out.row_node_ids);
  enc.commit(true);
  const NveBufferSlice slice = enc.finish();
  out.bytes.assign(slice.ptr, slice.ptr + slice.len);
  out.total_nodes = 1 + row_count * 4;
  return out;
}

/// Deterministic fast custom TextMeasurerCallback simulating CoreText / Skia glyph shaping.
NveIntrinsicSize fast_custom_text_measurer(
    std::string_view text,
    float font_size,
    float max_width) {
  if (text.empty()) {
    return NveIntrinsicSize{0.0f, 0.0f};
  }
  const float advance = font_size * 0.58f;
  const float line_h = font_size * 1.35f;
  float total_w = 0.0f;
  for (unsigned char ch : text) {
    total_w += (ch == ' ' || ch == 'i' || ch == 'l') ? (advance * 0.65f) : advance;
  }
  if (max_width > 0.0f && total_w > max_width) {
    const float lines = std::ceil(total_w / max_width);
    return NveIntrinsicSize{max_width, lines * line_h};
  }
  return NveIntrinsicSize{total_w, line_h};
}

/// Minimal embedded component implementing `nve_get_interface` for C ABI + bridge roundtrip benchmarks.
struct BenchComponent {
  uint32_t surface_id{1};
  uint64_t presses{0};
  bool mounted{false};
  nve::CommandBufferEncoder encoder{1, 1};
  nve::EventRingWriter outbound_events;
  std::vector<uint8_t> outbound_snapshot;

  static NveAppHandle create(uint32_t sid) {
    auto* self = new BenchComponent();
    self->surface_id = sid;
    self->outbound_events.clear();
    return self;
  }

  static void destroy(NveAppHandle h) {
    delete static_cast<BenchComponent*>(h);
  }

  static NveBufferSlice render(NveAppHandle h, uint64_t frame_id) {
    auto* self = static_cast<BenchComponent*>(h);
    self->encoder.reset(self->surface_id, frame_id);
    if (!self->mounted) {
      self->encoder.create_node(2, NVE_COMP_PRESSABLE);
      self->encoder.create_node(3, NVE_COMP_TEXT);
      self->encoder.set_children(NVE_ROOT_NODE_ID, {2});
      self->encoder.set_children(2, {3});
      nve::LayoutProps lp{};
      lp.width = {1, 240.0f};
      lp.height = {1, 48.0f};
      lp.align_items = 2;
      lp.justify_content = 1;
      self->encoder.set_layout(2, lp);
      self->encoder.set_event_mask(2, 1ULL << 0);
      self->mounted = true;
    }
    char label[48];
    const int n = std::snprintf(label, sizeof(label), "Presses: %llu",
                                static_cast<unsigned long long>(self->presses));
    self->encoder.set_text(3, std::string_view(label, n > 0 ? static_cast<size_t>(n) : 0));
    self->encoder.commit(true);
    return self->encoder.finish();
  }

  static uint32_t dispatch_events(NveAppHandle h, NveBufferSlice events) {
    auto* self = static_cast<BenchComponent*>(h);
    std::vector<nve::DecodedEvent> decoded;
    if (!nve::decode_event_ring(events, decoded)) return 0;
    uint32_t dirty = 0;
    for (const auto& ev : decoded) {
      if (ev.kind == nve::EventKind::Press && ev.node_id == 2) {
        ++self->presses;
        const NveMapEntry entry{
            nve::make_str_slice("presses"),
            nve::make_i64_value(static_cast<int64_t>(self->presses)),
        };
        const NveValue payload = nve::make_map_value(&entry, 1);
        self->outbound_events.push_custom_value(
            self->surface_id, 2, ev.timestamp_us, "onPressCount", payload);
        dirty = 1;
      }
    }
    return dirty;
  }

  static uint32_t dispatch_events_direct(NveAppHandle h, NveEventBatch batch) {
    auto* self = static_cast<BenchComponent*>(h);
    if (!self || !batch.ptr) return 0;
    self->presses += batch.len;
    return batch.len > 0 ? 1u : 0u;
  }

  static NveIntrinsicSize measure(NveAppHandle, NveSizeConstraints c) {
    const float w = (c.max_w > 0.0f && c.max_w < 240.0f) ? c.max_w : 240.0f;
    return NveIntrinsicSize{w, 48.0f};
  }

  static uint8_t hit_test(NveAppHandle, float x, float y, uint8_t) {
    return static_cast<uint8_t>((x >= 0.0f && x <= 240.0f && y >= 0.0f && y <= 48.0f)
                                    ? NVE_GESTURE_CLAIM
                                    : NVE_GESTURE_PASS_THROUGH);
  }

  static uint32_t set_props(NveAppHandle, NveBufferSlice) { return 1; }
  static uint32_t set_props_direct(NveAppHandle, const NveMapEntry*, uint32_t count) {
    return count > 0 ? 1u : 0u;
  }

  static NveBufferSlice drain_emitted_events(NveAppHandle h) {
    auto* self = static_cast<BenchComponent*>(h);
    self->outbound_snapshot = self->outbound_events.finalize();
    self->outbound_events.clear();
    return NveBufferSlice{self->outbound_snapshot.data(), self->outbound_snapshot.size()};
  }

  static const void* get_interface(uint32_t interface_id) {
    static const NveSurfaceApi kSurface{
        &create, &destroy, &render, &dispatch_events, &dispatch_events_direct};
    static const NveLayoutApi kLayout{&measure, &hit_test};
    static const NveComponentApi kComponent{&set_props, &drain_emitted_events, &set_props_direct};
    switch (interface_id) {
      case NVE_IFACE_SURFACE: return &kSurface;
      case NVE_IFACE_LAYOUT: return &kLayout;
      case NVE_IFACE_COMPONENT: return &kComponent;
      default: return nullptr;
    }
  }
};

std::string format_duration(double ns) {
  char buf[64];
  if (ns >= 1e6) {
    std::snprintf(buf, sizeof(buf), "%.2f ms", ns / 1e6);
  } else if (ns >= 1e3) {
    std::snprintf(buf, sizeof(buf), "%.2f us", ns / 1e3);
  } else {
    std::snprintf(buf, sizeof(buf), "%.1f ns", ns);
  }
  return buf;
}

std::string format_ops(double ops_per_sec) {
  char buf[64];
  if (ops_per_sec >= 1e6) {
    std::snprintf(buf, sizeof(buf), "%.2f M/s", ops_per_sec / 1e6);
  } else if (ops_per_sec >= 1e3) {
    std::snprintf(buf, sizeof(buf), "%.2f K/s", ops_per_sec / 1e3);
  } else {
    std::snprintf(buf, sizeof(buf), "%.1f /s", ops_per_sec);
  }
  return buf;
}

void print_usage(const char* argv0) {
  std::cout << "Usage: " << argv0 << " [options]\n"
            << "  --json             Output machine-readable JSON\n"
            << "  --warmup <N>       Override warmup iterations for all benchmarks\n"
            << "  --iters <N>        Override measured iterations for all benchmarks\n"
            << "  --filter <substr>  Run only benchmarks whose name contains <substr>\n"
            << "  --help             Show this help message\n";
}

void print_table(const std::vector<BenchResult>& results, std::string_view mode_str) {
  std::cout << "nve_fabric_bridge_bench (" << mode_str << ")\n";
  std::cout << std::string(118, '-') << "\n";
  std::cout << std::left << std::setw(42) << "Benchmark"
            << std::right << std::setw(8) << "Iters"
            << std::setw(13) << "p50"
            << std::setw(13) << "p95"
            << std::setw(13) << "p99"
            << std::setw(14) << "ns/op"
            << std::setw(15) << "ops/sec"
            << "\n";
  std::cout << std::string(118, '-') << "\n";

  for (const auto& r : results) {
    char ns_op_buf[32];
    std::snprintf(ns_op_buf, sizeof(ns_op_buf), "%.1f", r.ns_per_op);
    std::cout << std::left << std::setw(42) << r.name
              << std::right << std::setw(8) << r.iterations
              << std::setw(13) << format_duration(r.p50_ns)
              << std::setw(13) << format_duration(r.p95_ns)
              << std::setw(13) << format_duration(r.p99_ns)
              << std::setw(14) << ns_op_buf
              << std::setw(15) << format_ops(r.ops_per_sec)
              << "\n";
  }
  std::cout << std::string(118, '-') << "\n";
}

void print_json(const std::vector<BenchResult>& results, std::string_view mode_str) {
  std::cout << "{\n"
            << "  \"suite\": \"nve_fabric_bridge_bench\",\n"
            << "  \"mode\": \"" << mode_str << "\",\n"
            << "  \"has_yoga\": " << (nve::fabric_host_has_yoga_layout() ? "true" : "false") << ",\n"
#if defined(NVE_WITH_RN_JSI) && NVE_WITH_RN_JSI
            << "  \"has_jsi\": true,\n"
#else
            << "  \"has_jsi\": false,\n"
#endif
            << "  \"benchmarks\": [\n";
  for (size_t i = 0; i < results.size(); ++i) {
    const auto& r = results[i];
    std::cout << "    {\n"
              << "      \"name\": \"" << r.name << "\",\n"
              << "      \"warmup\": " << r.warmup << ",\n"
              << "      \"iterations\": " << r.iterations << ",\n"
              << "      \"ops_per_iter\": " << r.ops_per_iter << ",\n"
              << std::fixed << std::setprecision(2)
              << "      \"p50_ns\": " << r.p50_ns << ",\n"
              << "      \"p95_ns\": " << r.p95_ns << ",\n"
              << "      \"p99_ns\": " << r.p99_ns << ",\n"
              << "      \"mean_ns\": " << r.mean_ns << ",\n"
              << "      \"min_ns\": " << r.min_ns << ",\n"
              << "      \"max_ns\": " << r.max_ns << ",\n"
              << "      \"ns_per_op\": " << r.ns_per_op << ",\n"
              << "      \"ops_per_sec\": " << r.ops_per_sec << "\n"
              << "    }" << (i + 1 < results.size() ? "," : "") << "\n";
  }
  std::cout << "  ]\n}\n";
}

} // namespace

int main(int argc, char** argv) {
  BenchConfig cfg;
  for (int i = 1; i < argc; ++i) {
    const std::string_view arg(argv[i]);
    if (arg == "--json") {
      cfg.json_output = true;
    } else if (arg == "--warmup" && i + 1 < argc) {
      cfg.warmup_override = std::atoi(argv[++i]);
    } else if ((arg == "--iters" || arg == "--iterations") && i + 1 < argc) {
      cfg.iters_override = std::atoi(argv[++i]);
    } else if (arg == "--filter" && i + 1 < argc) {
      cfg.filter = argv[++i];
    } else if (arg == "--help" || arg == "-h") {
      print_usage(argv[0]);
      return 0;
    } else {
      std::cerr << "Unknown argument: " << arg << "\n";
      print_usage(argv[0]);
      return 2;
    }
  }

  const bool has_yoga = nve::fabric_host_has_yoga_layout();
#if defined(NVE_WITH_RN_JSI) && NVE_WITH_RN_JSI
  constexpr bool has_jsi = true;
#else
  constexpr bool has_jsi = false;
#endif
  const std::string mode_str =
      (has_yoga && has_jsi) ? "Yoga+JSI" : (has_yoga ? "Yoga" : "Standalone");

  std::vector<BenchResult> results;
  const NveSurfaceConfig surface_cfg{
      .surface_id = 1, .width = 393.0f, .height = 852.0f, .scale_factor = 3.0f};

  // 1. Binary command buffer decode + full tree mount (100, 1,000, 5,000 nodes).
  for (const uint32_t node_count : {100u, 1000u, 5000u}) {
    const std::string name = "decode_mount_" + std::to_string(node_count) + "_nodes";
    if (!should_run(name, cfg)) continue;

    const auto built = encode_tree_mount(1, 1, node_count);
    const NveBufferSlice slice{built.bytes.data(), built.bytes.size()};
    const int def_warmup = (node_count >= 5000u) ? 5 : 15;
    const int def_iters = (node_count >= 5000u) ? 30 : (node_count >= 1000u ? 100 : 300);

    NveFabricHostHandle host = nullptr;
    NveFrameStats stats{};
    results.push_back(run_bench_with_hooks(
        name,
        def_warmup,
        def_iters,
        /*ops_per_iter=*/1,
        cfg,
        [&](int) { host = nve_fabric_host_create(surface_cfg); },
        [&](int) {
          const int32_t rc = nve_fabric_host_submit_frame(host, slice, &stats);
          do_not_optimize(rc);
          do_not_optimize(stats.mutation_count);
        },
        [&](int) {
          nve_fabric_host_destroy(host);
          host = nullptr;
        }));
  }

  // 2. Incremental patch replay (10 text/style mutations on a 1,000-node tree).
  if (should_run("incremental_patch_10_on_1000_nodes", cfg)) {
    const auto base = encode_tree_mount(1, 1, 1000);
    NveFabricHostHandle host = nve_fabric_host_create(surface_cfg);
    NveFrameStats stats{};
    const int32_t mount_rc = nve_fabric_host_submit_frame(
        host, NveBufferSlice{base.bytes.data(), base.bytes.size()}, &stats);
    do_not_optimize(mount_rc);

    auto encode_patch = [&](uint64_t frame_id, bool variant) {
      nve::CommandBufferEncoder enc(1, frame_id);
      const size_t stride = std::max<size_t>(1, base.text_node_ids.size() / 5);
      for (size_t i = 0; i < 5 && i * stride < base.text_node_ids.size(); ++i) {
        const uint32_t tid = base.text_node_ids[i * stride];
        enc.set_text(
            tid,
            variant ? ("Updated live metric " + std::to_string(i) + " [active]")
                    : ("Updated live metric " + std::to_string(i) + " [idle]"));
        nve::StyleProps st{};
        st.text_color_rgba = variant ? 0xFF38BDF8u : 0xFFF8FAFCu;
        st.font_size = variant ? 16.0f : 15.0f;
        enc.set_style(tid, st);
      }
      enc.commit(true);
      const NveBufferSlice s = enc.finish();
      return std::vector<uint8_t>(s.ptr, s.ptr + s.len);
    };

    const std::vector<uint8_t> patch_a = encode_patch(2, true);
    const std::vector<uint8_t> patch_b = encode_patch(3, false);

    results.push_back(run_bench(
        "incremental_patch_10_on_1000_nodes",
        30,
        400,
        /*ops_per_iter=*/1,
        cfg,
        [&](int iter) {
          const auto& buf = (iter & 1) ? patch_a : patch_b;
          const int32_t rc = nve_fabric_host_submit_frame(
              host, NveBufferSlice{buf.data(), buf.size()}, &stats);
          do_not_optimize(rc);
          do_not_optimize(stats.mutation_count);
        }));
    nve_fabric_host_destroy(host);
  }

  // 2b. Direct zero-copy NveSceneView publish + dirty-slice read (10 mutations on 1,000 nodes).
  if (should_run("scene_view_publish_10_on_1000", cfg)) {
    const auto base = encode_tree_mount(1, 1, 1000);
    NveFabricHostHandle host = nve_fabric_host_create(surface_cfg);
    NveFrameStats stats{};
    const int32_t mount_rc = nve_fabric_host_submit_frame(
        host, NveBufferSlice{base.bytes.data(), base.bytes.size()}, &stats);
    do_not_optimize(mount_rc);

    auto encode_patch = [&](uint64_t frame_id, bool variant) {
      nve::CommandBufferEncoder enc(1, frame_id);
      const size_t stride = std::max<size_t>(1, base.text_node_ids.size() / 5);
      for (size_t i = 0; i < 5 && i * stride < base.text_node_ids.size(); ++i) {
        const uint32_t tid = base.text_node_ids[i * stride];
        enc.set_text(
            tid,
            variant ? ("Scene metric " + std::to_string(i) + " [live]")
                    : ("Scene metric " + std::to_string(i) + " [idle]"));
        nve::StyleProps st{};
        st.text_color_rgba = variant ? 0xFF38BDF8u : 0xFFF8FAFCu;
        st.font_size = variant ? 16.0f : 15.0f;
        enc.set_style(tid, st);
      }
      enc.commit(true);
      const NveBufferSlice s = enc.finish();
      return std::vector<uint8_t>(s.ptr, s.ptr + s.len);
    };

    const std::vector<uint8_t> patch_a = encode_patch(2, true);
    const std::vector<uint8_t> patch_b = encode_patch(3, false);

    results.push_back(run_bench(
        "scene_view_publish_10_on_1000",
        30,
        400,
        /*ops_per_iter=*/1,
        cfg,
        [&](int iter) {
          const auto& buf = (iter & 1) ? patch_a : patch_b;
          const int32_t rc = nve_fabric_host_submit_frame(
              host, NveBufferSlice{buf.data(), buf.size()}, &stats);
          const NveSceneView view = nve::fabric_host_scene_view(host);
          size_t text_bytes = 0u;
          for (uint32_t d = 0u; d < view.dirty_count; ++d) {
            const NveRenderNode& node = view.nodes[view.dirty_indices[d]];
            text_bytes += nve::scene_node_text(view, node).size();
          }
          do_not_optimize(rc);
          do_not_optimize(view.dirty_count);
          do_not_optimize(text_bytes);
        }));
    nve_fabric_host_destroy(host);
  }

  // 3. Flexbox layout pass (100 & 1,000 rows, default heuristic vs custom TextMeasurerCallback).
  for (const uint32_t rows : {100u, 1000u}) {
    for (const bool use_custom_measurer : {false, true}) {
      const std::string name =
          "layout_pass_" + std::to_string(rows) + "_rows_" +
          (use_custom_measurer ? "custom_measurer" : "heuristic");
      if (!should_run(name, cfg)) continue;

      const auto tree = encode_flex_rows_tree(1, 1, rows);
      NveFabricHostHandle host = nve_fabric_host_create(surface_cfg);
      if (use_custom_measurer) {
        nve::fabric_host_set_text_measurer(host, &fast_custom_text_measurer);
      }
      const int32_t rc = nve_fabric_host_submit_frame(
          host, NveBufferSlice{tree.bytes.data(), tree.bytes.size()}, nullptr);
      do_not_optimize(rc);

      const int def_warmup = (rows >= 1000u) ? 10 : 25;
      const int def_iters = (rows >= 1000u) ? 100 : 300;
      const uint32_t probe_id = tree.text_node_ids.back();

      results.push_back(run_bench(
          name,
          def_warmup,
          def_iters,
          /*ops_per_iter=*/1,
          cfg,
          [&](int iter) {
            // Alternate viewport width by 1pt so Yoga performs a full real layout pass every iter.
            const float w = (iter & 1) ? 393.0f : 394.0f;
            nve::fabric_host_set_surface_size(host, w, 852.0f, 3.0f);
            nve::LayoutRect rect{};
            const bool ok = nve::fabric_host_get_node_layout(host, probe_id, &rect);
            do_not_optimize(ok);
            do_not_optimize(rect.height);
          }));
      nve_fabric_host_destroy(host);
    }
  }

  // 4a. Modular C ABI (`nve_get_interface` + `nve_resolve_module` + zero-serialization measure/hit_test).
  if (should_run("c_abi_resolve_measure_hit_test", cfg)) {
    const NveGetInterfaceFn volatile get_iface_fn = &BenchComponent::get_interface;
    const NveModule init_mod = nve_resolve_module(get_iface_fn);
    NveAppHandle app = init_mod.surface->create(1);
    constexpr uint64_t kBatch = 1000;

    results.push_back(run_bench(
        "c_abi_resolve_measure_hit_test",
        50,
        500,
        /*ops_per_iter=*/kBatch,
        cfg,
        [&](int iter) {
          for (uint64_t k = 0; k < kBatch; ++k) {
            const NveModule mod = nve_resolve_module(get_iface_fn);
            const NveIntrinsicSize sz = mod.layout->measure(
                app, NveSizeConstraints{0.0f, 0.0f, 393.0f, 852.0f});
            const uint8_t disp = mod.layout->hit_test(
                app, static_cast<float>((iter + k) & 127), 20.0f, 1);
            do_not_optimize(sz.width);
            do_not_optimize(disp);
          }
        }));
    init_mod.surface->destroy(app);
  }

  // 4b. `NveRnComponentBridge` render & press-event roundtrip.
  if (should_run("rn_bridge_render_press_roundtrip", cfg)) {
    uint64_t emitted_count = 0;
    nve::NveRnComponentBridge bridge(
        &BenchComponent::get_interface,
        surface_cfg,
        [&](const nve::DecodedEvent& ev) {
          emitted_count += ev.payload_bytes.size();
        });
    const int32_t init_rc = bridge.render(nullptr);
    do_not_optimize(init_rc);

    results.push_back(run_bench(
        "rn_bridge_render_press_roundtrip",
        30,
        500,
        /*ops_per_iter=*/1,
        cfg,
        [&](int iter) {
          nve::fabric_host_enqueue_press(
              bridge.fabric_host(), 2, static_cast<uint64_t>(iter) * 16667ULL, 42.0f, 24.0f);
          const NveBufferSlice inbound = nve_fabric_host_drain_events(bridge.fabric_host());
          const uint32_t dirty = bridge.dispatch_native_events(inbound, /*auto_render=*/true);
          const auto outbound = bridge.drain_emitted_events();
          do_not_optimize(dirty);
          do_not_optimize(outbound.size());
          do_not_optimize(emitted_count);
        }));
  }

  // 5. `MockJsiRecorderRuntime` zero-copy arena FFI vs binary EventRing throughput (when NVE_WITH_RN_JSI is enabled).
#if defined(NVE_WITH_RN_JSI) && NVE_WITH_RN_JSI
  if (should_run("mock_jsi_direct_arena_ffi_dispatch", cfg)) {
    nve::MockJsiRecorderRuntime rt;
    constexpr uint64_t kBatch = 10;

    auto direct_sink = [](NveEventBatch batch) -> uint32_t {
      double acc = 0.0;
      for (uint32_t i = 0; i < batch.len; ++i) {
        const NveValue& val = batch.ptr[i].value;
        if (val.tag == NVE_VAL_MAP) {
          for (size_t j = 0; j < val.data.map_val.len; ++j) {
            const NveMapEntry& e = val.data.map_val.ptr[j];
            const std::string_view k = nve::str_slice_view(e.key);
            if (k == "x" && e.value.tag == NVE_VAL_F64) {
              acc += e.value.data.f64_val;
            } else if (k == "velocity" && e.value.tag == NVE_VAL_ARRAY &&
                       e.value.data.array_val.len >= 2) {
              acc += e.value.data.array_val.ptr[0].data.f64_val;
            }
          }
        }
      }
      do_not_optimize(acc);
      return batch.len;
    };

    results.push_back(run_bench(
        "mock_jsi_direct_arena_ffi_dispatch",
        50,
        1000,
        /*ops_per_iter=*/kBatch,
        cfg,
        [&](int iter) {
          rt.reset_frame();
          for (uint64_t k = 0; k < kBatch; ++k) {
            facebook::jsi::Object payload(rt);
            payload.setProperty(rt, "event", "onGestureUpdate");
            payload.setProperty(rt, "x", static_cast<double>(iter + k) * 1.25);
            payload.setProperty(rt, "y", 84.5);
            payload.setProperty(rt, "active", true);
            facebook::jsi::Array velocity(rt, 2);
            velocity.setValueAtIndex(rt, 0, 12.5);
            velocity.setValueAtIndex(rt, 1, -4.25);
            payload.setProperty(rt, "velocity", std::move(velocity));

            rt.record_direct_event(
                1,
                static_cast<uint32_t>(10 + k),
                static_cast<uint64_t>(iter) * 1000ULL + k,
                "onGestureUpdate",
                facebook::jsi::Value(rt, payload));
          }
          const uint32_t handled = rt.dispatch_direct(direct_sink);
          do_not_optimize(handled);
        }));
  }

  if (should_run("mock_jsi_binary_ring_dispatch", cfg)) {
    nve::MockJsiRecorderRuntime rt;
    nve::EventRingWriter ring;
    std::vector<nve::DecodedEvent> decoded;
    constexpr uint64_t kBatch = 10;

    results.push_back(run_bench(
        "mock_jsi_binary_ring_dispatch",
        50,
        1000,
        /*ops_per_iter=*/kBatch,
        cfg,
        [&](int iter) {
          rt.reset_frame();
          ring.clear();
          for (uint64_t k = 0; k < kBatch; ++k) {
            facebook::jsi::Object payload(rt);
            payload.setProperty(rt, "event", "onGestureUpdate");
            payload.setProperty(rt, "x", static_cast<double>(iter + k) * 1.25);
            payload.setProperty(rt, "y", 84.5);
            payload.setProperty(rt, "active", true);
            facebook::jsi::Array velocity(rt, 2);
            velocity.setValueAtIndex(rt, 0, 12.5);
            velocity.setValueAtIndex(rt, 1, -4.25);
            payload.setProperty(rt, "velocity", std::move(velocity));

            rt.record_custom_event(
                ring,
                1,
                static_cast<uint32_t>(10 + k),
                static_cast<uint64_t>(iter) * 1000ULL + k,
                "onGestureUpdate",
                facebook::jsi::Value(rt, payload));
          }
          const auto& bytes = ring.finalize();
          const bool ok = nve::decode_event_ring(NveBufferSlice{bytes.data(), bytes.size()}, decoded);
          double acc = 0.0;
          for (const auto& ev : decoded) {
            if (const auto view = nve::BinValueView::parse(
                    ev.payload_bytes.data(), ev.payload_bytes.size())) {
              if (const auto x = view->map_get("x"); x && x->as_f64()) {
                acc += *x->as_f64();
              }
              if (const auto v = view->map_get("velocity"); v && v->array_at(0)) {
                acc += v->array_at(0)->as_f64().value_or(0.0);
              }
            }
          }
          do_not_optimize(ok);
          do_not_optimize(acc);
        }));
  }
#endif

  if (cfg.json_output) {
    print_json(results, mode_str);
  } else {
    print_table(results, mode_str);
  }

  return 0;
}
