// Drives the generated JNI glue (`CryptoHybridJni.generated.cpp`) like the Kotlin client does,
// against the live Rust provider: direct `ByteBuffer`s, UTF-16 Java strings, and the Java
// exceptions every malformed input must raise. Built with `fake_jni.h` as `<jni.h>`.
#include "jni.h"
#include "CryptoHybridSpec.generated.hpp"
#include "jni_test_support.h"
#include <nve/host/jni.hpp>

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

/// Asserts the tag of every `VideoSurface` prop entry: scalars in place, `Struct` / `Vec` props as
/// zero-copy encoded `NVE_VAL_BYTES`, and the `NveValue` prop as a lossless `NveValue` tree.
void inspect_video_props(std::span<const NveMapEntry> entries, std::span<const uint8_t> buffer, bool with_options) {
  CHECK(entries.size() == 9u);
  const NveValue* url = find_prop(entries, "source_url");
  CHECK(url != nullptr && url->tag == NVE_VAL_STR && str_of(*url) == "https://nve.dev/clip.mp4");
  CHECK(url != nullptr && url->data.str_val.ptr >= buffer.data() && url->data.str_val.ptr < buffer.data() + buffer.size());
  const NveValue* muted = find_prop(entries, "muted");
  CHECK(muted != nullptr && muted->tag == NVE_VAL_BOOL && muted->data.bool_val == 1u);
  const NveValue* rate = find_prop(entries, "playback_rate");
  CHECK(rate != nullptr && rate->tag == NVE_VAL_F64 && rate->data.f64_val == 1.5);
  const NveValue* quality = find_prop(entries, "quality");
  CHECK(quality != nullptr && quality->tag == NVE_VAL_I64 && quality->data.i64_val == 11);
  const NveValue* poster = find_prop(entries, "poster_url");
  const NveValue* focus = find_prop(entries, "focus_point");
  const NveValue* drm = find_prop(entries, "drm_key");
  const NveValue* chapters = find_prop(entries, "chapters");
  const NveValue* metadata = find_prop(entries, "metadata");
  if (!with_options) {
    CHECK(poster != nullptr && poster->tag == NVE_VAL_NULL);
    CHECK(focus != nullptr && focus->tag == NVE_VAL_NULL);
    CHECK(is_borrowed_bytes(drm, buffer, {}));
    CHECK(is_borrowed_bytes(chapters, buffer, encoded([](BinWriter& w) { w.end(w.begin_array(0)); })));
    CHECK(metadata != nullptr && metadata->tag == NVE_VAL_NULL);
    return;
  }
  CHECK(poster != nullptr && poster->tag == NVE_VAL_STR && str_of(*poster) == "p.png");
  CHECK(is_borrowed_bytes(focus, buffer, encoded([](BinWriter& w) { FramePoint{7.0f, 0.0f}.encode_bin(w); })));
  CHECK(is_borrowed_bytes(drm, buffer, std::vector<uint8_t>{0xAA, 0xBB}));
  CHECK(is_borrowed_bytes(chapters, buffer, encoded([](BinWriter& w) {
    const auto arr = w.begin_array(3);
    w.write_i64(10);
    w.write_i64(20);
    w.write_i64(30);
    w.end(arr);
  })));
  // {"tracks": [42, "en", null], "hdr": true} as a real tree, never double-wrapped as Bytes.
  CHECK(metadata != nullptr && metadata->tag == NVE_VAL_MAP && metadata->data.map_val.len == 2u);
  if (metadata == nullptr || metadata->tag != NVE_VAL_MAP || metadata->data.map_val.len != 2u) return;
  const NveMapEntry* meta = metadata->data.map_val.ptr;
  CHECK(key_of(meta[0]) == "tracks" && meta[0].value.tag == NVE_VAL_ARRAY && meta[0].value.data.array_val.len == 3u);
  if (meta[0].value.tag == NVE_VAL_ARRAY && meta[0].value.data.array_val.len == 3u) {
    const NveValue* tracks = meta[0].value.data.array_val.ptr;
    CHECK(tracks[0].tag == NVE_VAL_I64 && tracks[0].data.i64_val == 42);
    CHECK(tracks[1].tag == NVE_VAL_STR && str_of(tracks[1]) == "en");
    CHECK(tracks[2].tag == NVE_VAL_NULL);
  }
  CHECK(key_of(meta[1]) == "hdr" && meta[1].value.tag == NVE_VAL_BOOL && meta[1].value.data.bool_val == 1u);
}

