#pragma once

#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <memory>
#include <string>
#include <string_view>
#include <utility>
#include <variant>
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
 * are captured via `MockJsiRecorderRuntime` (below) directly into this EventRing —
 * requiring zero Hermes runtime and zero JS heap.
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
    ensure_header();
    write_u32(surface_id);
    write_u32(node_id);
    write_u64(ts_us);
    buf_.push_back(kind);
    buf_.push_back(0); /* EventPayload::Empty */
    ++count_;
  }

  void push_press(uint32_t surface_id, uint32_t node_id, uint64_t ts_us, float x, float y) {
    ensure_header();
    write_u32(surface_id);
    write_u32(node_id);
    write_u64(ts_us);
    buf_.push_back(1); /* NativeEventKind::Press */
    buf_.push_back(1); /* EventPayload::Point */
    write_f32(x);
    write_f32(y);
    ++count_;
  }

  void push_long_press(uint32_t surface_id, uint32_t node_id, uint64_t ts_us, float x, float y) {
    ensure_header();
    write_u32(surface_id);
    write_u32(node_id);
    write_u64(ts_us);
    buf_.push_back(2); /* NativeEventKind::LongPress */
    buf_.push_back(1); /* EventPayload::Point */
    write_f32(x);
    write_f32(y);
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
    ensure_header();
    write_u32(surface_id);
    write_u32(node_id);
    write_u64(ts_us);
    buf_.push_back(3); /* NativeEventKind::Scroll */
    buf_.push_back(2); /* EventPayload::Scroll */
    write_f32(x);
    write_f32(y);
    write_f32(content_width);
    write_f32(content_height);
    ++count_;
  }

  void push_text_change(
      uint32_t surface_id,
      uint32_t node_id,
      uint64_t ts_us,
      std::string_view text) {
    ensure_header();
    write_u32(surface_id);
    write_u32(node_id);
    write_u64(ts_us);
    buf_.push_back(4); /* NativeEventKind::TextChange */
    buf_.push_back(3); /* EventPayload::Text */
    write_u32(static_cast<uint32_t>(text.size()));
    buf_.insert(
        buf_.end(),
        reinterpret_cast<const uint8_t*>(text.data()),
        reinterpret_cast<const uint8_t*>(text.data()) + text.size());
    ++count_;
  }

  void push_layout(
      uint32_t surface_id,
      uint32_t node_id,
      uint64_t ts_us,
      float x,
      float y,
      float width,
      float height) {
    ensure_header();
    write_u32(surface_id);
    write_u32(node_id);
    write_u64(ts_us);
    buf_.push_back(7); /* NativeEventKind::Layout */
    buf_.push_back(4); /* EventPayload::Layout */
    write_f32(x);
    write_f32(y);
    write_f32(width);
    write_f32(height);
    ++count_;
  }

  void push_custom(
      uint32_t surface_id,
      uint32_t node_id,
      uint64_t ts_us,
      std::string_view name,
      const uint8_t* data,
      size_t len) {
    ensure_header();
    write_u32(surface_id);
    write_u32(node_id);
    write_u64(ts_us);
    buf_.push_back(8); /* NativeEventKind::Custom */
    buf_.push_back(5); /* EventPayload::Custom */
    const uint16_t nlen = static_cast<uint16_t>(name.size());
    write_u16(nlen);
    buf_.insert(
        buf_.end(),
        reinterpret_cast<const uint8_t*>(name.data()),
        reinterpret_cast<const uint8_t*>(name.data()) + nlen);
    write_u32(static_cast<uint32_t>(len));
    if (len > 0 && data != nullptr) {
      buf_.insert(buf_.end(), data, data + len);
    }
    ++count_;
  }

  const std::vector<uint8_t>& finalize() {
    ensure_header();
    std::memcpy(buf_.data(), &count_, sizeof(uint32_t));
    return buf_;
  }

 private:
  void ensure_header() {
    if (buf_.size() < 4) {
      buf_.resize(4, 0);
    }
  }
  void write_u16(uint16_t v) {
    const auto* p = reinterpret_cast<const uint8_t*>(&v);
    buf_.insert(buf_.end(), p, p + 2);
  }
  void write_u32(uint32_t v) {
    const auto* p = reinterpret_cast<const uint8_t*>(&v);
    buf_.insert(buf_.end(), p, p + 4);
  }
  void write_u64(uint64_t v) {
    const auto* p = reinterpret_cast<const uint8_t*>(&v);
    buf_.insert(buf_.end(), p, p + 8);
  }
  void write_f32(float v) {
    const auto* p = reinterpret_cast<const uint8_t*>(&v);
    buf_.insert(buf_.end(), p, p + 4);
  }

  uint32_t count_{0};
  std::vector<uint8_t> buf_;
};

