// Drives the generated OverlayView JNI glue (`OverlayViewHybridJni.generated.cpp`) against the live
// Rust provider on a fake `JNIEnv`, asserting the exact `NveValue` of every prop entry
// `nativeSetProps` builds: scalars in place, `Struct` / `Vec` props as zero-copy `NVE_VAL_BYTES`
// slices of the direct buffer, and `NveValue` props (and undeclared keys) as lossless trees.
#include "jni.h"
#include "OverlayViewHybridSpec.generated.hpp"
#include "jni_test_support.h"

extern "C" {
void Java_dev_nve_hybrid_OverlayViewHybridClient_nativeDestroy(JNIEnv*, jclass, jlong);
void Java_dev_nve_hybrid_OverlayViewHybridClient_nativeSetProps(JNIEnv*, jclass, jlong, jobject);
jbyteArray Java_dev_nve_hybrid_OverlayViewHybridClient_nativeCurrentOverlay(JNIEnv*, jclass, jlong);
jbyteArray Java_dev_nve_hybrid_OverlayViewHybridClient_nativeCurrentOverlays(JNIEnv*, jclass, jlong);
jbyteArray Java_dev_nve_hybrid_OverlayViewHybridClient_nativeCurrentExtra(JNIEnv*, jclass, jlong);
jstring Java_dev_nve_hybrid_OverlayViewHybridClient_nativeSummary(JNIEnv*, jclass, jlong);
jbyteArray Java_dev_nve_hybrid_OverlayViewHybridClient_nativeEcho(JNIEnv*, jclass, jlong, jobject);
jint Java_dev_nve_hybrid_OverlayViewHybridClient_nativeShift(JNIEnv*, jclass, jlong, jint, jobject);
}

using namespace nve::hybrid;

