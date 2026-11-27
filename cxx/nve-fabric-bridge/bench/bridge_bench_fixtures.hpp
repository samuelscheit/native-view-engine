#pragma once

#include "command_encoder.hpp"
#include "mock_jsi_recorder.hpp"
#include <nve/generated/nve_abi.generated.h>
#include "nve_host_runtime.hpp"
#include "nve_rn_component.hpp"

#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <numeric>
#include <string>
#include <string_view>
#include <vector>

namespace nve_bench {

using Clock = std::chrono::steady_clock;

inline volatile uint64_t g_bench_sink = 0;

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

inline double percentile(const std::vector<double>& sorted, double p) {
  if (sorted.empty()) return 0.0;
  if (sorted.size() == 1) return sorted.front();
  const double idx = (p / 100.0) * static_cast<double>(sorted.size() - 1);
  const size_t lo = static_cast<size_t>(std::floor(idx));
  const size_t hi = static_cast<size_t>(std::ceil(idx));
  const double frac = idx - static_cast<double>(lo);
  return sorted[lo] + (sorted[hi] - sorted[lo]) * frac;
}

template <typename SetupFn, typename BodyFn, typename TeardownFn>
inline BenchResult run_bench_with_hooks(
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
    samples.push_back(std::chrono::duration<double, std::nano>(t1 - t0).count());
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
inline BenchResult run_bench(
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

inline bool should_run(std::string_view name, const BenchConfig& cfg) {
  return cfg.filter.empty() || name.find(cfg.filter) != std::string_view::npos;
}

struct BuiltTreeBuffer {
  std::vector<uint8_t> bytes;
  std::vector<uint32_t> text_node_ids;
  std::vector<uint32_t> row_node_ids;
  uint32_t total_nodes{0};
};

inline BuiltTreeBuffer encode_tree_mount(uint32_t surface_id, uint64_t frame_id, uint32_t target_nodes) {
  BuiltTreeBuffer out;
  nve::CommandBufferEncoder enc(surface_id, frame_id);
  enc.reserve(static_cast<size_t>(target_nodes) * 160u);

  const uint32_t list_id = 2;
  enc.create_node(list_id, NVE_COMP_SCROLL_VIEW);
  nve::LayoutProps list_layout{};
  list_layout.width = {2, 100.0f};
  list_layout.height = {2, 100.0f};
  list_layout.flex_direction = 0;
  list_layout.padding = {12.0f, 12.0f, 12.0f, 12.0f};
  list_layout.gap = 8.0f;
  enc.set_layout(list_id, list_layout);

  uint32_t next_id = 3;
  uint32_t created = 1;

  nve::LayoutProps row_layout{};
  row_layout.width = {2, 100.0f};
  row_layout.height = {1, 56.0f};
  row_layout.flex_direction = 1;
  row_layout.align_items = 2;
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
      enc.set_event_mask(btn_id, 1ULL << 0);
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

inline BuiltTreeBuffer encode_flex_rows_tree(uint32_t surface_id, uint64_t frame_id, uint32_t row_count) {
  BuiltTreeBuffer out;
  nve::CommandBufferEncoder enc(surface_id, frame_id);
  enc.reserve(static_cast<size_t>(row_count) * 420u);

  const uint32_t list_id = 2;
  enc.create_node(list_id, NVE_COMP_VIEW);
  nve::LayoutProps list_layout{};
  list_layout.width = {2, 100.0f};
  list_layout.flex_direction = 0;
  list_layout.padding = {8.0f, 8.0f, 8.0f, 8.0f};
  list_layout.gap = 6.0f;
  enc.set_layout(list_id, list_layout);

  nve::LayoutProps row_layout{};
  row_layout.width = {2, 100.0f};
  row_layout.flex_direction = 1;
  row_layout.justify_content = 3;
  row_layout.align_items = 2;
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
    enc.set_text(text_id, "Row " + std::to_string(r) + ": Native View Engine flexbox workload");
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

inline NveIntrinsicSize fast_custom_text_measurer(
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

struct BenchComponent {
  uint32_t surface_id{1};
  uint64_t presses{0};
  bool mounted{false};
  nve::CommandBufferEncoder encoder{1, 1};
  nve::BumpArena outbound_arena{1024u};
  std::vector<NveDirectEvent> outbound_pending;
  std::vector<NveDirectEvent> outbound_drained;

  static NveAppHandle create(uint32_t sid) {
    auto* self = new BenchComponent();
    self->surface_id = sid;
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
    const int n = std::snprintf(
        label, sizeof(label), "Presses: %llu", static_cast<unsigned long long>(self->presses));
    self->encoder.set_text(3, std::string_view(label, n > 0 ? static_cast<size_t>(n) : 0));
    self->encoder.commit(true);
    return self->encoder.finish();
  }

  static uint32_t dispatch_events(NveAppHandle h, NveEventBatch batch) {
    auto* self = static_cast<BenchComponent*>(h);
    if (!self || !batch.ptr || batch.len == 0) return 0;
    uint32_t dirty = 0;
    for (uint32_t i = 0; i < batch.len; ++i) {
      const NveDirectEvent& ev = batch.ptr[i];
      if (ev.kind == NVE_EVENT_PRESS && ev.node_id == 2) {
        ++self->presses;
        self->outbound_arena.reset();
        NveMapEntry* entry = self->outbound_arena.alloc_array<NveMapEntry>(1u);
        entry[0] = NveMapEntry{
            nve::make_str_slice("presses"),
            nve::make_i64_value(static_cast<int64_t>(self->presses)),
        };
        NveDirectEvent out{};
        out.surface_id = self->surface_id;
        out.node_id = 2;
        out.timestamp_us = ev.timestamp_us;
        out.kind = NVE_EVENT_CUSTOM;
        out.payload_tag = NVE_EVENT_PAYLOAD_CUSTOM;
        out.name = nve::make_str_slice("onPressCount");
        out.value = nve::make_map_value(entry, 1u);
        self->outbound_pending.push_back(out);
        dirty = 1;
      }
    }
    return dirty;
  }

  static NveIntrinsicSize measure(NveAppHandle, NveSizeConstraints c) {
    const float w = (c.max_w > 0.0f && c.max_w < 240.0f) ? c.max_w : 240.0f;
    return NveIntrinsicSize{w, 48.0f};
  }

  static uint8_t hit_test(NveAppHandle, float x, float y, uint8_t) {
    return static_cast<uint8_t>(
        (x >= 0.0f && x <= 240.0f && y >= 0.0f && y <= 48.0f) ? NVE_GESTURE_CLAIM
                                                              : NVE_GESTURE_PASS_THROUGH);
  }

  static uint32_t set_props(NveAppHandle, const NveMapEntry*, uint32_t count) {
    return count > 0 ? 1u : 0u;
  }

  static NveEventBatch drain_emitted_events(NveAppHandle h) {
    auto* self = static_cast<BenchComponent*>(h);
    self->outbound_drained = std::move(self->outbound_pending);
    self->outbound_pending.clear();
    return NveEventBatch{
        self->outbound_drained.empty() ? nullptr : self->outbound_drained.data(),
        static_cast<uint32_t>(self->outbound_drained.size()),
        0u,
    };
  }

  static const void* get_interface(uint32_t interface_id) {
    static const NveSurfaceApi kSurface{&create, &destroy, &render, &dispatch_events};
    static const NveLayoutApi kLayout{&measure, &hit_test};
    static const NveComponentApi kComponent{&set_props, &drain_emitted_events};
    switch (interface_id) {
      case NVE_IFACE_SURFACE: return &kSurface;
      case NVE_IFACE_LAYOUT: return &kLayout;
      case NVE_IFACE_COMPONENT: return &kComponent;
      default: return nullptr;
    }
  }
};

} // namespace nve_bench
