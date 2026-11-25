#pragma once

#include "nve_abi.h"
#include "nve_value.hpp"

#include <algorithm>
#include <bit>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <functional>
#include <limits>
#include <memory>
#include <string>
#include <string_view>
#include <type_traits>
#include <utility>
#include <vector>

#if defined(NVE_WITH_RN_JSI) || defined(NVE_WITH_RN_FABRIC)
#include <jsi/jsi.h>
#endif

namespace nve {

/**
 * Lightweight binary EventRing writer matching `crates/nve-abi/src/events.rs`.
 *
 * When compiled with React Native Fabric (`NVE_WITH_RN_FABRIC=1`), our `EventListener`
 * intercepts `facebook::react::RawEvent` in C++ before JSI (returning `true` to suppress JS
 * dispatch). Built-in events downcast `EventPayload::getType()` directly; arbitrary 3rd-party
 * RN Fabric components whose `ValueFactoryEventPayload` only exposes `asJSIValue(jsi::Runtime&)`
 * are captured via `MockJsiRecorderRuntime` (below) directly into `nve::BumpArena` for zero-copy
 * direct FFI dispatch (`dispatch_events_direct`), or encoded in-place into compact binary
 * `NveBinValue` inside this `EventRingWriter` for non-shared boundaries — zero Hermes, zero JSON!
 */
class EventRingWriter {
 public:
  void clear() {
    count_ = 0;
    buf_.clear();
    buf_.resize(4, 0);
  }

  uint32_t count() const noexcept { return count_; }

  void push_empty(uint32_t surface_id, uint32_t node_id, uint64_t ts_us, uint8_t kind) {
    if (count_ == std::numeric_limits<uint32_t>::max()) {
      return;
    }
    write_event_header(surface_id, node_id, ts_us, kind, 0u /* EventPayload::Empty */);
    ++count_;
  }

  void push_press(uint32_t surface_id, uint32_t node_id, uint64_t ts_us, float x, float y) {
    if (count_ == std::numeric_limits<uint32_t>::max()) {
      return;
    }
    write_event_header(
        surface_id, node_id, ts_us, 1u /* NativeEventKind::Press */, 1u /* EventPayload::Point */);
    write_le<float>(x);
    write_le<float>(y);
    ++count_;
  }

  void push_long_press(uint32_t surface_id, uint32_t node_id, uint64_t ts_us, float x, float y) {
    if (count_ == std::numeric_limits<uint32_t>::max()) {
      return;
    }
    write_event_header(
        surface_id,
        node_id,
        ts_us,
        2u /* NativeEventKind::LongPress */,
        1u /* EventPayload::Point */);
    write_le<float>(x);
    write_le<float>(y);
    ++count_;
  }

  void push_scroll(
      uint32_t surface_id,
      uint32_t node_id,
      uint64_t ts_us,
      float x,
      float y,
      float content_width,
      float content_height) {
    if (count_ == std::numeric_limits<uint32_t>::max()) {
      return;
    }
    write_event_header(
        surface_id,
        node_id,
        ts_us,
        3u /* NativeEventKind::Scroll */,
        2u /* EventPayload::Scroll */);
    write_le<float>(x);
    write_le<float>(y);
    write_le<float>(content_width);
    write_le<float>(content_height);
    ++count_;
  }

  bool push_text_change(
      uint32_t surface_id,
      uint32_t node_id,
      uint64_t ts_us,
      std::string_view text) {
    if (count_ == std::numeric_limits<uint32_t>::max() ||
        text.size() > static_cast<size_t>(std::numeric_limits<uint32_t>::max()) ||
        (!text.empty() && text.data() == nullptr)) {
      return false;
    }
    write_event_header(
        surface_id,
        node_id,
        ts_us,
        4u /* NativeEventKind::TextChange */,
        3u /* EventPayload::Text */);
    write_le<uint32_t>(static_cast<uint32_t>(text.size()));
    if (!text.empty()) {
      buf_.insert(
          buf_.end(),
          reinterpret_cast<const uint8_t*>(text.data()),
          reinterpret_cast<const uint8_t*>(text.data()) + text.size());
    }
    ++count_;
    return true;
  }

  void push_layout(
      uint32_t surface_id,
      uint32_t node_id,
      uint64_t ts_us,
      float x,
      float y,
      float width,
      float height) {
    if (count_ == std::numeric_limits<uint32_t>::max()) {
      return;
    }
    write_event_header(
        surface_id,
        node_id,
        ts_us,
        7u /* NativeEventKind::Layout */,
        4u /* EventPayload::Layout */);
    write_le<float>(x);
    write_le<float>(y);
    write_le<float>(width);
    write_le<float>(height);
    ++count_;
  }

  bool push_custom(
      uint32_t surface_id,
      uint32_t node_id,
      uint64_t ts_us,
      std::string_view name,
      const uint8_t* data,
      size_t len) {
    if ((data == nullptr && len > 0u) ||
        len > static_cast<size_t>(std::numeric_limits<uint32_t>::max())) {
      return false;
    }
    return write_custom_event(surface_id, node_id, ts_us, name, [&](std::vector<uint8_t>& buf) {
      if (len > 0u) {
        buf.insert(buf.end(), data, data + len);
      }
      return true;
    });
  }

  /// Encodes `value` directly into this `EventRingWriter`'s preallocated byte buffer using compact
  /// binary `NveBinValue` (zero intermediate string/vector allocation, zero JSON).
  bool push_custom_value(
      uint32_t surface_id,
      uint32_t node_id,
      uint64_t ts_us,
      std::string_view name,
      const NveValue& value) {
    return write_custom_event(surface_id, node_id, ts_us, name, [&](std::vector<uint8_t>& buf) {
      return encode_bin_value(buf, value);
    });
  }