namespace {

DynamicValue dynamic(const std::function<void(BinWriter&)>& encode) {
  const std::vector<uint8_t> bytes = encoded(encode);
  return DynamicValue::from_bytes(bytes).value_or(DynamicValue());
}

/// `{"a": [1, "s", true, null, 2.5, b"\x00\xff"], "b": {"nested": "😀"}}`
DynamicValue sample_extra() {
  return dynamic([](BinWriter& w) {
    const auto map = w.begin_map(2);
    w.write_map_key("a");
    const auto arr = w.begin_array(6);
    w.write_i64(1);
    w.write_str("s");
    w.write_bool(true);
    w.write_null();
    w.write_f64(2.5);
    const std::vector<uint8_t> raw{0x00, 0xFF};
    w.write_bytes(raw);
    w.end(arr);
    w.write_map_key("b");
    const auto nested = w.begin_map(1);
    w.write_map_key("nested");
    w.write_str("\xF0\x9F\x98\x80");
    w.end(nested);
    w.end(map);
  });
}

Overlay sample_overlay(std::optional<std::string> caption) {
  Overlay o;
  o.kind = OverlayKind::Ad;
  o.cues = {1, -2, 2147483647, -2147483647 - 1};
  o.caption = std::move(caption);
  o.extra = sample_extra();
  o.labels = {"", "\xC3\xA9", "\xF0\x9F\x98\x80"};
  o.when = 0xFFFFFFFFu;
  return o;
}

bool same(const Overlay& a, const Overlay& b) {
  return a.kind == b.kind && a.cues == b.cues && a.caption == b.caption && a.extra == b.extra &&
         a.labels == b.labels && a.when == b.when;
}

/// `u32::MAX` / `u64::MAX` prop defaults (Rust-side, never sent here) end every summary.
constexpr std::u16string_view kSummaryTail = u"|4294967295|18446744073709551615";

struct Props {
  std::string title = "Live \xE2\x9C\x93 \xF0\x9F\x98\x80";
  int64_t kind = 7;
  std::optional<Overlay> overlay;
  std::vector<Overlay> overlays;
  std::vector<int32_t> cues{5, 6, 7, 8};
  DynamicValue extra = sample_extra();
  bool unknown_container = false;  // `zzz: [{"k": 1}]` instead of `cues`
};

/// The props map exactly as the generated Kotlin `setProps` encodes it.
std::vector<uint8_t> encode_props(const Props& p) {
  return encoded([&](BinWriter& w) {
    const auto map = w.begin_map(6);
    w.write_map_key("title");
    w.write_str(p.title);
    w.write_map_key("kind");
    w.write_i64(p.kind);
    w.write_map_key("overlay");
    if (p.overlay) {
      p.overlay->encode_bin(w);
    } else {
      w.write_null();
    }
    w.write_map_key("overlays");
    const auto arr = w.begin_array(static_cast<uint32_t>(p.overlays.size()));
    for (const Overlay& o : p.overlays) o.encode_bin(w);
    w.end(arr);
    if (p.unknown_container) {
      w.write_map_key("zzz");
      const auto outer = w.begin_array(1);
      const auto inner = w.begin_map(1);
      w.write_map_key("k");
      w.write_i64(1);
      w.end(inner);
      w.end(outer);
    } else {
      w.write_map_key("cues");
      const auto cues = w.begin_array(static_cast<uint32_t>(p.cues.size()));
      for (const int32_t c : p.cues) w.write_i64(c);
      w.end(cues);
    }
    w.write_map_key("extra");
    w.raw(p.extra.bytes());
    w.end(map);
  });
}

/// `sample_extra()` as the lossless `NveValue` tree the glue must build (strings and bytes
/// borrowed from `buffer`).
void inspect_extra_tree(const NveValue* v, std::span<const uint8_t> buffer) {
  CHECK(v != nullptr && v->tag == NVE_VAL_MAP && v->data.map_val.len == 2u);
  if (v == nullptr || v->tag != NVE_VAL_MAP || v->data.map_val.len != 2u) return;
  const NveMapEntry* top = v->data.map_val.ptr;
  CHECK(key_of(top[0]) == "a" && top[0].value.tag == NVE_VAL_ARRAY && top[0].value.data.array_val.len == 6u);
  if (top[0].value.tag == NVE_VAL_ARRAY && top[0].value.data.array_val.len == 6u) {
    const NveValue* a = top[0].value.data.array_val.ptr;
    CHECK(a[0].tag == NVE_VAL_I64 && a[0].data.i64_val == 1);
    CHECK(a[1].tag == NVE_VAL_STR && str_of(a[1]) == "s");
    CHECK(a[2].tag == NVE_VAL_BOOL && a[2].data.bool_val == 1u);
    CHECK(a[3].tag == NVE_VAL_NULL);
    CHECK(a[4].tag == NVE_VAL_F64 && a[4].data.f64_val == 2.5);
    CHECK(is_borrowed_bytes(&a[5], buffer, std::vector<uint8_t>{0x00, 0xFF}));
  }
  CHECK(key_of(top[1]) == "b" && top[1].value.tag == NVE_VAL_MAP && top[1].value.data.map_val.len == 1u);
  if (top[1].value.tag == NVE_VAL_MAP && top[1].value.data.map_val.len == 1u) {
    const NveMapEntry& nested = top[1].value.data.map_val.ptr[0];
    CHECK(key_of(nested) == "nested" && nested.value.tag == NVE_VAL_STR && str_of(nested.value) == "\xF0\x9F\x98\x80");
  }
}

void inspect_props(std::span<const NveMapEntry> entries, std::span<const uint8_t> buffer, const Props& p) {
  CHECK(entries.size() == 6u);
  const NveValue* title = find_prop(entries, "title");
  CHECK(title != nullptr && title->tag == NVE_VAL_STR && str_of(*title) == p.title);
  CHECK(title != nullptr && title->data.str_val.ptr > buffer.data() && title->data.str_val.ptr < buffer.data() + buffer.size());
  const NveValue* kind = find_prop(entries, "kind");
  CHECK(kind != nullptr && kind->tag == NVE_VAL_I64 && kind->data.i64_val == p.kind);
  const NveValue* overlay = find_prop(entries, "overlay");
  if (p.overlay) {
    CHECK(is_borrowed_bytes(overlay, buffer, encoded([&](BinWriter& w) { p.overlay->encode_bin(w); })));
  } else {
    CHECK(overlay != nullptr && overlay->tag == NVE_VAL_NULL);
  }
  CHECK(is_borrowed_bytes(find_prop(entries, "overlays"), buffer, encoded([&](BinWriter& w) {
    const auto arr = w.begin_array(static_cast<uint32_t>(p.overlays.size()));
    for (const Overlay& o : p.overlays) o.encode_bin(w);
    w.end(arr);
  })));
  if (p.unknown_container) {
    // An undeclared key keeps its full value as a tree (Rust ignores it).
    const NveValue* zzz = find_prop(entries, "zzz");
    CHECK(zzz != nullptr && zzz->tag == NVE_VAL_ARRAY && zzz->data.array_val.len == 1u);
    if (zzz != nullptr && zzz->tag == NVE_VAL_ARRAY && zzz->data.array_val.len == 1u) {
      const NveValue& inner = zzz->data.array_val.ptr[0];
      CHECK(inner.tag == NVE_VAL_MAP && inner.data.map_val.len == 1u);
      CHECK(inner.tag == NVE_VAL_MAP && key_of(inner.data.map_val.ptr[0]) == "k" &&
            inner.data.map_val.ptr[0].value.data.i64_val == 1);
    }
  } else {
    CHECK(is_borrowed_bytes(find_prop(entries, "cues"), buffer, encoded([&](BinWriter& w) {
      const auto arr = w.begin_array(static_cast<uint32_t>(p.cues.size()));
      for (const int32_t c : p.cues) w.write_i64(c);
      w.end(arr);
    })));
  }
  const NveValue* extra = find_prop(entries, "extra");
  if (p.extra == sample_extra()) {
    inspect_extra_tree(extra, buffer);
  } else if (p.extra.tag() == nve::host::BinTag::Str) {
    CHECK(extra != nullptr && extra->tag == NVE_VAL_STR && str_of(*extra) == "solo");
  } else {
    CHECK(extra != nullptr && extra->tag == NVE_VAL_NULL);
  }
}

class Harness {
 public:
  explicit Harness(JNIEnv& env) : env_(env) {
    const auto* recording = recording_vtable<NveHybridOverlayViewVTable>(NVE_HYBRID_OVERLAY_VIEW_INTERFACE_ID);
    CHECK(recording != nullptr);
    if (recording != nullptr) handle_ = reinterpret_cast<jlong>(new OverlayViewClient(recording));
  }
  ~Harness() { Java_dev_nve_hybrid_OverlayViewHybridClient_nativeDestroy(&env_, nullptr, handle_); }
  Harness(const Harness&) = delete;
  Harness& operator=(const Harness&) = delete;

