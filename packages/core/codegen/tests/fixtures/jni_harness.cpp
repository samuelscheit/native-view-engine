// Drives the generated JNI glue (`CryptoHybridJni.generated.cpp`) like the Kotlin client does,
// against the live Rust provider: direct `ByteBuffer`s, UTF-16 Java strings, and the Java
// exceptions every malformed input must raise. Built with `fake_jni.h` as `<jni.h>`.
#include <cstdio>
#include <span>
#include <string>
#include <vector>

#include "jni.h"
#include "CryptoHybridSpec.generated.hpp"

extern "C" {
jlong Java_dev_nve_hybrid_CryptoHybridClient_nativeCreate(JNIEnv*, jclass);
void Java_dev_nve_hybrid_CryptoHybridClient_nativeDestroy(JNIEnv*, jclass, jlong);
jstring Java_dev_nve_hybrid_CryptoHybridClient_nativeVersion(JNIEnv*, jclass, jlong);
jbyteArray Java_dev_nve_hybrid_CryptoHybridClient_nativeFindNonce(JNIEnv*, jclass, jlong, jstring, jobject);
jbyteArray Java_dev_nve_hybrid_CryptoHybridClient_nativeInspectDynamic(JNIEnv*, jclass, jlong, jobject);
jbyteArray Java_dev_nve_hybrid_CryptoHybridClient_nativeTranspose(JNIEnv*, jclass, jlong, jobject);
jlong Java_dev_nve_hybrid_VideoSurfaceHybridClient_nativeCreate(JNIEnv*, jclass);
void Java_dev_nve_hybrid_VideoSurfaceHybridClient_nativeDestroy(JNIEnv*, jclass, jlong);
void Java_dev_nve_hybrid_VideoSurfaceHybridClient_nativeSetProps(JNIEnv*, jclass, jlong, jobject);
void Java_dev_nve_hybrid_VideoSurfaceHybridClient_nativeSeek(JNIEnv*, jclass, jlong, jlong);
jbyteArray Java_dev_nve_hybrid_VideoSurfaceHybridClient_nativeCaptureFrame(JNIEnv*, jclass, jlong, jobject);
jbyteArray Java_dev_nve_hybrid_VideoSurfaceHybridClient_nativeCurrentMetadata(JNIEnv*, jclass, jlong);
}

using namespace nve::hybrid;

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

std::vector<uint8_t> sample_metadata_bin() {
  BinWriter w;
  const auto map = w.begin_map(2);
  w.write_map_key("tracks");
  const auto arr = w.begin_array(3);
  w.write_i64(42);
  w.write_str("en");
  w.write_null();
  w.end(arr);
  w.write_map_key("hdr");
  w.write_bool(true);
  w.end(map);
  const auto bytes = checked_bytes(w);
  return {bytes.begin(), bytes.end()};
}

/// The VideoSurface props map, optionally with poster, focus point, drm_key (`Bytes`),
/// chapters (`Array`), and nested `metadata` (`NveValue`).
std::vector<uint8_t> video_props(bool with_options) {
  BinWriter w;
  const auto map = w.begin_map(9);
  w.write_map_key("source_url");
  w.write_str("https://nve.dev/clip.mp4");
  w.write_map_key("muted");
  w.write_bool(true);
  w.write_map_key("playback_rate");
  w.write_f64(1.5);
  w.write_map_key("quality");
  w.write_i64(11);
  w.write_map_key("poster_url");
  if (with_options) {
    w.write_str("p.png");
  } else {
    w.write_null();
  }
  w.write_map_key("focus_point");
  if (with_options) {
    FramePoint{7.0f, 0.0f}.encode_bin(w);
  } else {
    w.write_null();
  }
  w.write_map_key("drm_key");
  if (with_options) {
    const std::vector<uint8_t> key{0xAA, 0xBB};
    w.write_bytes(key);
  } else {
    w.write_bytes({});
  }
  w.write_map_key("chapters");
  if (with_options) {
    const auto arr = w.begin_array(3);
    w.write_i64(10);
    w.write_i64(20);
    w.write_i64(30);
    w.end(arr);
  } else {
    const auto arr = w.begin_array(0);
    w.end(arr);
  }
  w.write_map_key("metadata");
  if (with_options) {
    const std::vector<uint8_t> meta = sample_metadata_bin();
    w.raw(meta);
  } else {
    w.write_null();
  }
  w.end(map);
  const auto bytes = checked_bytes(w);
  return {bytes.begin(), bytes.end()};
}

