#pragma once

#include "event_ring_writer.hpp"
#include "mock_jsi_arena.hpp"

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <functional>
#include <limits>
#include <memory>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

#if defined(NVE_WITH_RN_JSI) || defined(NVE_WITH_RN_FABRIC)

namespace nve {

/**
 * Zero-Hermes, zero-JS-heap, zero-allocation `facebook::jsi::Runtime` recorder subclass.
 *
 * All JSI objects, arrays, strings, `PropNameID`s, `NveValue`s, and `NveMapEntry` arrays are
 * allocated directly inside a frame-reusable `nve::BumpArena`:
 * - Primary in-process hot path: `record_direct_event(...)` + `dispatch_direct(...)` passes an
 *   `NveEventBatch` pointer directly across `NveSurfaceApi::dispatch_events_direct` with **0 bytes
 *   serialized, 0 heap allocations after warmup, and 0 JSON**.
 * - Non-shared boundary path: `record_custom_event(EventRingWriter&, ...)` encodes captured
 *   `NveValue` trees directly into `EventRingWriter`'s preallocated buffer as compact binary
 *   `NveBinValue` (readable with zero-copy `nve::BinValueView` / Rust `BinValueView`).
 */
class MockJsiRecorderRuntime final : public facebook::jsi::Runtime {
 public:
  using DirectEventSink = std::function<uint32_t(NveEventBatch)>;
  using NodeKind = detail::JsiNodeKind;
  using ArenaNode = detail::JsiArenaNode;
  using ArenaPointerValue = detail::JsiArenaPointerValue;

  static constexpr uint32_t kNoSideSlot = detail::kJsiNoSideSlot;
  static constexpr uint32_t kInitialObjectCap = detail::kJsiInitialObjectCap;

  explicit MockJsiRecorderRuntime(size_t initial_arena_bytes = BumpArena::kDefaultInitialSlabBytes)
      : arena_(initial_arena_bytes), global_arena_(4096u) {
    global_node_ = detail::alloc_jsi_object_node(global_arena_, /*is_global=*/true);
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
  /// (zero serialization, zero copy). Returns 0 when `dispatch_events_direct` is absent.
  uint32_t dispatch_direct(const NveSurfaceApi* surface, NveAppHandle app) const {
    if (surface == nullptr || app == nullptr || direct_events_.empty() ||
        surface->dispatch_events_direct == nullptr) {
      return 0u;
    }
    return surface->dispatch_events_direct(app, direct_event_batch());
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
    return make<facebook::jsi::PropNameID>(detail::alloc_jsi_string_pv(arena_, str, length));
  }

  facebook::jsi::PropNameID createPropNameIDFromUtf8(const uint8_t* utf8, size_t length) override {
    return make<facebook::jsi::PropNameID>(
        detail::alloc_jsi_string_pv(arena_, reinterpret_cast<const char*>(utf8), length));
  }

  facebook::jsi::PropNameID createPropNameIDFromString(const facebook::jsi::String& str) override {
    ArenaNode* n = extract_node(str);
    return make<facebook::jsi::PropNameID>(arena_.emplace<ArenaPointerValue>(n));
  }

  facebook::jsi::PropNameID createPropNameIDFromSymbol(const facebook::jsi::Symbol& sym) override {
    ArenaNode* n = extract_node(sym);
    const std::string_view sv = str_slice_view(n ? n->str_val : NveStrSlice{nullptr, 0});
    return make<facebook::jsi::PropNameID>(detail::alloc_jsi_string_pv(arena_, sv.data(), sv.size()));
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
    return make<facebook::jsi::String>(detail::alloc_jsi_string_pv(arena_, str, length));
  }

  facebook::jsi::String createStringFromUtf8(const uint8_t* utf8, size_t length) override {
    return make<facebook::jsi::String>(
        detail::alloc_jsi_string_pv(arena_, reinterpret_cast<const char*>(utf8), length));
  }

  std::string utf8(const facebook::jsi::String& str) override {
    const ArenaNode* n = extract_node(str);
    return std::string(n ? str_slice_view(n->str_val) : std::string_view{});
  }

  facebook::jsi::Object createObject() override {
    ArenaNode* n = detail::alloc_jsi_object_node(arena_, /*is_global=*/false);
    return make<facebook::jsi::Object>(arena_.emplace<ArenaPointerValue>(n));
  }

  facebook::jsi::Object createObject(std::shared_ptr<facebook::jsi::HostObject> ho) override {
    ArenaNode* n = detail::alloc_jsi_object_node(arena_, /*is_global=*/false);
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
    return n && kn && detail::find_jsi_prop_index(n, str_slice_view(kn->str_val)) >= 0;
  }

  bool hasProperty(
      const facebook::jsi::Object& obj,
      const facebook::jsi::String& name) override {
    const ArenaNode* n = extract_node(obj);
    const ArenaNode* kn = extract_node(name);
    return n && kn && detail::find_jsi_prop_index(n, str_slice_view(kn->str_val)) >= 0;
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
      detail::grow_jsi_array_storage(n, arena_, i + 1u);
    }
    if (i >= n->len) {
      n->len = static_cast<uint32_t>(i + 1u);
    }
    ArenaNode* child_container = nullptr;
    n->array_elems[i] = jsi_value_to_slot(value, child_container);
    if (child_container != nullptr) {
      detail::ensure_jsi_child_nodes(n, arena_);
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
    return const_cast<PointerValue*>(pv);
  }

  static ArenaNode* extract_node(const facebook::jsi::Pointer& p) noexcept {
    const auto* pv = static_cast<const ArenaPointerValue*>(getPointerValue(p));
    return pv ? pv->node : nullptr;
  }

  void set_object_prop(ArenaNode* obj, NveStrSlice key, const facebook::jsi::Value& value);
  NveValue jsi_value_to_slot(const facebook::jsi::Value& v, ArenaNode*& out_container);
  NveValue materialize_jsi_value(const facebook::jsi::Value& v, uint32_t depth);
  facebook::jsi::Value lookup_prop(const ArenaNode* obj, std::string_view key);
  facebook::jsi::Value node_to_jsi_value(ArenaNode* n);
  facebook::jsi::Value nve_value_to_jsi_value(const NveValue& val);

  BumpArena arena_;
  BumpArena global_arena_;
  ArenaNode* global_node_{nullptr};
  ArenaPointerValue* global_pv_{nullptr};
  std::vector<NveDirectEvent> direct_events_;

  // Rare side-tables for non-trivial JSI shared_ptr objects (untouched on hot path).
  std::vector<std::shared_ptr<facebook::jsi::MutableBuffer>> buffers_;
  std::vector<std::shared_ptr<facebook::jsi::HostObject>> host_objects_;
  std::vector<facebook::jsi::HostFunctionType> host_functions_;
  std::vector<std::shared_ptr<facebook::jsi::NativeState>> native_states_;
};

} // namespace nve

#include "mock_jsi_runtime_helpers.hpp"

#endif // defined(NVE_WITH_RN_JSI) || defined(NVE_WITH_RN_FABRIC)