  /// Encode a single `NveDirectEvent` into this binary `EventRingWriter` (fallback path).
  /// Rejects unknown `payload_tag` values cleanly instead of silently mapping to Custom.
  bool push_direct_event(const NveDirectEvent& ev) {
    switch (ev.payload_tag) {
      case 0u:
        push_empty(ev.surface_id, ev.node_id, ev.timestamp_us, ev.kind);
        return true;
      case 1u:
        if (ev.kind == 2u) {
          push_long_press(ev.surface_id, ev.node_id, ev.timestamp_us, ev.x, ev.y);
        } else {
          push_press(ev.surface_id, ev.node_id, ev.timestamp_us, ev.x, ev.y);
        }
        return true;
      case 2u:
        push_scroll(ev.surface_id, ev.node_id, ev.timestamp_us, ev.x, ev.y, ev.width, ev.height);
        return true;
      case 3u:
        return push_text_change(ev.surface_id, ev.node_id, ev.timestamp_us, str_slice_view(ev.text));
      case 4u:
        push_layout(ev.surface_id, ev.node_id, ev.timestamp_us, ev.x, ev.y, ev.width, ev.height);
        return true;
      case 5u:
        return push_custom_value(
            ev.surface_id, ev.node_id, ev.timestamp_us, str_slice_view(ev.name), ev.value);
      default:
        return false;
    }
  }

  const std::vector<uint8_t>& finalize() {
    ensure_header();
    std::memcpy(buf_.data(), &count_, sizeof(uint32_t));
    return buf_;
  }

 private:
  void ensure_header() {
    if (buf_.size() < 4u) {
      buf_.resize(4u, 0u);
    }
  }

  template <typename T>
  void write_le(T v) {
    static_assert(std::is_trivially_copyable_v<T>);
    if constexpr (std::is_same_v<T, float>) {
      write_le<uint32_t>(std::bit_cast<uint32_t>(v));
    } else if constexpr (std::is_integral_v<T>) {
      using U = std::make_unsigned_t<T>;
      const U u = static_cast<U>(v);
      for (size_t i = 0u; i < sizeof(U); ++i) {
        buf_.push_back(static_cast<uint8_t>((u >> (i * 8u)) & 0xFFu));
      }
    } else {
      const auto* p = reinterpret_cast<const uint8_t*>(&v);
      buf_.insert(buf_.end(), p, p + sizeof(T));
    }
  }

  void write_event_header(
      uint32_t surface_id,
      uint32_t node_id,
      uint64_t ts_us,
      uint8_t kind,
      uint8_t payload_tag) {
    ensure_header();
    write_le<uint32_t>(surface_id);
    write_le<uint32_t>(node_id);
    write_le<uint64_t>(ts_us);
    buf_.push_back(kind);
    buf_.push_back(payload_tag);
  }

  template <typename PayloadWriter>
  bool write_custom_event(
      uint32_t surface_id,
      uint32_t node_id,
      uint64_t ts_us,
      std::string_view name,
      PayloadWriter&& write_payload) {
    if (count_ == std::numeric_limits<uint32_t>::max() ||
        name.size() > static_cast<size_t>(std::numeric_limits<uint16_t>::max()) ||
        (!name.empty() && name.data() == nullptr)) {
      return false;
    }
    ensure_header();
    const size_t rollback = buf_.size();
    write_event_header(
        surface_id,
        node_id,
        ts_us,
        8u /* NativeEventKind::Custom */,
        5u /* EventPayload::Custom */);
    const uint16_t nlen = static_cast<uint16_t>(name.size());
    write_le<uint16_t>(nlen);
    if (nlen > 0u) {
      buf_.insert(
          buf_.end(),
          reinterpret_cast<const uint8_t*>(name.data()),
          reinterpret_cast<const uint8_t*>(name.data()) + nlen);
    }
    const size_t len_offset = buf_.size();
    write_le<uint32_t>(0u);
    const size_t payload_start = buf_.size();
    if (!write_payload(buf_)) {
      buf_.resize(rollback);
      return false;
    }
    const size_t payload_bytes = buf_.size() - payload_start;
    if (payload_bytes > static_cast<size_t>(std::numeric_limits<uint32_t>::max())) {
      buf_.resize(rollback);
      return false;
    }
    const uint32_t payload_len = static_cast<uint32_t>(payload_bytes);
    buf_[len_offset] = static_cast<uint8_t>(payload_len & 0xFFu);
    buf_[len_offset + 1u] = static_cast<uint8_t>((payload_len >> 8u) & 0xFFu);
    buf_[len_offset + 2u] = static_cast<uint8_t>((payload_len >> 16u) & 0xFFu);
    buf_[len_offset + 3u] = static_cast<uint8_t>((payload_len >> 24u) & 0xFFu);
    ++count_;
    return true;
  }

  uint32_t count_{0};
  std::vector<uint8_t> buf_;
};

#if defined(NVE_WITH_RN_JSI) || defined(NVE_WITH_RN_FABRIC)

/**
 * Zero-Hermes, zero-JS-heap, zero-allocation `facebook::jsi::Runtime` recorder subclass.
 *
 * All JSI objects, arrays, strings, `PropNameID`s, `NveValue`s, and `NveMapEntry` arrays are
 * allocated directly inside a frame-reusable `nve::BumpArena`:
 * - Primary in-process hot path: `record_direct_event(...)` + `dispatch_direct(...)` passes an
 *   `NveEventBatch` pointer directly across FFI with **0 bytes serialized, 0 heap allocations
 *   after warmup, and 0 JSON**.
 * - Non-shared boundary path: `record_custom_event(EventRingWriter&, ...)` encodes captured
 *   `NveValue` trees directly into `EventRingWriter`'s preallocated buffer as compact binary
 *   `NveBinValue` (readable with zero-copy `nve::BinValueView` / Rust `BinValueView`).
 */
class MockJsiRecorderRuntime final : public facebook::jsi::Runtime {
 public:
  using DirectEventSink = std::function<uint32_t(NveEventBatch)>;

  static constexpr uint32_t kNoSideSlot = std::numeric_limits<uint32_t>::max();
  static constexpr uint32_t kInitialObjectCap = 8u;

  enum class NodeKind : uint8_t {
    Undefined = 0,
    Null = 1,
    Bool = 2,
    Number = 3,
    BigIntSigned = 4,
    BigIntUnsigned = 5,
    String = 6,
    Symbol = 7,
    Object = 8,
    Array = 9,
    ArrayBuffer = 10,
    HostObject = 11,
    HostFunction = 12,
  };

  /// Trivially-destructible arena node storing `NveMapEntry[]` / `NveValue[]` directly in-place.
  struct ArenaNode {
    NodeKind kind{NodeKind::Undefined};
    bool is_global{false};
    bool has_nested_containers{false};
    uint32_t len{0};
    uint32_t cap{0};
    uint32_t side_slot{kNoSideSlot};
    uint32_t native_state_slot{kNoSideSlot};