#if defined(NVE_WITH_RN_JSI) || defined(NVE_WITH_RN_FABRIC)

namespace detail {

inline void append_json_escaped(std::string& out, std::string_view s) {
  out.push_back('"');
  for (unsigned char c : s) {
    switch (c) {
      case '"': out += "\\\""; break;
      case '\\': out += "\\\\"; break;
      case '\b': out += "\\b"; break;
      case '\f': out += "\\f"; break;
      case '\n': out += "\\n"; break;
      case '\r': out += "\\r"; break;
      case '\t': out += "\\t"; break;
      default:
        if (c < 0x20) {
          char hex[8];
          std::snprintf(hex, sizeof(hex), "\\u%04x", static_cast<unsigned>(c));
          out += hex;
        } else {
          out.push_back(static_cast<char>(c));
        }
        break;
    }
  }
  out.push_back('"');
}

} // namespace detail

/**
 * Zero-Hermes, zero-JS-heap `facebook::jsi::Runtime` recorder subclass.
 *
 * Any React Native Fabric `EventPayload::asJSIValue(jsi::Runtime&)` (including
 * `ValueFactoryEventPayload` emitted by arbitrary 3rd-party RN Fabric libraries)
 * constructs `jsi::Object` / `jsi::Array` / `jsi::String` / primitive `jsi::Value`
 * trees against this runtime. We record every write directly in C++ value structs
 * and serialize the resulting value to UTF-8 JSON for NVE's binary `EventRing`.
 */
class MockJsiRecorderRuntime final : public facebook::jsi::Runtime {
 public:
  struct Node;
  using NodePtr = std::shared_ptr<Node>;

  struct Node {
    enum class Kind {
      Undefined,
      Null,
      Bool,
      Number,
      BigIntSigned,
      BigIntUnsigned,
      String,
      Symbol,
      Object,
      Array,
      ArrayBuffer,
      HostObject,
      HostFunction,
    };

    Kind kind{Kind::Undefined};
    bool bool_val{false};
    double num_val{0.0};
    int64_t i64_val{0};
    uint64_t u64_val{0};
    std::string str_val;
    std::vector<std::pair<std::string, NodePtr>> props;
    std::vector<NodePtr> elements;
    std::shared_ptr<facebook::jsi::MutableBuffer> buffer;
    std::shared_ptr<facebook::jsi::HostObject> host_object;
    facebook::jsi::HostFunctionType host_function;
    std::shared_ptr<facebook::jsi::NativeState> native_state;

    static NodePtr make(Kind k) {
      auto n = std::make_shared<Node>();
      n->kind = k;
      return n;
    }

    void set_prop(const std::string& key, NodePtr val) {
      for (auto& kv : props) {
        if (kv.first == key) {
          kv.second = std::move(val);
          return;
        }
      }
      props.emplace_back(key, std::move(val));
    }

    NodePtr get_prop(const std::string& key) const {
      for (const auto& kv : props) {
        if (kv.first == key) {
          return kv.second;
        }
      }
      return make(Kind::Undefined);
    }

    bool has_prop(const std::string& key) const {
      for (const auto& kv : props) {
        if (kv.first == key) return true;
      }
      return false;
    }
  };

  struct RecordedPointerValue final : public PointerValue {
    NodePtr node;
    explicit RecordedPointerValue(NodePtr n) : node(std::move(n)) {}
    void invalidate() noexcept override { delete this; }
  };

  MockJsiRecorderRuntime() : global_node_(Node::make(Node::Kind::Object)) {}
  ~MockJsiRecorderRuntime() override = default;

  using facebook::jsi::Runtime::getProperty;
  using facebook::jsi::Runtime::hasProperty;
  using facebook::jsi::Runtime::setPropertyValue;

