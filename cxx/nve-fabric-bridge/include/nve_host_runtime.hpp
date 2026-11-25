#pragma once

#include "nve_abi.h"
#include "nve_rn_component.hpp"

#include <chrono>
#include <cstdint>
#include <functional>
#include <limits>
#include <memory>
#include <string>
#include <string_view>
#include <unordered_map>
#include <unordered_set>
#include <utility>
#include <vector>

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

/// Compute a retained-view type tag distinguishing wrapper `NVE_COMP_TEXT` (`!children.empty()`)
/// from leaf `NVE_COMP_TEXT`.
inline uint32_t view_tag_for_snapshot(const RenderedNodeSnapshot& snap) noexcept {
  const uint32_t is_wrapper_text =
      (snap.kind == NVE_COMP_TEXT && !snap.children.empty()) ? 0x10000u : 0u;
  return static_cast<uint32_t>(snap.kind) | is_wrapper_text;
}

/// Platform-agnostic 3-pass retained native view reconciler shared across UIKit/AppKit,
/// WinUI/RNW, Qt 6, and GTK4 host shells.
///
/// `Adapter` must provide:
/// - `void update_root(const RenderedNodeSnapshot& snap)`
/// - `ViewHandle create_view(const RenderedNodeSnapshot& snap)`
/// - `void update_view(ViewHandle& view, const RenderedNodeSnapshot& snap)`
/// - `void destroy_view(uint32_t node_id, ViewHandle& view)`
/// - `template <typename LookupFn> void sync_children(uint32_t parent_id, ViewHandle* parent_view,
///       const std::vector<uint32_t>& child_ids, LookupFn&& lookup_fn)`
template <typename ViewHandle, typename Adapter>
class RetainedViewReconciler {
 public:
  void sync(const std::vector<RenderedNodeSnapshot>& snapshots, Adapter& adapter) {
    // Pass 1: Prune removed node IDs.
    std::unordered_set<uint32_t> live_ids;
    live_ids.reserve(snapshots.size());
    for (const auto& snap : snapshots) {
      live_ids.insert(snap.node_id);
    }
    for (auto it = views_.begin(); it != views_.end();) {
      if (live_ids.find(it->first) == live_ids.end()) {
        adapter.destroy_view(it->first, it->second);
        tags_.erase(it->first);
        it = views_.erase(it);
      } else {
        ++it;
      }
    }

    // Pass 2: Update root and create/reuse/update retained views.
    for (const auto& snap : snapshots) {
      if (snap.node_id == NVE_ROOT_NODE_ID) {
        adapter.update_root(snap);
        continue;
      }

      const uint32_t tag = view_tag_for_snapshot(snap);
      auto existing = views_.find(snap.node_id);
      auto tag_it = tags_.find(snap.node_id);
      if (existing == views_.end() || tag_it == tags_.end() || tag_it->second != tag) {
        if (existing != views_.end()) {
          adapter.destroy_view(snap.node_id, existing->second);
          views_.erase(existing);
          tags_.erase(snap.node_id);
        }
        auto [inserted, ok] = views_.emplace(snap.node_id, adapter.create_view(snap));
        (void)ok;
        tags_[snap.node_id] = tag;
        existing = inserted;
      }

      adapter.update_view(existing->second, snap);
    }

    // Pass 3: Reconcile parent-child hierarchy in snapshot child order.
    auto lookup_fn = [this](uint32_t child_id) -> ViewHandle* { return this->find(child_id); };
    for (const auto& snap : snapshots) {
      ViewHandle* parent_view =
          (snap.node_id == NVE_ROOT_NODE_ID) ? nullptr : find(snap.node_id);
      adapter.sync_children(snap.node_id, parent_view, snap.children, lookup_fn);
    }
  }

  void clear(Adapter& adapter) {
    for (auto& [node_id, view] : views_) {
      adapter.destroy_view(node_id, view);
    }
    views_.clear();
    tags_.clear();
  }

  ViewHandle* find(uint32_t node_id) noexcept {
    auto it = views_.find(node_id);
    return (it != views_.end()) ? &it->second : nullptr;
  }