    union {
      bool bool_val;
      double num_val;
      int64_t i64_val;
      uint64_t u64_val;
      NveStrSlice str_val;
      NveMapEntry* map_entries;
      NveValue* array_elems;
    };

    /// Lazily allocated parallel child-node pointers only when an Object/Array stores nested
    /// JSI containers (allows O(1) finalize for flat objects and handles post-assignment mutation).
    ArenaNode** child_nodes{nullptr};

    ArenaNode() noexcept : u64_val(0) {}
  };

  static_assert(std::is_trivially_destructible_v<ArenaNode>);

  /// Arena-allocated `PointerValue` whose `invalidate()` is a no-op (reclaimed on `reset_frame()`).
  struct ArenaPointerValue final : public PointerValue {
    ArenaNode* node{nullptr};
    explicit ArenaPointerValue(ArenaNode* n) noexcept : node(n) {}
    void invalidate() noexcept override {}
  };

  explicit MockJsiRecorderRuntime(size_t initial_arena_bytes = BumpArena::kDefaultInitialSlabBytes)
      : arena_(initial_arena_bytes), global_arena_(4096u) {
    global_node_ = alloc_object_node(global_arena_, /*is_global=*/true);
    global_pv_ = global_arena_.emplace<ArenaPointerValue>(global_node_);
    direct_events_.reserve(64);
  }

  ~MockJsiRecorderRuntime() override = default;

  using facebook::jsi::Runtime::getProperty;
  using facebook::jsi::Runtime::hasProperty;
  using facebook::jsi::Runtime::setPropertyValue;

  /// Reset frame arena and direct event batch in O(1) with zero deallocations (reuses slabs).
  void reset_frame() noexcept {
    direct_events_.clear();
    if (!buffers_.empty()) buffers_.clear();
    if (!host_objects_.empty()) host_objects_.clear();
    if (!host_functions_.empty()) host_functions_.clear();
    if (!native_states_.empty()) native_states_.clear();
    arena_.reset();
  }

  /// Capture any `facebook::jsi::Value` as a zero-copy `NveValue` backed by `arena_`.
  /// Valid until the next `reset_frame()` call.
  NveValue capture_value(const facebook::jsi::Value& value) {
    return materialize_jsi_value(value, 0u);
  }

  /// Primary in-process hot path: capture `value` directly into `arena_` and append an
  /// `NveDirectEvent` to the current frame's batch (0 bytes serialized, 0 JSON).
  void record_direct_event(
      uint32_t surface_id,
      uint32_t node_id,
      uint64_t ts_us,
      std::string_view event_name,
      const facebook::jsi::Value& value) {
    NveDirectEvent ev{};
    ev.surface_id = surface_id;
    ev.node_id = node_id;
    ev.timestamp_us = ts_us;
    ev.kind = 8u;        /* NativeEventKind::Custom */
    ev.payload_tag = 5u; /* EventPayload::Custom */
    ev.name = arena_.copy_str(event_name);
    ev.value = capture_value(value);
    direct_events_.push_back(ev);
  }

  /// Borrowed view of all direct events recorded since the last `reset_frame()`.
  NveEventBatch direct_event_batch() const noexcept {
    return NveEventBatch{
        direct_events_.empty() ? nullptr : direct_events_.data(),
        static_cast<uint32_t>(direct_events_.size()),
    };
  }

  /// Dispatch the recorded `NveEventBatch` directly into `NveSurfaceApi::dispatch_events_direct`
  /// (zero serialization, zero copy). Falls back to binary `EventRing` if the module only
  /// implements `dispatch_events`.
  uint32_t dispatch_direct(const NveSurfaceApi* surface, NveAppHandle app) {
    if (surface == nullptr || app == nullptr || direct_events_.empty()) {
      return 0u;
    }
    if (surface->dispatch_events_direct != nullptr) {
      return surface->dispatch_events_direct(app, direct_event_batch());
    }
    if (surface->dispatch_events != nullptr) {
      fallback_ring_.clear();
      for (const auto& ev : direct_events_) {
        fallback_ring_.push_direct_event(ev);
      }
      const auto& bytes = fallback_ring_.finalize();
      return surface->dispatch_events(app, NveBufferSlice{bytes.data(), bytes.size()});
    }
    return 0u;
  }

  /// Dispatch the recorded `NveEventBatch` directly to any callable sink (`uint32_t(NveEventBatch)`).
  template <typename SinkFn>
  uint32_t dispatch_direct(SinkFn&& sink) const {
    if (direct_events_.empty()) {
      return 0u;
    }
    return static_cast<uint32_t>(sink(direct_event_batch()));
  }

  /// Non-shared boundary path: capture `value` in the arena and encode it as compact binary
  /// `NveBinValue` directly inside `EventRingWriter`'s preallocated buffer (zero JSON).
  bool record_custom_event(
      EventRingWriter& ring,
      uint32_t surface_id,
      uint32_t node_id,
      uint64_t ts_us,
      std::string_view event_name,
      const facebook::jsi::Value& value) {
    const NveValue captured = capture_value(value);
    return ring.push_custom_value(surface_id, node_id, ts_us, event_name, captured);
  }

  const BumpArena& arena() const noexcept { return arena_; }

  // ---- IRuntime / Runtime pure virtual overrides ----

  facebook::jsi::Value evaluateJavaScript(
      const std::shared_ptr<const facebook::jsi::Buffer>&,
      const std::string&) override {
    return facebook::jsi::Value::undefined();
  }

  std::shared_ptr<const facebook::jsi::PreparedJavaScript> prepareJavaScript(
      const std::shared_ptr<const facebook::jsi::Buffer>&,
      std::string) override {
    return nullptr;
  }

  facebook::jsi::Value evaluatePreparedJavaScript(
      const std::shared_ptr<const facebook::jsi::PreparedJavaScript>&) override {
    return facebook::jsi::Value::undefined();
  }

  void queueMicrotask(const facebook::jsi::Function&) override {}
  bool drainMicrotasks(int) override { return true; }

  facebook::jsi::Object global() override {
    return make<facebook::jsi::Object>(global_pv_);
  }

  std::string description() override { return "nve::MockJsiRecorderRuntime"; }
  bool isInspectable() override { return false; }

