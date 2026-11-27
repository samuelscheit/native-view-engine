// NVE host runtime: instant-boot snapshot file (C++20, header-only).
//
// One file = the runtime's Frame-0 keyframe (`RuntimeSession::encode_scene`, replayed with
// `submit_nve2` before any app attaches) + the app state (`snapshot_state`, handed back with
// `restore_state` once the app attaches) + a host-chosen schema version guarding stale files.
//
// Layout (little-endian): "NVSF" | version u32 (=1) | schema u32 | frame0_len u32 | state_len u32 |
// frame0 bytes | state bytes. Writes are atomic (temp file + rename) so a crash never leaves a torn
// snapshot behind.
#pragma once

#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <system_error>
#include <vector>

#include "xxh64.hpp"

namespace nve::host {

struct SnapshotFile {
  static constexpr uint32_t kMagic = 0x4653564Eu;  // "NVSF"
  static constexpr uint32_t kVersion = 1u;
  static constexpr size_t kHeaderBytes = 20u;

  uint32_t schema = 0u;
  std::vector<uint8_t> frame0;
  std::vector<uint8_t> state;

  [[nodiscard]] std::vector<uint8_t> encode() const {
    std::vector<uint8_t> out;
    out.reserve(kHeaderBytes + frame0.size() + state.size());
    for (const uint32_t v : {kMagic, kVersion, schema, static_cast<uint32_t>(frame0.size()), static_cast<uint32_t>(state.size())}) {
      for (uint32_t s = 0u; s < 32u; s += 8u) out.push_back(static_cast<uint8_t>((v >> s) & 0xFFu));
    }
    out.insert(out.end(), frame0.begin(), frame0.end());
    out.insert(out.end(), state.begin(), state.end());
    return out;
  }

  /// `nullopt` for a foreign / truncated / oversized file or a schema other than `expected_schema`.
  [[nodiscard]] static std::optional<SnapshotFile> decode(std::span<const uint8_t> bytes, uint32_t expected_schema) {
    if (bytes.size() < kHeaderBytes) return std::nullopt;
    auto u32 = [&](size_t off) {
      return uint32_t{bytes[off]} | (uint32_t{bytes[off + 1u]} << 8u) | (uint32_t{bytes[off + 2u]} << 16u) |
             (uint32_t{bytes[off + 3u]} << 24u);
    };
    if (u32(0u) != kMagic || u32(4u) != kVersion || u32(8u) != expected_schema) return std::nullopt;
    const uint64_t frame0_len = u32(12u);
    const uint64_t state_len = u32(16u);
    if (kHeaderBytes + frame0_len + state_len != bytes.size()) return std::nullopt;
    SnapshotFile f;
    f.schema = expected_schema;
    const auto body = bytes.subspan(kHeaderBytes);
    f.frame0.assign(body.begin(), body.begin() + static_cast<std::ptrdiff_t>(frame0_len));
    f.state.assign(body.begin() + static_cast<std::ptrdiff_t>(frame0_len), body.end());
    return f;
  }

  [[nodiscard]] static std::optional<SnapshotFile> read(const std::filesystem::path& path, uint32_t expected_schema) {
    std::ifstream in(path, std::ios::binary);
    if (!in) return std::nullopt;
    const std::vector<uint8_t> bytes((std::istreambuf_iterator<char>(in)), std::istreambuf_iterator<char>());
    return decode(bytes, expected_schema);
  }

  /// Atomically replaces `path` (creating parent directories).
  [[nodiscard]] bool write(const std::filesystem::path& path) const {
    std::error_code ec;
    if (path.has_parent_path()) std::filesystem::create_directories(path.parent_path(), ec);
    std::filesystem::path tmp = path;
    tmp += ".tmp";
    {
      std::ofstream out(tmp, std::ios::binary | std::ios::trunc);
      if (!out) return false;
      const std::vector<uint8_t> bytes = encode();
      out.write(reinterpret_cast<const char*>(bytes.data()), static_cast<std::streamsize>(bytes.size()));
      if (!out.flush()) return false;
    }
    std::filesystem::rename(tmp, path, ec);
    if (ec) std::filesystem::remove(tmp, ec);
    return !ec;
  }

  /// Per-app default location: `$NVE_SNAPSHOT_DIR`, else `$HOME/Library/Caches/dev.nve.snapshots`
  /// (`$TMPDIR` without `HOME`), named by `xxh64(app_identity)`.
  [[nodiscard]] static std::filesystem::path default_path(std::string_view app_identity) {
    std::filesystem::path dir;
    if (const char* custom = std::getenv("NVE_SNAPSHOT_DIR"); custom != nullptr && *custom != '\0') {
      dir = custom;
    } else if (const char* home = std::getenv("HOME"); home != nullptr && *home != '\0') {
      dir = std::filesystem::path(home) / "Library" / "Caches" / "dev.nve.snapshots";
    } else {
      dir = std::filesystem::temp_directory_path() / "dev.nve.snapshots";
    }
    char name[32];
    std::snprintf(name, sizeof(name), "%016llx.nvesnap", static_cast<unsigned long long>(xxh64(app_identity)));
    return dir / name;
  }
};

}  // namespace nve::host
