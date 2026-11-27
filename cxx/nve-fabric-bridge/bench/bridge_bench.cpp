#include "bridge_bench_fixtures.hpp"

#include <iomanip>
#include <iostream>

namespace {

using namespace nve_bench;

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
          const NveEvent press = nve::make_press_event(
              surface_cfg.surface_id, 2, static_cast<uint64_t>(iter) * 16667ULL, 42.0f, 24.0f);
          const uint32_t dirty =
              bridge.dispatch_events(nve::make_single_event_batch(press), /*auto_render=*/true);
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
      for (uint32_t i = 0; i < batch.event_count; ++i) {
        std::string_view cname;
        std::span<const uint8_t> cpayload;
        if (nve::event_custom_parts(
                batch.events[i], batch.bytes, batch.bytes_len, cname, cpayload)) {
          if (const auto view = nve::BinValueView::parse(cpayload.data(), cpayload.size())) {
            if (const auto x = view->map_get("x"); x && x->as_f64()) {
              acc += *x->as_f64();
            }
            if (const auto v = view->map_get("velocity"); v && v->array_at(0)) {
              acc += v->array_at(0)->as_f64().value_or(0.0);
            }
          }
        }
      }
      do_not_optimize(acc);
      return batch.event_count;
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
    nve::EventBatchBuilder ring;
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
