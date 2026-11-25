#pragma once

#include "nve_nitro_promise.hpp"

#include <atomic>
#include <cstddef>
#include <cstdint>
#include <cstring>
#include <functional>
#include <memory>
#include <mutex>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <type_traits>
#include <unordered_map>
#include <utility>
#include <variant>
#include <vector>

#if defined(NVE_WITH_RN_JSI) && NVE_WITH_RN_JSI
#  include <jsi/jsi.h>
#endif

// =============================================================================
// C-ABI Opaque Handle Bridge (Zero JS VM — callable from Rust / Swift / Kotlin)
// =============================================================================

#ifdef __cplusplus
extern "C" {
#endif

typedef struct NveNitroHandleOpaque* NveNitroHandle;

/// Status codes returned by `nve_nitro_invoke_bytes`.
typedef enum NveNitroStatus {
  NVE_NITRO_OK = 0,
  NVE_NITRO_ERR_INVALID_HANDLE = 1,
  NVE_NITRO_ERR_NOT_FOUND = 2,
  NVE_NITRO_ERR_DISPOSED = 3,
  NVE_NITRO_ERR_BUFFER_TOO_SMALL = 4,
  NVE_NITRO_ERR_INVOCATION_FAILED = 5,
} NveNitroStatus;

/// Instantiates a registered `HybridObject` from the global registry. Returns `NULL` if unregistered.
NveNitroHandle nve_nitro_create(const char* name);

/// Increments the strong reference count of `handle` (no-op on `NULL`).
void nve_nitro_retain(NveNitroHandle handle);

/// Decrements the strong reference count of `handle`, freeing it when count reaches 0.
void nve_nitro_release(NveNitroHandle handle);

/// Marks the underlying `HybridObject` as disposed (idempotent, no-op on `NULL`).
void nve_nitro_dispose(NveNitroHandle handle);

/// Returns `1` if `handle` is `NULL` or disposed, `0` otherwise.
int32_t nve_nitro_is_disposed(NveNitroHandle handle);

/// Returns the null-terminated `HybridObject` name, or `NULL` for an invalid handle.
const char* nve_nitro_get_name(NveNitroHandle handle);

/// Returns `memory_size()` of the underlying `HybridObject`, or `0` for an invalid handle.
size_t nve_nitro_memory_size(NveNitroHandle handle);

/// Returns `1` if `name` is registered in `HybridObjectRegistry::global()`, `0` otherwise.
int32_t nve_nitro_has_registered(const char* name);

/// Invokes a zero-JSON binary method on `handle` with raw argument bytes (`NVNP`/`NVNC`/`NVCF` or
/// module-defined binary payload). Writes result bytes to `out_buf` and actual length to `*out_len`.
/// If `out_cap` is smaller than required, sets `*out_len` to required size and returns
/// `NVE_NITRO_ERR_BUFFER_TOO_SMALL`.
int32_t nve_nitro_invoke_bytes(
    NveNitroHandle handle,
    const char* method,
    const uint8_t* arg_bytes,
    size_t arg_len,
    uint8_t* out_buf,
    size_t out_cap,
    size_t* out_len);

#ifdef __cplusplus
} // extern "C"
#endif

// =============================================================================
// C++20 Zero-RN NitroCore Compatibility Runtime
// =============================================================================

namespace nve::nitro {

class HybridObject;

// -----------------------------------------------------------------------------
// ArrayBuffer (Zero-Copy Owning or Borrowed Byte Buffer)
// -----------------------------------------------------------------------------

class ArrayBuffer final {
 public:
  using Deleter = std::function<void()>;

  /// Allocates a zero-initialized owning buffer of `size` bytes.
  static std::shared_ptr<ArrayBuffer> allocate(size_t size) {
    return std::shared_ptr<ArrayBuffer>(new ArrayBuffer(std::vector<uint8_t>(size, 0u)));
  }

  /// Copies `bytes` into a new owning `ArrayBuffer`.
  static std::shared_ptr<ArrayBuffer> copy(std::span<const uint8_t> bytes) {
    return std::shared_ptr<ArrayBuffer>(new ArrayBuffer(std::vector<uint8_t>(bytes.begin(), bytes.end())));
  }

  /// Takes ownership of an existing `std::vector<uint8_t>` without copying.
  static std::shared_ptr<ArrayBuffer> from_vector(std::vector<uint8_t> storage) {
    return std::shared_ptr<ArrayBuffer>(new ArrayBuffer(std::move(storage)));
  }

  /// Zero-copy borrowed view over externally managed memory (`is_owning() == false`).
  static std::shared_ptr<ArrayBuffer> wrap(uint8_t* ptr, size_t size) {
    return std::shared_ptr<ArrayBuffer>(new ArrayBuffer(ptr, size, false, {}));
  }

