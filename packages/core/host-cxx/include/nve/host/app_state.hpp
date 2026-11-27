// NVE host runtime: where an app's persisted state lives. The file itself (format, checksum,
// atomic write, quarantine, schema rotation, deletion) is the runtime's `StateFile`, driven
// through `HostSession::save_state` / `restore_state_file` / `clear_state_file`.
//
// `resolve_path` gives each app a stable per-user cache path, honoring:
//   NVE_STATE_DISABLE=1      no persistence (empty path)
//   NVE_STATE_PATH=<file>    explicit path
// and `reset_requested` reads NVE_RESET_STATE=1 (cold start: the host clears the file).
#pragma once

#include <cctype>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <filesystem>
#include <string>
#include <string_view>
#include <system_error>

#include "xxh64.hpp"

namespace nve::host::app_state {

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
/// Pure: resolving never touches the file.
[[nodiscard]] inline std::string resolve_path(std::string_view identity) {
  if (env_truthy("NVE_STATE_DISABLE")) return {};
  if (std::string path = env("NVE_STATE_PATH"); !path.empty()) return path;
  std::error_code ec;
  std::string canonical(identity);
  if (!canonical.empty()) {
    const auto resolved = std::filesystem::weakly_canonical(canonical, ec);
    if (!ec) canonical = resolved.string();
  }
  char hash[17] = {};
  std::snprintf(hash, sizeof(hash), "%08x", static_cast<uint32_t>(xxh64(canonical)));
  return (cache_dir() / (slug(identity) + "-" + hash + ".nvestate")).string();
}

/// `NVE_RESET_STATE` asks for a cold start: clear the state file instead of restoring it.
[[nodiscard]] inline bool reset_requested() { return env_truthy("NVE_RESET_STATE"); }

}  // namespace nve::host::app_state
