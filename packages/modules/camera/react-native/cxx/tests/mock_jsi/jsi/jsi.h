#pragma once

// Test-only lightweight C++20 mock of React Native's `<jsi/jsi.h>` surface.
// Used ONLY by `nve_rn_camera_jsi_mock_test` when `RN_REACT_COMMON_DIR` is not installed locally,
// so the `NVE_WITH_RN_JSI=1` HostObject / NativeState bridge is compiled under `-Wall -Wextra -Werror`
// and executed in CTest on every machine. Never exported on library targets.

#include <cstddef>
#include <cstdint>
#include <functional>
#include <memory>
#include <stdexcept>
#include <string>
#include <unordered_map>
#include <utility>
#include <variant>
#include <vector>

namespace facebook::jsi {

class Runtime;
class Value;
class Object;
class Array;
class ArrayBuffer;
class Function;
class String;
class PropNameID;
class HostObject;

using HostFunctionType =
    std::function<Value(Runtime& rt, const Value& thisVal, const Value* args, size_t count)>;

class JSError : public std::runtime_error {
 public:
  JSError(Runtime&, const std::string& msg) : std::runtime_error(msg) {}
  JSError(Runtime&, const char* msg) : std::runtime_error(msg ? msg : "") {}
};

class NativeState {
 public:
  virtual ~NativeState() = default;
};

class MutableBuffer {
 public:
  virtual ~MutableBuffer() = default;
  virtual size_t size() const = 0;
  virtual uint8_t* data() = 0;
};

class PropNameID {
 public:
  explicit PropNameID(std::string name) : name_(std::move(name)) {}
  static PropNameID forAscii(Runtime&, const char* name) { return PropNameID(name ? name : ""); }
  static PropNameID forAscii(Runtime&, const std::string& name) { return PropNameID(name); }
  static PropNameID forUtf8(Runtime&, const std::string& name) { return PropNameID(name); }
  std::string utf8(Runtime&) const { return name_; }

 private:
  std::string name_;
};

class String {
 public:
  explicit String(std::string value) : value_(std::move(value)) {}
  static String createFromUtf8(Runtime&, const std::string& s) { return String(s); }
  static String createFromAscii(Runtime&, const char* s) { return String(s ? s : ""); }
  std::string utf8(Runtime&) const { return value_; }

 private:
  std::string value_;
};

struct ObjectRecord;

class Value {
 public:
  struct UndefinedTag {};
  struct NullTag {};

  Value() : storage_(UndefinedTag{}) {}
  Value(std::nullptr_t) : storage_(NullTag{}) {}
  explicit Value(bool b) : storage_(b) {}
  explicit Value(int n) : storage_(static_cast<double>(n)) {}
  explicit Value(double d) : storage_(d) {}
  Value(String s) : storage_(std::move(s)) {}
  Value(Object o);
  Value(Array a);
  Value(ArrayBuffer ab);
  Value(Function fn);

  static Value undefined() { return Value(); }
  static Value null() { return Value(nullptr); }

  bool isUndefined() const noexcept { return std::holds_alternative<UndefinedTag>(storage_); }
  bool isNull() const noexcept { return std::holds_alternative<NullTag>(storage_); }
  bool isBool() const noexcept { return std::holds_alternative<bool>(storage_); }
  bool isNumber() const noexcept { return std::holds_alternative<double>(storage_); }
  bool isString() const noexcept { return std::holds_alternative<String>(storage_); }
  bool isObject() const noexcept { return std::holds_alternative<std::shared_ptr<ObjectRecord>>(storage_); }

  bool getBool() const { return std::get<bool>(storage_); }
  bool asBool() const {
    if (!isBool()) throw std::runtime_error("JSI Value is not a bool");
    return getBool();
  }
  double getNumber() const { return std::get<double>(storage_); }
  double asNumber() const {
    if (!isNumber()) throw std::runtime_error("JSI Value is not a number");
    return getNumber();
  }
  String getString(Runtime&) const { return std::get<String>(storage_); }
  String asString(Runtime& rt) const {
    if (!isString()) throw std::runtime_error("JSI Value is not a string");
    return getString(rt);
  }
  Object getObject(Runtime&) const;
  Object asObject(Runtime& rt) const;

 private:
  friend class Object;
  std::variant<UndefinedTag, NullTag, bool, double, String, std::shared_ptr<ObjectRecord>> storage_;
};

class HostObject {
 public:
  virtual ~HostObject() = default;
  virtual Value get(Runtime&, const PropNameID&) { return Value::undefined(); }
  virtual void set(Runtime&, const PropNameID&, const Value&) {}
  virtual std::vector<PropNameID> getPropertyNames(Runtime&) { return {}; }
};

struct ObjectRecord {
  enum class Kind { Plain, Array, ArrayBuffer, Function, HostObject };
  Kind kind{Kind::Plain};
  std::unordered_map<std::string, Value> props;
  std::vector<Value> elems;
  std::shared_ptr<MutableBuffer> buffer;
  HostFunctionType fn;
  std::shared_ptr<HostObject> host_object;
  std::shared_ptr<NativeState> native_state;
};

class Object {
 public:
  explicit Object(Runtime&) : rec_(std::make_shared<ObjectRecord>()) {}
  explicit Object(std::shared_ptr<ObjectRecord> rec) : rec_(std::move(rec)) {}

  static Object createFromHostObject(Runtime&, std::shared_ptr<HostObject> ho) {
    auto r = std::make_shared<ObjectRecord>();
    r->kind = ObjectRecord::Kind::HostObject;
    r->host_object = std::move(ho);
    return Object(std::move(r));
  }