  const ViewHandle* find(uint32_t node_id) const noexcept {
    auto it = views_.find(node_id);
    return (it != views_.end()) ? &it->second : nullptr;
  }

  size_t size() const noexcept { return views_.size(); }
  bool empty() const noexcept { return views_.empty(); }

 private:
  std::unordered_map<uint32_t, ViewHandle> views_;
  std::unordered_map<uint32_t, uint32_t> tags_;
};

/// Shared C++20 native host session owning a loaded dynamic library (optional),
/// `NveRnComponentBridge`, platform text measurer, outbound event callback, and surface config.
class NativeHostSession {
 public:
  NativeHostSession() = default;
  ~NativeHostSession() { reset(); }

  NativeHostSession(const NativeHostSession&) = delete;
  NativeHostSession& operator=(const NativeHostSession&) = delete;

  NativeHostSession(NativeHostSession&& other) noexcept
      : lib_(std::move(other.lib_)),
        bridge_(std::move(other.bridge_)),
        text_measurer_(std::move(other.text_measurer_)),
        event_callback_(std::move(other.event_callback_)),
        config_(other.config_) {}

  NativeHostSession& operator=(NativeHostSession&& other) noexcept {
    if (this != &other) {
      // Destroy bridge before unloading our library, then transfer ownership.
      reset();
      lib_ = std::move(other.lib_);
      bridge_ = std::move(other.bridge_);
      text_measurer_ = std::move(other.text_measurer_);
      event_callback_ = std::move(other.event_callback_);
      config_ = other.config_;
    }
    return *this;
  }

  void reset() noexcept {
    bridge_.reset();
    lib_.close();
  }

  /// Attach an in-process `nve_get_interface` function pointer directly (no dylib owned).
  bool attach_interface(NveGetInterfaceFn get_interface, NveSurfaceConfig config) {
    const NveModule mod = nve_resolve_module(get_interface);
    if (mod.surface == nullptr || mod.surface->create == nullptr) {
      return false;
    }
    return install_module(mod, DynamicLibrary{}, config, {});
  }

  /// Open an NVE `cdylib`, mount the surface, and perform an initial render.
  bool load_dylib(
      std::string_view path,
      NveSurfaceConfig config,
      int* out_err_code = nullptr,
      std::string* out_err_msg = nullptr) {
    LoadedModule loaded = load_nve_dylib(path);
    if (!loaded.ok()) {
      if (out_err_code != nullptr) {
        *out_err_code = loaded.error_code;
      }
      if (out_err_msg != nullptr) {
        *out_err_msg = std::move(loaded.error_message);
      }
      return false;
    }
    if (!install_module(loaded.module, std::move(loaded.lib), config, {})) {
      if (out_err_code != nullptr) {
        *out_err_code = 3;
      }
      if (out_err_msg != nullptr) {
        *out_err_msg = "Failed to initialize NVE surface instance";
      }
      return false;
    }
    if (out_err_code != nullptr) {
      *out_err_code = 0;
    }
    if (out_err_msg != nullptr) {
      out_err_msg->clear();
    }
    return true;
  }

  /// Atomically hot-reload a new NVE `cdylib`: loads & validates the replacement before tearing
  /// down the active bridge, preserving state via `snapshot_state` / `restore_state` and
  /// re-applying `text_measurer_` and `event_callback_`.
  bool reload_dylib(
      std::string_view path,
      int* out_err_code = nullptr,
      std::string* out_err_msg = nullptr) {
    LoadedModule loaded = load_nve_dylib(path);
    if (!loaded.ok()) {
      if (out_err_code != nullptr) {
        *out_err_code = loaded.error_code;
      }
      if (out_err_msg != nullptr) {
        *out_err_msg = std::move(loaded.error_message);
      }
      return false;
    }

    std::vector<uint8_t> saved_state;
    if (is_valid()) {
      saved_state = bridge_->snapshot_state();
    }

    if (!install_module(loaded.module, std::move(loaded.lib), config_, saved_state)) {
      if (out_err_code != nullptr) {
        *out_err_code = 3;
      }
      if (out_err_msg != nullptr) {
        *out_err_msg = "Failed to initialize replacement NVE surface instance";
      }
      return false;
    }

    if (out_err_code != nullptr) {
      *out_err_code = 0;
    }
    if (out_err_msg != nullptr) {
      out_err_msg->clear();
    }
    return true;
  }

