// nve-windows-shell: runs an NVE app (built-in counter demo, a `cdylib`, or a paired `nve dev`
// server) in a native Win32 + Direct2D window, persisting the app's state across launches, or
// renders it headlessly to a `.bmp` (`--screenshot`).
#include "NveWinHostWindow.hpp"

#include <nve/host/app_state.hpp>
#include <nve/host/demo_counter_app.hpp>

#include <atomic>
#include <chrono>
#include <cmath>
#include <cstdlib>
#include <cstring>
#include <iostream>
#include <string>
#include <thread>

#if defined(_MSC_VER)  // Common Controls v6: EDIT cue banners (placeholders), themed controls
#pragma comment(linker, "/manifestdependency:\"type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'\"")
#endif

namespace {

using nve::host::demo::CounterApp;
using nve::win::NveWinHost;
namespace app_state = nve::host::app_state;

struct Options {
  std::string dylib_path;
  std::string pair_uri;
  std::string pair_token;
  std::string state_path;
  std::string screenshot_path;
  uint32_t schema_version = 1u;
  bool no_state = false;
  bool reset_state = false;
  int width = 640;
  int height = 420;
  float scale = 1.0f;  // --screenshot only (a window follows its monitor's DPI)
};

std::atomic<HWND> g_main_window{nullptr};

void print_usage(const char* argv0) {
  std::cout << "Usage: " << argv0
            << " [--dylib <app.dll>] [--pair <nve://|quic://|wt://|ws(s)://...>] [--pair-token <token>]"
               " [--state <file>] [--no-state] [--reset-state] [--schema <ver>] [--width W] [--height H]"
               " [--screenshot <out.bmp> [--scale S]]\n"
               "Env: NVE_STATE_DISABLE=1, NVE_STATE_PATH=<file>, NVE_RESET_STATE=1\n";
}

/// Runs the requested source. When pairing or the dylib load fails, runs the built-in counter
/// instead and returns `false`: the failed app's state must neither be restored nor overwritten.
bool load_app(NveWinHost& host, const Options& o) {
  if (!o.pair_uri.empty()) {
    std::string error;
    if (host.connect_paired(o.pair_uri, o.pair_token, &error)) return true;
    std::cerr << "[nve-windows-shell] pairing with '" << o.pair_uri << "' failed: " << error << "\n";
  } else if (!o.dylib_path.empty()) {
    nve::host::ModuleError code = nve::host::ModuleError::None;
    std::string error;
    if (host.load_dylib(o.dylib_path, &code, &error)) return true;
    std::cerr << "[nve-windows-shell] loading '" << o.dylib_path << "' failed (" << static_cast<int>(code)
              << "): " << error << "\n";
  } else {
    return host.attach_interface(&CounterApp::get_interface);
  }
  (void)host.attach_interface(&CounterApp::get_interface);
  return false;
}

/// Restores the app state, or on `--reset-state` / `NVE_RESET_STATE` clears it (cold start).
void restore(NveWinHost& host, const Options& o) {
  if (o.no_state) return;
  if (o.reset_state) {
    if (!host.clear_state(o.state_path)) {
      std::cerr << "[nve-windows-shell] clearing " << o.state_path << " failed: " << host.last_error() << "\n";
    }
  } else if (host.restore_state(o.state_path, o.schema_version)) {
    std::cerr << "[nve-windows-shell] restored app state from " << o.state_path << "\n";
  }
}

void persist(NveWinHost& host, const Options& o, const char* trigger) {
  if (o.no_state || host.is_paired()) return;
  if (host.save_state(o.state_path, o.schema_version)) {
    std::cerr << "[nve-windows-shell] saved app state to " << o.state_path << " (" << trigger << ")\n";
  }
}

int screenshot(Options& o) {
  NveWinHost host(nve::win::WinHostConfig{.surface_id = 1u,
                                          .width = static_cast<float>(o.width),
                                          .height = static_cast<float>(o.height),
                                          .scale_factor = o.scale});
  o.no_state = !load_app(host, o) || o.no_state;
  // Paired frames arrive in the background: give the first one a moment.
  const auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(2);
  while (host.is_paired() && host.view_count() == 0u && std::chrono::steady_clock::now() < deadline) {
    std::this_thread::sleep_for(std::chrono::milliseconds(10));
    (void)host.pump();
  }
  restore(host, o);
  const auto px = [&o](int dips) { return static_cast<uint32_t>(std::max(1.0f, std::round(dips * o.scale))); };
  std::string error;
  if (!nve::win::render_visual_tree_to_bmp(host.visual_tree(), host.text_measurer(), px(o.width), px(o.height), o.scale,
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
  if (!window.create(::GetModuleHandleW(nullptr))) {
    std::cerr << "[nve-windows-shell] creating the window failed\n";
    return 1;
  }
  NveWinHost& host = window.host();
  o.no_state = !load_app(host, o) || o.no_state;
  restore(host, o);
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

int main(int argc, char* argv[]) {
  Options o;
  for (int i = 1; i < argc; ++i) {
    const auto is = [&](const char* flag) { return std::strcmp(argv[i], flag) == 0; };
    const bool has_value = i + 1 < argc;
    if (is("--help") || is("-h")) {
      print_usage(argv[0]);
      return 0;
    } else if (is("--no-state")) {
      o.no_state = true;
    } else if (is("--reset-state")) {
      o.reset_state = true;
    } else if (is("--dylib") && has_value) {
      o.dylib_path = argv[++i];
    } else if (is("--pair") && has_value) {
      o.pair_uri = argv[++i];
    } else if (is("--pair-token") && has_value) {
      o.pair_token = argv[++i];
    } else if (is("--state") && has_value) {
      o.state_path = argv[++i];
    } else if (is("--schema") && has_value) {
      o.schema_version = static_cast<uint32_t>(std::strtoul(argv[++i], nullptr, 10));
    } else if (is("--width") && has_value) {
      o.width = std::max(1, std::atoi(argv[++i]));
    } else if (is("--height") && has_value) {
      o.height = std::max(1, std::atoi(argv[++i]));
    } else if (is("--screenshot") && has_value) {
      o.screenshot_path = argv[++i];
    } else if (is("--scale") && has_value) {
      const float s = std::strtof(argv[++i], nullptr);
      o.scale = std::isfinite(s) && s > 0.0f ? s : 1.0f;
    } else {
      std::cerr << "[nve-windows-shell] unknown argument '" << argv[i] << "'\n";
      print_usage(argv[0]);
      return 2;
    }
  }
  // Paired apps keep their state on the `nve dev` server.
  if (!o.no_state && o.state_path.empty() && o.pair_uri.empty()) {
    o.state_path = app_state::resolve_path(o.dylib_path.empty() ? "nve-windows-shell-builtin" : o.dylib_path);
  }
  o.no_state = o.no_state || o.state_path.empty() || !o.pair_uri.empty();
  o.reset_state = o.reset_state || app_state::reset_requested();

  ::SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
  return o.screenshot_path.empty() ? run_window(o) : screenshot(o);
}