void test_video(JNIEnv& env) {
  const jlong video = Java_dev_nve_hybrid_VideoSurfaceHybridClient_nativeCreate(&env, nullptr);
  CHECK(video != 0 && env.thrown == 0);
  BinWriter point;
  FramePoint{5.0f, 9.0f}.encode_bin(point);
  DirectBuffer at(checked_bytes(point));
  const auto set_props = [&](std::span<const uint8_t> bytes) {
    DirectBuffer props(bytes);
    Java_dev_nve_hybrid_VideoSurfaceHybridClient_nativeSetProps(&env, nullptr, video, props.get());
  };

  // Null options (the Kotlin encoding of `null`) are `NVE_VAL_NULL`, which Rust accepts.
  set_props(video_props(false));
  CHECK(env.thrown == 0);
  Java_dev_nve_hybrid_VideoSurfaceHybridClient_nativeSeek(&env, nullptr, video, 9876);
  jbyteArray frame = Java_dev_nve_hybrid_VideoSurfaceHybridClient_nativeCaptureFrame(&env, nullptr, video, at.get());
  CHECK(env.thrown == 0 && frame != nullptr && frame->bytes.size() == 3u);
  CHECK(frame->bytes[0] == 5 && frame->bytes[1] == 9 && frame->bytes[2] == 24);
  set_props(video_props(true));
  CHECK(env.thrown == 0);
  frame = Java_dev_nve_hybrid_VideoSurfaceHybridClient_nativeCaptureFrame(&env, nullptr, video, at.get());
  CHECK(frame != nullptr && frame->bytes.size() == 10u);
  if (frame != nullptr && frame->bytes.size() == 10u) {
    CHECK(frame->bytes[3] == 7 && frame->bytes[4] == 5);
    CHECK(static_cast<uint8_t>(frame->bytes[5]) == 0xAAu && static_cast<uint8_t>(frame->bytes[6]) == 0xBBu);
    CHECK(frame->bytes[7] == 10 && frame->bytes[8] == 20 && frame->bytes[9] == 30);
  }
  // `NveValue` nested map/array prop round-trips byte-identically (never double-wrapped as Bytes).
  jbyteArray meta_out = Java_dev_nve_hybrid_VideoSurfaceHybridClient_nativeCurrentMetadata(&env, nullptr, video);
  CHECK(env.thrown == 0 && meta_out != nullptr);
  if (meta_out != nullptr) {
    const auto got = bytes_of(meta_out);
    CHECK(std::vector<uint8_t>(got.begin(), got.end()) == sample_metadata_bin());
  }

  // Supplementary UTF-8 error message crosses via UTF-16 `IllegalStateException(String)` with
  // zero CheckJNI errors and zero leaked local refs.
  {
    BinWriter neg;
    FramePoint{-1.0f, 0.0f}.encode_bin(neg);
    DirectBuffer neg_at(checked_bytes(neg));
    const int refs_before = env.live_local_refs;
    CHECK(Java_dev_nve_hybrid_VideoSurfaceHybridClient_nativeCaptureFrame(&env, nullptr, video, neg_at.get()) ==
          nullptr);
    CHECK(env.thrown == 1 && !env.checkjni_error && env.live_local_refs == refs_before);
    CHECK(env.pending_utf16 == u"bad \U0001F600");
    env.ExceptionClear();
  }

  // Every malformed props buffer is a Java exception, never a partial update.
  const std::vector<uint8_t> good = video_props(false);
  set_props(std::span<const uint8_t>(good.data(), 7));
  expect_thrown(env, "malformed NveBinValue");
  set_props(std::vector<uint8_t>{9});
  expect_thrown(env, "malformed NveBinValue: InvalidTag");
  std::vector<uint8_t> trailing = good;
  trailing.push_back(0);
  set_props(trailing);
  expect_thrown(env, "trailing bytes");
  {
    BinWriter w;
    const auto map = w.begin_map(10);
    for (int i = 0; i < 10; ++i) {
      w.write_map_key(std::string(1, static_cast<char>('a' + i)));
      w.write_null();
    }
    w.end(map);
    set_props(checked_bytes(w));
    expect_thrown(env, "more props than VideoSurface declares");
  }
  FakeByteBuffer heap;  // a non-direct buffer: no address, capacity -1
  Java_dev_nve_hybrid_VideoSurfaceHybridClient_nativeSetProps(&env, nullptr, video, &heap);
  expect_thrown(env, "ByteBuffer is not direct");
  Java_dev_nve_hybrid_VideoSurfaceHybridClient_nativeSetProps(&env, nullptr, video, nullptr);
  expect_thrown(env, "null ByteBuffer");
  Java_dev_nve_hybrid_VideoSurfaceHybridClient_nativeSetProps(&env, nullptr, 0, nullptr);
  expect_thrown(env, "null hybrid handle");
  Java_dev_nve_hybrid_VideoSurfaceHybridClient_nativeDestroy(&env, nullptr, video);
}