  PointerValue* cloneSymbol(const PointerValue* pv) override { return clone_pv(pv); }
  PointerValue* cloneBigInt(const PointerValue* pv) override { return clone_pv(pv); }
  PointerValue* cloneString(const PointerValue* pv) override { return clone_pv(pv); }
  PointerValue* cloneObject(const PointerValue* pv) override { return clone_pv(pv); }
  PointerValue* clonePropNameID(const PointerValue* pv) override { return clone_pv(pv); }

  facebook::jsi::PropNameID createPropNameIDFromAscii(const char* str, size_t length) override {
    return make<facebook::jsi::PropNameID>(make_string_pv(str, length));
  }

  facebook::jsi::PropNameID createPropNameIDFromUtf8(const uint8_t* utf8, size_t length) override {
    return make<facebook::jsi::PropNameID>(
        make_string_pv(reinterpret_cast<const char*>(utf8), length));
  }

  facebook::jsi::PropNameID createPropNameIDFromString(const facebook::jsi::String& str) override {
    ArenaNode* n = extract_node(str);
    return make<facebook::jsi::PropNameID>(arena_.emplace<ArenaPointerValue>(n));
  }

  facebook::jsi::PropNameID createPropNameIDFromSymbol(const facebook::jsi::Symbol& sym) override {
    ArenaNode* n = extract_node(sym);
    const std::string_view sv = str_slice_view(n ? n->str_val : NveStrSlice{nullptr, 0});
    return make<facebook::jsi::PropNameID>(make_string_pv(sv.data(), sv.size()));
  }

  std::string utf8(const facebook::jsi::PropNameID& id) override {
    const ArenaNode* n = extract_node(id);
    return std::string(n ? str_slice_view(n->str_val) : std::string_view{});
  }

  bool compare(const facebook::jsi::PropNameID& a, const facebook::jsi::PropNameID& b) override {
    const ArenaNode* na = extract_node(a);
    const ArenaNode* nb = extract_node(b);
    return str_slice_view(na ? na->str_val : NveStrSlice{nullptr, 0}) ==
           str_slice_view(nb ? nb->str_val : NveStrSlice{nullptr, 0});
  }

  std::string symbolToString(const facebook::jsi::Symbol& sym) override {
    const ArenaNode* n = extract_node(sym);
    std::string out = "Symbol(";
    if (n) {
      out.append(str_slice_view(n->str_val));
    }
    out.push_back(')');
    return out;
  }

  facebook::jsi::BigInt createBigIntFromInt64(int64_t v) override {
    ArenaNode* n = arena_.make<ArenaNode>();
    n->kind = NodeKind::BigIntSigned;
    n->i64_val = v;
    return make<facebook::jsi::BigInt>(arena_.emplace<ArenaPointerValue>(n));
  }

  facebook::jsi::BigInt createBigIntFromUint64(uint64_t v) override {
    ArenaNode* n = arena_.make<ArenaNode>();
    n->kind = NodeKind::BigIntUnsigned;
    n->u64_val = v;
    return make<facebook::jsi::BigInt>(arena_.emplace<ArenaPointerValue>(n));
  }

  bool bigintIsInt64(const facebook::jsi::BigInt& b) override {
    const ArenaNode* n = extract_node(b);
    return n && n->kind == NodeKind::BigIntSigned;
  }

  bool bigintIsUint64(const facebook::jsi::BigInt& b) override {
    const ArenaNode* n = extract_node(b);
    return n && n->kind == NodeKind::BigIntUnsigned;
  }

  uint64_t truncate(const facebook::jsi::BigInt& b) override {
    const ArenaNode* n = extract_node(b);
    if (!n) return 0;
    return (n->kind == NodeKind::BigIntSigned) ? static_cast<uint64_t>(n->i64_val) : n->u64_val;
  }

  facebook::jsi::String bigintToString(const facebook::jsi::BigInt& b, int) override {
    const ArenaNode* n = extract_node(b);
    if (!n) return createStringFromAscii("0", 1);
    const std::string s = (n->kind == NodeKind::BigIntSigned)
                              ? std::to_string(n->i64_val)
                              : std::to_string(n->u64_val);
    return createStringFromAscii(s.data(), s.size());
  }

  facebook::jsi::String createStringFromAscii(const char* str, size_t length) override {
    return make<facebook::jsi::String>(make_string_pv(str, length));
  }

  facebook::jsi::String createStringFromUtf8(const uint8_t* utf8, size_t length) override {
    return make<facebook::jsi::String>(
        make_string_pv(reinterpret_cast<const char*>(utf8), length));
  }

  std::string utf8(const facebook::jsi::String& str) override {
    const ArenaNode* n = extract_node(str);
    return std::string(n ? str_slice_view(n->str_val) : std::string_view{});
  }

  facebook::jsi::Object createObject() override {
    ArenaNode* n = alloc_object_node(arena_, /*is_global=*/false);
    return make<facebook::jsi::Object>(arena_.emplace<ArenaPointerValue>(n));
  }

  facebook::jsi::Object createObject(std::shared_ptr<facebook::jsi::HostObject> ho) override {
    ArenaNode* n = alloc_object_node(arena_, /*is_global=*/false);
    n->kind = NodeKind::HostObject;
    n->side_slot = static_cast<uint32_t>(host_objects_.size());
    host_objects_.push_back(std::move(ho));
    return make<facebook::jsi::Object>(arena_.emplace<ArenaPointerValue>(n));
  }

  std::shared_ptr<facebook::jsi::HostObject> getHostObject(
      const facebook::jsi::Object& obj) override {
    const ArenaNode* n = extract_node(obj);
    if (n && n->kind == NodeKind::HostObject && n->side_slot < host_objects_.size()) {
      return host_objects_[n->side_slot];
    }
    return nullptr;
  }

  facebook::jsi::HostFunctionType& getHostFunction(const facebook::jsi::Function& fn) override {
    const ArenaNode* n = extract_node(fn);
    if (n && n->kind == NodeKind::HostFunction && n->side_slot < host_functions_.size()) {
      return host_functions_[n->side_slot];
    }
    static facebook::jsi::HostFunctionType empty_fn;
    return empty_fn;
  }

  bool hasNativeState(const facebook::jsi::Object& obj) override {
    const ArenaNode* n = extract_node(obj);
    return n && n->native_state_slot < native_states_.size() &&
           native_states_[n->native_state_slot] != nullptr;
  }

