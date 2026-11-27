#include "test_common.hpp"

namespace nve_test {

// 8. Deterministic seeded fuzz corpus (600+ CommandBuffers + 300+ EventRings) asserting zero crashes/UB
//    and clean negative error codes on malformed inputs.
void test_seeded_fuzz_corpus() {
  std::mt19937_64 rng(0x4E56455F46555A5AuLL);
  auto rand_u8 = [&]() -> uint8_t { return static_cast<uint8_t>(rng() & 0xFFu); };
  auto rand_range = [&](size_t lo, size_t hi) -> size_t {
    return lo + static_cast<size_t>(rng() % (hi - lo + 1));
  };

  NveSurfaceConfig cfg{.surface_id = 1, .width = 393.0f, .height = 852.0f, .scale_factor = 3.0f};
  NveFabricHostHandle shared_host = nve_fabric_host_create(cfg);
  assert(shared_host != nullptr);

  // Build a rich valid baseline CommandBuffer containing all 10 opcodes for truncation/mutation fuzzing.
  CommandBufferEncoder baseline_enc(1, 1);
  baseline_enc.create_node(2, NVE_COMP_VIEW);
  baseline_enc.create_node(3, NVE_COMP_TEXT);
  baseline_enc.create_node(4, NVE_COMP_CUSTOM, "FuzzComp");
  baseline_enc.set_children(NVE_ROOT_NODE_ID, {2});
  baseline_enc.set_children(2, {3, 4});
  baseline_enc.set_text(3, "Fuzz baseline text");
  nve::LayoutProps base_lp{};
  base_lp.width = {1, 200.0f};
  base_lp.height = {1, 80.0f};
  base_lp.padding = {4.0f, 4.0f, 4.0f, 4.0f};
  baseline_enc.set_layout(2, base_lp);
  nve::StyleProps base_st{};
  base_st.background_rgba = 0xFF112233u;
  base_st.font_size = 16.0f;
  baseline_enc.set_style(3, base_st);
  baseline_enc.set_raw_prop_null(4, "n");
  baseline_enc.set_raw_prop_bool(4, "b", true);
  baseline_enc.set_raw_prop_i64(4, "i", -42);
  baseline_enc.set_raw_prop_f64(4, "f", 3.14);
  baseline_enc.set_raw_prop_string(4, "s", "hello");
  baseline_enc.set_raw_prop_bytes(4, "bytes", {1, 2, 3, 4});
  baseline_enc.set_event_mask(2, (1ULL << 0) | (1ULL << 6));
  baseline_enc.dispatch_command(4, "cmd", {9, 8});
  baseline_enc.remove_node(4);
  baseline_enc.commit(true);
  const NveBufferSlice base_slice = baseline_enc.finish();
  const std::vector<uint8_t> base_bytes(base_slice.ptr, base_slice.ptr + base_slice.len);

  size_t cmd_fuzz_cases = 0;

  // (a) Every prefix truncation of the baseline CommandBuffer MUST fail cleanly with a negative error code.
  for (size_t cut = 0; cut < base_bytes.size(); ++cut) {
    const int32_t rc = nve_fabric_host_submit_frame(
        shared_host, NveBufferSlice{base_bytes.data(), cut}, nullptr);
    assert(rc < 0);
    ++cmd_fuzz_cases;
  }

  // (b) Explicit adversarial/oversized/cyclic/NaN CommandBuffers (must all return < 0 without OOM/crash).
  {
    auto expect_err = [&](const std::vector<uint8_t>& buf, int32_t expected_or_any_neg = 0) {
      const int32_t rc = nve_fabric_host_submit_frame(
          shared_host, NveBufferSlice{buf.data(), buf.size()}, nullptr);
      if (expected_or_any_neg < 0) {
        assert(rc == expected_or_any_neg);
      } else {
        assert(rc < 0);
      }
      ++cmd_fuzz_cases;
    };

    // Oversized SET_CHILDREN count (0xFFFFFFFF) -> -4 without OOM
    {
      CommandBufferEncoder enc(1, 10);
      enc.create_node(50, NVE_COMP_VIEW);
      enc.finish();
      auto& raw = enc.raw();
      uint32_t op_count = 2;
      std::memcpy(raw.data() + 20, &op_count, 4);
      raw.push_back(NVE_OP_SET_CHILDREN);
      uint32_t pid = 50;
      uint32_t huge_cnt = 0xFFFFFFFFu;
      raw.insert(raw.end(), reinterpret_cast<uint8_t*>(&pid), reinterpret_cast<uint8_t*>(&pid) + 4);
      raw.insert(raw.end(), reinterpret_cast<uint8_t*>(&huge_cnt), reinterpret_cast<uint8_t*>(&huge_cnt) + 4);
      expect_err(raw, -4);
    }

    // Oversized SET_TEXT length (0xFFFFFFFF) -> -4 without OOM
    {
      CommandBufferEncoder enc(1, 11);
      enc.create_node(51, NVE_COMP_TEXT);
      enc.finish();
      auto& raw = enc.raw();
      uint32_t op_count = 2;
      std::memcpy(raw.data() + 20, &op_count, 4);
      raw.push_back(NVE_OP_SET_TEXT);
      uint32_t nid = 51;
      uint32_t huge_len = 0xFFFFFFFFu;
      raw.insert(raw.end(), reinterpret_cast<uint8_t*>(&nid), reinterpret_cast<uint8_t*>(&nid) + 4);
      raw.insert(raw.end(), reinterpret_cast<uint8_t*>(&huge_len), reinterpret_cast<uint8_t*>(&huge_len) + 4);
      expect_err(raw, -4);
    }

    // Self-parenting cycle (52 -> {52}) -> -6
    {
      CommandBufferEncoder enc(1, 12);
      enc.create_node(52, NVE_COMP_VIEW);
      enc.set_children(52, {52});
      enc.commit(true);
      enc.finish();
      expect_err(enc.raw(), -6);
    }

    // Root-as-child (53 -> {NVE_ROOT_NODE_ID}) -> -6
    {
      CommandBufferEncoder enc(1, 13);
      enc.create_node(53, NVE_COMP_VIEW);
      enc.set_children(53, {NVE_ROOT_NODE_ID});
      enc.commit(true);
      enc.finish();
      expect_err(enc.raw(), -6);
    }

    // 2-node cycle (54 -> 55 -> 54) -> -6
    {
      CommandBufferEncoder enc(1, 14);
      enc.create_node(54, NVE_COMP_VIEW);
      enc.create_node(55, NVE_COMP_VIEW);
      enc.set_children(54, {55});
      enc.set_children(55, {54});
      enc.commit(true);
      enc.finish();
      expect_err(enc.raw(), -6);
    }

    // Duplicate child in same parent (56 -> {57, 57}) -> -6
    {
      CommandBufferEncoder enc(1, 15);
      enc.create_node(56, NVE_COMP_VIEW);
      enc.create_node(57, NVE_COMP_VIEW);
      enc.set_children(56, {57, 57});
      enc.commit(true);
      enc.finish();
      expect_err(enc.raw(), -6);
    }

    // NaN / Inf layout dimensions and negative flex_grow -> -6
    {
      CommandBufferEncoder enc(1, 16);
      enc.create_node(58, NVE_COMP_VIEW);
      nve::LayoutProps bad_lp{};
      bad_lp.width = {1, std::numeric_limits<float>::quiet_NaN()};
      enc.set_layout(58, bad_lp);
      enc.finish();
      expect_err(enc.raw(), -6);
    }
    {
      CommandBufferEncoder enc(1, 17);
      enc.create_node(59, NVE_COMP_VIEW);
      nve::LayoutProps bad_lp{};
      bad_lp.flex_grow = -1.0f;
      enc.set_layout(59, bad_lp);
      enc.finish();
      expect_err(enc.raw(), -6);
    }
    {
      CommandBufferEncoder enc(1, 18);
      enc.create_node(60, NVE_COMP_TEXT);
      nve::StyleProps bad_st{};
      bad_st.font_size = std::numeric_limits<float>::infinity();
      enc.set_style(60, bad_st);
      enc.finish();
      expect_err(enc.raw(), -6);
    }
  }

  // (c) 200 pure random buffers (sizes 0..2048) -> must all return clean negative error codes.
  for (int i = 0; i < 200; ++i) {
    const size_t len = (i == 0) ? 0 : rand_range(1, 2048);
    std::vector<uint8_t> buf(len);
    for (auto& b : buf) b = rand_u8();
    const int32_t rc = nve_fabric_host_submit_frame(
        shared_host, NveBufferSlice{buf.data(), buf.size()}, nullptr);
    assert(rc < 0);
    ++cmd_fuzz_cases;
  }

  // (d) 200 valid-header + random op_count + random payload buffers.
  for (int i = 0; i < 200; ++i) {
    const size_t payload_len = rand_range(0, 512);
    std::vector<uint8_t> buf(24 + payload_len);
    const uint32_t magic = NVE_ABI_MAGIC;
    const uint16_t version = NVE_ABI_VERSION;
    const uint16_t flags = 0;
    const uint32_t sid = 1;
    const uint64_t fid = static_cast<uint64_t>(i + 100);
    const uint32_t op_count = (i % 5 == 0) ? 0xFFFFFFFFu : static_cast<uint32_t>(rand_range(1, 64));
    std::memcpy(buf.data() + 0, &magic, 4);
    std::memcpy(buf.data() + 4, &version, 2);
    std::memcpy(buf.data() + 6, &flags, 2);
    std::memcpy(buf.data() + 8, &sid, 4);
    std::memcpy(buf.data() + 12, &fid, 8);
    std::memcpy(buf.data() + 20, &op_count, 4);
    for (size_t j = 24; j < buf.size(); ++j) {
      buf[j] = rand_u8();
    }
    const int32_t rc = nve_fabric_host_submit_frame(
        shared_host, NveBufferSlice{buf.data(), buf.size()}, nullptr);
    // Random bytes with op_count >= 1 will almost always fail cleanly with < 0; even if by rare
    // chance a valid encoding occurs, rc <= 0 and zero crashes/UB.
    assert(rc <= 0);
    ++cmd_fuzz_cases;
  }

  // (e) 200 mutated copies of the rich baseline buffer (bit flips, random overwrites, random splice/append).
  for (int i = 0; i < 200; ++i) {
    std::vector<uint8_t> mut = base_bytes;
    const size_t num_edits = rand_range(1, 6);
    for (size_t e = 0; e < num_edits; ++e) {
      const size_t mode = rng() % 4;
      if (mode == 0 && !mut.empty()) {
        mut[rng() % mut.size()] ^= static_cast<uint8_t>(1u << (rng() % 8));
      } else if (mode == 1 && !mut.empty()) {
        mut[rng() % mut.size()] = rand_u8();
      } else if (mode == 2 && mut.size() > 1) {
        mut.resize(rand_range(1, mut.size() - 1));
      } else {
        const size_t extra = rand_range(1, 32);
        for (size_t k = 0; k < extra; ++k) mut.push_back(rand_u8());
      }
    }
    NveFabricHostHandle fresh = nve_fabric_host_create(cfg);
    const int32_t rc = nve_fabric_host_submit_frame(
        fresh, NveBufferSlice{mut.data(), mut.size()}, nullptr);
    assert(rc <= 0);
    nve_fabric_host_destroy(fresh);
    ++cmd_fuzz_cases;
  }

  assert(cmd_fuzz_cases >= 600);

  // Verify shared_host survived 600+ hostile fuzz buffers and still mounts a clean valid frame!
  {
    CommandBufferEncoder recovery(1, 9999);
    recovery.create_node(9001, NVE_COMP_VIEW);
    recovery.create_node(9002, NVE_COMP_TEXT);
    recovery.set_text(9002, "Recovered cleanly");
    nve::LayoutProps lp{};
    lp.width = {1, 250.0f};
    lp.height = {1, 100.0f};
    recovery.set_layout(9001, lp);
    recovery.set_children(NVE_ROOT_NODE_ID, {9001});
    recovery.set_children(9001, {9002});
    recovery.commit(true);
    assert(nve_fabric_host_submit_frame(shared_host, recovery.finish(), nullptr) == 0);
    nve::LayoutRect r{};
    assert(nve::fabric_host_get_node_layout(shared_host, 9001, &r));
    assert(approx_eq(r.width, 250.0f) && approx_eq(r.height, 100.0f));
  }

  // Deep chain stress (1,000 nested nodes -> exercises iterative text style propagation & safe layout).
  {
    NveFabricHostHandle deep_host = nve_fabric_host_create(cfg);
    CommandBufferEncoder deep_enc(1, 1);
    uint32_t prev = NVE_ROOT_NODE_ID;
    for (uint32_t id = 2; id <= 1001; ++id) {
      deep_enc.create_node(id, static_cast<uint16_t>((id == 1001) ? NVE_COMP_TEXT : NVE_COMP_VIEW));
      deep_enc.set_children(prev, {id});
      prev = id;
    }
    deep_enc.set_text(1001, "Deep leaf");
    deep_enc.commit(true);
    assert(nve_fabric_host_submit_frame(deep_host, deep_enc.finish(), nullptr) == 0);
    nve_fabric_host_destroy(deep_host);
  }

  nve_fabric_host_destroy(shared_host);

  // (f) EventRing fuzz corpus (300+ cases): truncations, oversized counts/lengths, trailing garbage, random bytes.
  nve::EventRingWriter valid_ring;
  valid_ring.clear();
  valid_ring.push_press(1, 2, 100, 12.5f, 34.5f);
  valid_ring.push_long_press(1, 2, 200, 15.0f, 36.0f);
  valid_ring.push_scroll(1, 3, 300, 0.0f, 120.0f, 393.0f, 2000.0f);
  valid_ring.push_text_change(1, 4, 400, "hello world");
  valid_ring.push_focus(1, 5, 500);
  valid_ring.push_blur(1, 5, 600);
  valid_ring.push_layout(1, 6, 700, 0.0f, 0.0f, 300.0f, 200.0f);
  const std::string custom_json = "{\"ok\":true}";
  valid_ring.push_custom(
      1, 7, 800, "onCustom", reinterpret_cast<const uint8_t*>(custom_json.data()), custom_json.size());
  const std::vector<uint8_t> valid_ring_bytes = valid_ring.finalize();

  std::vector<nve::DecodedEvent> out_events;
  assert(nve::decode_event_ring(
      NveBufferSlice{valid_ring_bytes.data(), valid_ring_bytes.size()}, out_events));
  assert(out_events.size() == 8);

  // Null pointer + non-zero len -> false; zero len -> true
  assert(!nve::decode_event_ring(NveBufferSlice{nullptr, 16}, out_events));
  assert(nve::decode_event_ring(NveBufferSlice{nullptr, 0}, out_events));
  assert(out_events.empty());

  size_t ring_fuzz_cases = 0;

  // Every non-empty prefix truncation of valid_ring_bytes must return false and leave out_events empty.
  for (size_t cut = 1; cut < valid_ring_bytes.size(); ++cut) {
    assert(!nve::decode_event_ring(NveBufferSlice{valid_ring_bytes.data(), cut}, out_events));
    assert(out_events.empty());
    ++ring_fuzz_cases;
  }

  // Trailing garbage byte after valid ring -> false and out_events cleared.
  {
    std::vector<uint8_t> trailing = valid_ring_bytes;
    trailing.push_back(0xAA);
    assert(!nve::decode_event_ring(NveBufferSlice{trailing.data(), trailing.size()}, out_events));
    assert(out_events.empty());
    ++ring_fuzz_cases;
  }

  // Oversized count (0xFFFFFFFF) -> false without OOM.
  {
    const uint8_t huge_cnt[14] = {
        'N', 'V', 'E', 'V', 2, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0};
    assert(!nve::decode_event_ring(NveBufferSlice{huge_cnt, sizeof(huge_cnt)}, out_events));
    assert(out_events.empty());
    ++ring_fuzz_cases;
  }

  // 150 random EventRing buffers + 100 mutated copies of valid_ring_bytes.
  for (int i = 0; i < 150; ++i) {
    const size_t len = rand_range(1, 1024);
    std::vector<uint8_t> buf(len);
    for (auto& b : buf) b = rand_u8();
    const bool ok = nve::decode_event_ring(NveBufferSlice{buf.data(), buf.size()}, out_events);
    if (!ok) {
      assert(out_events.empty());
    }
    ++ring_fuzz_cases;
  }
  for (int i = 0; i < 100; ++i) {
    std::vector<uint8_t> mut = valid_ring_bytes;
    const size_t flips = rand_range(1, 4);
    for (size_t f = 0; f < flips; ++f) {
      mut[rng() % mut.size()] ^= static_cast<uint8_t>(1u << (rng() % 8));
    }
    const bool ok = nve::decode_event_ring(NveBufferSlice{mut.data(), mut.size()}, out_events);
    if (!ok) {
      assert(out_events.empty());
    }
    ++ring_fuzz_cases;
  }

  assert(ring_fuzz_cases >= 300);
}

} // namespace nve_test