void test_crypto(JNIEnv& env) {
  const jlong crypto = Java_dev_nve_hybrid_CryptoHybridClient_nativeCreate(&env, nullptr);
  CHECK(crypto != 0 && env.thrown == 0);

  jstring version = Java_dev_nve_hybrid_CryptoHybridClient_nativeVersion(&env, nullptr, crypto);
  CHECK(version != nullptr && version->units == u"nve-hybrid-1.0");

  // Strings cross as UTF-16 -> UTF-8, never Modified UTF-8: `find_nonce` adds the label's UTF-8
  // length (`h é l l o 😀` = 1 + 2 + 3 + 4 = 10 bytes; Modified UTF-8 would make it 12).
  BinWriter hint;
  hint.write_i64(100);
  DirectBuffer hint_buf(checked_bytes(hint));
  _jstring label;
  label.units = u"h\u00E9llo\U0001F600";
  jbyteArray nonce = Java_dev_nve_hybrid_CryptoHybridClient_nativeFindNonce(&env, nullptr, crypto, &label, hint_buf.get());
  CHECK(env.thrown == 0 && nonce != nullptr);
  if (nonce != nullptr) {
    BinReader r(bytes_of(nonce));
    CHECK(r.read_i64() == 110 && r.ok() && r.at_end());
  }
  _jstring empty;
  jbyteArray empty_nonce =
      Java_dev_nve_hybrid_CryptoHybridClient_nativeFindNonce(&env, nullptr, crypto, &empty, hint_buf.get());
  CHECK(env.thrown == 0 && !env.checkjni_error && empty_nonce != nullptr);
  if (empty_nonce != nullptr) {
    BinReader r(bytes_of(empty_nonce));
    CHECK(r.read_i64() == 100 && r.ok() && r.at_end());
  }
  _jstring lone;
  lone.units = std::u16string(1, char16_t{0xD800});
  CHECK(Java_dev_nve_hybrid_CryptoHybridClient_nativeFindNonce(&env, nullptr, crypto, &lone, hint_buf.get()) == nullptr);
  expect_thrown(env, "lone surrogate");
  std::vector<uint8_t> long_hint(checked_bytes(hint).begin(), checked_bytes(hint).end());
  long_hint.push_back(0);
  DirectBuffer long_hint_buf(long_hint);
  CHECK(Java_dev_nve_hybrid_CryptoHybridClient_nativeFindNonce(&env, nullptr, crypto, &label, long_hint_buf.get()) == nullptr);
  expect_thrown(env, "trailing bytes");

  // A dynamic value crosses as an `NveValue` tree and comes back byte-identical.
  BinWriter meta;
  const auto map = meta.begin_map(1);
  meta.write_map_key("k");
  const auto arr = meta.begin_array(3);
  meta.write_i64(1);
  meta.write_str("x");
  meta.write_null();
  meta.end(arr);
  meta.end(map);
  DirectBuffer meta_buf(checked_bytes(meta));
  jbyteArray echoed = Java_dev_nve_hybrid_CryptoHybridClient_nativeInspectDynamic(&env, nullptr, crypto, meta_buf.get());
  CHECK(env.thrown == 0 && echoed != nullptr);
  if (echoed != nullptr) {
    const auto got = bytes_of(echoed);
    CHECK(std::vector<uint8_t>(got.begin(), got.end()) == meta_buf.bytes);
  }
  DirectBuffer truncated(std::span<const uint8_t>(meta_buf.bytes.data(), 6));
  CHECK(Java_dev_nve_hybrid_CryptoHybridClient_nativeInspectDynamic(&env, nullptr, crypto, truncated.get()) == nullptr);
  expect_thrown(env, "malformed NveBinValue argument");

  // A struct argument and result, with fields named like the codec's locals and a nested Vec.
  Cursor cursor;
  cursor.mark = 1u;
  cursor.w = {1, 2, 3};
  cursor.r = "ab";
  cursor.count = 4u;
  cursor.tag = 42;
  cursor.grid = {{1, 2, 3}, {4, 5, 6}};
  BinWriter cw;
  cursor.encode_bin(cw);
  DirectBuffer cursor_buf(checked_bytes(cw));
  jbyteArray transposed = Java_dev_nve_hybrid_CryptoHybridClient_nativeTranspose(&env, nullptr, crypto, cursor_buf.get());
  CHECK(env.thrown == 0 && transposed != nullptr);
  if (transposed != nullptr) {
    BinReader r(bytes_of(transposed));
    const Cursor t = Cursor::decode_bin(r);
    CHECK(r.ok() && r.at_end());
    CHECK(t.mark == 2u && t.count == 8u && t.r == std::optional<std::string>("AB") && t.tag == std::optional<int32_t>(-42));
    CHECK((t.w == std::vector<uint8_t>{3, 2, 1}));
    CHECK((t.grid == std::vector<std::vector<int32_t>>{{1, 4}, {2, 5}, {3, 6}}));
  }
  CHECK(!env.checkjni_error);
  Java_dev_nve_hybrid_CryptoHybridClient_nativeDestroy(&env, nullptr, crypto);
}

}  // namespace

int main() {
  JNIEnv env;
  test_video(env);
  test_crypto(env);
  if (g_failures != 0) {
    std::fprintf(stderr, "jni harness: %d failure(s)\n", g_failures);
    return 1;
  }
  std::printf("jni harness: ok\n");
  return 0;
}
