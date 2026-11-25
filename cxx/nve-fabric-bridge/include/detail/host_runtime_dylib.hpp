#pragma once

#include "nve_abi.generated.h"
#include "nve_rn_component.hpp"

#include <chrono>
#include <cstdint>
#include <limits>
#include <string>
#include <string_view>
#include <utility>

#if defined(_WIN32)
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#else
#include <dlfcn.h>
#endif

namespace nve {

/// Event mask bit matching `EventMask::PRESS` in `crates/nve-abi`.
inline constexpr uint64_t kEventMaskPress = 1ULL << 0;

/// Callback type for outbound component events drained from `NveComponentApi`.
using EventCallback = EventEmitterCallback;

/// Monotonic `std::chrono::steady_clock` timestamp in microseconds.
inline uint64_t current_timestamp_us() noexcept {
  using Clock = std::chrono::steady_clock;
  return static_cast<uint64_t>(
      std::chrono::duration_cast<std::chrono::microseconds>(Clock::now().time_since_epoch())
          .count());
}

/// Unpacked RGBA8888 color (8-bit channels and normalized `[0, 1]` floats).
struct RgbaColor {
  uint8_t r8{0};
  uint8_t g8{0};
  uint8_t b8{0};
  uint8_t a8{0};
  float r{0.0f};
  float g{0.0f};
  float b{0.0f};
  float a{0.0f};
};

/// Unpack little-endian RGBA8888 (`R` in low byte, `A` in high byte).
constexpr RgbaColor unpack_rgba(uint32_t rgba) noexcept {
  const uint8_t r8 = static_cast<uint8_t>(rgba & 0xFFu);
  const uint8_t g8 = static_cast<uint8_t>((rgba >> 8) & 0xFFu);
  const uint8_t b8 = static_cast<uint8_t>((rgba >> 16) & 0xFFu);
  const uint8_t a8 = static_cast<uint8_t>((rgba >> 24) & 0xFFu);
  return RgbaColor{
      .r8 = r8,
      .g8 = g8,
      .b8 = b8,
      .a8 = a8,
      .r = static_cast<float>(r8) / 255.0f,
      .g = static_cast<float>(g8) / 255.0f,
      .b = static_cast<float>(b8) / 255.0f,
      .a = static_cast<float>(a8) / 255.0f,
  };
}

/// Move-only RAII wrapper for platform dynamic libraries (`LoadLibraryW` / `dlopen`).
class DynamicLibrary {
 public:
  DynamicLibrary() noexcept = default;
  ~DynamicLibrary() { close(); }

  DynamicLibrary(const DynamicLibrary&) = delete;
  DynamicLibrary& operator=(const DynamicLibrary&) = delete;

  DynamicLibrary(DynamicLibrary&& other) noexcept : handle_(other.handle_) {
    other.handle_ = nullptr;
  }

  DynamicLibrary& operator=(DynamicLibrary&& other) noexcept {
    if (this != &other) {
      close();
      handle_ = other.handle_;
      other.handle_ = nullptr;
    }
    return *this;
  }

  bool is_open() const noexcept { return handle_ != nullptr; }
  explicit operator bool() const noexcept { return is_open(); }

  void* native_handle() const noexcept {
#if defined(_WIN32)
    return reinterpret_cast<void*>(handle_);
#else
    return handle_;
#endif
  }

  void close() noexcept {
    if (handle_ != nullptr) {
#if defined(_WIN32)
      ::FreeLibrary(handle_);
#else
      ::dlclose(handle_);
#endif
      handle_ = nullptr;
    }
  }