  /// Zero-copy owned view over external memory released via `deleter` (`is_owning() == true`).
  static std::shared_ptr<ArrayBuffer> wrap_owned(uint8_t* ptr, size_t size, Deleter deleter) {
    return std::shared_ptr<ArrayBuffer>(new ArrayBuffer(ptr, size, true, std::move(deleter)));
  }

  ArrayBuffer(const ArrayBuffer&) = delete;
  ArrayBuffer& operator=(const ArrayBuffer&) = delete;

  ~ArrayBuffer() {
    if (deleter_) {
      deleter_();
    }
  }

  uint8_t* data() noexcept { return owned_ ? owned_->data() : external_ptr_; }
  const uint8_t* data() const noexcept { return owned_ ? owned_->data() : external_ptr_; }
  size_t size() const noexcept { return owned_ ? owned_->size() : size_; }
  bool empty() const noexcept { return size() == 0u; }
  bool is_owning() const noexcept { return is_owning_; }

  std::span<uint8_t> span() noexcept { return {data(), size()}; }
  std::span<const uint8_t> span() const noexcept { return {data(), size()}; }

  std::vector<uint8_t> copy_bytes() const {
    const uint8_t* ptr = data();
    const size_t n = size();
    if (!ptr || n == 0u) return {};
    return std::vector<uint8_t>(ptr, ptr + n);
  }

 private:
  explicit ArrayBuffer(std::vector<uint8_t> storage)
      : owned_(std::move(storage)), is_owning_(true) {}

  ArrayBuffer(uint8_t* ptr, size_t size, bool owning, Deleter deleter)
      : external_ptr_(ptr), size_(size), is_owning_(owning), deleter_(std::move(deleter)) {}

  std::optional<std::vector<uint8_t>> owned_{};
  uint8_t* external_ptr_{nullptr};
  size_t size_{0u};
  bool is_owning_{false};
  Deleter deleter_{};
};

// -----------------------------------------------------------------------------
// NitroVariant (Zero-JSON Typed Value for Direct C++ / C-ABI Invocation)
// -----------------------------------------------------------------------------

enum class NitroValueKind : uint8_t {
  Null = 0,
  Bool = 1,
  Int64 = 2,
  Double = 3,
  String = 4,
  ArrayBuffer = 5,
  HybridObject = 6,
  Error = 7,
};

struct NitroError {
  std::string message;
};

struct NitroVariant {
  using Storage = std::variant<
      std::monostate,
      bool,
      int64_t,
      double,
      std::string,
      std::shared_ptr<ArrayBuffer>,
      std::shared_ptr<HybridObject>,
      NitroError>;

  Storage storage{};

  static NitroVariant null() { return NitroVariant{std::monostate{}}; }
  static NitroVariant from_bool(bool v) { return NitroVariant{v}; }
  static NitroVariant from_i64(int64_t v) { return NitroVariant{v}; }
  static NitroVariant from_f64(double v) { return NitroVariant{v}; }
  static NitroVariant from_string(std::string v) { return NitroVariant{std::move(v)}; }
  static NitroVariant from_bytes(std::shared_ptr<ArrayBuffer> v) { return NitroVariant{std::move(v)}; }
  static NitroVariant from_object(std::shared_ptr<HybridObject> v) { return NitroVariant{std::move(v)}; }
  static NitroVariant from_error(std::string msg) { return NitroVariant{NitroError{std::move(msg)}}; }

  NitroValueKind kind() const noexcept { return static_cast<NitroValueKind>(storage.index()); }
  bool is_null() const noexcept { return std::holds_alternative<std::monostate>(storage); }
  bool is_error() const noexcept { return std::holds_alternative<NitroError>(storage); }

  std::optional<bool> as_bool() const noexcept {
    if (const auto* p = std::get_if<bool>(&storage)) return *p;
    return std::nullopt;
  }
  std::optional<int64_t> as_i64() const noexcept {
    if (const auto* p = std::get_if<int64_t>(&storage)) return *p;
    return std::nullopt;
  }
  std::optional<double> as_f64() const noexcept {
    if (const auto* p = std::get_if<double>(&storage)) return *p;
    return std::nullopt;
  }
  const std::string* as_string() const noexcept { return std::get_if<std::string>(&storage); }
  std::shared_ptr<ArrayBuffer> as_array_buffer() const noexcept {
    if (const auto* p = std::get_if<std::shared_ptr<ArrayBuffer>>(&storage)) return *p;
    return nullptr;
  }
  std::shared_ptr<HybridObject> as_hybrid_object() const noexcept {
    if (const auto* p = std::get_if<std::shared_ptr<HybridObject>>(&storage)) return *p;
    return nullptr;
  }
  const std::string* as_error() const noexcept {
    if (const auto* p = std::get_if<NitroError>(&storage)) return &p->message;
    return nullptr;
  }
};

// -----------------------------------------------------------------------------
// HybridObject (Zero-RN Base Class + Optional `NVE_WITH_RN_JSI` NativeState Hook)
// -----------------------------------------------------------------------------

#if defined(NVE_WITH_RN_JSI) && NVE_WITH_RN_JSI
class HybridJsiNativeState final : public facebook::jsi::NativeState {
 public:
  explicit HybridJsiNativeState(std::shared_ptr<HybridObject> target) : target_(std::move(target)) {}
  std::shared_ptr<HybridObject> target() const noexcept { return target_; }