  /// `nativeSetProps` with `p`, inspecting the entries Rust receives; returns whether Rust saw them.
  bool set(const Props& p) {
    DirectBuffer buffer(encode_props(p));
    bool seen = false;
    g_inspect = [&](std::span<const NveMapEntry> entries) {
      seen = true;
      inspect_props(entries, buffer.bytes, p);
    };
    Java_dev_nve_hybrid_OverlayViewHybridClient_nativeSetProps(&env_, nullptr, handle_, buffer.get());
    g_inspect = nullptr;
    return seen;
  }

  void set_raw(std::span<const uint8_t> bytes) {
    DirectBuffer buffer(bytes);
    Java_dev_nve_hybrid_OverlayViewHybridClient_nativeSetProps(&env_, nullptr, handle_, buffer.get());
  }

  template <class T>
  T read(jbyteArray (*native)(JNIEnv*, jclass, jlong), const std::function<T(BinReader&)>& decode) {
    jbyteArray out = native(&env_, nullptr, handle_);
    CHECK(env_.thrown == 0 && out != nullptr);
    if (out == nullptr) return T{};
    BinReader r(bytes_of(out));
    T value = decode(r);
    CHECK(r.ok() && r.at_end());
    return value;
  }

  std::optional<Overlay> current_overlay() {
    return read<std::optional<Overlay>>(&Java_dev_nve_hybrid_OverlayViewHybridClient_nativeCurrentOverlay,
                                        [](BinReader& r) -> std::optional<Overlay> {
                                          if (r.read_null()) return std::nullopt;
                                          return Overlay::decode_bin(r);
                                        });
  }

