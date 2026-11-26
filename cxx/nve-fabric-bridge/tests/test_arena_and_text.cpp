#include "test_common.hpp"

#include <cmath>
#include <cstdint>
#include <limits>
#include <random>
#include <string>
#include <vector>

namespace nve_test {

// 5b. Test BumpArena alignment/reuse across 1,500 frames and BinValueView roundtrip + 600+ seeded fuzz corpus.
void test_bump_arena_and_bin_value_fuzz() {
  {
    nve::BumpArena arena(2048u);
    size_t warmup_slabs = 0;
    for (int frame = 0; frame < 1500; ++frame) {
      arena.reset();
      for (size_t align : {size_t{1}, size_t{2}, size_t{4}, size_t{8}, size_t{16}, size_t{32}, size_t{64}}) {
        void* p = arena.alloc(37u, align);
        assert(p != nullptr);
        assert((reinterpret_cast<uintptr_t>(p) & (align - 1u)) == 0u);
      }
      auto* big = arena.alloc_array<NveMapEntry>(80u);
      assert(big != nullptr);
      big[0] = NveMapEntry{arena.copy_str("key"), nve::make_i64_value(frame)};
      if (frame == 0) {
        warmup_slabs = arena.slab_count();
        assert(warmup_slabs >= 2u);
      } else {
        assert(arena.slab_count() == warmup_slabs);
      }
    }
  }

  std::vector<uint8_t> baseline;
  {
    const uint8_t blob[4] = {0x11, 0x22, 0x33, 0x44};
    const NveValue arr_elems[3] = {
        nve::make_null_value(),
        nve::make_bool_value(false),
        nve::make_i64_value(-1234567890123LL),
    };
    const NveMapEntry inner_entries[2] = {
        {nve::make_str_slice("arr"), nve::make_array_value(arr_elems, 3)},
        {nve::make_str_slice("bytes"), nve::make_bytes_value(NveBufferSlice{blob, sizeof(blob)})},
    };
    const NveMapEntry root_entries[4] = {
        {nve::make_str_slice("str"), nve::make_str_value("hello-utf8")},
        {nve::make_str_slice("num"), nve::make_f64_value(3.141592653589793)},
        {nve::make_str_slice("inner"), nve::make_map_value(inner_entries, 2)},
        {nve::make_str_slice("flag"), nve::make_bool_value(true)},
    };
    const NveValue root = nve::make_map_value(root_entries, 4);
    assert(nve::encode_bin_value(baseline, root));

    const auto view = nve::BinValueView::parse(baseline.data(), baseline.size());
    assert(view.has_value() && view->tag() == NVE_VAL_MAP && view->map_len() == 4);
    assert(view->map_get("str")->as_str() == "hello-utf8");
    assert(std::fabs(*view->map_get("num")->as_f64() - 3.141592653589793) < 1e-12);
    assert(view->map_get("flag")->as_bool() == true);
    const auto inner = view->map_get("inner");
    assert(inner.has_value() && inner->map_get("arr")->array_at(2)->as_i64() == -1234567890123LL);
    assert(inner->map_get("bytes")->as_bytes()->len == 4);

    auto build_nested = [](uint32_t depth) {
      std::vector<uint8_t> cur{static_cast<uint8_t>(NVE_VAL_NULL)};
      for (uint32_t d = 0; d < depth; ++d) {
        std::vector<uint8_t> next;
        next.reserve(cur.size() + 9u);
        next.push_back(static_cast<uint8_t>(NVE_VAL_ARRAY));
        const uint32_t payload_len = static_cast<uint32_t>(4u + cur.size());
        const uint32_t count = 1u;
        next.insert(next.end(), reinterpret_cast<const uint8_t*>(&payload_len),
                    reinterpret_cast<const uint8_t*>(&payload_len) + 4);
        next.insert(next.end(), reinterpret_cast<const uint8_t*>(&count),
                    reinterpret_cast<const uint8_t*>(&count) + 4);
        next.insert(next.end(), cur.begin(), cur.end());
        cur = std::move(next);
      }
      return cur;
    };
    const auto ok64 = build_nested(NVE_BIN_VALUE_MAX_DEPTH);
    assert(nve::BinValueView::parse(ok64.data(), ok64.size()).has_value());
    const auto bad65 = build_nested(NVE_BIN_VALUE_MAX_DEPTH + 1u);
    assert(!nve::BinValueView::parse(bad65.data(), bad65.size()).has_value());
  }

  std::mt19937_64 rng(0xB1A7C0DE5EEDCAFEULL);
  auto rand_u8 = [&]() -> uint8_t { return static_cast<uint8_t>(rng() & 0xFFu); };
  size_t fuzz_cases = 0;

  for (size_t cut = 0; cut < baseline.size(); ++cut) {
    assert(!nve::BinValueView::parse(baseline.data(), cut).has_value());
    ++fuzz_cases;
  }
  {
    std::vector<uint8_t> trailing = baseline;
    trailing.push_back(0x00);
    assert(!nve::BinValueView::parse(trailing.data(), trailing.size()).has_value());
    ++fuzz_cases;
  }
  {
    const uint8_t bad_bool[2] = {NVE_VAL_BOOL, 2};
    const uint8_t bad_tag[1] = {99};
    const uint8_t bad_utf8[8] = {NVE_VAL_STR, 3, 0, 0, 0, 0xFF, 0xFE, 0x80};
    const uint8_t huge_arr[9] = {NVE_VAL_ARRAY, 4, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF};
    const uint8_t huge_map[9] = {NVE_VAL_MAP, 4, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF};
    assert(!nve::BinValueView::parse(bad_bool, sizeof(bad_bool)).has_value());
    assert(!nve::BinValueView::parse(bad_tag, sizeof(bad_tag)).has_value());
    assert(!nve::BinValueView::parse(bad_utf8, sizeof(bad_utf8)).has_value());
    assert(!nve::BinValueView::parse(huge_arr, sizeof(huge_arr)).has_value());
    assert(!nve::BinValueView::parse(huge_map, sizeof(huge_map)).has_value());
    fuzz_cases += 5;
  }
  for (int i = 0; i < 300; ++i) {
    const size_t len = static_cast<size_t>(rng() % 513u);
    std::vector<uint8_t> buf(len);
    for (auto& b : buf) b = rand_u8();
    if (const auto parsed = nve::BinValueView::parse(buf.data(), buf.size())) {
      (void)parsed->as_str();
      (void)parsed->map_get("str");
      (void)parsed->array_at(0);
    }
    ++fuzz_cases;
  }
  for (int i = 0; i < 250; ++i) {
    std::vector<uint8_t> mut = baseline;
    const size_t edits = 1u + static_cast<size_t>(rng() % 6u);
    for (size_t e = 0; e < edits; ++e) {
      const size_t mode = rng() % 4u;
      if (mode == 0 && !mut.empty()) {
        mut[rng() % mut.size()] ^= static_cast<uint8_t>(1u << (rng() % 8u));
      } else if (mode == 1 && !mut.empty()) {
        mut[rng() % mut.size()] = rand_u8();
      } else if (mode == 2 && mut.size() > 1) {
        mut.resize(1u + static_cast<size_t>(rng() % (mut.size() - 1u)));
      } else {
        mut.push_back(rand_u8());
      }
    }
    if (const auto parsed = nve::BinValueView::parse(mut.data(), mut.size())) {
      (void)parsed->as_str();
      (void)parsed->map_get("inner");
      (void)parsed->array_at(0);
    }
    ++fuzz_cases;
  }
  assert(fuzz_cases >= 600);
}

// 6. Test NVE_COMP_TEXT multi-level style inheritance, explicit overrides, reparent reset, and default opaque black.
void test_text_style_inheritance() {
  NveSurfaceConfig cfg{.surface_id = 11, .width = 400.0f, .height = 600.0f, .scale_factor = 2.0f};
  NveFabricHostHandle host = nve_fabric_host_create(cfg);
  assert(host != nullptr);

  CommandBufferEncoder enc(11, 1);
  enc.create_node(10, NVE_COMP_TEXT);
  enc.create_node(20, NVE_COMP_TEXT);
  enc.create_node(21, NVE_COMP_TEXT);
  enc.create_node(22, NVE_COMP_VIEW);
  enc.create_node(23, NVE_COMP_TEXT);
  enc.create_node(24, NVE_COMP_TEXT);
  enc.create_node(25, NVE_COMP_TEXT);

  enc.set_text(10, "Default black text");
  enc.set_text(21, "Inherited child");
  enc.set_text(23, "Inherited grandchild");
  enc.set_text(25, "Inherited under override");

  nve::StyleProps parent_style{};
  parent_style.text_color_rgba = 0xFF112233u;
  parent_style.font_size = 24.0f;
  enc.set_style(20, parent_style);

  nve::StyleProps override_style{};
  override_style.text_color_rgba = 0xFFAA5500u;
  override_style.font_size = 12.0f;
  enc.set_style(24, override_style);

  enc.set_children(NVE_ROOT_NODE_ID, {10, 20});
  enc.set_children(20, {21, 22, 24});
  enc.set_children(22, {23});
  enc.set_children(24, {25});
  enc.commit(true);

  assert(nve_fabric_host_submit_frame(host, enc.finish(), nullptr) == 0);

  auto snaps = snapshot_by_id(host);
  assert(snaps.at(10).text_color_rgba == 0xFF000000u);
  assert(approx_eq(snaps.at(10).font_size, 16.0f, 1e-3f));
  assert(snaps.at(20).text_color_rgba == 0xFF112233u);
  assert(approx_eq(snaps.at(20).font_size, 24.0f, 1e-3f));
  assert(snaps.at(21).text_color_rgba == 0xFF112233u);
  assert(approx_eq(snaps.at(21).font_size, 24.0f, 1e-3f));
  assert(snaps.at(23).text_color_rgba == 0xFF112233u);
  assert(approx_eq(snaps.at(23).font_size, 24.0f, 1e-3f));
  assert(snaps.at(24).text_color_rgba == 0xFFAA5500u);
  assert(approx_eq(snaps.at(24).font_size, 12.0f, 1e-3f));
  assert(snaps.at(25).text_color_rgba == 0xFFAA5500u);
  assert(approx_eq(snaps.at(25).font_size, 12.0f, 1e-3f));

  nve::LayoutRect r21{}, r25{};
  assert(nve::fabric_host_get_node_layout(host, 21, &r21));
  assert(nve::fabric_host_get_node_layout(host, 25, &r25));
  assert(r21.height > r25.height);

  CommandBufferEncoder enc2(11, 2);
  nve::StyleProps updated_parent{};
  updated_parent.text_color_rgba = 0xFF00CC88u;
  updated_parent.font_size = 32.0f;
  enc2.set_style(20, updated_parent);
  enc2.commit(true);
  assert(nve_fabric_host_submit_frame(host, enc2.finish(), nullptr) == 0);

  auto snaps2 = snapshot_by_id(host);
  assert(snaps2.at(21).text_color_rgba == 0xFF00CC88u);
  assert(approx_eq(snaps2.at(21).font_size, 32.0f, 1e-3f));
  assert(snaps2.at(23).text_color_rgba == 0xFF00CC88u);
  assert(approx_eq(snaps2.at(23).font_size, 32.0f, 1e-3f));
  assert(snaps2.at(24).text_color_rgba == 0xFFAA5500u);
  assert(approx_eq(snaps2.at(24).font_size, 12.0f, 1e-3f));
  assert(snaps2.at(25).text_color_rgba == 0xFFAA5500u);
  assert(approx_eq(snaps2.at(25).font_size, 12.0f, 1e-3f));

  CommandBufferEncoder enc3(11, 3);
  enc3.set_children(22, {});
  enc3.set_children(NVE_ROOT_NODE_ID, {10, 20, 23});
  enc3.commit(true);
  assert(nve_fabric_host_submit_frame(host, enc3.finish(), nullptr) == 0);

  auto snaps3 = snapshot_by_id(host);
  assert(snaps3.at(23).text_color_rgba == 0xFF000000u);
  assert(approx_eq(snaps3.at(23).font_size, 16.0f, 1e-3f));

  nve_fabric_host_destroy(host);
}

// 7. Test fabric_host_set_text_measurer callback, max_width wrapping, empty strings, and heuristic reset.
void test_text_measurer_callback() {
  NveSurfaceConfig cfg{.surface_id = 12, .width = 400.0f, .height = 600.0f, .scale_factor = 2.0f};
  NveFabricHostHandle host = nve_fabric_host_create(cfg);
  assert(host != nullptr);

  int call_count = 0;
  bool saw_empty = false;
  float last_narrow_max_w = -1.0f;
  float last_wide_max_w = -1.0f;
  float last_font_size = 0.0f;

  nve::fabric_host_set_text_measurer(
      host,
      [&](std::string_view text, float font_size, float max_width) -> NveIntrinsicSize {
        ++call_count;
        if (text.empty()) {
          saw_empty = true;
        }
        last_font_size = font_size;
        const float raw_w = static_cast<float>(text.size()) * (font_size * 0.625f);
        const float line_h = font_size * 2.5f;
        if (text.find("NARROW:") == 0) {
          last_narrow_max_w = max_width;
        } else if (text.find("WIDE:") == 0) {
          last_wide_max_w = max_width;
        }
        if (max_width > 0.0f && raw_w > max_width) {
          const float lines = std::ceil(raw_w / max_width);
          return NveIntrinsicSize{max_width, lines * line_h};
        }
        return NveIntrinsicSize{raw_w, line_h};
      });

  CommandBufferEncoder enc(12, 1);
  enc.create_node(2, NVE_COMP_VIEW);
  enc.create_node(3, NVE_COMP_TEXT);
  enc.create_node(4, NVE_COMP_VIEW);
  enc.create_node(5, NVE_COMP_TEXT);
  enc.create_node(6, NVE_COMP_TEXT);

  nve::LayoutProps narrow_box{};
  narrow_box.width = {1, 120.0f};
  narrow_box.padding = {0.0f, 10.0f, 0.0f, 10.0f};
  narrow_box.align_items = 1;
  enc.set_layout(2, narrow_box);

  nve::LayoutProps wide_box{};
  wide_box.width = {1, 360.0f};
  wide_box.align_items = 1;
  enc.set_layout(4, wide_box);

  nve::StyleProps st{};
  st.font_size = 16.0f;
  enc.set_style(3, st);
  enc.set_style(5, st);
  enc.set_style(6, st);

  enc.set_text(3, "NARROW:01234567890123456789012");
  enc.set_text(5, "WIDE:0123456789012345678901234");
  enc.set_text(6, "");

  enc.set_children(NVE_ROOT_NODE_ID, {2, 4, 6});
  enc.set_children(2, {3});
  enc.set_children(4, {5});
  enc.commit(true);

  assert(nve_fabric_host_submit_frame(host, enc.finish(), nullptr) == 0);
  assert(call_count >= 2);
  assert(!saw_empty);
  assert(approx_eq(last_font_size, 16.0f, 0.1f));
  assert(approx_eq(last_narrow_max_w, 100.0f, 1.0f));
  assert(approx_eq(last_wide_max_w, 360.0f, 1.0f));

  nve::LayoutRect r_narrow{}, r_wide{}, r_empty{};
  assert(nve::fabric_host_get_node_layout(host, 3, &r_narrow));
  assert(nve::fabric_host_get_node_layout(host, 5, &r_wide));
  assert(nve::fabric_host_get_node_layout(host, 6, &r_empty));

  assert(approx_eq(r_narrow.width, 100.0f, 1.0f));
  assert(approx_eq(r_narrow.height, 120.0f, 1.0f));
  assert(approx_eq(r_wide.width, 300.0f, 1.0f));
  assert(approx_eq(r_wide.height, 40.0f, 1.0f));
  assert(r_narrow.height > r_wide.height * 2.0f);
  assert(approx_eq(r_empty.height, 0.0f, 0.5f));

  const int calls_before_reset = call_count;
  nve::fabric_host_set_text_measurer(host, {});
  nve::fabric_host_set_surface_size(host, 400.0f, 600.0f, 2.0f);

  assert(call_count == calls_before_reset);
  nve::LayoutRect r_wide_heuristic{}, r_narrow_heuristic{};
  assert(nve::fabric_host_get_node_layout(host, 5, &r_wide_heuristic));
  assert(nve::fabric_host_get_node_layout(host, 3, &r_narrow_heuristic));
  assert(approx_eq(r_wide_heuristic.height, 16.0f * 1.3f, 1.5f));
  assert(r_narrow_heuristic.height > r_wide_heuristic.height);

  nve::fabric_host_set_text_measurer(
      host,
      [](std::string_view, float, float) -> NveIntrinsicSize {
        return NveIntrinsicSize{std::numeric_limits<float>::quiet_NaN(), -50.0f};
      });
  nve::fabric_host_set_surface_size(host, 400.0f, 600.0f, 2.0f);
  nve::LayoutRect r_sanitized{};
  assert(nve::fabric_host_get_node_layout(host, 5, &r_sanitized));
  assert(std::isfinite(r_sanitized.width) && r_sanitized.width >= 0.0f);
  assert(std::isfinite(r_sanitized.height) && r_sanitized.height >= 0.0f);

  nve_fabric_host_destroy(host);
}

} // namespace nve_test
