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

  // 12a. Canonical XXH64 golden vector check (exact match with Rust `nve_hash::xxh64`).
  {
    assert(nve::xxh64(nullptr, 0u, 0u) == 0xEF46DB3751D8E999ULL);
    assert(nve::xxh64(reinterpret_cast<const uint8_t*>("a"), 1u, 0u) == 0xD24EC4F1A98C6E5BULL);
    assert(nve::xxh64(reinterpret_cast<const uint8_t*>("abc"), 3u, 0u) == 0x44BC2CF5AD770999ULL);

    std::vector<uint8_t> seq(1024u);
    for (size_t i = 0; i < seq.size(); ++i) {
      seq[i] = static_cast<uint8_t>((i * 31u + 7u) & 0xFFu);
    }
    assert(nve::xxh64(seq.data(), seq.size(), 0u) == 0x149AA44972CDAE00ULL);
    // Verify streaming split at byte 24 matches one-shot digest.
    nve::Xxh64Hasher stream(0u);
    stream.update(seq.data(), 24u);
    stream.update(seq.data() + 24u, seq.size() - 24u);
    assert(stream.digest() == 0x149AA44972CDAE00ULL);
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
    assert(snap.format_version() == nve::kRelocFormatVersion);
    assert(snap.app_schema_version() == 3u);
    assert(snap.flags() == 0x00010002u);
    assert(snap.header().root_offset == nve::kRelocHeaderSize);
    assert(snap.bundle_root().frame0.offset == nve::kRelocBundleMinSize);
    assert(snap.bundle_root().frame0.offset % 8u == 0u);
    assert(snap.bundle_root().state.offset % 8u == 0u);
    assert(snap.bundle_root().meta.offset % 8u == 0u);
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

  // 12c. Empty sections roundtrip (exact 96-byte NVERELOC bundle container).
  {
    const fs::path empty_path = tmp.dir / "empty_sections.nvereloc";
    assert(nve::write_snapshot_atomic(
        empty_path.string(), 0u, NveBufferSlice{nullptr, 0u}, NveBufferSlice{nullptr, 0u}));
    nve::MmapSnapshot snap;
    assert(nve::MmapSnapshot::open(empty_path.string(), snap));
    assert(snap.size() == nve::kRelocBundleMinSize);
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

    // Non-zero reserved field (bytes 40..64 in 64-byte RelocHeader)
    {
      auto v = base;
      v[40] = 1u;
      assert(!nve::MmapSnapshot::from_bytes(std::move(v), bad, true, &st));
      assert(st == nve::SnapshotStatus::InvalidReservedHeader);
    }

    // Flipped bit in header prefix, checksum, schema_version, and payload
    for (size_t off : {size_t{12}, size_t{24}, size_t{32}, nve::kRelocHeaderSize, base.size() - 1u}) {
      auto v = base;
      v[off] ^= 0x01u;
      assert(!nve::MmapSnapshot::from_bytes(std::move(v), bad, true, &st));
      assert(st == nve::SnapshotStatus::ChecksumMismatch);
    }

    // Unaligned section offset (97) in RelocBundleRoot.frame0.offset (at byte 64)
    {
      auto v = base;
      const uint32_t bad_off = 97u;
      std::memcpy(v.data() + nve::kRelocHeaderSize + 0u, &bad_off, sizeof(bad_off));
      assert(!nve::MmapSnapshot::from_bytes(std::move(v), bad, false, &st));
      assert(st == nve::SnapshotStatus::UnalignedSection);
    }

    // Section offset inside header (< 64)
    {
      auto v = base;
      const uint32_t bad_off = 16u;
      std::memcpy(v.data() + nve::kRelocHeaderSize + 0u, &bad_off, sizeof(bad_off));
      assert(!nve::MmapSnapshot::from_bytes(std::move(v), bad, false, &st));
      assert(st == nve::SnapshotStatus::SectionOutOfBounds);
    }

    // Out-of-bounds length past EOF (state.len at offset 64 + 12)
    {
      auto v = base;
      const uint32_t huge_len = 999999u;
      std::memcpy(v.data() + nve::kRelocHeaderSize + 12u, &huge_len, sizeof(huge_len));
      assert(!nve::MmapSnapshot::from_bytes(std::move(v), bad, false, &st));
      assert(st == nve::SnapshotStatus::SectionOutOfBounds);
    }

    // uint32_t + uint32_t out-of-bounds (meta.offset + meta.len at offset 64 + 16)
    {
      auto v = base;
      const uint32_t off = static_cast<uint32_t>(nve::kRelocBundleMinSize);
      const uint32_t max_u32 = std::numeric_limits<uint32_t>::max();
      std::memcpy(v.data() + nve::kRelocHeaderSize + 16u, &off, sizeof(off));
      std::memcpy(v.data() + nve::kRelocHeaderSize + 20u, &max_u32, sizeof(max_u32));
      assert(!nve::MmapSnapshot::from_bytes(std::move(v), bad, false, &st));
      assert(st == nve::SnapshotStatus::SectionOutOfBounds);
    }

    // Overlapping sections (`state` points at `frame0`)
    {
      auto v = base;
      const uint32_t f0_off = static_cast<uint32_t>(nve::kRelocBundleMinSize);
      const uint32_t overlap_len = 8u;
      std::memcpy(v.data() + nve::kRelocHeaderSize + 8u, &f0_off, sizeof(f0_off));
      std::memcpy(v.data() + nve::kRelocHeaderSize + 12u, &overlap_len, sizeof(overlap_len));
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

  // 12h. Default snapshot path resolver, env overrides (NVE_SNAPSHOT_DISABLE, NVE_SNAPSHOT_PATH,
  //      NVE_RESET_STATE), sidecar frame0 lookup, and nested parent dir auto-creation.
  {
    struct EnvRestoreGuard {
      std::string name;
      bool had_value{false};
      std::string prev_value;
      explicit EnvRestoreGuard(std::string env_name) : name(std::move(env_name)) {
#if defined(_MSC_VER)
        char* raw = nullptr;
        size_t len = 0;
        if (_dupenv_s(&raw, &len, name.c_str()) == 0 && raw != nullptr) {
          had_value = true;
          prev_value = raw;
          std::free(raw);
        }
#else
        if (const char* raw = std::getenv(name.c_str()); raw != nullptr) {
          had_value = true;
          prev_value = raw;
        }
#endif
      }
      void set(const char* val) const {
#if defined(_WIN32)
        (void)_putenv_s(name.c_str(), val != nullptr ? val : "");
#else
        if (val != nullptr) {
          (void)::setenv(name.c_str(), val, 1);
        } else {
          (void)::unsetenv(name.c_str());
        }
#endif
      }
      ~EnvRestoreGuard() {
        if (had_value) {
          set(prev_value.c_str());
        } else {
          set(nullptr);
        }
      }
    };

    EnvRestoreGuard g_disable("NVE_SNAPSHOT_DISABLE");
    EnvRestoreGuard g_path("NVE_SNAPSHOT_PATH");
    EnvRestoreGuard g_reset("NVE_RESET_STATE");
    g_disable.set(nullptr);
    g_path.set(nullptr);
    g_reset.set(nullptr);

    const std::string p1a = nve::resolve_default_snapshot_path("/apps/one/libdemo.dylib");
    const std::string p1b = nve::resolve_default_snapshot_path("/apps/one/libdemo.dylib");
    const std::string p2 = nve::resolve_default_snapshot_path("/apps/two/libdemo.dylib");
    assert(!p1a.empty());
    assert(p1a == p1b);
    assert(p1a != p2);
    assert(p1a.find("libdemo-") != std::string::npos);
    assert(p1a.ends_with(".nvereloc"));

    // NVE_SNAPSHOT_DISABLE=1 disables default snapshot path resolution.
    g_disable.set("1");
    assert(nve::resolve_default_snapshot_path("/apps/one/libdemo.dylib").empty());
    g_disable.set(nullptr);

    // NVE_SNAPSHOT_PATH overrides path cleanly.
    const fs::path custom_override = tmp.dir / "custom_override" / "nested" / "app.nvereloc";
    g_path.set(custom_override.string().c_str());
    assert(nve::resolve_default_snapshot_path("/apps/one/libdemo.dylib") ==
           custom_override.string());

    // write_snapshot_atomic auto-creates missing nested parent directories!
    const std::string f0_stub = "frame0-bytes";
    const std::string st_stub = "state-bytes";
    assert(nve::write_snapshot_atomic(
        custom_override.string(),
        1u,
        NveBufferSlice{reinterpret_cast<const uint8_t*>(f0_stub.data()), f0_stub.size()},
        NveBufferSlice{reinterpret_cast<const uint8_t*>(st_stub.data()), st_stub.size()}));
    assert(fs::is_regular_file(custom_override));

    // NVE_RESET_STATE=1 deletes existing snapshot before boot and still returns target path.
    g_reset.set("1");
    assert(nve::resolve_default_snapshot_path("/apps/one/libdemo.dylib") ==
           custom_override.string());
    assert(!fs::exists(custom_override));
    g_reset.set(nullptr);
    g_path.set(nullptr);

    // Sidecar Frame-0 lookup priority: <stem>.frame0.nvereloc > <stem-no-lib>.frame0.nvereloc > frame0.nvereloc.
    const fs::path sidecar_dir = tmp.dir / "sidecar";
    fs::create_directories(sidecar_dir);
    const fs::path fake_dylib = sidecar_dir / "libsample_app.dylib";
    {
      std::ofstream out(fake_dylib, std::ios::binary);
      out << "dylib";
    }
    assert(nve::resolve_sidecar_frame0_path(fake_dylib.string()).empty());

    const fs::path generic_f0 = sidecar_dir / "frame0.nvereloc";
    {
      std::ofstream out(generic_f0, std::ios::binary);
      out << "f0";
    }
    assert(nve::resolve_sidecar_frame0_path(fake_dylib.string()) == generic_f0.string());

    const fs::path stripped_f0 = sidecar_dir / "sample_app.frame0.nvereloc";
    {
      std::ofstream out(stripped_f0, std::ios::binary);
      out << "f0";
    }
    assert(nve::resolve_sidecar_frame0_path(fake_dylib.string()) == stripped_f0.string());

    const fs::path stem_f0 = sidecar_dir / "libsample_app.frame0.nvereloc";
    {
      std::ofstream out(stem_f0, std::ios::binary);
      out << "f0";
    }
    assert(nve::resolve_sidecar_frame0_path(fake_dylib.string()) == stem_f0.string());

    // 2-launch lifecycle with auto-created nested directory and continued mutation after restore.
    const fs::path nested_lifecycle =
        tmp.dir / "lifecycle" / "deep" / "cache" / "app-state.nvereloc";
    const NveSurfaceConfig cfg{
        .surface_id = 15, .width = 480.0f, .height = 320.0f, .scale_factor = 1.0f};
    {
      nve::NativeHostSession launch1;
      assert(launch1.attach_interface(&MockEmbeddedRustComponent::get_interface, cfg));
      for (int i = 0; i < 5; ++i) {
        assert(launch1.dispatch_press(2, 12.0f, 12.0f) == 1u);
      }
      assert(launch1.save_snapshot(nested_lifecycle.string(), 1u));
    }
    assert(fs::is_regular_file(nested_lifecycle));

    {
      nve::NativeHostSession launch2;
      launch2.set_surface_config(cfg);
      assert(launch2.replay_frame0_snapshot(nested_lifecycle.string(), 1u));
      assert(!launch2.is_valid());
      assert(launch2.has_replayed_frame0());
      assert(launch2.has_pending_snapshot_state());

      bool saw_count5_frame0 = false;
      for (const auto& node : launch2.snapshot_nodes()) {
        if (node.node_id == 3u && node.text == "Count: 5") {
          saw_count5_frame0 = true;
        }
      }
      assert(saw_count5_frame0);

      // Mount live app: pending snapshot state must be restored BEFORE first live render!
      assert(launch2.attach_interface(&MockEmbeddedRustComponent::get_interface, cfg));
      assert(launch2.is_valid());
      assert(!launch2.has_pending_snapshot_state());

      bool saw_count5_live = false;
      for (const auto& node : launch2.snapshot_nodes()) {
        if (node.node_id == 3u && node.text == "Count: 5") {
          saw_count5_live = true;
        }
      }
      assert(saw_count5_live);

      // Mutating after restore increments from 5 -> 6 (proves state was truly restored, not just Frame-0!).
      assert(launch2.dispatch_press(2, 12.0f, 12.0f) == 1u);
      bool saw_count6_live = false;
      for (const auto& node : launch2.snapshot_nodes()) {
        if (node.node_id == 3u && node.text == "Count: 6") {
          saw_count6_live = true;
        }
      }
      assert(saw_count6_live);
    }

    if (const std::string studio_dylib = read_env("NVE_TEST_STUDIO_DYLIB"); !studio_dylib.empty()) {
      const fs::path studio_snap = tmp.dir / "studio" / "cache" / "studio.nvereloc";
      const NveSurfaceConfig studio_cfg{.surface_id = 1, .width = 960.0f, .height = 640.0f, .scale_factor = 1.0f};
      {
        nve::NativeHostSession s1;
        assert(s1.load_dylib(studio_dylib, studio_cfg));
        const NveMapEntry studio_props[3] = {
            {nve::make_str_slice("view"), nve::make_str_value("docs")},
            {nve::make_str_slice("studioOpen"), nve::make_bool_value(true)},
            {nve::make_str_slice("accent"), nve::make_str_value("emerald")},
        };
        assert(s1.update_props(studio_props, 3u, /*auto_render=*/true));
        assert(s1.save_snapshot(studio_snap.string(), 1u));
      }
      nve::MmapSnapshot verify_snap;
      assert(nve::MmapSnapshot::open_with_schema(studio_snap.string(), 1u, verify_snap));
      assert(verify_snap.frame0_slice().len > 64u && verify_snap.state_slice().len > 4u);
      assert(std::memcmp(verify_snap.state_slice().ptr, "NWS1", 4u) == 0);
      const std::vector<uint8_t> saved_state_bytes(
          verify_snap.state_slice().ptr, verify_snap.state_slice().ptr + verify_snap.state_slice().len);
      verify_snap.close();
      {
        nve::NativeHostSession s2;
        s2.set_surface_config(studio_cfg);
        assert(s2.replay_frame0_snapshot(studio_snap.string(), 1u));
        assert(!s2.is_valid() && s2.has_replayed_frame0() && s2.has_pending_snapshot_state());
        assert(s2.node_count() > 20u);
        assert(s2.load_dylib(studio_dylib, studio_cfg));
        assert(s2.is_valid() && !s2.has_pending_snapshot_state() && s2.node_count() > 20u);
        const std::vector<uint8_t> restored = s2.bridge()->snapshot_state();
        assert(restored == saved_state_bytes);
      }
    }
  }
}

} // namespace nve_test
