#include "test_common.hpp"

namespace nve_test {

// 12. Test nve::MmapSnapshot, atomic persistence, corruption/overflow/overlap guards,
//     zero-copy pointer-in-mapped-region proof, Rust<->C++ byte-for-byte wire compatibility,
//     and NativeHostSession::save_snapshot -> fresh NativeHostSession::replay_frame0_snapshot.
void test_mmap_snapshot_and_session_instant_boot() {
  namespace fs = std::filesystem;

  struct TempDirGuard {
    fs::path dir;
    explicit TempDirGuard(const std::string& tag) {
      dir = fs::temp_directory_path() /
            ("nve-cxx-snap-test-" + tag + "-" + std::to_string(nve::current_timestamp_us()));
      std::error_code ec;
      fs::remove_all(dir, ec);
      fs::create_directories(dir, ec);
      assert(!ec);
    }
    ~TempDirGuard() {
      std::error_code ec;
      fs::remove_all(dir, ec);
    }
  };

  auto assert_slice_in_region = [](const nve::MmapSnapshot& snap, NveBufferSlice slice) {
    const auto base = reinterpret_cast<uintptr_t>(snap.data());
    const auto end = base + snap.size();
    const auto s_start = reinterpret_cast<uintptr_t>(slice.ptr);
    const auto s_end = s_start + slice.len;
    assert(s_start >= base && s_end <= end);
  };

  TempDirGuard tmp("core");

  // 12a. FNV-1a 64-bit golden vector check (exact match with Rust `nve_engine::snapshot::fnv1a_64`).
  {
    const char* sample = "NVE_SNAP";
    const uint64_t h = nve::fnv1a_64(reinterpret_cast<const uint8_t*>(sample), 8u);
    // Compute reference step-by-step with FNV-1a 64-bit parameters.
    uint64_t ref = 0xcbf29ce484222325ULL;
    for (size_t i = 0; i < 8u; ++i) {
      ref ^= static_cast<uint64_t>(static_cast<uint8_t>(sample[i]));
      ref *= 0x00000100000001b3ULL;
    }
    assert(h == ref);
  }

  // 12b. Atomic write -> mmap open -> 8-byte alignment + true zero-copy pointer-in-region proof.
  {
    const fs::path snap_path = tmp.dir / "roundtrip.nvesnap";

    CommandBufferEncoder enc(7, 42);
    enc.create_node(2, NVE_COMP_VIEW);
    enc.create_node(3, NVE_COMP_TEXT);
    enc.set_children(NVE_ROOT_NODE_ID, {2});
    enc.set_children(2, {3});
    enc.set_text(3, "Instant-boot Frame-0 payload");
    enc.commit(true);
    const NveBufferSlice f0 = enc.finish();

    // Unaligned lengths (37 and 25 bytes) exercise 8-byte zero padding.
    std::vector<uint8_t> state(37u, 0xABu);
    const std::string meta = "viewport=640x420@2x;gen=7";

    nve::SnapshotStatus write_st = nve::SnapshotStatus::IoError;
    assert(nve::write_snapshot_atomic(
        snap_path.string(),
        3u,
        f0,
        NveBufferSlice{state.data(), state.size()},
        NveBufferSlice{reinterpret_cast<const uint8_t*>(meta.data()), meta.size()},
        0x00010002u,
        &write_st));
    assert(write_st == nve::SnapshotStatus::Ok);

    // Ensure no leftover .tmp files exist in directory.
    size_t file_count = 0;
    for (const auto& entry : fs::directory_iterator(tmp.dir)) {
      (void)entry;
      ++file_count;
    }
    assert(file_count == 1u);

    nve::MmapSnapshot snap;
    nve::SnapshotStatus open_st = nve::SnapshotStatus::IoError;
    assert(nve::MmapSnapshot::open_with_schema(
        snap_path.string(), 3u, snap, /*verify_checksum=*/true, &open_st));
    assert(open_st == nve::SnapshotStatus::Ok);
    assert(snap.is_open());
    assert(snap.format_version() == nve::kSnapshotFormatVersion);
    assert(snap.app_schema_version() == 3u);
    assert(snap.flags() == 0x00010002u);
    assert(snap.header().frame0_offset == nve::kSnapshotHeaderSize);
    assert(snap.header().frame0_offset % 8u == 0u);
    assert(snap.header().state_offset % 8u == 0u);
    assert(snap.header().meta_offset % 8u == 0u);
    assert(snap.verify_checksum());

    const NveBufferSlice mapped_f0 = snap.frame0_slice();
    const NveBufferSlice mapped_st = snap.state_slice();
    const NveBufferSlice mapped_mt = snap.meta_slice();
    assert_slice_in_region(snap, mapped_f0);
    assert_slice_in_region(snap, mapped_st);
    assert_slice_in_region(snap, mapped_mt);

    assert(mapped_f0.len == f0.len && std::memcmp(mapped_f0.ptr, f0.ptr, f0.len) == 0);
    assert(mapped_st.len == state.size() && std::memcmp(mapped_st.ptr, state.data(), state.size()) == 0);
    assert(mapped_mt.len == meta.size() && std::memcmp(mapped_mt.ptr, meta.data(), meta.size()) == 0);

    // Move semantics preserve mapped zero-copy view cleanly.
    nve::MmapSnapshot moved = std::move(snap);
    assert(!snap.is_open());
    assert(moved.is_open());
    assert(moved.frame0_slice().ptr == mapped_f0.ptr);

    // Schema mismatch rejected cleanly.
    nve::MmapSnapshot wrong_schema;
    nve::SnapshotStatus schema_st = nve::SnapshotStatus::Ok;
    assert(!nve::MmapSnapshot::open_with_schema(
        snap_path.string(), 99u, wrong_schema, true, &schema_st));
    assert(schema_st == nve::SnapshotStatus::SchemaVersionMismatch);

    // In-memory `encode_snapshot` matches on-disk bytes byte-for-byte.
    const std::vector<uint8_t> encoded = nve::encode_snapshot(
        3u,
        0x00010002u,
        f0,
        NveBufferSlice{state.data(), state.size()},
        NveBufferSlice{reinterpret_cast<const uint8_t*>(meta.data()), meta.size()});
    assert(encoded.size() == moved.size());
    assert(std::memcmp(encoded.data(), moved.data(), encoded.size()) == 0);

#if !defined(_WIN32)
    // Atomic overwrite while `moved` is still mapped: new reader sees v4, old mapping stays intact.
    const std::string state_v4 = "state-v4-replacement";
    assert(nve::write_snapshot_atomic(
        snap_path.string(),
        4u,
        f0,
        NveBufferSlice{reinterpret_cast<const uint8_t*>(state_v4.data()), state_v4.size()}));
    nve::MmapSnapshot snap_v4;
    assert(nve::MmapSnapshot::open_with_schema(snap_path.string(), 4u, snap_v4));
    assert(snap_v4.state_slice().len == state_v4.size());
    assert(moved.app_schema_version() == 3u);
    assert(moved.state_slice().len == state.size());
#endif
  }

  // 12c. Empty sections roundtrip (exact 80-byte container).
  {
    const fs::path empty_path = tmp.dir / "empty_sections.nvesnap";
    assert(nve::write_snapshot_atomic(
        empty_path.string(), 0u, NveBufferSlice{nullptr, 0u}, NveBufferSlice{nullptr, 0u}));
    nve::MmapSnapshot snap;
    assert(nve::MmapSnapshot::open(empty_path.string(), snap));
    assert(snap.size() == nve::kSnapshotHeaderSize);
    assert(snap.frame0_slice().len == 0u);
    assert(snap.state_slice().len == 0u);
    assert(snap.meta_slice().len == 0u);
  }

  // 12d. Exhaustive corrupt/0-byte/truncated/flipped-bit/alignment/overflow/overlap rejections.
  {
    nve::MmapSnapshot bad;
    nve::SnapshotStatus st = nve::SnapshotStatus::Ok;

    // Missing file
    assert(!nve::MmapSnapshot::open((tmp.dir / "missing.nvesnap").string(), bad, true, &st));
    assert(st == nve::SnapshotStatus::IoError);

    // 0-byte file (must not pass len=0 to mmap / CreateFileMappingW)
    const fs::path zero_path = tmp.dir / "zero.nvesnap";
    {
      std::ofstream out(zero_path, std::ios::binary);
    }
    assert(!nve::MmapSnapshot::open(zero_path.string(), bad, true, &st));
    assert(st == nve::SnapshotStatus::Truncated);

    // Truncated (< 80 bytes)
    const fs::path trunc_path = tmp.dir / "trunc.nvesnap";
    {
      std::ofstream out(trunc_path, std::ios::binary);
      const char zeros[40] = {};
      out.write(zeros, sizeof(zeros));
    }
    assert(!nve::MmapSnapshot::open(trunc_path.string(), bad, true, &st));
    assert(st == nve::SnapshotStatus::Truncated);

    const std::string f0_str = "frame0-payload-data";
    const std::string st_str = "state-payload-data";
    const std::string mt_str = "meta";
    const std::vector<uint8_t> base = nve::encode_snapshot(
        5u,
        7u,
        NveBufferSlice{reinterpret_cast<const uint8_t*>(f0_str.data()), f0_str.size()},
        NveBufferSlice{reinterpret_cast<const uint8_t*>(st_str.data()), st_str.size()},
        NveBufferSlice{reinterpret_cast<const uint8_t*>(mt_str.data()), mt_str.size()});

    // Bad magic
    {
      auto v = base;
      v[0] = 'X';
      assert(!nve::MmapSnapshot::from_bytes(std::move(v), bad, true, &st));
      assert(st == nve::SnapshotStatus::InvalidMagic);
    }

    // Unsupported format version
    {
      auto v = base;
      v[8] = 2u;
      assert(!nve::MmapSnapshot::from_bytes(std::move(v), bad, true, &st));
      assert(st == nve::SnapshotStatus::UnsupportedFormatVersion);
    }

    // Non-zero reserved field
    {
      auto v = base;
      v[20] = 1u;
      assert(!nve::MmapSnapshot::from_bytes(std::move(v), bad, true, &st));
      assert(st == nve::SnapshotStatus::InvalidReservedHeader);
    }

    // Flipped bit in header prefix, checksum, and payload
    for (size_t off : {size_t{12}, size_t{16}, size_t{24}, nve::kSnapshotHeaderSize, base.size() - 1u}) {
      auto v = base;
      v[off] ^= 0x01u;
      assert(!nve::MmapSnapshot::from_bytes(std::move(v), bad, true, &st));
      assert(st == nve::SnapshotStatus::ChecksumMismatch);
    }

    // Unaligned section offset (81)
    {
      auto v = base;
      const uint64_t bad_off = 81u;
      std::memcpy(v.data() + 32u, &bad_off, sizeof(bad_off));
      assert(!nve::MmapSnapshot::from_bytes(std::move(v), bad, false, &st));
      assert(st == nve::SnapshotStatus::UnalignedSection);
    }

    // Section offset inside header (< 80)
    {
      auto v = base;
      const uint64_t bad_off = 16u;
      std::memcpy(v.data() + 32u, &bad_off, sizeof(bad_off));
      assert(!nve::MmapSnapshot::from_bytes(std::move(v), bad, false, &st));
      assert(st == nve::SnapshotStatus::SectionOutOfBounds);
    }

    // Out-of-bounds length past EOF
    {
      auto v = base;
      const uint64_t huge_len = 999999u;
      std::memcpy(v.data() + 56u, &huge_len, sizeof(huge_len));
      assert(!nve::MmapSnapshot::from_bytes(std::move(v), bad, false, &st));
      assert(st == nve::SnapshotStatus::SectionOutOfBounds);
    }

    // uint64_t overflow (`offset + len`)
    {
      auto v = base;
      const uint64_t off = nve::kSnapshotHeaderSize;
      const uint64_t max_u64 = std::numeric_limits<uint64_t>::max();
      std::memcpy(v.data() + 64u, &off, sizeof(off));
      std::memcpy(v.data() + 72u, &max_u64, sizeof(max_u64));
      assert(!nve::MmapSnapshot::from_bytes(std::move(v), bad, false, &st));
      assert(st == nve::SnapshotStatus::SectionOutOfBounds);
    }

    // Overlapping sections (`state` points at `frame0`)
    {
      auto v = base;
      const uint64_t f0_off = nve::kSnapshotHeaderSize;
      const uint64_t overlap_len = 8u;
      std::memcpy(v.data() + 48u, &f0_off, sizeof(f0_off));
      std::memcpy(v.data() + 56u, &overlap_len, sizeof(overlap_len));
      assert(!nve::MmapSnapshot::from_bytes(std::move(v), bad, false, &st));
      assert(st == nve::SnapshotStatus::OverlappingSections);
    }
  }

  // 12e. NativeHostSession::save_snapshot -> brand-new NativeHostSession::replay_frame0_snapshot
  //      restores exact rendered tree BEFORE app loads, and seamlessly restores state upon load!
  {
    const fs::path session_snap = tmp.dir / "mock_session.nvesnap";
    const NveSurfaceConfig cfg{.surface_id = 11, .width = 360.0f, .height = 240.0f, .scale_factor = 2.0f};

    nve::NativeHostSession writer;
    assert(writer.attach_interface(&MockEmbeddedRustComponent::get_interface, cfg));
    assert(writer.dispatch_press(2, 10.0f, 10.0f) == 1u);
    assert(writer.dispatch_press(2, 10.0f, 10.0f) == 1u);
    assert(writer.dispatch_press(2, 10.0f, 10.0f) == 1u);
    assert(writer.save_snapshot(session_snap.string(), 7u));

    nve::NativeHostSession cold;
    cold.set_surface_config(cfg);
    assert(!cold.is_valid());
    assert(!cold.has_replayed_frame0());

    // Wrong schema must fail cleanly without mutating cold session.
    nve::SnapshotStatus bad_schema_st = nve::SnapshotStatus::Ok;
    assert(!cold.replay_frame0_snapshot(session_snap.string(), 999u, nullptr, &bad_schema_st));
    assert(bad_schema_st == nve::SnapshotStatus::SchemaVersionMismatch);
    assert(!cold.has_replayed_frame0());

    NveFrameStats f0_stats{};
    const auto t0 = std::chrono::steady_clock::now();
    assert(cold.replay_frame0_snapshot(session_snap.string(), 7u, &f0_stats));
    const auto t1 = std::chrono::steady_clock::now();
    const auto restore_us =
        std::chrono::duration_cast<std::chrono::microseconds>(t1 - t0).count();
    std::cout << "[bridge_test] Cold mmap Frame-0 restore (mock session): " << restore_us
              << " us (ops=" << f0_stats.op_count << ")\n";

    // Frame 0 is immediately rendered BEFORE attach_interface / app boot!
    assert(!cold.is_valid());
    assert(cold.has_replayed_frame0());
    assert(cold.has_pending_snapshot_state());
    assert(cold.node_count() == 3u);
    bool saw_count3_frame0 = false;
    for (const auto& node : cold.snapshot_nodes()) {
      if (node.node_id == 3u && node.text == "Count: 3") {
        saw_count3_frame0 = true;
      }
    }
    assert(saw_count3_frame0);

    // Now attach live app: cached snapshot state is automatically restored via `restore_state`!
    assert(cold.attach_interface(&MockEmbeddedRustComponent::get_interface, cfg));
    assert(cold.is_valid());
    assert(!cold.has_replayed_frame0());
    assert(!cold.has_pending_snapshot_state());
    bool saw_count3_live = false;
    for (const auto& node : cold.snapshot_nodes()) {
      if (node.node_id == 3u && node.text == "Count: 3") {
        saw_count3_live = true;
      }
    }
    assert(saw_count3_live);
  }

#ifdef NVE_BRIDGE_FIXTURE_VALID
  // 12f. Real dynamic library session save_snapshot -> fresh session replay_frame0_snapshot -> load_dylib!
  {
    const fs::path dylib_snap = tmp.dir / "dylib_session.nvesnap";
    const NveSurfaceConfig cfg{.surface_id = 12, .width = 360.0f, .height = 240.0f, .scale_factor = 2.0f};

    nve::NativeHostSession s1;
    assert(s1.load_dylib(NVE_BRIDGE_FIXTURE_VALID, cfg));
    assert(s1.dispatch_press(2, 10.0f, 10.0f) == 1u);
    assert(s1.dispatch_press(2, 10.0f, 10.0f) == 1u);
    assert(s1.save_snapshot(dylib_snap.string(), 1u));

    nve::NativeHostSession s2;
    s2.set_surface_config(cfg);
    assert(s2.replay_frame0_snapshot(dylib_snap.string(), 1u));
    assert(!s2.is_valid() && s2.has_replayed_frame0());
    bool saw_f0 = false;
    for (const auto& n : s2.snapshot_nodes()) {
      if (n.node_id == 3u && n.text == "Fixture:2") {
        saw_f0 = true;
      }
    }
    assert(saw_f0);

    assert(s2.load_dylib(NVE_BRIDGE_FIXTURE_VALID, cfg));
    bool saw_live = false;
    for (const auto& n : s2.snapshot_nodes()) {
      if (n.node_id == 3u && n.text == "Fixture:2") {
        saw_live = true;
      }
    }
    assert(saw_live);
  }
#endif

  // 12g. Optional cross-language Rust <-> C++ compatibility probe via environment variables.
  const auto read_env = [](const char* name) -> std::string {
#if defined(_MSC_VER)
    char* raw = nullptr;
    size_t len = 0;
    if (_dupenv_s(&raw, &len, name) != 0 || raw == nullptr) {
      return {};
    }
    std::string out(raw);
    std::free(raw);
    return out;
#else
    const char* raw = std::getenv(name);
    return raw != nullptr ? std::string(raw) : std::string{};
#endif
  };

  if (const std::string rust_snap = read_env("NVE_TEST_RUST_SNAPSHOT_IN"); !rust_snap.empty()) {
    nve::MmapSnapshot snap;
    assert(nve::MmapSnapshot::open_with_schema(rust_snap, 1u, snap, /*verify_checksum=*/true));
    assert(snap.frame0_slice().len > 24u);
    assert(snap.state_slice().len > 0u);
    assert_slice_in_region(snap, snap.frame0_slice());
    assert_slice_in_region(snap, snap.state_slice());

    const NveSurfaceConfig cfg{.surface_id = 1, .width = 960.0f, .height = 640.0f, .scale_factor = 1.0f};
    nve::NativeHostSession cross;
    cross.set_surface_config(cfg);
    NveFrameStats stats{};
    const auto t0 = std::chrono::steady_clock::now();
    assert(cross.replay_frame0_snapshot(rust_snap, 1u, &stats));
    const auto t1 = std::chrono::steady_clock::now();
    std::cout << "[bridge_test] Cross-language Rust->C++ mmap Frame-0 replay: "
              << std::chrono::duration_cast<std::chrono::microseconds>(t1 - t0).count()
              << " us (nodes=" << cross.node_count() << ", ops=" << stats.op_count << ")\n";
    assert(cross.node_count() > 10u);

    if (const std::string cpp_out = read_env("NVE_TEST_CPP_SNAPSHOT_OUT"); !cpp_out.empty()) {
      assert(cross.save_snapshot(cpp_out, 1u, "cpp-roundtrip"));
    }
  }
}

} // namespace nve_test