  std::vector<Overlay> current_overlays() {
    return read<std::vector<Overlay>>(&Java_dev_nve_hybrid_OverlayViewHybridClient_nativeCurrentOverlays,
                                      [](BinReader& r) {
                                        std::vector<Overlay> out;
                                        const auto arr = r.begin_array();
                                        for (uint32_t i = 0; i < arr.count && r.ok(); ++i) out.push_back(Overlay::decode_bin(r));
                                        r.end(arr);
                                        return out;
                                      });
  }

  std::vector<uint8_t> current_extra() {
    jbyteArray out = Java_dev_nve_hybrid_OverlayViewHybridClient_nativeCurrentExtra(&env_, nullptr, handle_);
    CHECK(env_.thrown == 0 && out != nullptr);
    if (out == nullptr) return {};
    const auto bytes = bytes_of(out);
    return {bytes.begin(), bytes.end()};
  }

  std::u16string summary() {
    jstring s = Java_dev_nve_hybrid_OverlayViewHybridClient_nativeSummary(&env_, nullptr, handle_);
    CHECK(env_.thrown == 0 && s != nullptr);
    return s != nullptr ? s->units : std::u16string();
  }

  jint shift(jint handle, std::optional<int32_t> object) {
    DirectBuffer arg(encoded([&](BinWriter& w) {
      if (object) {
        w.write_i64(*object);
      } else {
        w.write_null();
      }
    }));
    return Java_dev_nve_hybrid_OverlayViewHybridClient_nativeShift(&env_, nullptr, handle_, handle, arg.get());
  }

  jbyteArray echo(const Overlay& o) {
    DirectBuffer arg(encoded([&](BinWriter& w) { o.encode_bin(w); }));
    return Java_dev_nve_hybrid_OverlayViewHybridClient_nativeEcho(&env_, nullptr, handle_, arg.get());
  }

