// NVE host runtime: the command line and app-state policy shared by every native shell.
//
//   --dylib <app>  --pair <uri>  --pair-token <token>  --state <file>  --no-state  --reset-state
//   --schema <ver>  --width W  --height H     (+ host-specific flags through `extra`)
//   env: NVE_STATE_DISABLE=1 (wins even over --state), NVE_STATE_PATH=<file> (without --state),
//        NVE_RESET_STATE=1 (same as --reset-state)
//
// State lifecycle: `resolve_launch_state` after parsing picks the file; once the app source is
// attached, `state_action_after_load` says whether to restore or clear it. A failed `--dylib` /
// `--pair` that fell back to a built-in app disables persistence: the fallback must neither
// restore, clear, nor overwrite the failed app's state. Paired apps keep their state on the server.
#pragma once

#include <charconv>
#include <cstdint>
#include <functional>
#include <string>
#include <string_view>
#include <utility>

#include "app_state.hpp"

namespace nve::host {

struct LaunchOptions {
  std::string dylib_path;
  std::string pair_uri;
  std::string pair_token;
  std::string state_path;
  uint32_t schema_version = 1u;
  bool no_state = false;
  bool reset_state = false;
  int width = 640;
  int height = 420;
};

inline constexpr std::string_view kLaunchUsage =
    "[--dylib <app cdylib>] [--pair <nve://|quic://|wt://|ws(s)://...>] [--pair-token <token>]"
    " [--state <file>] [--no-state] [--reset-state] [--schema <ver>] [--width W] [--height H]";
inline constexpr std::string_view kLaunchEnvUsage = "Env: NVE_STATE_DISABLE=1, NVE_STATE_PATH=<file>, NVE_RESET_STATE=1";

enum class LaunchParse : uint8_t { Run, Help, Error };

/// Host-specific flag: `value` is the following argument (or `nullptr`); returns how many
/// arguments it consumed (0: unknown flag, 1: the flag, 2: flag + value).
using LaunchExtraFlag = std::function<int(std::string_view flag, const char* value)>;

namespace launch_detail {
template <class T>
[[nodiscard]] bool parse_number(const char* s, T min, T& out) noexcept {
  if (s == nullptr) return false;
  const std::string_view v(s);
  T parsed{};
  const auto [end, ec] = std::from_chars(v.data(), v.data() + v.size(), parsed);
  if (ec != std::errc() || end != v.data() + v.size() || parsed < min) return false;
  out = parsed;
  return true;
}
}  // namespace launch_detail

/// Parses `argv[1..argc)`; `error` gets the reason for `Error`.
[[nodiscard]] inline LaunchParse parse_launch_args(int argc, const char* const argv[], LaunchOptions& o,
                                                   std::string* error, const LaunchExtraFlag& extra = {}) {
  const auto fail = [error](std::string message) {
    if (error != nullptr) *error = std::move(message);
    return LaunchParse::Error;
  };
  for (int i = 1; i < argc; ++i) {
    const std::string_view flag(argv[i] != nullptr ? argv[i] : "");
    const char* value = i + 1 < argc ? argv[i + 1] : nullptr;
    const auto text = [&](std::string& out) {
      if (value == nullptr) return false;
      out = value;
      ++i;
      return true;
    };
    const auto number = [&](auto min, auto& out) {
      if (!launch_detail::parse_number(value, min, out)) return false;
      ++i;
      return true;
    };
    bool ok = true;
    if (flag == "--help" || flag == "-h") return LaunchParse::Help;
    if (flag == "--no-state") {
      o.no_state = true;
    } else if (flag == "--reset-state") {
      o.reset_state = true;
    } else if (flag == "--dylib") {
      ok = text(o.dylib_path);
    } else if (flag == "--pair") {
      ok = text(o.pair_uri);
    } else if (flag == "--pair-token") {
      ok = text(o.pair_token);
    } else if (flag == "--state") {
      ok = text(o.state_path);
    } else if (flag == "--schema") {
      ok = number(uint32_t{0}, o.schema_version);
    } else if (flag == "--width") {
      ok = number(1, o.width);
    } else if (flag == "--height") {
      ok = number(1, o.height);
    } else if (const int used = extra ? extra(flag, value) : 0; used > 0) {
      i += used - 1;
    } else {
      return fail("unknown argument '" + std::string(flag) + "'");
    }
    if (!ok) return fail("'" + std::string(flag) + "' needs a valid value");
  }
  if (!o.dylib_path.empty() && !o.pair_uri.empty()) return fail("--dylib and --pair are exclusive");
  return LaunchParse::Run;
}

/// Picks the state file after parsing (`builtin_identity` names the built-in app's file).
inline void resolve_launch_state(LaunchOptions& o, std::string_view builtin_identity) {
  o.reset_state = o.reset_state || app_state::reset_requested();
  if (o.no_state || !o.pair_uri.empty() || app_state::env_truthy("NVE_STATE_DISABLE")) {
    o.no_state = true;
    return;
  }
  if (o.state_path.empty()) o.state_path = app_state::resolve_path(o.dylib_path.empty() ? builtin_identity : o.dylib_path);
  o.no_state = o.state_path.empty();
}

enum class StateAction : uint8_t { None, Restore, Clear };

/// What to do with the state file once an app is attached. `requested_app_loaded`: the requested
/// source (`--dylib` / `--pair` / the built-in app) runs; `false` (a fallback) disables persistence.
[[nodiscard]] inline StateAction state_action_after_load(LaunchOptions& o, bool requested_app_loaded) {
  if (!requested_app_loaded) o.no_state = true;
  if (o.no_state) return StateAction::None;
  return o.reset_state ? StateAction::Clear : StateAction::Restore;
}

}  // namespace nve::host