 private:
  std::shared_ptr<HybridObject> target_;
};
#endif

class HybridObject : public std::enable_shared_from_this<HybridObject> {
 public:
  explicit HybridObject(std::string name) : name_(std::move(name)) {}
  virtual ~HybridObject() = default;

  HybridObject(const HybridObject&) = delete;
  HybridObject& operator=(const HybridObject&) = delete;

  virtual std::string_view name() const noexcept { return name_; }
  virtual size_t memory_size() const noexcept { return sizeof(*this) + name_.capacity(); }

  /// Releases native resources held by this HybridObject (idempotent).
  virtual void dispose() { disposed_.store(true, std::memory_order_release); }
  bool is_disposed() const noexcept { return disposed_.load(std::memory_order_acquire); }

  /// Dynamic zero-JSON method dispatch hook (used by `nve_nitro_invoke_bytes` and cross-language FFI).
  virtual NitroVariant invoke_method(std::string_view method, std::span<const NitroVariant> args) {
    (void)args;
    return NitroVariant::from_error("METHOD_NOT_FOUND: " + std::string(method));
  }

  /// Dynamic property read/write hooks for host inspection without JS VM.
  virtual NitroVariant get_property(std::string_view prop) const {
    if (prop == "name") return NitroVariant::from_string(name_);
    if (prop == "disposed") return NitroVariant::from_bool(is_disposed());
    return NitroVariant::null();
  }

  virtual bool set_property(std::string_view prop, const NitroVariant& value) {
    (void)prop;
    (void)value;
    return false;
  }

#if defined(NVE_WITH_RN_JSI) && NVE_WITH_RN_JSI
  /// Wraps this HybridObject into a `facebook::jsi::NativeState` for RN Fabric / Nitro JSI hosts.
  std::shared_ptr<facebook::jsi::NativeState> as_jsi_native_state() {
    return std::make_shared<HybridJsiNativeState>(shared_from_this());
  }
#endif

 protected:
  void mark_disposed() noexcept { disposed_.store(true, std::memory_order_release); }

 private:
  std::string name_;
  std::atomic<bool> disposed_{false};
};

// -----------------------------------------------------------------------------
// HybridObjectRegistry (Thread-Safe Factory Registry)
// -----------------------------------------------------------------------------

class HybridObjectRegistry final {
 public:
  using Factory = std::function<std::shared_ptr<HybridObject>()>;

  static HybridObjectRegistry& global() {
    static HybridObjectRegistry instance;
    return instance;
  }

  void register_hybrid_object(std::string name, Factory factory) {
    std::lock_guard<std::mutex> lock(mu_);
    factories_[std::move(name)] = std::move(factory);
  }

  bool unregister_hybrid_object(std::string_view name) {
    std::lock_guard<std::mutex> lock(mu_);
    auto it = factories_.find(std::string(name));
    if (it == factories_.end()) return false;
    factories_.erase(it);
    return true;
  }

  bool has_hybrid_object(std::string_view name) const {
    std::lock_guard<std::mutex> lock(mu_);
    return factories_.find(std::string(name)) != factories_.end();
  }

  std::vector<std::string> registered_names() const {
    std::lock_guard<std::mutex> lock(mu_);
    std::vector<std::string> out;
    out.reserve(factories_.size());
    for (const auto& [k, _] : factories_) out.push_back(k);
    return out;
  }

  void clear() {
    std::lock_guard<std::mutex> lock(mu_);
    factories_.clear();
  }

  std::shared_ptr<HybridObject> create_raw(std::string_view name) const {
    Factory factory{};
    {
      std::lock_guard<std::mutex> lock(mu_);
      auto it = factories_.find(std::string(name));
      if (it == factories_.end()) return nullptr;
      factory = it->second;
    }
    return factory ? factory() : nullptr;
  }

  template <typename T = HybridObject>
  std::shared_ptr<T> create(std::string_view name) const {
    static_assert(std::is_base_of_v<HybridObject, T>, "T must derive from nve::nitro::HybridObject");
    auto obj = create_raw(name);
    if (!obj) return nullptr;
    return std::dynamic_pointer_cast<T>(std::move(obj));
  }

 private:
  mutable std::mutex mu_;
  std::unordered_map<std::string, Factory> factories_;
};

} // namespace nve::nitro