  std::shared_ptr<facebook::jsi::NativeState> getNativeState(
      const facebook::jsi::Object& obj) override {
    const ArenaNode* n = extract_node(obj);
    if (n && n->native_state_slot < native_states_.size()) {
      return native_states_[n->native_state_slot];
    }
    return nullptr;
  }

  void setNativeState(
      const facebook::jsi::Object& obj,
      std::shared_ptr<facebook::jsi::NativeState> state) override {
    ArenaNode* n = extract_node(obj);
    if (!n) return;
    if (n->native_state_slot < native_states_.size()) {
      native_states_[n->native_state_slot] = std::move(state);
    } else {
      n->native_state_slot = static_cast<uint32_t>(native_states_.size());
      native_states_.push_back(std::move(state));
    }
  }

  facebook::jsi::Value getProperty(
      const facebook::jsi::Object& obj,
      const facebook::jsi::PropNameID& name) override {
    const ArenaNode* n = extract_node(obj);
    const ArenaNode* kn = extract_node(name);
    if (!n || !kn) return facebook::jsi::Value::undefined();
    return lookup_prop(n, str_slice_view(kn->str_val));
  }

  facebook::jsi::Value getProperty(
      const facebook::jsi::Object& obj,
      const facebook::jsi::String& name) override {
    const ArenaNode* n = extract_node(obj);
    const ArenaNode* kn = extract_node(name);
    if (!n || !kn) return facebook::jsi::Value::undefined();
    return lookup_prop(n, str_slice_view(kn->str_val));
  }

  bool hasProperty(
      const facebook::jsi::Object& obj,
      const facebook::jsi::PropNameID& name) override {
    const ArenaNode* n = extract_node(obj);
    const ArenaNode* kn = extract_node(name);
    return n && kn && find_prop_index(n, str_slice_view(kn->str_val)) >= 0;
  }

  bool hasProperty(
      const facebook::jsi::Object& obj,
      const facebook::jsi::String& name) override {
    const ArenaNode* n = extract_node(obj);
    const ArenaNode* kn = extract_node(name);
    return n && kn && find_prop_index(n, str_slice_view(kn->str_val)) >= 0;
  }

  void setPropertyValue(
      const facebook::jsi::Object& obj,
      const facebook::jsi::PropNameID& name,
      const facebook::jsi::Value& value) override {
    ArenaNode* n = extract_node(obj);
    const ArenaNode* kn = extract_node(name);
    if (!n || !kn) return;
    set_object_prop(n, kn->str_val, value);
  }

  void setPropertyValue(
      const facebook::jsi::Object& obj,
      const facebook::jsi::String& name,
      const facebook::jsi::Value& value) override {
    ArenaNode* n = extract_node(obj);
    const ArenaNode* kn = extract_node(name);
    if (!n || !kn) return;
    set_object_prop(n, kn->str_val, value);
  }

  bool isArray(const facebook::jsi::Object& obj) const override {
    const ArenaNode* n = extract_node(obj);
    return n && n->kind == NodeKind::Array;
  }

  bool isArrayBuffer(const facebook::jsi::Object& obj) const override {
    const ArenaNode* n = extract_node(obj);
    return n && n->kind == NodeKind::ArrayBuffer;
  }

  bool isFunction(const facebook::jsi::Object& obj) const override {
    const ArenaNode* n = extract_node(obj);
    return n && n->kind == NodeKind::HostFunction;
  }

  bool isHostObject(const facebook::jsi::Object& obj) const override {
    const ArenaNode* n = extract_node(obj);
    return n && n->kind == NodeKind::HostObject;
  }

  bool isHostFunction(const facebook::jsi::Function& fn) const override {
    const ArenaNode* n = extract_node(fn);
    return n && n->kind == NodeKind::HostFunction;
  }

  facebook::jsi::Array getPropertyNames(const facebook::jsi::Object& obj) override {
    const ArenaNode* n = extract_node(obj);
    if (!n || (n->kind != NodeKind::Object && n->kind != NodeKind::HostObject)) {
      return createArray(0);
    }
    auto arr = createArray(n->len);
    for (uint32_t i = 0; i < n->len; ++i) {
      const NveStrSlice k = n->map_entries[i].key;
      setValueAtIndexImpl(arr, i, facebook::jsi::Value(createStringFromUtf8(k.ptr, k.len)));
    }
    return arr;
  }

  facebook::jsi::WeakObject createWeakObject(const facebook::jsi::Object& obj) override {
    return make<facebook::jsi::WeakObject>(arena_.emplace<ArenaPointerValue>(extract_node(obj)));
  }

  facebook::jsi::Value lockWeakObject(const facebook::jsi::WeakObject& obj) override {
    ArenaNode* n = extract_node(obj);
    if (!n) return facebook::jsi::Value::undefined();
    return facebook::jsi::Value(make<facebook::jsi::Object>(arena_.emplace<ArenaPointerValue>(n)));
  }

  facebook::jsi::Array createArray(size_t length) override {
    constexpr size_t kMaxArrayLen = static_cast<size_t>(std::numeric_limits<uint32_t>::max());
    const uint32_t bounded_len = static_cast<uint32_t>(std::min(length, kMaxArrayLen));
    ArenaNode* n = arena_.make<ArenaNode>();
    n->kind = NodeKind::Array;
    n->len = bounded_len;
    n->cap = bounded_len;
    if (bounded_len > 0u) {
      n->array_elems = arena_.alloc_array<NveValue>(bounded_len);
      for (uint32_t i = 0u; i < bounded_len; ++i) {
        n->array_elems[i] = make_null_value();
      }
    } else {
      n->array_elems = nullptr;
    }
    return make<facebook::jsi::Array>(arena_.emplace<ArenaPointerValue>(n));
  }

  facebook::jsi::ArrayBuffer createArrayBuffer(
      std::shared_ptr<facebook::jsi::MutableBuffer> buffer) override {
    ArenaNode* n = arena_.make<ArenaNode>();
    n->kind = NodeKind::ArrayBuffer;
    n->side_slot = static_cast<uint32_t>(buffers_.size());
    buffers_.push_back(std::move(buffer));
    return make<facebook::jsi::ArrayBuffer>(arena_.emplace<ArenaPointerValue>(n));
  }

