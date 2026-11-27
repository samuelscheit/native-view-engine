// Live `libnve_runtime` tests of app files through `HostSession`: `cdylib` modules loaded from
// staged copies (status mapping, hot reload of a file rebuilt in place runs the NEW code with the
// old state, failed loads keep the running app, staged copies are deleted on unload), runtime
// `StateFile` app state (envelope, schema rules, quarantine, rotation, cold-start clearing), and
// the host-side state path policy (`app_state.hpp`).
#include <chrono>
#include <cstdint>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <iterator>
#include <string>
#include <string_view>
#include <system_error>
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

namespace {

namespace fs = std::filesystem;
using namespace nve::host;
using demo::CounterApp;

/// The counter app's count, read from its `NveDevApi` snapshot (4 LE bytes; -1 without one).
int32_t count(HostSession& host) {
  const std::vector<uint8_t> s = host.snapshot_state();
  if (s.size() != 4u) return -1;
  return static_cast<int32_t>(uint32_t{s[0]} | (uint32_t{s[1]} << 8u) | (uint32_t{s[2]} << 16u) |
                              (uint32_t{s[3]} << 24u));
}

bool press(HostSession& host, uint32_t node) { return host.dispatch(host.events().press(node, 0.0f, 0.0f)); }

/// This process's staged copies of modules with file stem `stem`
/// (`<tmp>/nve-runtime-modules/<stem>_<label>_<pid>_<seq>.<ext>`).
size_t staged_copies(std::string_view stem) {
#if defined(_WIN32)
  const std::string pid = "_" + std::to_string(_getpid()) + "_";
#else
  const std::string pid = "_" + std::to_string(getpid()) + "_";
#endif
  size_t n = 0u;
  std::error_code ec;
  for (const auto& e : fs::directory_iterator(fs::temp_directory_path() / "nve-runtime-modules", ec)) {
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

/// A per-run scratch directory (removed on scope exit).
struct ScratchDir {
  fs::path path;
  explicit ScratchDir(std::string_view tag)
      : path(fs::temp_directory_path() /
             (std::string(tag) + "_" +
              std::to_string(std::chrono::steady_clock::now().time_since_epoch().count()))) {
    fs::create_directories(path);
  }
  ~ScratchDir() {
    std::error_code ec;
    fs::remove_all(path, ec);
  }
};

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
  CHECK(!host->has_app() && !host->has_module() && host->source_generation() == 0u);
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
  const std::string stem = dir.path.filename().string();  // unique per run
  const fs::path app = dir.path / (stem + fs::path(NVE_TEST_COUNTER_MODULE).extension().string());
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
  const uint64_t gen = host->source_generation();
  CHECK(host->reload_module(app.string(), &err, &msg) && err == ModuleError::None);
  CHECK(host->source_generation() == gen + 1u && host->has_module());
  CHECK(count(*host) == 2);  // state carried into the new app before it went live
  CHECK(press(*host, CounterApp::kIncrement));
  CHECK(count(*host) == 4);  // a press counts 2: new code
  CHECK(staged_copies(stem) == 1u);  // the old copy was unloaded and deleted

  // Failed reloads keep the running app, its state, and the generation.
  CHECK(!host->reload_module(NVE_TEST_PARTIAL_SURFACE_MODULE, &err, &msg) && err == ModuleError::MissingSurface);
  CHECK(!host->reload_module(NVE_TEST_NO_ENTRY_MODULE, &err, &msg) && err == ModuleError::MissingEntry);
  CHECK(!host->reload_module((dir.path / "missing.so").string(), &err, &msg) && err == ModuleError::Open);
  CHECK(host->source_generation() == gen + 1u && host->has_module());
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

void test_state_files() {
  auto host = HostSession::create(RuntimeSession::default_config());
  CHECK(host != nullptr);
  if (!host) return;
  const ScratchDir dir("nve_host_state");
  const fs::path path = dir.path / "nested" / "app.nvestate";  // the runtime creates `nested`
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

  const ScratchDir dir("nve_app_state_paths");
  const std::string explicit_path = (dir.path / "s.nvestate").string();
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
  test_state_files();
  test_state_paths();
  if (g_failures != 0) {
    fprintf(stderr, "nve_host_app_files_test: %d failure(s)\n", g_failures);
    return 1;
  }
  printf("nve_host_app_files_test: ok\n");
  return 0;
}
