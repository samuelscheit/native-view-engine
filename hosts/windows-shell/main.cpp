// nve-windows-shell: runs an NVE app (built-in counter demo, a `cdylib`, or a paired `nve dev`
// server) in a native Win32 + Direct2D window, persisting the app's state across launches, or
// renders it headlessly to a `.bmp` (`--screenshot`). `wmain`: paths are UTF-16 on the command
// line and UTF-8 inside (any code page).
#include "NveWinHostWindow.hpp"

#include <nve/host/demo_counter_app.hpp>
#include <nve/host/launch_options.hpp>

#include <atomic>
#include <charconv>
#include <chrono>
#include <cmath>
#include <iostream>
#include <string>
#include <thread>

#if defined(_MSC_VER)  // Common Controls v6: EDIT cue banners (placeholders), themed controls
#pragma comment(linker, "/manifestdependency:\"type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'\"")
#endif

namespace {

using nve::host::demo::CounterApp;
using nve::win::NveWinHost;

struct Options : nve::host::LaunchOptions {
  std::string screenshot_path;
  float scale = 1.0f;  // --screenshot only (a window follows its monitor's DPI)
};

std::atomic<HWND> g_main_window{nullptr};

void print_usage() {
  std::cout << "Usage: nve_windows_shell " << nve::host::kLaunchUsage << " [--screenshot <out.bmp> [--scale S]]\n"
            << nve::host::kLaunchEnvUsage << "\n";
}

/// Runs the requested source; `false` when pairing or the dylib failed (the caller falls back
/// to the built-in counter, which then never touches the failed app's state).
bool load_app(NveWinHost& host, const Options& o) {
  if (!o.pair_uri.empty()) {
    std::string error;
    if (host.connect_paired(o.pair_uri, o.pair_token, &error)) return true;
    std::cerr << "[nve-windows-shell] pairing with '" << o.pair_uri << "' failed: " << error << "\n";
    return false;
  }
  if (!o.dylib_path.empty()) {
    nve::host::ModuleError code = nve::host::ModuleError::None;
    std::string error;
    if (host.load_dylib(o.dylib_path, &code, &error)) return true;
    std::cerr << "[nve-windows-shell] loading '" << o.dylib_path << "' failed (" << static_cast<int>(code)
              << "): " << error << "\n";
    return false;
  }
  return host.attach_interface(&CounterApp::get_interface);
}

/// Restores / clears the state per `state_action_after_load` (only once the requested app runs).
void apply_state(NveWinHost& host, Options& o, bool loaded) {
  switch (nve::host::state_action_after_load(o, loaded)) {
    case nve::host::StateAction::Clear:
      if (!host.clear_state(o.state_path)) {
        std::cerr << "[nve-windows-shell] clearing " << o.state_path << " failed: " << host.last_error() << "\n";
      }
      break;
    case nve::host::StateAction::Restore:
      if (host.restore_state(o.state_path, o.schema_version)) {
        std::cerr << "[nve-windows-shell] restored app state from " << o.state_path << "\n";
      }
      break;
    case nve::host::StateAction::None: break;
  }
}

void persist(NveWinHost& host, const Options& o, const char* trigger) {
  if (o.no_state || host.is_paired()) return;
  if (host.save_state(o.state_path, o.schema_version)) {
    std::cerr << "[nve-windows-shell] saved app state to " << o.state_path << " (" << trigger << ")\n";
  }
}

int screenshot(Options& o) {
  std::string error;
  const auto host = NveWinHost::create(nve::win::WinHostConfig{.surface_id = 1u,
                                                               .width = static_cast<float>(o.width),
                                                               .height = static_cast<float>(o.height),
                                                               .scale_factor = o.scale},
                                       &error);
  if (host == nullptr) {
    std::cerr << "[nve-windows-shell] " << error << "\n";
    return 1;
  }
  if (!load_app(*host, o)) return 1;  // a screenshot of a fallback app would be a false result
  // Paired frames arrive in the background: give the first one a moment.
  const auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(2);
  while (host->is_paired() && host->view_count() == 0u && std::chrono::steady_clock::now() < deadline) {
    std::this_thread::sleep_for(std::chrono::milliseconds(10));
    (void)host->pump();
  }
  apply_state(*host, o, true);
  const auto px = [&o](int dips) { return static_cast<uint32_t>(std::max(1.0f, std::round(static_cast<float>(dips) * o.scale))); };
  if (!nve::win::render_visual_tree_to_bmp(host->visual_tree(), host->text_measurer(), px(o.width), px(o.height), o.scale,
                                           o.screenshot_path, &error)) {
    std::cerr << "[nve-windows-shell] screenshot '" << o.screenshot_path << "' failed: " << error << "\n";
    return 1;
  }
  std::cout << "Rendered " << px(o.width) << "x" << px(o.height) << " to " << o.screenshot_path << "\n";
  return 0;
}

BOOL WINAPI on_console_ctrl(DWORD) {
  // Ctrl+C / console close arrive on another thread: close the window on its own thread.
  if (HWND w = g_main_window.load(); w != nullptr && ::PostMessageW(w, WM_CLOSE, 0, 0)) return TRUE;
  return FALSE;
}

int run_window(Options& o) {
  nve::win::NveWinHostWindow window(nve::win::WinHostWindowOptions{
      .title = "NVE Windows Host Shell", .width = o.width, .height = o.height, .surface_id = 1u, .quit_on_destroy = true});
  std::string error;
  if (!window.create(::GetModuleHandleW(nullptr), &error)) {
    std::cerr << "[nve-windows-shell] creating the window failed: " << error << "\n";
    return 1;
  }
  NveWinHost& host = window.host();
  const bool loaded = load_app(host, o);
  if (!loaded) (void)host.attach_interface(&CounterApp::get_interface);
  apply_state(host, o, loaded);
  if (host.is_paired()) window.set_title("NVE Windows Host Shell (paired: " + o.pair_uri + ")");
  window.set_on_close_request([&host, &o](uint32_t) {
    persist(host, o, "window-close");
    return true;
  });
  g_main_window.store(window.hwnd());
  (void)::SetConsoleCtrlHandler(&on_console_ctrl, TRUE);
  window.show(SW_SHOWDEFAULT);
  const int status = nve::win::NveWinHostWindow::run_message_loop();
  g_main_window.store(nullptr);
  (void)::SetConsoleCtrlHandler(&on_console_ctrl, FALSE);
  return status;
}

}  // namespace

int wmain(int argc, wchar_t* argv[]) {
  Options o;
  const nve::host::LaunchExtraFlag extra = [&o](std::string_view flag, const char* value) {
    if (value == nullptr) return 0;
    if (flag == "--screenshot") {
      o.screenshot_path = value;
      return 2;
    }
    if (flag == "--scale") {
      const std::string_view v(value);
      float s = 0.0f;
      const auto [end, ec] = std::from_chars(v.data(), v.data() + v.size(), s);
      if (ec != std::errc() || end != v.data() + v.size() || !std::isfinite(s) || s <= 0.0f) return 0;
      o.scale = s;
      return 2;
    }
    return 0;
  };
  std::string error;
  switch (nve::host::parse_launch_args(argc, argv, o, &error, extra)) {
    case nve::host::LaunchParse::Help: print_usage(); return 0;
    case nve::host::LaunchParse::Error:
      std::cerr << "[nve-windows-shell] " << error << "\n";
      print_usage();
      return 2;
    case nve::host::LaunchParse::Run: break;
  }
  nve::host::resolve_launch_state(o, "nve-windows-shell-builtin");
  ::SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
  return o.screenshot_path.empty() ? run_window(o) : screenshot(o);
}