  bool open(std::string_view path, std::string* out_error = nullptr) {
    close();
    if (out_error != nullptr) {
      out_error->clear();
    }
    if (path.empty()) {
      if (out_error != nullptr) {
        *out_error = "Empty dynamic library path";
      }
      return false;
    }

#if defined(_WIN32)
    if (path.size() > static_cast<size_t>(std::numeric_limits<int>::max())) {
      if (out_error != nullptr) {
        *out_error = "Dynamic library path exceeds maximum length";
      }
      return false;
    }
    const int wide_len = ::MultiByteToWideChar(
        CP_UTF8, 0, path.data(), static_cast<int>(path.size()), nullptr, 0);
    if (wide_len <= 0) {
      if (out_error != nullptr) {
        *out_error = format_win32_error(::GetLastError(), "MultiByteToWideChar failed");
      }
      return false;
    }
    std::wstring wide_path(static_cast<size_t>(wide_len), L'\0');
    if (::MultiByteToWideChar(
            CP_UTF8, 0, path.data(), static_cast<int>(path.size()), wide_path.data(), wide_len) <=
        0) {
      if (out_error != nullptr) {
        *out_error = format_win32_error(::GetLastError(), "MultiByteToWideChar failed");
      }
      return false;
    }

    HMODULE mod = ::LoadLibraryW(wide_path.c_str());
    if (mod == nullptr) {
      if (out_error != nullptr) {
        *out_error = format_win32_error(::GetLastError(), "LoadLibraryW failed");
      }
      return false;
    }
    handle_ = mod;
    return true;
#else
    const std::string path_cstr(path);
    (void)::dlerror();
    void* mod = ::dlopen(path_cstr.c_str(), RTLD_NOW | RTLD_LOCAL);
    if (mod == nullptr) {
      if (out_error != nullptr) {
        const char* err = ::dlerror();
        *out_error = (err != nullptr) ? std::string(err) : std::string("dlopen failed");
      }
      return false;
    }
    handle_ = mod;
    return true;
#endif
  }

  void* symbol(const char* name) const noexcept {
    if (handle_ == nullptr || name == nullptr) {
      return nullptr;
    }
#if defined(_WIN32)
    return reinterpret_cast<void*>(::GetProcAddress(handle_, name));
#else
    return ::dlsym(handle_, name);
#endif
  }

  template <typename Fn>
  Fn symbol_as(const char* name) const noexcept {
    return reinterpret_cast<Fn>(symbol(name));
  }

 private:
#if defined(_WIN32)
  static std::string format_win32_error(DWORD err_code, const char* fallback) {
    LPSTR msg_buf = nullptr;
    const DWORD flags =
        FORMAT_MESSAGE_ALLOCATE_BUFFER | FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS;
    const DWORD len = ::FormatMessageA(
        flags,
        nullptr,
        err_code,
        MAKELANGID(LANG_NEUTRAL, SUBLANG_DEFAULT),
        reinterpret_cast<LPSTR>(&msg_buf),
        0,
        nullptr);
    std::string msg;
    if (len > 0 && msg_buf != nullptr) {
      msg.assign(msg_buf, static_cast<size_t>(len));
      while (!msg.empty() && (msg.back() == '\r' || msg.back() == '\n' || msg.back() == ' ')) {
        msg.pop_back();
      }
    }
    if (msg_buf != nullptr) {
      ::LocalFree(msg_buf);
    }
    if (msg.empty()) {
      msg = std::string(fallback) + " (error " + std::to_string(err_code) + ")";
    }
    return msg;
  }

  HMODULE handle_{nullptr};
#else
  void* handle_{nullptr};
#endif
};

/// Result of opening and validating an NVE `cdylib` (`nve_get_interface` + `NVE_IFACE_SURFACE`).
/// `error_code`: `0` OK, `1` open/empty-path failed, `2` missing `nve_get_interface`,
/// `3` missing `NVE_IFACE_SURFACE`.
struct LoadedModule {
  DynamicLibrary lib;
  NveGetInterfaceFn get_interface{nullptr};
  NveModule module{};
  int error_code{0};
  std::string error_message;

  bool ok() const noexcept { return error_code == 0 && module.surface != nullptr; }
};

/// Open an NVE dynamic library, resolve `nve_get_interface`, and validate `NVE_IFACE_SURFACE`.
inline LoadedModule load_nve_dylib(std::string_view path) {
  LoadedModule result{};
  if (path.empty()) {
    result.error_code = 1;
    result.error_message = "Empty dynamic library path";
    return result;
  }

  std::string open_error;
  if (!result.lib.open(path, &open_error)) {
    result.error_code = 1;
    result.error_message =
        open_error.empty() ? "Failed to open dynamic library" : std::move(open_error);
    return result;
  }

  auto get_iface = result.lib.symbol_as<NveGetInterfaceFn>("nve_get_interface");
  if (get_iface == nullptr) {
    result.lib.close();
    result.error_code = 2;
    result.error_message = "Missing nve_get_interface symbol";
    return result;
  }

  const NveModule mod = nve_resolve_module(get_iface);
  if (mod.surface == nullptr || mod.surface->create == nullptr) {
    result.lib.close();
    result.error_code = 3;
    result.error_message = "Missing NVE_IFACE_SURFACE interface";
    return result;
  }

  result.get_interface = get_iface;
  result.module = mod;
  result.error_code = 0;
  return result;
}

} // namespace nve