  void set_text_measurer(TextMeasurerCallback cb) {
    text_measurer_ = std::move(cb);
    if (is_valid()) {
      fabric_host_set_text_measurer(bridge_->fabric_host(), text_measurer_);
    }
  }

  void set_event_callback(EventCallback cb) {
    event_callback_ = std::move(cb);
    if (bridge_ != nullptr) {
      bridge_->set_event_callback(event_callback_);
    }
  }

  void set_surface_size(float width, float height, float scale_factor = 0.0f) {
    config_.width = width;
    config_.height = height;
    if (scale_factor > 0.0f) {
      config_.scale_factor = scale_factor;
    }
    if (bridge_ != nullptr) {
      bridge_->set_surface_size(width, height, scale_factor);
    }
  }

  bool update_props_json(std::string_view props_json, bool auto_render = true) {
    if (!is_valid()) {
      return false;
    }
    return bridge_->update_props_json(props_json, auto_render);
  }

  bool update_props_direct(const NveMapEntry* entries, uint32_t count, bool auto_render = true) {
    if (!is_valid()) {
      return false;
    }
    return bridge_->update_props_direct(entries, count, auto_render);
  }

  int32_t render(NveFrameStats* out_stats = nullptr) {
    if (!is_valid()) {
      return -100;
    }
    return bridge_->render(out_stats);
  }

  uint32_t dispatch_press(uint32_t node_id, float x, float y, bool auto_render = true) {
    if (!is_valid()) {
      return 0;
    }
    fabric_host_enqueue_press(bridge_->fabric_host(), node_id, current_timestamp_us(), x, y);
    const NveBufferSlice inbound = nve_fabric_host_drain_events(bridge_->fabric_host());
    return bridge_->dispatch_native_events(inbound, auto_render);
  }

  uint32_t dispatch_direct_events(NveEventBatch batch, bool auto_render = true) {
    if (!is_valid()) {
      return 0;
    }
    return bridge_->dispatch_direct_events(batch, auto_render);
  }

  std::vector<DecodedEvent> drain_events() {
    if (!is_valid()) {
      return {};
    }
    return bridge_->drain_emitted_events();
  }

  std::vector<RenderedNodeSnapshot> snapshot_nodes() const {
    if (!is_valid()) {
      return {};
    }
    return fabric_host_snapshot_nodes(bridge_->fabric_host());
  }

  NveRnComponentBridge* bridge() noexcept { return bridge_.get(); }
  const NveRnComponentBridge* bridge() const noexcept { return bridge_.get(); }
  const NveSurfaceConfig& surface_config() const noexcept { return config_; }
  bool is_valid() const noexcept { return bridge_ != nullptr && bridge_->is_valid(); }

 private:
  bool install_module(
      NveModule mod,
      DynamicLibrary new_lib,
      NveSurfaceConfig config,
      const std::vector<uint8_t>& saved_state) {
    auto next_bridge = std::make_unique<NveRnComponentBridge>(mod, config, event_callback_);
    if (!next_bridge->is_valid()) {
      return false;
    }
    if (text_measurer_) {
      fabric_host_set_text_measurer(next_bridge->fabric_host(), text_measurer_);
    }
    if (!saved_state.empty()) {
      next_bridge->restore_state(
          NveBufferSlice{saved_state.data(), saved_state.size()}, /*auto_render=*/false);
    }

    // Tear down old bridge before unloading old dylib, then commit the replacement atomically.
    bridge_.reset();
    lib_.close();
    lib_ = std::move(new_lib);
    config_ = config;
    bridge_ = std::move(next_bridge);
    (void)bridge_->render(nullptr);
    return true;
  }

  // Declaration order matters: `bridge_` is destroyed before `lib_` is closed.
  DynamicLibrary lib_;
  std::unique_ptr<NveRnComponentBridge> bridge_;
  TextMeasurerCallback text_measurer_;
  EventCallback event_callback_;
  NveSurfaceConfig config_{};
};

} // namespace nve