  size_t size(const facebook::jsi::Array& arr) override {
    const ArenaNode* n = extract_node(arr);
    return (n && n->kind == NodeKind::Array) ? n->len : 0u;
  }

  size_t size(const facebook::jsi::ArrayBuffer& buf) override {
    const ArenaNode* n = extract_node(buf);
    if (n && n->kind == NodeKind::ArrayBuffer && n->side_slot < buffers_.size() &&
        buffers_[n->side_slot]) {
      return buffers_[n->side_slot]->size();
    }
    return 0u;
  }

  uint8_t* data(const facebook::jsi::ArrayBuffer& buf) override {
    const ArenaNode* n = extract_node(buf);
    if (n && n->kind == NodeKind::ArrayBuffer && n->side_slot < buffers_.size() &&
        buffers_[n->side_slot]) {
      return buffers_[n->side_slot]->data();
    }
    return nullptr;
  }

  facebook::jsi::Value getValueAtIndex(const facebook::jsi::Array& arr, size_t i) override {
    const ArenaNode* n = extract_node(arr);
    if (!n || n->kind != NodeKind::Array || i >= n->len) {
      return facebook::jsi::Value::undefined();
    }
    if (n->child_nodes != nullptr && n->child_nodes[i] != nullptr) {
      return node_to_jsi_value(n->child_nodes[i]);
    }
    return nve_value_to_jsi_value(n->array_elems[i]);
  }

  void setValueAtIndexImpl(
      const facebook::jsi::Array& arr,
      size_t i,
      const facebook::jsi::Value& value) override {
    ArenaNode* n = extract_node(arr);
    if (!n || n->kind != NodeKind::Array) return;
    constexpr size_t kMaxIndex = static_cast<size_t>(std::numeric_limits<uint32_t>::max()) - 1u;
    if (i > kMaxIndex) return;

    if (i >= n->cap) {
      constexpr size_t kMaxCap = static_cast<size_t>(std::numeric_limits<uint32_t>::max());
      const size_t doubled = (n->cap == 0u) ? size_t{4u} : (static_cast<size_t>(n->cap) * 2u);
      const size_t required = i + 1u;
      const uint32_t new_cap =
          static_cast<uint32_t>(std::min(kMaxCap, std::max(doubled, required)));
      auto* new_elems = arena_.alloc_array<NveValue>(new_cap);
      if (n->len > 0 && n->array_elems != nullptr) {
        std::memcpy(new_elems, n->array_elems, sizeof(NveValue) * n->len);
      }
      for (uint32_t j = n->len; j < new_cap; ++j) {
        new_elems[j] = make_null_value();
      }
      if (n->child_nodes != nullptr) {
        auto** new_children = arena_.alloc_array<ArenaNode*>(new_cap);
        std::memcpy(new_children, n->child_nodes, sizeof(ArenaNode*) * n->len);
        std::memset(new_children + n->len, 0, sizeof(ArenaNode*) * (new_cap - n->len));
        n->child_nodes = new_children;
      }
      n->array_elems = new_elems;
      n->cap = new_cap;
    }
    if (i >= n->len) {
      n->len = static_cast<uint32_t>(i + 1u);
    }

    ArenaNode* child_container = nullptr;
    n->array_elems[i] = jsi_value_to_slot(value, child_container);
    if (child_container != nullptr) {
      ensure_child_nodes(n, arena_);
      n->child_nodes[i] = child_container;
      n->has_nested_containers = true;
    } else if (n->child_nodes != nullptr) {
      n->child_nodes[i] = nullptr;
    }
  }

  facebook::jsi::Function createFunctionFromHostFunction(
      const facebook::jsi::PropNameID&,
      unsigned int,
      facebook::jsi::HostFunctionType func) override {
    ArenaNode* n = arena_.make<ArenaNode>();
    n->kind = NodeKind::HostFunction;
    n->side_slot = static_cast<uint32_t>(host_functions_.size());
    host_functions_.push_back(std::move(func));
    return make<facebook::jsi::Function>(arena_.emplace<ArenaPointerValue>(n));
  }

  facebook::jsi::Value call(
      const facebook::jsi::Function& fn,
      const facebook::jsi::Value& jsThis,
      const facebook::jsi::Value* args,
      size_t count) override {
    const ArenaNode* n = extract_node(fn);
    if (n && n->kind == NodeKind::HostFunction && n->side_slot < host_functions_.size() &&
        host_functions_[n->side_slot]) {
      return host_functions_[n->side_slot](*this, jsThis, args, count);
    }
    return facebook::jsi::Value::undefined();
  }

  facebook::jsi::Value callAsConstructor(
      const facebook::jsi::Function& fn,
      const facebook::jsi::Value* args,
      size_t count) override {
    return call(fn, facebook::jsi::Value::undefined(), args, count);
  }

  bool strictEquals(const facebook::jsi::Symbol& a, const facebook::jsi::Symbol& b) const override {
    return extract_node(a) == extract_node(b);
  }

  bool strictEquals(const facebook::jsi::BigInt& a, const facebook::jsi::BigInt& b) const override {
    const ArenaNode* na = extract_node(a);
    const ArenaNode* nb = extract_node(b);
    if (!na || !nb) return na == nb;
    return na->u64_val == nb->u64_val && na->kind == nb->kind;
  }

  bool strictEquals(const facebook::jsi::String& a, const facebook::jsi::String& b) const override {
    const ArenaNode* na = extract_node(a);
    const ArenaNode* nb = extract_node(b);
    if (!na || !nb) return na == nb;
    return str_slice_view(na->str_val) == str_slice_view(nb->str_val);
  }

  bool strictEquals(const facebook::jsi::Object& a, const facebook::jsi::Object& b) const override {
    return extract_node(a) == extract_node(b);
  }

  bool instanceOf(const facebook::jsi::Object&, const facebook::jsi::Function&) override {
    return false;
  }

  void setExternalMemoryPressure(const facebook::jsi::Object&, size_t) override {}

 private:
  static PointerValue* clone_pv(const PointerValue* pv) noexcept {
    // ArenaPointerValue::invalidate() is a no-op; reusing the pointer is O(1) and zero-alloc.
    return const_cast<PointerValue*>(pv);
  }

  static ArenaNode* extract_node(const facebook::jsi::Pointer& p) noexcept {
    const auto* pv = static_cast<const ArenaPointerValue*>(getPointerValue(p));
    return pv ? pv->node : nullptr;
  }

