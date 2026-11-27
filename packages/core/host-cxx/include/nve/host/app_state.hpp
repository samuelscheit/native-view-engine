// NVE host runtime: where an app's persisted state lives. The file itself (format, checksum,
// atomic write, quarantine, schema rotation, deletion) is the runtime's `StateFile`, driven
// through `HostSession::save_state` / `restore_state_file` / `clear_state_file`.
//
// `resolve_path` gives each app a stable per-user cache path, honoring:
//   NVE_STATE_DISABLE=1      no persistence (empty path)
//   NVE_STATE_PATH=<file>    explicit path
// and `reset_requested` reads NVE_RESET_STATE=1 (cold start: the host clears the file).
//
// Paths cross this API as UTF-8 (`std::string`), the encoding the runtime's path slots take; on
// Windows env vars are read wide and paths convert via `u8string`, never the ANSI code page.
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

/// UTF-8 text of `p` (`u8string`: lossless on every platform).
[[nodiscard]] inline std::string utf8(const std::filesystem::path& p) {
  const std::u8string s = p.u8string();
  return std::string(s.begin(), s.end());
}

/// Path from UTF-8 text.
[[nodiscard]] inline std::filesystem::path from_utf8(std::string_view s) {
  return std::filesystem::path(std::u8string(s.begin(), s.end()));
}

/// Env var `name` as a path ("" when unset); read wide on Windows.
[[nodiscard]] inline std::filesystem::path env_path(const char* name) {
#if defined(_WIN32)
  const std::wstring wide(name, name + std::char_traits<char>::length(name));  // names are ASCII
  wchar_t* raw = nullptr;
  size_t len = 0u;
  if (_wdupenv_s(&raw, &len, wide.c_str()) != 0 || raw == nullptr) return {};
  std::filesystem::path out(raw);
  std::free(raw);
  return out;
#else
  const char* raw = std::getenv(name);
  return raw != nullptr ? std::filesystem::path(raw) : std::filesystem::path();
#endif
}

[[nodiscard]] inline std::string env(const char* name) { return utf8(env_path(name)); }

[[nodiscard]] inline bool env_truthy(const char* name) {
  std::string v = env(name);
  for (char& c : v) c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
  return v == "1" || v == "true" || v == "yes" || v == "on";
}

/// Per-user cache directory for app state files.
[[nodiscard]] inline std::filesystem::path cache_dir() {
  namespace fs = std::filesystem;
#if defined(__APPLE__)
  const fs::path base = env_path("HOME");
  if (!base.empty()) return base / "Library" / "Caches" / "dev.nve" / "app-state";
#elif defined(_WIN32)
  const fs::path base = env_path("LOCALAPPDATA");
  if (!base.empty()) return base / "nve" / "app-state";
#else
  const fs::path xdg = env_path("XDG_CACHE_HOME");
  if (!xdg.empty()) return xdg / "nve" / "app-state";
  const fs::path base = env_path("HOME");
  if (!base.empty()) return base / ".cache" / "nve" / "app-state";
#endif
  return fs::temp_directory_path() / "nve" / "app-state";
}

/// `[a-z0-9_-]{1,48}` slug of the identity's file stem (`nve-app` when empty).
[[nodiscard]] inline std::string slug(std::string_view identity) {
  const std::string stem = utf8(from_utf8(identity).stem());
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
    const auto resolved = std::filesystem::weakly_canonical(from_utf8(canonical), ec);
    if (!ec) canonical = utf8(resolved);
  }
  char hash[17] = {};
  std::snprintf(hash, sizeof(hash), "%08x", static_cast<uint32_t>(xxh64(canonical)));
  return utf8(cache_dir() / from_utf8(slug(identity) + "-" + hash + ".nvestate"));
}

/// `NVE_RESET_STATE` asks for a cold start: clear the state file instead of restoring it.
[[nodiscard]] inline bool reset_requested() { return env_truthy("NVE_RESET_STATE"); }

}  // namespace nve::host::app_state
