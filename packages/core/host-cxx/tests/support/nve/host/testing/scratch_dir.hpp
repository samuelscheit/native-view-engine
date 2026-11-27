// Test support: a per-process, per-instance scratch directory, removed on scope exit, so host
// suites (and parallel CTest runs of them) never share or leak temp files.
#pragma once

#include <atomic>
#include <chrono>
#include <cstdint>
#include <filesystem>
#include <string>
#include <string_view>
#include <system_error>

#if defined(_WIN32)
#include <process.h>
#else
#include <unistd.h>
#endif

namespace nve::host::testing {

class ScratchDir {
 public:
  explicit ScratchDir(std::string_view tag) : path_(unique_path(tag)) { std::filesystem::create_directories(path_); }
  ~ScratchDir() {
    std::error_code ec;
    std::filesystem::remove_all(path_, ec);
  }
  ScratchDir(const ScratchDir&) = delete;
  ScratchDir& operator=(const ScratchDir&) = delete;

  [[nodiscard]] const std::filesystem::path& path() const noexcept { return path_; }
  /// `path() / name` as a string.
  [[nodiscard]] std::string file(std::string_view name) const { return (path_ / std::string(name)).string(); }

 private:
  [[nodiscard]] static std::filesystem::path unique_path(std::string_view tag) {
    static std::atomic<uint32_t> counter{0u};
#if defined(_WIN32)
    const auto pid = static_cast<long long>(_getpid());
#else
    const auto pid = static_cast<long long>(getpid());
#endif
    const auto ticks = std::chrono::steady_clock::now().time_since_epoch().count();
    return std::filesystem::temp_directory_path() /
           (std::string(tag) + "-" + std::to_string(pid) + "-" + std::to_string(counter++) + "-" +
            std::to_string(ticks));
  }

  std::filesystem::path path_;
};

}  // namespace nve::host::testing
