// Live `libnve_runtime` tests of `HostSession` lifecycles: `cdylib` modules loaded from staged
// copies (status mapping, hot reload of a file rebuilt in place runs the NEW code with the old
// state, failed loads keep the running app, staged copies are deleted on unload), re-entrant
// emitted-event sinks, runtime `StateFile` app state (envelope, schema rules, quarantine,
// rotation, cold-start clearing), and the host-side state path policy (`app_state.hpp`).
#include <chrono>
#include <cstdint>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <iterator>
#include <string>
#include <string_view>
#include <system_error>
#include <thread>
#include <utility>
#include <vector>

#if defined(_WIN32)
#include <process.h>
#else
#include <unistd.h>
#endif

#include "check.h"
#include "nve/host/app_state.hpp"
#include "nve/host/demo_counter_app.hpp"
#include "nve/host/host_session.hpp"
#include "nve/host/nve2_writer.hpp"
#include "nve/host/testing/recorder.hpp"
#include "nve/host/testing/scratch_dir.hpp"

namespace {

namespace fs = std::filesystem;
using namespace nve::host;
using demo::CounterApp;
using testing::ScratchDir;

/// The counter app's count, read from its `NveDevApi` snapshot (4 LE bytes; -1 without one).
int32_t count(HostSession& host) {
  const std::vector<uint8_t> s = host.snapshot_state();
  if (s.size() != 4u) return -1;
  return static_cast<int32_t>(uint32_t{s[0]} | (uint32_t{s[1]} << 8u) | (uint32_t{s[2]} << 16u) |
                              (uint32_t{s[3]} << 24u));
}

bool press(HostSession& host, uint32_t node) { return host.dispatch(host.events().press(node, 0.0f, 0.0f)); }

/// This process's staged copies of modules with file stem `stem`
/// (`<tmp>/nve-runtime-modules[-<euid>]/<stem>_<label>_<pid>_<seq>.<ext>`: per user on POSIX).
size_t staged_copies(std::string_view stem) {
#if defined(_WIN32)
  const std::string pid = "_" + std::to_string(_getpid()) + "_";
  const fs::path dir = fs::temp_directory_path() / "nve-runtime-modules";
#else
  const std::string pid = "_" + std::to_string(getpid()) + "_";
  const fs::path dir = fs::temp_directory_path() / ("nve-runtime-modules-" + std::to_string(geteuid()));
#endif
  size_t n = 0u;
  std::error_code ec;
  for (const auto& e : fs::directory_iterator(dir, ec)) {
    const std::string name = e.path().filename().string();
    if (name.starts_with(std::string(stem) + "_") && name.find(pid) != std::string::npos) ++n;
  }
  return n;
}

std::string file_bytes(const fs::path& p) {
  std::ifstream in(p, std::ios::binary);
  return std::string(std::istreambuf_iterator<char>(in), std::istreambuf_iterator<char>());
}

void write_bytes(const fs::path& p, std::string_view bytes) {
  std::ofstream out(p, std::ios::binary | std::ios::trunc);
  out.write(bytes.data(), static_cast<std::streamsize>(bytes.size()));
}

void test_module_status_mapping() {
  auto host = HostSession::create(RuntimeSession::default_config());
  CHECK(host != nullptr);
  if (!host) return;
  ModuleError err = ModuleError::None;
  std::string msg;
  CHECK(!host->load_module("", &err, &msg) && err == ModuleError::Open && !msg.empty());
  CHECK(!host->load_module("/nonexistent/libnope.so", &err, &msg) && err == ModuleError::Open);
  CHECK(!host->load_module(NVE_TEST_NO_ENTRY_MODULE, &err, &msg) && err == ModuleError::MissingEntry);
  CHECK(msg.find("nve_get_interface") != std::string::npos);
  CHECK(!host->load_module(NVE_TEST_NO_SURFACE_MODULE, &err, &msg) && err == ModuleError::MissingSurface);
  CHECK(!host->load_module(NVE_TEST_PARTIAL_SURFACE_MODULE, &err, &msg) && err == ModuleError::MissingSurface);
  CHECK(msg.find("NVE_IFACE_SURFACE") != std::string::npos);
  CHECK(!host->has_app() && !host->has_module());
  // Rejected modules leave no staged copy behind.
  for (const char* m : {NVE_TEST_NO_ENTRY_MODULE, NVE_TEST_NO_SURFACE_MODULE, NVE_TEST_PARTIAL_SURFACE_MODULE}) {
    CHECK(staged_copies(fs::path(m).stem().string()) == 0u);
  }
  CHECK(module_error(NVE_RUNTIME_BUSY) == ModuleError::Attach && module_error(NVE_RUNTIME_OK) == ModuleError::None);
}

/// The dev loop: one app path rebuilt in place and hot-reloaded.
void test_hot_reload_of_a_rebuilt_module() {
  auto host = HostSession::create(RuntimeSession::default_config());
  CHECK(host != nullptr);
  if (!host) return;
  const ScratchDir dir("nve_host_reload");
  const std::string stem = dir.path().filename().string();  // unique per run
  const fs::path app = dir.path() / (stem + fs::path(NVE_TEST_COUNTER_MODULE).extension().string());
  const auto build = [&](const char* fixture) { fs::copy_file(fixture, app, fs::copy_options::overwrite_existing); };
  ModuleError err = ModuleError::None;
  std::string msg;

  build(NVE_TEST_COUNTER_MODULE);
  CHECK(host->load_module(app.string(), &err, &msg) && err == ModuleError::None && host->has_module());
  CHECK(staged_copies(stem) == 1u);
  CHECK(press(*host, CounterApp::kIncrement) && press(*host, CounterApp::kIncrement));
  CHECK(count(*host) == 2);

  // Rebuild the same path with new behaviour: the reload runs the NEW code, with the old state.
  build(NVE_TEST_COUNTER_DOUBLE_MODULE);
  const uint32_t epoch = testing::published_scene_epoch(*host);
  CHECK(host->reload_module(app.string(), &err, &msg) && err == ModuleError::None);
  const uint32_t reloaded = testing::published_scene_epoch(*host);
  CHECK(reloaded != epoch && reloaded != UINT32_MAX && host->has_module());
  CHECK(count(*host) == 2);  // state carried into the new app before it went live
  CHECK(press(*host, CounterApp::kIncrement));
  CHECK(count(*host) == 4);  // a press counts 2: new code
  CHECK(staged_copies(stem) == 1u);  // the old copy was unloaded and deleted

  // Failed reloads keep the running app, its state, and the generation.
  CHECK(!host->reload_module(NVE_TEST_PARTIAL_SURFACE_MODULE, &err, &msg) && err == ModuleError::MissingSurface);
  CHECK(!host->reload_module(NVE_TEST_NO_ENTRY_MODULE, &err, &msg) && err == ModuleError::MissingEntry);
  CHECK(!host->reload_module((dir.path() / "missing.so").string(), &err, &msg) && err == ModuleError::Open);
  CHECK(testing::published_scene_epoch(*host) == reloaded && host->has_module());
  CHECK(press(*host, CounterApp::kIncrement));
  CHECK(count(*host) == 6);
  CHECK(staged_copies(stem) == 1u);

  // Rebuild back: the original code runs again (a press counts 1).
  build(NVE_TEST_COUNTER_MODULE);
  CHECK(host->reload_module(app.string(), &err, &msg));
  CHECK(press(*host, CounterApp::kIncrement));
  CHECK(count(*host) == 7);
  // The scene follows the reloaded app.
  (void)host->pump();
  {
    auto guard = host->runtime().acquire_scene();
    CHECK(guard.has_value() && guard->reader().node_capacity() > CounterApp::kReset);
  }

  // `load_module` starts fresh; an in-process app replaces the module and deletes its copy.
  CHECK(host->load_module(app.string(), &err, &msg));
  CHECK(count(*host) == 0);
  CHECK(host->attach(&CounterApp::get_interface) && !host->has_module());
  CHECK(staged_copies(stem) == 0u);

  // Destroying the session unloads a running module and deletes its copy too.
  CHECK(host->load_module(app.string(), &err, &msg) && staged_copies(stem) == 1u);
  host.reset();
  CHECK(staged_copies(stem) == 0u);
}

/// The emitted-event sink may re-enter the session: nested dispatches (and their nested pumps)
/// grow the app's event buffer and must not invalidate the batch the outer sink still reads, and
/// replacing the sink from inside it must not destroy the running callback.
void test_reentrant_event_sink() {
  auto host = HostSession::create(RuntimeSession::default_config());
  CHECK(host != nullptr);
  if (!host) return;
  CHECK(host->attach(&CounterApp::get_interface));
  struct Seen {
    std::string name;
    int64_t value = 0;
  };
  std::vector<Seen> seen;  // recorded AFTER the nested calls returned
  host->set_event_sink([&](std::span<const NveDirectEvent> evs) {
    for (const NveDirectEvent& ev : evs) {
      if (ev.value.data.i64_val < 4) (void)press(*host, CounterApp::kIncrement);
      seen.push_back({std::string(reinterpret_cast<const char*>(ev.name.ptr), ev.name.len), ev.value.data.i64_val});
    }
  });
  CHECK(press(*host, CounterApp::kIncrement));
  CHECK(seen.size() == 4u);
  for (size_t i = 0u; i < seen.size(); ++i) {
    CHECK(seen[i].name == CounterApp::kCountEventName && seen[i].value == static_cast<int64_t>(4u - i));
  }
  CHECK(count(*host) == 4);

  // A sink that replaces itself keeps running on its own (copied) state.
  int calls = 0;
  const std::string tag(64, 'x');  // heap-allocated capture: freed if the running sink were destroyed
  host->set_event_sink([&host, &calls, tag](std::span<const NveDirectEvent>) {
    host->set_event_sink([&calls](std::span<const NveDirectEvent>) { calls += 100; });
    calls += tag.size() == 64u ? 1 : 1000;
  });
  CHECK(press(*host, CounterApp::kIncrement) && calls == 1);
  CHECK(press(*host, CounterApp::kIncrement) && calls == 101);
}

/// A sink that hot-reloads the module destroys the app and unloads the image that owns the
/// event names mid-batch: the batch it reads must be the session's own copy.
void test_sink_reloading_the_module() {
  auto host = HostSession::create(RuntimeSession::default_config());
  CHECK(host != nullptr);
  if (!host) return;
  CHECK(host->load_module(NVE_TEST_COUNTER_MODULE));
  std::vector<std::string> names;
  std::vector<int64_t> values;
  bool reloaded = false;
  host->set_event_sink([&](std::span<const NveDirectEvent> evs) {
    if (!std::exchange(reloaded, true)) CHECK(host->reload_module(NVE_TEST_COUNTER_DOUBLE_MODULE));
    for (const NveDirectEvent& ev : evs) {
      names.emplace_back(reinterpret_cast<const char*>(ev.name.ptr), ev.name.len);
      values.push_back(ev.value.data.i64_val);
    }
  });
  CHECK(press(*host, CounterApp::kIncrement));
  CHECK(reloaded && names == std::vector<std::string>{"count"} && values == std::vector<int64_t>{1});
  CHECK(press(*host, CounterApp::kIncrement) && count(*host) == 3);  // the reloaded (double) code
  // The restored state re-emits `count` (1); then one event per applied press.
  CHECK(values == (std::vector<int64_t>{1, 1, 2, 3}));
}

/// `unpair` stops the pairing worker whether it connected or not: against an endpoint nobody
/// listens on the worker keeps (re)connecting, and a stale "connected?" check never stopped it.
void test_unpair_stops_a_connecting_worker() {
#if defined(NVE_RUNTIME_TEST_HAS_WIRE)
  auto host = HostSession::create(RuntimeSession::default_config());
  CHECK(host != nullptr);
  if (!host) return;
  int wakes = 0;
  host->set_wake_poster([&wakes] { ++wakes; });
  std::string error;
  CHECK(host->pair("ws://127.0.0.1:9/nve-wire", "token", &error));  // discard port: refused
  CHECK(!host->runtime().pair_is_connected());
  Nve2Writer w;
  w.create(1u);
  const std::vector<uint8_t> frame = [&] {
    const auto f = w.finish(1u);
    return std::vector<uint8_t>(f.begin(), f.end());
  }();
  // While a pairing worker owns the surface, local frames are refused.
  CHECK(host->runtime().submit_nve2(frame) == NVE_RUNTIME_INVALID_ARG);
  host->unpair();
  CHECK(host->runtime().submit_nve2(frame) == NVE_RUNTIME_OK);  // the worker is gone
  host->unpair();                                                 // idempotent
  CHECK(!host->runtime().pair_is_connected());
  const int seen = wakes;
  std::this_thread::sleep_for(std::chrono::milliseconds(300));  // past the first reconnect attempt
  CHECK(wakes == seen && !host->runtime().pair_is_connected());

  // Any source switch ends pairing too: `unpair` after an attach is a no-op.
  CHECK(host->pair("ws://127.0.0.1:9/nve-wire", "token", &error));
  CHECK(host->attach(&CounterApp::get_interface));
  host->unpair();
  CHECK(host->has_app() && count(*host) == 0);
#endif
}

void test_state_files() {
  auto host = HostSession::create(RuntimeSession::default_config());
  CHECK(host != nullptr);
  if (!host) return;
  const ScratchDir dir("nve_host_state");
  const fs::path path = dir.path() / "nested" / "app.nvestate";  // the runtime creates `nested`
  const std::string p = path.string();
  RuntimeSession& rt = host->runtime();

  CHECK(rt.save_state_file(p, 7u) == NVE_RUNTIME_UNSUPPORTED);  // no app
  CHECK(host->attach(&CounterApp::get_interface));
  CHECK(rt.restore_state_file(p, 7u) == NVE_RUNTIME_NOT_FOUND);  // no file yet
  CHECK(press(*host, CounterApp::kIncrement) && press(*host, CounterApp::kIncrement));
  CHECK(host->save_state(p, 7u));
  const std::string saved = file_bytes(path);
  CHECK(saved.size() == 24u + 4u && saved.starts_with("NVESTATE"));  // runtime `StateFile` envelope
  for (const auto& e : fs::directory_iterator(path.parent_path())) {
    CHECK(e.path() == path);  // atomic write: no temp file left behind
  }

  CHECK(press(*host, CounterApp::kReset) && count(*host) == 0);
  CHECK(rt.restore_state_file(p, 6u) == NVE_RUNTIME_IO);  // written by a newer host: kept, not read
  CHECK(fs::exists(path) && count(*host) == 0);
  CHECK(host->restore_state_file(p, 7u) && count(*host) == 2);

  // A flipped payload bit fails the checksum: quarantined, the app keeps its state.
  std::string flipped = saved;
  flipped.back() = static_cast<char>(flipped.back() ^ 1);
  write_bytes(path, flipped);
  CHECK(press(*host, CounterApp::kIncrement) && count(*host) == 3);
  CHECK(rt.restore_state_file(p, 7u) == NVE_RUNTIME_NOT_FOUND);
  CHECK(rt.last_error().find("corrupt") != std::string::npos);
  CHECK(!fs::exists(path) && file_bytes(p + ".corrupt") == flipped && count(*host) == 3);

  // Truncation and trailing garbage are corrupt too.
  for (const std::string& bad : {saved.substr(0u, 10u), saved + "x"}) {
    fs::remove(p + ".corrupt");
    write_bytes(path, bad);
    CHECK(!host->restore_state_file(p, 7u));
    CHECK(!fs::exists(path) && fs::exists(p + ".corrupt"));
  }

  // An older schema is rotated aside (never misread), on restore and when saving over it.
  write_bytes(path, saved);
  CHECK(rt.restore_state_file(p, 8u) == NVE_RUNTIME_NOT_FOUND);
  CHECK(rt.last_error().find("older schema") != std::string::npos);
  CHECK(!fs::exists(path) && file_bytes(p + ".v7") == saved);
  fs::remove(p + ".v7");
  write_bytes(path, saved);
  CHECK(host->save_state(p, 9u));
  CHECK(file_bytes(p + ".v7") == saved && host->restore_state_file(p, 9u) && count(*host) == 3);

  // Cold start (`--reset-state` / `NVE_RESET_STATE`): the runtime deletes the file.
  CHECK(host->clear_state_file(p) && !fs::exists(path));
  CHECK(host->clear_state_file(p));  // already absent
  CHECK(!host->restore_state_file(p, 9u));
  CHECK(!host->save_state("", 1u) && !host->restore_state_file("", 1u));
}

void set_env(const char* name, const char* value) {
#if defined(_WIN32)
  _putenv_s(name, value != nullptr ? value : "");
#else
  if (value != nullptr) {
    setenv(name, value, 1);
  } else {
    unsetenv(name);
  }
#endif
}

void test_state_paths() {
  set_env("NVE_STATE_DISABLE", nullptr);
  set_env("NVE_STATE_PATH", nullptr);
  set_env("NVE_RESET_STATE", nullptr);
  const std::string a = app_state::resolve_path("/apps/My Cool App!.dylib");
  CHECK(a.find("my-cool-app-") != std::string::npos && a.ends_with(".nvestate"));
  CHECK(a == app_state::resolve_path("/apps/My Cool App!.dylib"));  // stable
  CHECK(a != app_state::resolve_path("/other/My Cool App!.dylib"));  // identity-scoped
  CHECK(app_state::slug("") == "nve-app" && app_state::slug("???") == "nve-app");
  CHECK(!app_state::reset_requested());
  // UTF-8 round-trips losslessly (Windows: no ANSI code page), and multi-byte stems slug safely.
  const std::string umlaut = "/apps/Grüße-✓/Über App.dylib";
  CHECK(app_state::utf8(app_state::from_utf8(umlaut)) == umlaut);
  CHECK(app_state::slug(umlaut) == "ber-app");
  CHECK(app_state::resolve_path(umlaut) != app_state::resolve_path("/apps/Grusse/Uber App.dylib"));

  const ScratchDir dir("nve_app_state_paths");
  const std::string explicit_path = (dir.path() / "s.nvestate").string();
  write_bytes(explicit_path, "keep");
  set_env("NVE_STATE_PATH", explicit_path.c_str());
  set_env("NVE_RESET_STATE", "yes");
  CHECK(app_state::reset_requested());
  // Resolving is pure: a reset request never deletes; the host clears through the runtime.
  CHECK(app_state::resolve_path("x") == explicit_path && file_bytes(explicit_path) == "keep");
  set_env("NVE_RESET_STATE", nullptr);
  set_env("NVE_STATE_DISABLE", "TRUE");
  CHECK(app_state::resolve_path("x").empty());
  set_env("NVE_STATE_DISABLE", nullptr);
  set_env("NVE_STATE_PATH", nullptr);
}

}  // namespace

int main() {
  test_module_status_mapping();
  test_hot_reload_of_a_rebuilt_module();
  test_reentrant_event_sink();
  test_sink_reloading_the_module();
  test_unpair_stops_a_connecting_worker();
  test_state_files();
  test_state_paths();
  if (g_failures != 0) {
    fprintf(stderr, "nve_host_session_test: %d failure(s)\n", g_failures);
    return 1;
  }
  printf("nve_host_session_test: ok\n");
  return 0;
}