  bool hasProperty(Runtime& rt, const char* name) const {
    if (rec_->kind == ObjectRecord::Kind::HostObject && rec_->host_object) {
      return !rec_->host_object->get(rt, PropNameID::forAscii(rt, name)).isUndefined();
    }
    return rec_->props.find(name) != rec_->props.end();
  }

  Value getProperty(Runtime& rt, const char* name) const {
    if (rec_->kind == ObjectRecord::Kind::HostObject && rec_->host_object) {
      return rec_->host_object->get(rt, PropNameID::forAscii(rt, name));
    }
    auto it = rec_->props.find(name);
    return it == rec_->props.end() ? Value::undefined() : it->second;
  }

  template <typename T>
  void setProperty(Runtime&, const char* name, T&& value) const {
    rec_->props[name] = Value(std::forward<T>(value));
  }

  bool isArray(Runtime&) const noexcept { return rec_->kind == ObjectRecord::Kind::Array; }
  bool isArrayBuffer(Runtime&) const noexcept { return rec_->kind == ObjectRecord::Kind::ArrayBuffer; }
  bool isFunction(Runtime&) const noexcept { return rec_->kind == ObjectRecord::Kind::Function; }
  bool isHostObject(Runtime&) const noexcept { return rec_->kind == ObjectRecord::Kind::HostObject; }

  Array getArray(Runtime&) const;
  ArrayBuffer getArrayBuffer(Runtime&) const;
  Function getFunction(Runtime&) const;
  Function asFunction(Runtime& rt) const;
  std::shared_ptr<HostObject> getHostObject(Runtime&) const { return rec_->host_object; }

  bool hasNativeState(Runtime&) const noexcept { return rec_->native_state != nullptr; }
  std::shared_ptr<NativeState> getNativeState(Runtime&) const { return rec_->native_state; }
  void setNativeState(Runtime&, std::shared_ptr<NativeState> state) const { rec_->native_state = std::move(state); }

  std::shared_ptr<ObjectRecord> record() const noexcept { return rec_; }

 protected:
  std::shared_ptr<ObjectRecord> rec_;
};

class Array : public Object {
 public:
  Array(Runtime&, size_t length) : Object(std::make_shared<ObjectRecord>()) {
    rec_->kind = ObjectRecord::Kind::Array;
    rec_->elems.resize(length);
  }
  explicit Array(std::shared_ptr<ObjectRecord> rec) : Object(std::move(rec)) {}

  size_t size(Runtime&) const noexcept { return rec_->elems.size(); }
  Value getValueAtIndex(Runtime&, size_t i) const { return rec_->elems.at(i); }
  template <typename T>
  void setValueAtIndex(Runtime&, size_t i, T&& val) const {
    rec_->elems.at(i) = Value(std::forward<T>(val));
  }
};

class ArrayBuffer : public Object {
 public:
  ArrayBuffer(Runtime&, std::shared_ptr<MutableBuffer> buf) : Object(std::make_shared<ObjectRecord>()) {
    rec_->kind = ObjectRecord::Kind::ArrayBuffer;
    rec_->buffer = std::move(buf);
  }
  explicit ArrayBuffer(std::shared_ptr<ObjectRecord> rec) : Object(std::move(rec)) {}

  size_t size(Runtime&) const noexcept { return rec_->buffer ? rec_->buffer->size() : 0u; }
  uint8_t* data(Runtime&) const noexcept { return rec_->buffer ? rec_->buffer->data() : nullptr; }
};

class Function : public Object {
 public:
  explicit Function(std::shared_ptr<ObjectRecord> rec) : Object(std::move(rec)) {}

  static Function createFromHostFunction(
      Runtime&,
      const PropNameID&,
      unsigned int,
      HostFunctionType fn) {
    auto r = std::make_shared<ObjectRecord>();
    r->kind = ObjectRecord::Kind::Function;
    r->fn = std::move(fn);
    return Function(std::move(r));
  }

  Value call(Runtime& rt, const Value* args, size_t count) const {
    return call_impl(rt, args, count);
  }

  Value call(Runtime& rt) const { return call_impl(rt, nullptr, 0u); }

  template <typename... Args>
  Value call(Runtime& rt, Args&&... args) const {
    Value arr[] = {Value(std::forward<Args>(args))...};
    return call_impl(rt, arr, sizeof...(Args));
  }

 private:
  Value call_impl(Runtime& rt, const Value* args, size_t count) const {
    return rec_->fn(rt, Value::undefined(), args, count);
  }
};

class Runtime {
 public:
  Runtime() : global_(std::make_shared<ObjectRecord>()) {}
  virtual ~Runtime() = default;
  Object global() { return Object(global_); }

 private:
  std::shared_ptr<ObjectRecord> global_;
};

inline Value::Value(Object o) : storage_(o.record()) {}
inline Value::Value(Array a) : storage_(a.record()) {}
inline Value::Value(ArrayBuffer ab) : storage_(ab.record()) {}
inline Value::Value(Function fn) : storage_(fn.record()) {}

inline Object Value::getObject(Runtime&) const { return Object(std::get<std::shared_ptr<ObjectRecord>>(storage_)); }
inline Object Value::asObject(Runtime& rt) const {
  if (!isObject()) throw std::runtime_error("JSI Value is not an object");
  return getObject(rt);
}

inline Array Object::getArray(Runtime&) const { return Array(rec_); }
inline ArrayBuffer Object::getArrayBuffer(Runtime&) const { return ArrayBuffer(rec_); }
inline Function Object::getFunction(Runtime&) const { return Function(rec_); }
inline Function Object::asFunction(Runtime& rt) const {
  if (!isFunction(rt)) throw std::runtime_error("JSI Object is not a function");
  return Function(rec_);
}

} // namespace facebook::jsi
