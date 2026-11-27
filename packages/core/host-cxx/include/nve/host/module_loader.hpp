// NVE host runtime: loads an NVE app `cdylib` (`nve_get_interface`) for `attach_app_interface`.
//
// `AppModule::open` dlopens / LoadLibraryW's a UTF-8 path, resolves `nve_get_interface`, and
// requires `NVE_IFACE_SURFACE` with a `create` slot before a runtime ever sees it. The library
// stays loaded for the `AppModule`'s lifetime: detach the app from the runtime before dropping it.
#pragma once

#include <cstdint>
#include <limits>
#include <string>
#include <string_view>
#include <utility>

#include "nve_abi.generated.h"

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

namespace nve::host {

/// Why `AppModule::open` failed.
enum class ModuleError : uint8_t { None = 0, Open = 1, MissingEntry = 2, MissingSurface = 3 };

class AppModule {
 public:
  AppModule() noexcept = default;
  ~AppModule() { close(); }
  AppModule(const AppModule&) = delete;
  AppModule& operator=(const AppModule&) = delete;
  AppModule(AppModule&& o) noexcept
      : handle_(std::exchange(o.handle_, nullptr)), get_interface_(std::exchange(o.get_interface_, nullptr)) {}
  AppModule& operator=(AppModule&& o) noexcept {
    if (this != &o) {
      close();
      handle_ = std::exchange(o.handle_, nullptr);
      get_interface_ = std::exchange(o.get_interface_, nullptr);
    }
    return *this;
  }

  /// Opens `path` and validates its `nve_get_interface` / `NVE_IFACE_SURFACE`.
  [[nodiscard]] static AppModule open(std::string_view path, ModuleError* out_error = nullptr,
                                      std::string* out_message = nullptr) {
    AppModule m;
    const auto fail = [&](ModuleError e, std::string msg) {
      m.close();
      if (out_error != nullptr) *out_error = e;
      if (out_message != nullptr) *out_message = std::move(msg);
      return std::move(m);
    };
    std::string err;
    if (!m.load(path, err)) return fail(ModuleError::Open, std::move(err));
    m.get_interface_ = reinterpret_cast<NveGetInterfaceFn>(m.symbol("nve_get_interface"));
    if (m.get_interface_ == nullptr) return fail(ModuleError::MissingEntry, "missing nve_get_interface symbol");
    const auto* surface = static_cast<const NveSurfaceApi*>(m.get_interface_(NVE_IFACE_SURFACE));
    if (surface == nullptr || surface->create == nullptr) {
      return fail(ModuleError::MissingSurface, "missing NVE_IFACE_SURFACE interface");
    }
    if (out_error != nullptr) *out_error = ModuleError::None;
    return m;
  }

  [[nodiscard]] bool loaded() const noexcept { return get_interface_ != nullptr; }
  [[nodiscard]] NveGetInterfaceFn get_interface() const noexcept { return get_interface_; }

  void close() noexcept {
    get_interface_ = nullptr;
    if (handle_ == nullptr) return;
#if defined(_WIN32)
    ::FreeLibrary(static_cast<HMODULE>(handle_));
#else
    ::dlclose(handle_);
#endif
    handle_ = nullptr;
  }

 private:
  bool load(std::string_view path, std::string& err) {
    if (path.empty()) {
      err = "empty dynamic library path";
      return false;
    }
#if defined(_WIN32)
    if (path.size() > static_cast<size_t>(std::numeric_limits<int>::max())) {
      err = "dynamic library path too long";
      return false;
    }
    const int n = ::MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, path.data(), static_cast<int>(path.size()),
                                        nullptr, 0);
    std::wstring wide(static_cast<size_t>(n > 0 ? n : 0), L'\0');
    if (n <= 0 || ::MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, path.data(), static_cast<int>(path.size()),
                                        wide.data(), n) != n) {
      err = "dynamic library path is not valid UTF-8";
      return false;
    }
    handle_ = ::LoadLibraryW(wide.c_str());
    if (handle_ == nullptr) err = "LoadLibraryW failed (error " + std::to_string(::GetLastError()) + ")";
#else
    const std::string p(path);
    (void)::dlerror();
    handle_ = ::dlopen(p.c_str(), RTLD_NOW | RTLD_LOCAL);
    if (handle_ == nullptr) {
      const char* e = ::dlerror();
      err = e != nullptr ? e : "dlopen failed";
    }
#endif
    return handle_ != nullptr;
  }

  [[nodiscard]] void* symbol(const char* name) const noexcept {
#if defined(_WIN32)
    return reinterpret_cast<void*>(::GetProcAddress(static_cast<HMODULE>(handle_), name));
#else
    return ::dlsym(handle_, name);
#endif
  }

  void* handle_ = nullptr;
  NveGetInterfaceFn get_interface_ = nullptr;
};

}  // namespace nve::host