 private:
  JNIEnv& env_;
  jlong handle_ = 0;
};

void test_props_and_results(JNIEnv& env) {
  Harness h(env);
  Props full;
  full.overlay = sample_overlay("cap");
  full.overlays = {sample_overlay(std::nullopt), Overlay{}, sample_overlay("second")};
  CHECK(h.set(full) && env.thrown == 0);
  const std::optional<Overlay> overlay = h.current_overlay();
  CHECK(overlay.has_value() && same(*overlay, *full.overlay));
  const std::vector<Overlay> overlays = h.current_overlays();
  CHECK(overlays.size() == 3u);
  for (size_t i = 0; i < overlays.size() && i < full.overlays.size(); ++i) CHECK(same(overlays[i], full.overlays[i]));
  const DynamicValue extra = sample_extra();
  CHECK(h.current_extra() == std::vector<uint8_t>(extra.bytes().begin(), extra.bytes().end()));
  CHECK(h.summary() == std::u16string(u"Live \u2713 \U0001F600|Ad|true|3|4") + std::u16string(kSummaryTail));
  // `u32` parameters named `handle` / `object` cross as their bits (`-1` is `u32::MAX`).
  CHECK(h.shift(-1, 2) == 1 && h.shift(5, std::nullopt) == 5 && env.thrown == 0);

  // Absent option, empty sequence, undeclared container key, and a scalar dynamic value.
  Props sparse;
  sparse.title = "t";
  sparse.kind = 3;
  sparse.unknown_container = true;
  sparse.extra = dynamic([](BinWriter& w) { w.write_str("solo"); });
  CHECK(h.set(sparse) && env.thrown == 0);
  CHECK(!h.current_overlay().has_value() && h.current_overlays().empty());
  const auto solo = sparse.extra.bytes();
  CHECK(h.current_extra() == std::vector<uint8_t>(solo.begin(), solo.end()));
  CHECK(h.summary() == std::u16string(u"t|Chapter|false|0|0") + std::u16string(kSummaryTail));
  Props null_extra;
  null_extra.extra = DynamicValue();
  CHECK(h.set(null_extra) && env.thrown == 0);
  CHECK(h.current_extra() == std::vector<uint8_t>{0});

  // The non-POD struct crosses as a method argument and result too.
  const Overlay sent = sample_overlay("cap");
  jbyteArray echoed = h.echo(sent);
  CHECK(env.thrown == 0 && echoed != nullptr);
  if (echoed != nullptr) {
    BinReader r(bytes_of(echoed));
    const Overlay back = Overlay::decode_bin(r);
    CHECK(r.ok() && r.at_end());
    Overlay expected = sent;
    expected.kind = OverlayKind::Caption;
    expected.cues = {-2147483647 - 1, 2147483647, -2, 1};
    expected.caption = "CAP";
    expected.labels = {"", "\xC3\x89", "\xF0\x9F\x98\x80"};
    expected.when = 0x7FFFFFFFu;
    CHECK(same(back, expected));
  }
  const int refs_before = env.live_local_refs;
  CHECK(h.echo(sample_overlay("fail")) == nullptr);
  CHECK(env.thrown == 1 && !env.checkjni_error && env.live_local_refs == refs_before);
  CHECK(env.pending_utf16 == u"bad \U0001F600");
  env.ExceptionClear();
}

void test_malformed_props(JNIEnv& env) {
  Harness h(env);
  // An encoded struct prop Rust cannot decode is a failed `set_props_direct`, never a partial update.
  const auto with_overlay = [](const std::function<void(BinWriter&)>& overlay) {
    return encoded([&](BinWriter& w) {
      const auto map = w.begin_map(1);
      w.write_map_key("overlay");
      overlay(w);
      w.end(map);
    });
  };
  h.set_raw(with_overlay([](BinWriter& w) {
    const auto m = w.begin_map(1);
    w.write_map_key("kind");
    w.write_str("not an enum");
    w.end(m);
  }));
  expect_thrown(env, "setProps failed on OverlayView");
  h.set_raw(with_overlay([](BinWriter& w) {
    const auto m = w.begin_map(1);
    w.write_map_key("kind");
    w.write_i64(5);  // not an `OverlayKind` discriminant
    w.end(m);
  }));
  expect_thrown(env, "setProps failed on OverlayView");
  // Invalid UTF-8 inside the zero-copy bytes: Rust's full parse still rejects it.
  std::vector<uint8_t> bad_utf8 = with_overlay([](BinWriter& w) {
    const auto m = w.begin_map(1);
    w.write_map_key("caption");
    w.write_str("ab");
    w.end(m);
  });
  bad_utf8[bad_utf8.size() - 1] = 0xFF;
  h.set_raw(bad_utf8);
  expect_thrown(env, "setProps failed on OverlayView");
  // A struct prop whose own container length overruns the props map never reaches Rust: the
  // zero-copy skip is still bounds-checked. Layout: map header (9) + key (4 + 7), then the
  // `overlay` map tag at 20 and its payload length at 21.
  std::vector<uint8_t> overrun = with_overlay([](BinWriter& w) { w.end(w.begin_map(0)); });
  CHECK(overrun.size() == 29u && overrun[20] == 7u && overrun[21] == 4u);
  overrun[21] = 0x40;
  h.set_raw(overrun);
  expect_thrown(env, "malformed NveBinValue");
  CHECK(!env.checkjni_error);
}

}  // namespace

int main() {
  JNIEnv env;
  test_props_and_results(env);
  test_malformed_props(env);
  if (g_failures != 0) {
    std::fprintf(stderr, "jni overlay harness: %d failure(s)\n", g_failures);
    return 1;
  }
  std::printf("jni overlay harness: ok\n");
  return 0;
}
