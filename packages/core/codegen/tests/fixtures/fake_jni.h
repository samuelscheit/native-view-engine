// A functional fake of the JNI surface the generated glue and `nve/host/jni.hpp` use (direct
// buffers, UTF-16 strings, byte arrays, exceptions), recording what Java would see, for tests
// that compile and *run* the glue without a JVM.
#pragma once

#include <cstdarg>
#include <cstdint>
#include <cstring>
#include <string>
#include <vector>

#define JNIEXPORT
#define JNICALL
#define JNI_FALSE 0
#define JNI_TRUE 1

typedef uint8_t jboolean;
typedef int8_t jbyte;
typedef uint16_t jchar;
typedef int32_t jint;
typedef int64_t jlong;
typedef float jfloat;
typedef double jdouble;
typedef int32_t jsize;

struct _jobject {
  virtual ~_jobject() = default;
};
typedef _jobject* jobject;
struct _jclass : _jobject {};
typedef _jclass* jclass;
struct _jstring : _jobject {
  std::u16string units;
};
typedef _jstring* jstring;
struct _jbyteArray : _jobject {
  std::vector<jbyte> bytes;
};
typedef _jbyteArray* jbyteArray;
struct _jthrowable : _jobject {
  std::u16string message;
};
typedef _jthrowable* jthrowable;
struct _jmethodID {};
typedef _jmethodID* jmethodID;

/// A `java.nio.ByteBuffer`: direct with `address`, or a heap buffer (`address == nullptr`,
/// capacity `-1`, as JNI reports it).
struct FakeByteBuffer : _jobject {
  void* address = nullptr;
  jlong capacity = -1;
};

struct JNIEnv {
  std::string pending;          // UTF-8 view of the pending exception message ("" = none)
  std::u16string pending_utf16; // exact UTF-16 message passed to `IllegalStateException(String)`
  int thrown = 0;
  int live_local_refs = 0;
  bool checkjni_error = false;
  // OOM hooks: `NewString` returns `nullptr`, silently or with a pending `OutOfMemoryError` (as a
  // real JVM does).
  bool fail_new_string = false;
  bool new_string_raises_oom = false;
  _jclass exception_class;
  _jmethodID string_ctor;
  std::vector<_jobject*> locals;

  JNIEnv() = default;
  JNIEnv(const JNIEnv&) = delete;
  JNIEnv& operator=(const JNIEnv&) = delete;
  ~JNIEnv() {
    for (_jobject* o : locals) delete o;
  }

  jboolean ExceptionCheck() { return thrown > 0 ? JNI_TRUE : JNI_FALSE; }
  void ExceptionClear() {
    pending.clear();
    pending_utf16.clear();
    thrown = 0;
  }
  jclass FindClass(const char* name) {
    if (ExceptionCheck()) checkjni_error = true;
    if (std::strcmp(name, "java/lang/IllegalStateException") != 0) return nullptr;
    ++live_local_refs;
    return &exception_class;
  }
  jmethodID GetMethodID(jclass cls, const char* name, const char* sig) {
    if (ExceptionCheck()) checkjni_error = true;
    if (cls == &exception_class && std::strcmp(name, "<init>") == 0 &&
        std::strcmp(sig, "(Ljava/lang/String;)V") == 0) {
      return &string_ctor;
    }
    return nullptr;
  }
  jobject NewObject(jclass cls, jmethodID method, ...) {
    if (ExceptionCheck()) checkjni_error = true;
    if (cls != &exception_class || method != &string_ctor) return nullptr;
    va_list args;
    va_start(args, method);
    jstring msg = va_arg(args, jstring);
    va_end(args);
    auto* exc = new _jthrowable();
    if (msg != nullptr) exc->message = msg->units;
    locals.push_back(exc);
    ++live_local_refs;
    return exc;
  }
  jint Throw(jthrowable exc) {
    if (ExceptionCheck() || exc == nullptr) {
      checkjni_error = true;
      return -1;
    }
    pending_utf16 = exc->message;
    pending.clear();
    for (char16_t u : exc->message) {
      if (u < 0x80) pending.push_back(static_cast<char>(u));
    }
    ++thrown;
    return 0;
  }
  jint ThrowNew(jclass, const char* message) {
    if (ExceptionCheck()) checkjni_error = true;
    // ART CheckJNI rejects 4-byte UTF-8 sequences in Modified-UTF-8 `ThrowNew`.
    for (const unsigned char* p = reinterpret_cast<const unsigned char*>(message); *p != 0; ++p) {
      if (*p >= 0xF0u) checkjni_error = true;
    }
    pending = message;
    pending_utf16.assign(pending.begin(), pending.end());
    ++thrown;
    return 0;
  }
  void DeleteLocalRef(jobject ref) {
    if (ref != nullptr) --live_local_refs;
  }
  void* GetDirectBufferAddress(jobject buffer) { return static_cast<FakeByteBuffer*>(buffer)->address; }
  jlong GetDirectBufferCapacity(jobject buffer) { return static_cast<FakeByteBuffer*>(buffer)->capacity; }
  jbyteArray NewByteArray(jsize len) {
    auto* array = new _jbyteArray();
    array->bytes.resize(static_cast<size_t>(len));
    locals.push_back(array);
    ++live_local_refs;
    return array;
  }
  void SetByteArrayRegion(jbyteArray array, jsize start, jsize len, const jbyte* src) {
    std::memcpy(array->bytes.data() + start, src, static_cast<size_t>(len));
  }
  jsize GetStringLength(jstring s) { return static_cast<jsize>(s->units.size()); }
  void GetStringRegion(jstring s, jsize start, jsize len, jchar* dst) {
    if (len <= 0) {
      checkjni_error = true;
      return;
    }
    std::memcpy(dst, s->units.data() + start, static_cast<size_t>(len) * sizeof(jchar));
  }
  jstring NewString(const jchar* units, jsize len) {
    if (ExceptionCheck()) checkjni_error = true;
    if (fail_new_string) {
      if (new_string_raises_oom) {
        pending = "java.lang.OutOfMemoryError";
        pending_utf16 = u"java.lang.OutOfMemoryError";
        ++thrown;
      }
      return nullptr;
    }
    auto* s = new _jstring();
    s->units.assign(reinterpret_cast<const char16_t*>(units), static_cast<size_t>(len));
    locals.push_back(s);
    ++live_local_refs;
    return s;
  }
};
