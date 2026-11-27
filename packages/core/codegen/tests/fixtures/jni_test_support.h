// Shared support of the fake-`JNIEnv` JNI harnesses: `CHECK`, direct buffers, exception
// expectations, and a `set_props_direct` recorder over the live Rust vtable. Include after
// `jni.h` (`fake_jni.h`) and the generated `*HybridSpec.generated.hpp` (one translation unit).
#pragma once

#include <cstdio>
#include <cstring>
#include <functional>
#include <span>
#include <string>
#include <string_view>
#include <vector>

namespace {

int g_failures = 0;

#define CHECK(cond)                                                          \
  do {                                                                       \
    if (!(cond)) {                                                           \
      std::fprintf(stderr, "%s:%d: CHECK(%s) failed\n", __FILE__, __LINE__, #cond); \
      ++g_failures;                                                          \
    }                                                                        \
  } while (0)

/// A direct `ByteBuffer` over its own copy of `bytes`.
struct DirectBuffer {
  std::vector<uint8_t> bytes;
  FakeByteBuffer buffer;
  explicit DirectBuffer(std::span<const uint8_t> b) : bytes(b.begin(), b.end()) {
    buffer.address = bytes.data();
    buffer.capacity = static_cast<jlong>(bytes.size());
  }
  DirectBuffer(const DirectBuffer&) = delete;
  DirectBuffer& operator=(const DirectBuffer&) = delete;
  jobject get() { return &buffer; }
};

// ---- `set_props_direct` recorder ------------------------------------------------------------
// A copy of the live Rust vtable whose `set_props_direct` first hands the entries the JNI glue
// built to `g_inspect` (while every borrowed pointer is still alive), then forwards to Rust.

int32_t (*g_rust_set_props)(NveAppHandle, const NveMapEntry*, uint32_t) = nullptr;
std::function<void(std::span<const NveMapEntry>)> g_inspect;

int32_t recording_set_props(NveAppHandle handle, const NveMapEntry* entries, uint32_t count) {
  if (g_inspect) g_inspect(std::span<const NveMapEntry>(entries, count));
  return g_rust_set_props(handle, entries, count);
}

std::string_view key_of(const NveMapEntry& e) {
  return {reinterpret_cast<const char*>(e.key.ptr), e.key.len};
}

std::string_view str_of(const NveValue& v) {
  return {reinterpret_cast<const char*>(v.data.str_val.ptr), v.data.str_val.len};
}

/// The entry named `key` (exactly one), or `nullptr` (a failure).
const NveValue* find_prop(std::span<const NveMapEntry> entries, std::string_view key) {
  const NveValue* found = nullptr;
  for (const NveMapEntry& e : entries) {
    if (key_of(e) == key) {
      CHECK(found == nullptr);
      found = &e.value;
    }
  }
  CHECK(found != nullptr);
  return found;
}

/// `NVE_VAL_BYTES` borrowing `buffer` in place (zero-copy) and holding exactly `expected`.
bool is_borrowed_bytes(const NveValue* v, std::span<const uint8_t> buffer, std::span<const uint8_t> expected) {
  if (v == nullptr || v->tag != NVE_VAL_BYTES) return false;
  const uint8_t* p = v->data.bytes_val.ptr;
  const size_t n = v->data.bytes_val.len;
  const bool inside = n == 0 || (p >= buffer.data() && p + n <= buffer.data() + buffer.size());
  return inside && n == expected.size() && (n == 0 || std::memcmp(p, expected.data(), n) == 0);
}

std::span<const uint8_t> bytes_of(jbyteArray array) {
  return {reinterpret_cast<const uint8_t*>(array->bytes.data()), array->bytes.size()};
}

/// Exactly one Java exception whose message contains `what` was raised; clears it.
void expect_thrown(JNIEnv& env, const char* what) {
  if (env.thrown != 1 || env.pending.find(what) == std::string::npos || env.checkjni_error) {
    std::fprintf(
        stderr, "expected one CheckJNI-clean exception with `%s`, got %d (checkjni_err=%d): `%s`\n", what, env.thrown,
        static_cast<int>(env.checkjni_error), env.pending.c_str());
    ++g_failures;
  }
  env.ExceptionClear();
}

/// The bytes `encode` writes.
std::vector<uint8_t> encoded(const std::function<void(nve::host::BinWriter&)>& encode) {
  nve::host::BinWriter w;
  encode(w);
  const auto bytes = nve::hybrid::checked_bytes(w);
  return {bytes.begin(), bytes.end()};
}

/// A static copy of the live Rust `VTable` whose `set_props_direct` is recorded (`g_inspect`).
template <class VTable>
const VTable* recording_vtable(uint32_t interface_id) {
  const auto* rust = static_cast<const VTable*>(nve_get_interface(interface_id));
  if (rust == nullptr) return nullptr;
  static VTable recording = *rust;
  g_rust_set_props = rust->set_props_direct;
  recording.set_props_direct = &recording_set_props;
  return &recording;
}

}  // namespace