  static ArenaNode* alloc_object_node(BumpArena& target_arena, bool is_global) {
    ArenaNode* n = target_arena.make<ArenaNode>();
    n->kind = NodeKind::Object;
    n->is_global = is_global;
    n->len = 0;
    n->cap = kInitialObjectCap;
    n->map_entries = target_arena.alloc_array<NveMapEntry>(kInitialObjectCap);
    return n;
  }

  ArenaPointerValue* make_string_pv(const char* str, size_t length) {
    ArenaNode* n = arena_.make<ArenaNode>();
    n->kind = NodeKind::String;
    if (length > 0 && str != nullptr) {
      auto* dst = static_cast<uint8_t*>(arena_.alloc(length, 1u));
      std::memcpy(dst, str, length);
      n->str_val = NveStrSlice{dst, length};
    } else {
      n->str_val = NveStrSlice{nullptr, 0};
    }
    return arena_.emplace<ArenaPointerValue>(n);
  }

  static void ensure_child_nodes(ArenaNode* n, BumpArena& target_arena) {
    if (n->child_nodes == nullptr && n->cap > 0) {
      n->child_nodes = target_arena.alloc_array<ArenaNode*>(n->cap);
      std::memset(n->child_nodes, 0, sizeof(ArenaNode*) * n->cap);
    }
  }

  static int32_t find_prop_index(const ArenaNode* obj, std::string_view key) noexcept {
    if (!obj || (obj->kind != NodeKind::Object && obj->kind != NodeKind::HostObject)) {
      return -1;
    }
    for (uint32_t i = 0; i < obj->len; ++i) {
      if (str_slice_view(obj->map_entries[i].key) == key) {
        return static_cast<int32_t>(i);
      }
    }
    return -1;
  }

  void set_object_prop(ArenaNode* obj, NveStrSlice key, const facebook::jsi::Value& value) {
    BumpArena& target_arena = obj->is_global ? global_arena_ : arena_;
    const std::string_view key_sv = str_slice_view(key);

    // If writing to global object, ensure the key string lives in global_arena_.
    const NveStrSlice stored_key = obj->is_global ? global_arena_.copy_str(key_sv) : key;

    ArenaNode* child_container = nullptr;
    const NveValue slot_val = jsi_value_to_slot(value, child_container);

    const int32_t existing = find_prop_index(obj, key_sv);
    if (existing >= 0) {
      const uint32_t idx = static_cast<uint32_t>(existing);
      obj->map_entries[idx].value = slot_val;
      if (child_container != nullptr) {
        ensure_child_nodes(obj, target_arena);
        obj->child_nodes[idx] = child_container;
        obj->has_nested_containers = true;
      } else if (obj->child_nodes != nullptr) {
        obj->child_nodes[idx] = nullptr;
      }
      return;
    }

    if (obj->len == std::numeric_limits<uint32_t>::max()) {
      return;
    }
    if (obj->len >= obj->cap) {
      constexpr size_t kMaxCap = static_cast<size_t>(std::numeric_limits<uint32_t>::max());
      const size_t grown =
          (obj->cap == 0u) ? static_cast<size_t>(kInitialObjectCap) : (static_cast<size_t>(obj->cap) * 2u);
      const uint32_t new_cap = static_cast<uint32_t>(std::min(kMaxCap, grown));
      auto* new_entries = target_arena.alloc_array<NveMapEntry>(new_cap);
      if (obj->len > 0 && obj->map_entries != nullptr) {
        std::memcpy(new_entries, obj->map_entries, sizeof(NveMapEntry) * obj->len);
      }
      if (obj->child_nodes != nullptr) {
        auto** new_children = target_arena.alloc_array<ArenaNode*>(new_cap);
        std::memcpy(new_children, obj->child_nodes, sizeof(ArenaNode*) * obj->len);
        std::memset(new_children + obj->len, 0, sizeof(ArenaNode*) * (new_cap - obj->len));
        obj->child_nodes = new_children;
      }
      obj->map_entries = new_entries;
      obj->cap = new_cap;
    }

    const uint32_t idx = obj->len++;
    obj->map_entries[idx] = NveMapEntry{stored_key, slot_val};
    if (child_container != nullptr) {
      ensure_child_nodes(obj, target_arena);
      obj->child_nodes[idx] = child_container;
      obj->has_nested_containers = true;
    } else if (obj->child_nodes != nullptr) {
      obj->child_nodes[idx] = nullptr;
    }
  }

  /// Convert a `jsi::Value` to an `NveValue` slot; sets `out_container` when the value is a
  /// nested Object/Array so post-assignment mutations stay synchronized on capture.
  NveValue jsi_value_to_slot(const facebook::jsi::Value& v, ArenaNode*& out_container) {
    out_container = nullptr;
    if (v.isUndefined() || v.isNull()) {
      return make_null_value();
    }
    if (v.isBool()) {
      return make_bool_value(v.getBool());
    }
    if (v.isNumber()) {
      const double d = v.getNumber();
      return std::isfinite(d) ? make_f64_value(d) : make_null_value();
    }
    const auto* pv = static_cast<const ArenaPointerValue*>(getPointerValue(v));
    if (!pv || !pv->node) {
      return make_null_value();
    }
    ArenaNode* n = pv->node;
    switch (n->kind) {
      case NodeKind::String:
      case NodeKind::Symbol:
        return make_str_value(n->str_val);
      case NodeKind::BigIntSigned:
        return make_i64_value(n->i64_val);
      case NodeKind::BigIntUnsigned:
        return make_i64_value(static_cast<int64_t>(n->u64_val));
      case NodeKind::ArrayBuffer: {
        if (n->side_slot < buffers_.size() && buffers_[n->side_slot]) {
          const auto& buf = buffers_[n->side_slot];
          return make_bytes_value(NveBufferSlice{buf->data(), buf->size()});
        }
        return make_null_value();
      }
      case NodeKind::Array:
        out_container = n;
        return make_array_value(n->array_elems, n->len);
      case NodeKind::Object:
      case NodeKind::HostObject:
        out_container = n;
        return make_map_value(n->map_entries, n->len);
      case NodeKind::Undefined:
      case NodeKind::Null:
      case NodeKind::HostFunction:
      default:
        return make_null_value();
      case NodeKind::Bool:
        return make_bool_value(n->bool_val);
      case NodeKind::Number:
        return std::isfinite(n->num_val) ? make_f64_value(n->num_val) : make_null_value();
    }
  }

