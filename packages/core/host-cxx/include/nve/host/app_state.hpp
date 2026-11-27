// NVE host runtime: persisted app state (`NveDevApi::snapshot_state` bytes) across host restarts.
//
// File: `"NVES"` magic, format version, host schema tag, payload length (u32 LE each), payload.
// Writes are atomic (temp file + rename), reads reject foreign / truncated / oversized / trailing
// data. `resolve_path` gives each app a stable per-user cache path, honoring:
//   NVE_STATE_DISABLE=1      no persistence (empty path)
//   NVE_STATE_PATH=<file>    explicit path
//   NVE_RESET_STATE=1        delete the existing file first (cold start)
#pragma once

#include <cctype>
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

namespace nve::host::app_state {

inline constexpr uint32_t kMagic = 0x5345564Eu;  // "NVES"
inline constexpr uint32_t kVersion = 1u;
inline constexpr uint32_t kMaxBytes = 64u * 1024u * 1024u;

[[nodiscard]] inline std::string env(const char* name) {
#if defined(_WIN32)
  char* raw = nullptr;
  size_t len = 0u;
  if (_dupenv_s(&raw, &len, name) != 0 || raw == nullptr) return {};
  std::string out(raw);
  std::free(raw);
  return out;
#else
  const char* raw = std::getenv(name);
  return raw != nullptr ? std::string(raw) : std::string();
#endif
}

[[nodiscard]] inline bool env_truthy(const char* name) {
  std::string v = env(name);
  for (char& c : v) c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
  return v == "1" || v == "true" || v == "yes" || v == "on";
}

/// Per-user cache directory for app state files.
[[nodiscard]] inline std::filesystem::path cache_dir() {
  namespace fs = std::filesystem;
#if defined(__APPLE__)
  const std::string base = env("HOME");
  if (!base.empty()) return fs::path(base) / "Library" / "Caches" / "dev.nve" / "app-state";
#elif defined(_WIN32)
  const std::string base = env("LOCALAPPDATA");
  if (!base.empty()) return fs::path(base) / "nve" / "app-state";
#else
  const std::string xdg = env("XDG_CACHE_HOME");
  if (!xdg.empty()) return fs::path(xdg) / "nve" / "app-state";
  const std::string base = env("HOME");
  if (!base.empty()) return fs::path(base) / ".cache" / "nve" / "app-state";
#endif
  return fs::temp_directory_path() / "nve" / "app-state";
}

/// `[a-z0-9_-]{1,48}` slug of the identity's file stem (`nve-app` when empty).
[[nodiscard]] inline std::string slug(std::string_view identity) {
  const std::string stem = std::filesystem::path(std::string(identity)).stem().string();
  std::string out;
  for (const char c : stem) {
    const auto uc = static_cast<unsigned char>(c);
    if (std::isalnum(uc) != 0 || c == '_' || c == '-') {
      out.push_back(static_cast<char>(std::tolower(uc)));
    } else if (!out.empty() && out.back() != '-') {
      out.push_back('-');
    }
    if (out.size() >= 48u) break;
  }
  while (!out.empty() && out.back() == '-') out.pop_back();
  return out.empty() ? std::string("nve-app") : out;
}

/// Stable state path of `identity` (an app module path or a host app name); "" when disabled.
[[nodiscard]] inline std::string resolve_path(std::string_view identity) {
  if (env_truthy("NVE_STATE_DISABLE")) return {};
  std::string path = env("NVE_STATE_PATH");
  if (path.empty()) {
    std::error_code ec;
    std::string canonical(identity);
    if (!canonical.empty()) {
      const auto resolved = std::filesystem::weakly_canonical(canonical, ec);
      if (!ec) canonical = resolved.string();
    }
    char hash[17] = {};
    std::snprintf(hash, sizeof(hash), "%08x", static_cast<uint32_t>(xxh64(canonical)));
    path = (cache_dir() / (slug(identity) + "-" + hash + ".nvestate")).string();
  }
  if (env_truthy("NVE_RESET_STATE")) {
    std::error_code ec;
    std::filesystem::remove(path, ec);
  }
  return path;
}

/// Atomically writes `state` tagged with `schema` (creates parent directories).
[[nodiscard]] inline bool write_file(const std::string& path, uint32_t schema, std::span<const uint8_t> state) {
  namespace fs = std::filesystem;
  if (path.empty() || state.size() > kMaxBytes) return false;
  std::error_code ec;
  const fs::path target(path);
  if (target.has_parent_path()) fs::create_directories(target.parent_path(), ec);
  const fs::path tmp = fs::path(path + ".tmp");
  {
    std::ofstream out(tmp, std::ios::binary | std::ios::trunc);
    const uint32_t header[4] = {kMagic, kVersion, schema, static_cast<uint32_t>(state.size())};
    for (const uint32_t w : header) {
      const char b[4] = {static_cast<char>(w), static_cast<char>(w >> 8u), static_cast<char>(w >> 16u),
                         static_cast<char>(w >> 24u)};
      out.write(b, 4);
    }
    out.write(reinterpret_cast<const char*>(state.data()), static_cast<std::streamsize>(state.size()));
    if (!out.flush()) {
      out.close();
      fs::remove(tmp, ec);
      return false;
    }
  }
  fs::rename(tmp, target, ec);
  if (ec) fs::remove(tmp, ec);
  return !ec;
}

/// Reads a state file written with the same `schema`; `nullopt` for anything else.
[[nodiscard]] inline std::optional<std::vector<uint8_t>> read_file(const std::string& path, uint32_t schema) {
  std::ifstream in(path, std::ios::binary);
  uint32_t header[4] = {};
  for (uint32_t& w : header) {
    unsigned char b[4];
    if (!in.read(reinterpret_cast<char*>(b), 4)) return std::nullopt;
    w = uint32_t{b[0]} | (uint32_t{b[1]} << 8u) | (uint32_t{b[2]} << 16u) | (uint32_t{b[3]} << 24u);
  }
  if (header[0] != kMagic || header[1] != kVersion || header[2] != schema || header[3] > kMaxBytes) {
    return std::nullopt;
  }
  std::vector<uint8_t> state(header[3]);
  if (!in.read(reinterpret_cast<char*>(state.data()), static_cast<std::streamsize>(state.size())) ||
      in.peek() != std::char_traits<char>::eof()) {
    return std::nullopt;
  }
  return state;
}

}  // namespace nve::host::app_state