void test_video(JNIEnv& env) {
  const jlong created = Java_dev_nve_hybrid_VideoSurfaceHybridClient_nativeCreate(&env, nullptr);
  CHECK(created != 0 && env.thrown == 0);
  Java_dev_nve_hybrid_VideoSurfaceHybridClient_nativeDestroy(&env, nullptr, created);

  // The same client `nativeCreate` builds, over the recording copy of the Rust vtable.
  const auto* recording = recording_vtable<NveHybridVideoSurfaceVTable>(NVE_HYBRID_VIDEO_SURFACE_INTERFACE_ID);
  CHECK(recording != nullptr);
  if (recording == nullptr) return;
  const jlong video = reinterpret_cast<jlong>(new VideoSurfaceClient(recording));
  BinWriter point;
  FramePoint{5.0f, 9.0f}.encode_bin(point);
  DirectBuffer at(checked_bytes(point));
  const auto set_props = [&](std::span<const uint8_t> bytes) {
    DirectBuffer props(bytes);
    Java_dev_nve_hybrid_VideoSurfaceHybridClient_nativeSetProps(&env, nullptr, video, props.get());
  };
  int inspected = 0;
  const auto set_inspected_props = [&](bool with_options) {
    DirectBuffer props(video_props(with_options));
    g_inspect = [&](std::span<const NveMapEntry> entries) {
      ++inspected;
      inspect_video_props(entries, props.bytes, with_options);
    };
    Java_dev_nve_hybrid_VideoSurfaceHybridClient_nativeSetProps(&env, nullptr, video, props.get());
    g_inspect = nullptr;
  };

  // Null options (the Kotlin encoding of `null`) are `NVE_VAL_NULL`, which Rust accepts.
  set_inspected_props(false);
  CHECK(env.thrown == 0 && inspected == 1);
  Java_dev_nve_hybrid_VideoSurfaceHybridClient_nativeSeek(&env, nullptr, video, 9876);
  jbyteArray frame = Java_dev_nve_hybrid_VideoSurfaceHybridClient_nativeCaptureFrame(&env, nullptr, video, at.get());
  CHECK(env.thrown == 0 && frame != nullptr && frame->bytes.size() == 3u);
  CHECK(frame->bytes[0] == 5 && frame->bytes[1] == 9 && frame->bytes[2] == 24);
  set_inspected_props(true);
  CHECK(env.thrown == 0 && inspected == 2);
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

/// `throw_java_exception` keeps every valid character exact (UTF-16 through `NewString`, never
/// Modified UTF-8) and replaces each ill-formed UTF-8 sequence with U+FFFD, never dropping the text.
void test_throw_java_exception(JNIEnv& env) {
  struct Case {
    const char* utf8;
    std::u16string_view utf16;
  };
  const Case cases[] = {
      {"bad \xF0\x9F\x98\x80", u"bad \U0001F600"},
      {"bad \xFF", u"bad \uFFFD"},
      {"\xC3(\xF0\x9F\x98 ok", u"\uFFFD(\uFFFD ok"},
      {"\xED\xA0\x80\xE2\x82\xAC", u"\uFFFD\uFFFD\uFFFD\u20AC"},
      {"", u""},
  };
  for (const Case& c : cases) {
    const int refs_before = env.live_local_refs;
    nve::host::jni::throw_java_exception(&env, c.utf8);
    CHECK(env.thrown == 1 && !env.checkjni_error && env.live_local_refs == refs_before);
    CHECK(env.pending_utf16 == c.utf16);
    env.ExceptionClear();
  }
  nve::host::jni::throw_java_exception(&env, nullptr);
  CHECK(env.thrown == 1 && env.pending_utf16 == u"NVE hybrid call failed");
  // A pending exception wins: nothing is thrown over it (CheckJNI would abort).
  nve::host::jni::throw_java_exception(&env, "second");
  CHECK(env.thrown == 1 && !env.checkjni_error && env.pending_utf16 == u"NVE hybrid call failed");
  env.ExceptionClear();
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
  test_throw_java_exception(env);
  if (g_failures != 0) {
    std::fprintf(stderr, "jni harness: %d failure(s)\n", g_failures);
    return 1;
  }
  std::printf("jni harness: ok\n");
  return 0;
}