  /// Serialize any `jsi::Value` produced by this runtime into compact UTF-8 JSON.
  std::string serialize_to_json(const facebook::jsi::Value& value) {
    std::string out;
    serialize_node(from_jsi_value(value), out);
    return out;
  }

  /// Convenience helper: serialize a `jsi::Value` and push it directly into an `EventRingWriter`.
  void record_custom_event(
      EventRingWriter& ring,
      uint32_t surface_id,
      uint32_t node_id,
      uint64_t ts_us,
      std::string_view event_name,
      const facebook::jsi::Value& value) {
    const std::string json = serialize_to_json(value);
    ring.push_custom(
        surface_id,
        node_id,
        ts_us,
        event_name,
        reinterpret_cast<const uint8_t*>(json.data()),
        json.size());
  }

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
    return make<facebook::jsi::Object>(new RecordedPointerValue(global_node_));
  }

  std::string description() override { return "nve::MockJsiRecorderRuntime"; }
  bool isInspectable() override { return false; }

  PointerValue* cloneSymbol(const PointerValue* pv) override { return clone_pv(pv); }
  PointerValue* cloneBigInt(const PointerValue* pv) override { return clone_pv(pv); }
  PointerValue* cloneString(const PointerValue* pv) override { return clone_pv(pv); }
  PointerValue* cloneObject(const PointerValue* pv) override { return clone_pv(pv); }
  PointerValue* clonePropNameID(const PointerValue* pv) override { return clone_pv(pv); }

  facebook::jsi::PropNameID createPropNameIDFromAscii(const char* str, size_t length) override {
    auto n = Node::make(Node::Kind::String);
    n->str_val.assign(str, length);
    return make<facebook::jsi::PropNameID>(new RecordedPointerValue(std::move(n)));
  }

  facebook::jsi::PropNameID createPropNameIDFromUtf8(const uint8_t* utf8, size_t length) override {
    auto n = Node::make(Node::Kind::String);
    n->str_val.assign(reinterpret_cast<const char*>(utf8), length);
    return make<facebook::jsi::PropNameID>(new RecordedPointerValue(std::move(n)));
  }

  facebook::jsi::PropNameID createPropNameIDFromString(const facebook::jsi::String& str) override {
    auto n = Node::make(Node::Kind::String);
    n->str_val = extract_node(str)->str_val;
    return make<facebook::jsi::PropNameID>(new RecordedPointerValue(std::move(n)));
  }

  facebook::jsi::PropNameID createPropNameIDFromSymbol(const facebook::jsi::Symbol& sym) override {
    auto n = Node::make(Node::Kind::String);
    n->str_val = extract_node(sym)->str_val;
    return make<facebook::jsi::PropNameID>(new RecordedPointerValue(std::move(n)));
  }

  std::string utf8(const facebook::jsi::PropNameID& id) override {
    return extract_node(id)->str_val;
  }

  bool compare(const facebook::jsi::PropNameID& a, const facebook::jsi::PropNameID& b) override {
    return extract_node(a)->str_val == extract_node(b)->str_val;
  }

  std::string symbolToString(const facebook::jsi::Symbol& sym) override {
    return "Symbol(" + extract_node(sym)->str_val + ")";
  }

  facebook::jsi::BigInt createBigIntFromInt64(int64_t v) override {
    auto n = Node::make(Node::Kind::BigIntSigned);
    n->i64_val = v;
    n->u64_val = static_cast<uint64_t>(v);
    return make<facebook::jsi::BigInt>(new RecordedPointerValue(std::move(n)));
  }

  facebook::jsi::BigInt createBigIntFromUint64(uint64_t v) override {
    auto n = Node::make(Node::Kind::BigIntUnsigned);
    n->u64_val = v;
    n->i64_val = static_cast<int64_t>(v);
    return make<facebook::jsi::BigInt>(new RecordedPointerValue(std::move(n)));
  }

  bool bigintIsInt64(const facebook::jsi::BigInt& b) override {
    return extract_node(b)->kind == Node::Kind::BigIntSigned;
  }

  bool bigintIsUint64(const facebook::jsi::BigInt& b) override {
    return extract_node(b)->kind == Node::Kind::BigIntUnsigned;
  }

  uint64_t truncate(const facebook::jsi::BigInt& b) override {
    return extract_node(b)->u64_val;
  }

  facebook::jsi::String bigintToString(const facebook::jsi::BigInt& b, int) override {
    const auto n = extract_node(b);
    const std::string s = (n->kind == Node::Kind::BigIntSigned)
                              ? std::to_string(n->i64_val)
                              : std::to_string(n->u64_val);
    return createStringFromAscii(s.data(), s.size());
  }

  facebook::jsi::String createStringFromAscii(const char* str, size_t length) override {
    auto n = Node::make(Node::Kind::String);
    n->str_val.assign(str, length);
    return make<facebook::jsi::String>(new RecordedPointerValue(std::move(n)));
  }

  facebook::jsi::String createStringFromUtf8(const uint8_t* utf8, size_t length) override {
    auto n = Node::make(Node::Kind::String);
    n->str_val.assign(reinterpret_cast<const char*>(utf8), length);
    return make<facebook::jsi::String>(new RecordedPointerValue(std::move(n)));
  }

  std::string utf8(const facebook::jsi::String& str) override {
    return extract_node(str)->str_val;
  }

  facebook::jsi::Object createObject() override {
    return make<facebook::jsi::Object>(new RecordedPointerValue(Node::make(Node::Kind::Object)));
  }

  facebook::jsi::Object createObject(std::shared_ptr<facebook::jsi::HostObject> ho) override {
    auto n = Node::make(Node::Kind::HostObject);
    n->host_object = std::move(ho);
    return make<facebook::jsi::Object>(new RecordedPointerValue(std::move(n)));
  }

  std::shared_ptr<facebook::jsi::HostObject> getHostObject(
      const facebook::jsi::Object& obj) override {
    return extract_node(obj)->host_object;
  }

  facebook::jsi::HostFunctionType& getHostFunction(const facebook::jsi::Function& fn) override {
    return extract_node(fn)->host_function;
  }

  bool hasNativeState(const facebook::jsi::Object& obj) override {
    return extract_node(obj)->native_state != nullptr;
  }

  std::shared_ptr<facebook::jsi::NativeState> getNativeState(
      const facebook::jsi::Object& obj) override {
    return extract_node(obj)->native_state;
  }

  void setNativeState(
      const facebook::jsi::Object& obj,
      std::shared_ptr<facebook::jsi::NativeState> state) override {
    extract_node(obj)->native_state = std::move(state);
  }

  facebook::jsi::Value getProperty(
      const facebook::jsi::Object& obj,
      const facebook::jsi::PropNameID& name) override {
    return to_jsi_value(extract_node(obj)->get_prop(extract_node(name)->str_val));
  }

  facebook::jsi::Value getProperty(
      const facebook::jsi::Object& obj,
      const facebook::jsi::String& name) override {
    return to_jsi_value(extract_node(obj)->get_prop(extract_node(name)->str_val));
  }

  bool hasProperty(
      const facebook::jsi::Object& obj,
      const facebook::jsi::PropNameID& name) override {
    return extract_node(obj)->has_prop(extract_node(name)->str_val);
  }

  bool hasProperty(
      const facebook::jsi::Object& obj,
      const facebook::jsi::String& name) override {
    return extract_node(obj)->has_prop(extract_node(name)->str_val);
  }

  void setPropertyValue(
      const facebook::jsi::Object& obj,
      const facebook::jsi::PropNameID& name,
      const facebook::jsi::Value& value) override {
    extract_node(obj)->set_prop(extract_node(name)->str_val, from_jsi_value(value));
  }

  void setPropertyValue(
      const facebook::jsi::Object& obj,
      const facebook::jsi::String& name,
      const facebook::jsi::Value& value) override {
    extract_node(obj)->set_prop(extract_node(name)->str_val, from_jsi_value(value));
  }

  bool isArray(const facebook::jsi::Object& obj) const override {
    return extract_node(obj)->kind == Node::Kind::Array;
  }

  bool isArrayBuffer(const facebook::jsi::Object& obj) const override {
    return extract_node(obj)->kind == Node::Kind::ArrayBuffer;
  }

  bool isFunction(const facebook::jsi::Object& obj) const override {
    return extract_node(obj)->kind == Node::Kind::HostFunction;
  }

  bool isHostObject(const facebook::jsi::Object& obj) const override {
    return extract_node(obj)->kind == Node::Kind::HostObject;
  }

  bool isHostFunction(const facebook::jsi::Function& fn) const override {
    return extract_node(fn)->kind == Node::Kind::HostFunction;
  }

  facebook::jsi::Array getPropertyNames(const facebook::jsi::Object& obj) override {
    const auto n = extract_node(obj);
    auto arr = createArray(n->props.size());
    for (size_t i = 0; i < n->props.size(); ++i) {
      setValueAtIndexImpl(
          arr,
          i,
          facebook::jsi::Value(
              createStringFromUtf8(
                  reinterpret_cast<const uint8_t*>(n->props[i].first.data()),
                  n->props[i].first.size())));
    }
    return arr;
  }

  facebook::jsi::WeakObject createWeakObject(const facebook::jsi::Object& obj) override {
    return make<facebook::jsi::WeakObject>(new RecordedPointerValue(extract_node(obj)));
  }

  facebook::jsi::Value lockWeakObject(const facebook::jsi::WeakObject& obj) override {
    return to_jsi_value(extract_node(obj));
  }

  facebook::jsi::Array createArray(size_t length) override {
    auto n = Node::make(Node::Kind::Array);
    n->elements.resize(length);
    for (auto& el : n->elements) {
      el = Node::make(Node::Kind::Undefined);
    }
    return make<facebook::jsi::Array>(new RecordedPointerValue(std::move(n)));
  }

  facebook::jsi::ArrayBuffer createArrayBuffer(
      std::shared_ptr<facebook::jsi::MutableBuffer> buffer) override {
    auto n = Node::make(Node::Kind::ArrayBuffer);
    n->buffer = std::move(buffer);
    return make<facebook::jsi::ArrayBuffer>(new RecordedPointerValue(std::move(n)));
  }

  size_t size(const facebook::jsi::Array& arr) override {
    return extract_node(arr)->elements.size();
  }

  size_t size(const facebook::jsi::ArrayBuffer& buf) override {
    const auto n = extract_node(buf);
    return n->buffer ? n->buffer->size() : 0;
  }

  uint8_t* data(const facebook::jsi::ArrayBuffer& buf) override {
    const auto n = extract_node(buf);
    return n->buffer ? n->buffer->data() : nullptr;
  }

  facebook::jsi::Value getValueAtIndex(const facebook::jsi::Array& arr, size_t i) override {
    const auto n = extract_node(arr);
    if (i >= n->elements.size()) {
      return facebook::jsi::Value::undefined();
    }
    return to_jsi_value(n->elements[i]);
  }

  void setValueAtIndexImpl(
      const facebook::jsi::Array& arr,
      size_t i,
      const facebook::jsi::Value& value) override {
    const auto n = extract_node(arr);
    if (i >= n->elements.size()) {
      n->elements.resize(i + 1, Node::make(Node::Kind::Undefined));
    }
    n->elements[i] = from_jsi_value(value);
  }

  facebook::jsi::Function createFunctionFromHostFunction(
      const facebook::jsi::PropNameID&,
      unsigned int,
      facebook::jsi::HostFunctionType func) override {
    auto n = Node::make(Node::Kind::HostFunction);
    n->host_function = std::move(func);
    return make<facebook::jsi::Function>(new RecordedPointerValue(std::move(n)));
  }

  facebook::jsi::Value call(
      const facebook::jsi::Function& fn,
      const facebook::jsi::Value& jsThis,
      const facebook::jsi::Value* args,
      size_t count) override {
    const auto n = extract_node(fn);
    if (n->host_function) {
      return n->host_function(*this, jsThis, args, count);
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
    return extract_node(a)->u64_val == extract_node(b)->u64_val;
  }

  bool strictEquals(const facebook::jsi::String& a, const facebook::jsi::String& b) const override {
    return extract_node(a)->str_val == extract_node(b)->str_val;
  }

  bool strictEquals(const facebook::jsi::Object& a, const facebook::jsi::Object& b) const override {
    return extract_node(a) == extract_node(b);
  }

  bool instanceOf(const facebook::jsi::Object&, const facebook::jsi::Function&) override {
    return false;
  }

  void setExternalMemoryPressure(const facebook::jsi::Object&, size_t) override {}

 private:
  static PointerValue* clone_pv(const PointerValue* pv) {
    if (!pv) return nullptr;
    const auto* rpv = static_cast<const RecordedPointerValue*>(pv);
    return new RecordedPointerValue(rpv->node);
  }

  static NodePtr extract_node(const facebook::jsi::Pointer& p) {
    const auto* pv = static_cast<const RecordedPointerValue*>(getPointerValue(p));
    return pv ? pv->node : Node::make(Node::Kind::Undefined);
  }

  NodePtr from_jsi_value(const facebook::jsi::Value& v) {
    if (v.isUndefined()) return Node::make(Node::Kind::Undefined);
    if (v.isNull()) return Node::make(Node::Kind::Null);
    if (v.isBool()) {
      auto n = Node::make(Node::Kind::Bool);
      n->bool_val = v.getBool();
      return n;
    }
    if (v.isNumber()) {
      auto n = Node::make(Node::Kind::Number);
      n->num_val = v.getNumber();
      return n;
    }
    const auto* pv = static_cast<const RecordedPointerValue*>(getPointerValue(v));
    return pv ? pv->node : Node::make(Node::Kind::Undefined);
  }

  facebook::jsi::Value to_jsi_value(const NodePtr& n) {
    if (!n) return facebook::jsi::Value::undefined();
    switch (n->kind) {
      case Node::Kind::Undefined:
        return facebook::jsi::Value::undefined();
      case Node::Kind::Null:
        return facebook::jsi::Value::null();
      case Node::Kind::Bool:
        return facebook::jsi::Value(n->bool_val);
      case Node::Kind::Number:
        return facebook::jsi::Value(n->num_val);
      case Node::Kind::BigIntSigned:
      case Node::Kind::BigIntUnsigned:
        return facebook::jsi::Value(make<facebook::jsi::BigInt>(new RecordedPointerValue(n)));
      case Node::Kind::String:
        return facebook::jsi::Value(make<facebook::jsi::String>(new RecordedPointerValue(n)));
      case Node::Kind::Symbol:
        return facebook::jsi::Value(make<facebook::jsi::Symbol>(new RecordedPointerValue(n)));
      case Node::Kind::Object:
      case Node::Kind::Array:
      case Node::Kind::ArrayBuffer:
      case Node::Kind::HostObject:
      case Node::Kind::HostFunction:
        return facebook::jsi::Value(make<facebook::jsi::Object>(new RecordedPointerValue(n)));
    }
    return facebook::jsi::Value::undefined();
  }

  static void serialize_node(const NodePtr& n, std::string& out) {
    if (!n) {
      out += "null";
      return;
    }
    switch (n->kind) {
      case Node::Kind::Undefined:
      case Node::Kind::Null:
        out += "null";
        break;
      case Node::Kind::Bool:
        out += (n->bool_val ? "true" : "false");
        break;
      case Node::Kind::Number: {
        if (!std::isfinite(n->num_val)) {
          out += "null";
        } else {
          char buf[64];
          std::snprintf(buf, sizeof(buf), "%.17g", n->num_val);
          out += buf;
        }
        break;
      }
      case Node::Kind::BigIntSigned:
        out += std::to_string(n->i64_val);
        break;
      case Node::Kind::BigIntUnsigned:
        out += std::to_string(n->u64_val);
        break;
      case Node::Kind::String:
        detail::append_json_escaped(out, n->str_val);
        break;
      case Node::Kind::Symbol:
        detail::append_json_escaped(out, "Symbol(" + n->str_val + ")");
        break;
      case Node::Kind::Array: {
        out.push_back('[');
        for (size_t i = 0; i < n->elements.size(); ++i) {
          if (i > 0) out.push_back(',');
          serialize_node(n->elements[i], out);
        }
        out.push_back(']');
        break;
      }
      case Node::Kind::Object:
      case Node::Kind::HostObject: {
        out.push_back('{');
        bool first = true;
        for (const auto& [k, v] : n->props) {
          if (v && v->kind == Node::Kind::Undefined) continue;
          if (!first) out.push_back(',');
          first = false;
          detail::append_json_escaped(out, k);
          out.push_back(':');
          serialize_node(v, out);
        }
        out.push_back('}');
        break;
      }
      case Node::Kind::ArrayBuffer:
      case Node::Kind::HostFunction:
        out += "null";
        break;
    }
  }

  NodePtr global_node_;
};

#endif // defined(NVE_WITH_RN_JSI) || defined(NVE_WITH_RN_FABRIC)

} // namespace nve