  /// Synchronize any nested container `{ptr, len}` headers (O(1) no-op for flat objects/arrays)
  /// with a strict depth guard against cyclic JS objects.
  NveValue materialize_node(ArenaNode* n, uint32_t depth) {
    if (!n || depth > NVE_BIN_VALUE_MAX_DEPTH) {
      return make_null_value();
    }
    switch (n->kind) {
      case NodeKind::String:
      case NodeKind::Symbol:
        return make_str_value(n->str_val);
      case NodeKind::BigIntSigned:
        return make_i64_value(n->i64_val);
      case NodeKind::BigIntUnsigned:
        return make_i64_value(static_cast<int64_t>(n->u64_val));
      case NodeKind::ArrayBuffer: {
        if (n->side_slot < buffers_.size() && buffers_[n->side_slot]) {
          const auto& buf = buffers_[n->side_slot];
          return make_bytes_value(NveBufferSlice{buf->data(), buf->size()});
        }
        return make_null_value();
      }
      case NodeKind::Array: {
        if (n->has_nested_containers && n->child_nodes != nullptr) {
          for (uint32_t i = 0; i < n->len; ++i) {
            if (n->child_nodes[i] != nullptr) {
              n->array_elems[i] = materialize_node(n->child_nodes[i], depth + 1u);
            }
          }
        }
        return make_array_value(n->array_elems, n->len);
      }
      case NodeKind::Object:
      case NodeKind::HostObject: {
        if (n->has_nested_containers && n->child_nodes != nullptr) {
          for (uint32_t i = 0; i < n->len; ++i) {
            if (n->child_nodes[i] != nullptr) {
              n->map_entries[i].value = materialize_node(n->child_nodes[i], depth + 1u);
            }
          }
        }
        return make_map_value(n->map_entries, n->len);
      }
      case NodeKind::Bool:
        return make_bool_value(n->bool_val);
      case NodeKind::Number:
        return std::isfinite(n->num_val) ? make_f64_value(n->num_val) : make_null_value();
      case NodeKind::Undefined:
      case NodeKind::Null:
      case NodeKind::HostFunction:
      default:
        return make_null_value();
    }
  }

  NveValue materialize_jsi_value(const facebook::jsi::Value& v, uint32_t depth) {
    if (v.isUndefined() || v.isNull()) return make_null_value();
    if (v.isBool()) return make_bool_value(v.getBool());
    if (v.isNumber()) {
      const double d = v.getNumber();
      return std::isfinite(d) ? make_f64_value(d) : make_null_value();
    }
    const auto* pv = static_cast<const ArenaPointerValue*>(getPointerValue(v));
    return pv ? materialize_node(pv->node, depth) : make_null_value();
  }

  facebook::jsi::Value lookup_prop(const ArenaNode* obj, std::string_view key) {
    const int32_t idx = find_prop_index(obj, key);
    if (idx < 0) return facebook::jsi::Value::undefined();
    const uint32_t u = static_cast<uint32_t>(idx);
    if (obj->child_nodes != nullptr && obj->child_nodes[u] != nullptr) {
      return node_to_jsi_value(obj->child_nodes[u]);
    }
    return nve_value_to_jsi_value(obj->map_entries[u].value);
  }

  facebook::jsi::Value node_to_jsi_value(ArenaNode* n) {
    if (!n) return facebook::jsi::Value::undefined();
    switch (n->kind) {
      case NodeKind::Undefined:
        return facebook::jsi::Value::undefined();
      case NodeKind::Null:
        return facebook::jsi::Value::null();
      case NodeKind::Bool:
        return facebook::jsi::Value(n->bool_val);
      case NodeKind::Number:
        return facebook::jsi::Value(n->num_val);
      case NodeKind::BigIntSigned:
      case NodeKind::BigIntUnsigned:
        return facebook::jsi::Value(
            make<facebook::jsi::BigInt>(arena_.emplace<ArenaPointerValue>(n)));
      case NodeKind::String:
        return facebook::jsi::Value(
            make<facebook::jsi::String>(arena_.emplace<ArenaPointerValue>(n)));
      case NodeKind::Symbol:
        return facebook::jsi::Value(
            make<facebook::jsi::Symbol>(arena_.emplace<ArenaPointerValue>(n)));
      case NodeKind::Object:
      case NodeKind::Array:
      case NodeKind::ArrayBuffer:
      case NodeKind::HostObject:
      case NodeKind::HostFunction:
        return facebook::jsi::Value(
            make<facebook::jsi::Object>(arena_.emplace<ArenaPointerValue>(n)));
    }
    return facebook::jsi::Value::undefined();
  }

  facebook::jsi::Value nve_value_to_jsi_value(const NveValue& val) {
    switch (val.tag) {
      case NVE_VAL_NULL:
        return facebook::jsi::Value::null();
      case NVE_VAL_BOOL:
        return facebook::jsi::Value(val.data.bool_val != 0u);
      case NVE_VAL_I64:
        return facebook::jsi::Value(createBigIntFromInt64(val.data.i64_val));
      case NVE_VAL_F64:
        return facebook::jsi::Value(val.data.f64_val);
      case NVE_VAL_STR:
        return facebook::jsi::Value(
            createStringFromUtf8(val.data.str_val.ptr, val.data.str_val.len));
      default:
        return facebook::jsi::Value::null();
    }
  }

  BumpArena arena_;
  BumpArena global_arena_;
  ArenaNode* global_node_{nullptr};
  ArenaPointerValue* global_pv_{nullptr};
  std::vector<NveDirectEvent> direct_events_;
  EventRingWriter fallback_ring_;

  // Rare side-tables for non-trivial JSI shared_ptr objects (untouched on hot path).
  std::vector<std::shared_ptr<facebook::jsi::MutableBuffer>> buffers_;
  std::vector<std::shared_ptr<facebook::jsi::HostObject>> host_objects_;
  std::vector<facebook::jsi::HostFunctionType> host_functions_;
  std::vector<std::shared_ptr<facebook::jsi::NativeState>> native_states_;
};

#endif // defined(NVE_WITH_RN_JSI) || defined(NVE_WITH_RN_FABRIC)

} // namespace nve
