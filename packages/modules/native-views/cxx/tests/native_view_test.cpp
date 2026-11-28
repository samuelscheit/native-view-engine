#include "nve_native_view.hpp"

#include <cmath>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <iostream>
#include <string>
#include <vector>

#define NVE_CHECK(expr)                                                                         \
  do {                                                                                          \
    if (!(expr)) {                                                                              \
      std::cerr << "NVE_CHECK failed at " << __FILE__ << ":" << __LINE__ << ": " #expr << "\n"; \
      std::abort();                                                                             \
    }                                                                                           \
  } while (0)

namespace {

template <typename T>
void push_le(std::vector<uint8_t>& buf, T v) {
  const auto* p = reinterpret_cast<const uint8_t*>(&v);
  buf.insert(buf.end(), p, p + sizeof(T));
}

struct NvnpBuilder {
  std::vector<uint8_t> buf;
  uint16_t count{0u};

  NvnpBuilder() {
    buf.insert(buf.end(), nve::kNativePropsMagic.begin(), nve::kNativePropsMagic.end());
    buf.push_back(nve::kNativePropsVersion);
    push_le<uint16_t>(buf, 0u);
  }

  void add_null(uint16_t id) {
    push_le<uint16_t>(buf, id);
    buf.push_back(0u);
    ++count;
  }
  void add_bool(uint16_t id, bool v) {
    push_le<uint16_t>(buf, id);
    buf.push_back(1u);
    buf.push_back(v ? 1u : 0u);
    ++count;
  }
  void add_u32(uint16_t id, uint32_t v) {
    push_le<uint16_t>(buf, id);
    buf.push_back(2u);
    push_le<uint32_t>(buf, v);
    ++count;
  }
  void add_i64(uint16_t id, int64_t v) {
    push_le<uint16_t>(buf, id);
    buf.push_back(3u);
    push_le<int64_t>(buf, v);
    ++count;
  }
  void add_u64(uint16_t id, uint64_t v) {
    push_le<uint16_t>(buf, id);
    buf.push_back(4u);
    push_le<uint64_t>(buf, v);
    ++count;
  }
  void add_f32(uint16_t id, float v) {
    push_le<uint16_t>(buf, id);
    buf.push_back(5u);
    push_le<float>(buf, v);
    ++count;
  }
  void add_f64(uint16_t id, double v) {
    push_le<uint16_t>(buf, id);
    buf.push_back(6u);
    push_le<double>(buf, v);
    ++count;
  }
  void add_str(uint16_t id, std::string_view s) {
    push_le<uint16_t>(buf, id);
    buf.push_back(7u);
    push_le<uint32_t>(buf, static_cast<uint32_t>(s.size()));
    buf.insert(buf.end(), s.begin(), s.end());
    ++count;
  }
  void add_bytes(uint16_t id, std::span<const uint8_t> b) {
    push_le<uint16_t>(buf, id);
    buf.push_back(8u);
    push_le<uint32_t>(buf, static_cast<uint32_t>(b.size()));
    buf.insert(buf.end(), b.begin(), b.end());
    ++count;
  }

  std::vector<uint8_t> finish() {
    buf[5] = static_cast<uint8_t>(count & 0xFFu);
    buf[6] = static_cast<uint8_t>((count >> 8u) & 0xFFu);
    return buf;
  }
};

struct NvncBuilder {
  std::vector<uint8_t> buf;
  uint8_t count{0u};

  explicit NvncBuilder(uint16_t opcode) {
    buf.insert(buf.end(), nve::kNativeCmdMagic.begin(), nve::kNativeCmdMagic.end());
    buf.push_back(nve::kNativeCmdVersion);
    push_le<uint16_t>(buf, opcode);
    buf.push_back(0u);
  }

  void arg_u64(uint64_t v) {
    buf.push_back(4u);
    push_le<uint64_t>(buf, v);
    ++count;
  }
  void arg_f32(float v) {
    buf.push_back(5u);
    push_le<float>(buf, v);
    ++count;
  }
  void arg_str(std::string_view s) {
    buf.push_back(7u);
    push_le<uint32_t>(buf, static_cast<uint32_t>(s.size()));
    buf.insert(buf.end(), s.begin(), s.end());
    ++count;
  }

  std::vector<uint8_t> finish() {
    buf[7] = count;
    return buf;
  }
};

void test_nvnp_all_value_types_and_specs() {
  // 1. All 9 scalar/string/bytes tags roundtrip cleanly.
  NvnpBuilder all;
  all.add_null(1u);
  all.add_bool(2u, true);
  all.add_u32(3u, 0xDEADBEEFu);
  all.add_i64(4u, -9876543210LL);
  all.add_u64(5u, 0x1122334455667788ULL);
  all.add_f32(6u, 0.75f);
  all.add_f64(7u, 3.1415926535);
  all.add_str(8u, "hello UTF-8 \xE2\x9C\x93 \xF0\x9F\x9A\x80");
  const std::vector<uint8_t> raw = {0xAAu, 0xBBu, 0xCCu};
  all.add_bytes(9u, raw);
  const auto bytes = all.finish();

  nve::NvnpDecoder dec;
  NVE_CHECK(nve::NvnpDecoder::parse(bytes, dec) == nve::NativeViewCodecError::Ok);
  NVE_CHECK(dec.field_count() == 9u);
  NVE_CHECK(dec.find(1u)->is_null());
  NVE_CHECK(dec.find(2u)->as_bool() == true);
  NVE_CHECK(dec.find(3u)->as_u32() == 0xDEADBEEFu);
  NVE_CHECK(dec.find(4u)->as_i64() == -9876543210LL);
  NVE_CHECK(dec.find(5u)->as_u64() == 0x1122334455667788ULL);
  NVE_CHECK(std::fabs(*dec.find(6u)->as_f32() - 0.75f) < 1e-6f);
  NVE_CHECK(std::fabs(*dec.find(7u)->as_f64() - 3.1415926535) < 1e-9);
  NVE_CHECK(*dec.find(8u)->as_str() == "hello UTF-8 \xE2\x9C\x93 \xF0\x9F\x9A\x80");
  const auto bspan = *dec.find(9u)->as_bytes();
  NVE_CHECK(bspan.size() == 3u && bspan[0] == 0xAAu && bspan[2] == 0xCCu);

  // 2. BlurSurfaceProps
  {
    NvnpBuilder b;
    b.add_u32(1u, static_cast<uint32_t>(nve::BlurMaterial::Chrome));
    b.add_f32(2u, 0.85f);
    b.add_u32(3u, 0x11223388u);
    nve::NvnpDecoder d;
    const auto buf = b.finish();
    NVE_CHECK(nve::NvnpDecoder::parse(buf, d) == nve::NativeViewCodecError::Ok);
    nve::BlurSurfaceProps props{};
    NVE_CHECK(nve::decode_blur_surface_props(d, props) == nve::NativeViewCodecError::Ok);
    NVE_CHECK(props.material == nve::BlurMaterial::Chrome);
    NVE_CHECK(std::fabs(props.intensity - 0.85f) < 1e-6f);
    NVE_CHECK(props.tint.has_value() && *props.tint == 0x11223388u);
  }

  // 3. WebViewProps + WebViewCommand
  {
    NvnpBuilder b;
    b.add_str(1u, "https://example.com/docs");
    b.add_bool(2u, false);
    const auto buf = b.finish();
    nve::NvnpDecoder d;
    NVE_CHECK(nve::NvnpDecoder::parse(buf, d) == nve::NativeViewCodecError::Ok);
    nve::WebViewProps props{};
    NVE_CHECK(nve::decode_web_view_props(d, props) == nve::NativeViewCodecError::Ok);
    NVE_CHECK(props.url == "https://example.com/docs" && !props.allows_back_forward);

    NvncBuilder nav(1u);
    nav.arg_str("https://nve.dev/guide");
    const auto cbuf = nav.finish();
    nve::NvncDecoder cdec;
    NVE_CHECK(nve::NvncDecoder::parse(cbuf, cdec) == nve::NativeViewCodecError::Ok);
    nve::WebViewCommand cmd{};
    NVE_CHECK(nve::decode_web_view_command(cdec, cmd) == nve::NativeViewCodecError::Ok);
    NVE_CHECK(cmd.kind == nve::WebViewCommandKind::Navigate && cmd.text == "https://nve.dev/guide");
  }

  // 4. VideoPlayerProps + VideoPlayerCommand
  {
    NvnpBuilder b;
    b.add_str(1u, "https://cdn.example.com/demo.mp4");
    b.add_bool(2u, true);
    b.add_bool(3u, true);
    b.add_bool(4u, false);
    b.add_bool(5u, true);
    const auto buf = b.finish();
    nve::NvnpDecoder d;
    NVE_CHECK(nve::NvnpDecoder::parse(buf, d) == nve::NativeViewCodecError::Ok);
    nve::VideoPlayerProps vp{};
    NVE_CHECK(nve::decode_video_player_props(d, vp) == nve::NativeViewCodecError::Ok);
    NVE_CHECK(vp.source_uri == "https://cdn.example.com/demo.mp4" && vp.autoplay && vp.muted &&
              !vp.looping && vp.controls);

    NvncBuilder seek(3u);
    seek.arg_u64(42500ULL);
    const auto sbuf = seek.finish();
    nve::NvncDecoder sdec;
    NVE_CHECK(nve::NvncDecoder::parse(sbuf, sdec) == nve::NativeViewCodecError::Ok);
    nve::VideoPlayerCommand scmd{};
    NVE_CHECK(nve::decode_video_player_command(sdec, scmd) == nve::NativeViewCodecError::Ok);
    NVE_CHECK(scmd.kind == nve::VideoPlayerCommandKind::SeekMs && scmd.seek_ms == 42500ULL);
  }

  // 5. WebElement packed attrs
  {
    std::vector<uint8_t> packed;
    auto push_kv = [&](std::string_view k, std::string_view v) {
      push_le<uint32_t>(packed, static_cast<uint32_t>(k.size()));
      packed.insert(packed.end(), k.begin(), k.end());
      push_le<uint32_t>(packed, static_cast<uint32_t>(v.size()));
      packed.insert(packed.end(), v.begin(), v.end());
    };
    push_kv("viewBox", "0 0 24 24");
    push_kv("aria-hidden", "true");
    NvnpBuilder b;
    b.add_str(1u, "svg");
    b.add_u32(2u, 2u);
    b.add_bytes(3u, packed);
    const auto buf = b.finish();
    nve::NvnpDecoder d;
    NVE_CHECK(nve::NvnpDecoder::parse(buf, d) == nve::NativeViewCodecError::Ok);
    std::string_view tag;
    std::vector<std::pair<std::string, std::string>> attrs;
    NVE_CHECK(nve::decode_web_element_props(d, tag, [&](std::string_view k, std::string_view v) {
                attrs.emplace_back(std::string(k), std::string(v));
              }) == nve::NativeViewCodecError::Ok);
    NVE_CHECK(tag == "svg" && attrs.size() == 2u);
    NVE_CHECK(attrs[0].first == "viewBox" && attrs[0].second == "0 0 24 24");
  }
}

void test_corrupt_and_adversarial_wire_buffers() {
  nve::NvnpDecoder pdec;
  nve::NvncDecoder cdec;

  // Truncated headers
  const std::vector<uint8_t> short_buf = {'N', 'V', 'N'};
  NVE_CHECK(nve::NvnpDecoder::parse(short_buf, pdec) == nve::NativeViewCodecError::Truncated);
  NVE_CHECK(nve::NvncDecoder::parse(short_buf, cdec) == nve::NativeViewCodecError::Truncated);

  // Bad magic
  const std::vector<uint8_t> bad_magic = {'B', 'A', 'D', '!', 1u, 0u, 0u, 0u};
  NVE_CHECK(nve::NvnpDecoder::parse(bad_magic, pdec) == nve::NativeViewCodecError::InvalidMagic);
  NVE_CHECK(nve::NvncDecoder::parse(bad_magic, cdec) == nve::NativeViewCodecError::InvalidMagic);

  // Unsupported version
  const std::vector<uint8_t> bad_ver_p = {'N', 'V', 'N', 'P', 2u, 0u, 0u};
  const std::vector<uint8_t> bad_ver_c = {'N', 'V', 'N', 'C', 9u, 1u, 0u, 0u};
  NVE_CHECK(nve::NvnpDecoder::parse(bad_ver_p, pdec) == nve::NativeViewCodecError::UnsupportedVersion);
  NVE_CHECK(nve::NvncDecoder::parse(bad_ver_c, cdec) == nve::NativeViewCodecError::UnsupportedVersion);

  // Invalid bool byte (2)
  const std::vector<uint8_t> bad_bool = {'N', 'V', 'N', 'P', 1u, 1u, 0u, 1u, 0u, 1u, 2u};
  NVE_CHECK(nve::NvnpDecoder::parse(bad_bool, pdec) == nve::NativeViewCodecError::InvalidBool);

  // Invalid value tag (99)
  const std::vector<uint8_t> bad_tag = {'N', 'V', 'N', 'P', 1u, 1u, 0u, 1u, 0u, 99u};
  NVE_CHECK(nve::NvnpDecoder::parse(bad_tag, pdec) == nve::NativeViewCodecError::InvalidTag);

  // Invalid UTF-8 (overlong 0xC0 0xAF, surrogate 0xED 0xA0 0x80)
  const std::vector<uint8_t> bad_utf8 = {
      'N', 'V', 'N', 'P', 1u, 1u, 0u, 1u, 0u, 7u, 2u, 0u, 0u, 0u, 0xC0u, 0xAFu};
  NVE_CHECK(nve::NvnpDecoder::parse(bad_utf8, pdec) == nve::NativeViewCodecError::InvalidUtf8);
  const std::vector<uint8_t> surrogate = {
      'N', 'V', 'N', 'C', 1u, 1u, 0u, 1u, 7u, 3u, 0u, 0u, 0u, 0xEDu, 0xA0u, 0x80u};
  NVE_CHECK(nve::NvncDecoder::parse(surrogate, cdec) == nve::NativeViewCodecError::InvalidUtf8);

  // Trailing bytes rejected
  const std::vector<uint8_t> trailing_p = {'N', 'V', 'N', 'P', 1u, 0u, 0u, 0xFFu};
  const std::vector<uint8_t> trailing_c = {'N', 'V', 'N', 'C', 1u, 2u, 0u, 0u, 0x00u};
  NVE_CHECK(nve::NvnpDecoder::parse(trailing_p, pdec) == nve::NativeViewCodecError::TrailingBytes);
  NVE_CHECK(nve::NvncDecoder::parse(trailing_c, cdec) == nve::NativeViewCodecError::TrailingBytes);

  // OOM field/arg count bomb rejected
  const std::vector<uint8_t> bomb_p = {'N', 'V', 'N', 'P', 1u, 0xFFu, 0x7Fu};
  const std::vector<uint8_t> bomb_c = {'N', 'V', 'N', 'C', 1u, 1u, 0u, 200u};
  NVE_CHECK(nve::NvnpDecoder::parse(bomb_p, pdec) == nve::NativeViewCodecError::TooManyFields);
  NVE_CHECK(nve::NvncDecoder::parse(bomb_c, cdec) == nve::NativeViewCodecError::TooManyFields);
}

struct MockInstance {
  std::string kind;
  std::string last_url;
  int prop_updates{0};
  int reloads{0};
  bool unmounted{false};
};

void test_registry_wildcards_mount_and_commands() {
  nve::NativeViewRegistry reg;
  int wildcard_creates = 0;
  int exact_creates = 0;
  std::vector<MockInstance*> owned;

  nve::NativeViewAdapter web_wild{};
  web_wild.create = [&](uint32_t, std::string_view kind, const nve::NvnpDecoder*) -> void* {
    ++wildcard_creates;
    auto* m = new MockInstance{std::string(kind), "", 0, 0, false};
    owned.push_back(m);
    return m;
  };
  web_wild.unmount = [](void* inst, uint32_t) {
    static_cast<MockInstance*>(inst)->unmounted = true;
  };

  nve::NativeViewAdapter webview_exact{};
  webview_exact.create = [&](uint32_t, std::string_view kind, const nve::NvnpDecoder*) -> void* {
    ++exact_creates;
    auto* m = new MockInstance{std::string(kind), "", 0, 0, false};
    owned.push_back(m);
    return m;
  };
  webview_exact.update_props = [](void* inst, uint32_t, const nve::NvnpDecoder& dec) {
    nve::WebViewProps wp{};
    if (nve::decode_web_view_props(dec, wp) == nve::NativeViewCodecError::Ok) {
      auto* m = static_cast<MockInstance*>(inst);
      m->last_url = std::string(wp.url);
      ++m->prop_updates;
    }
  };
  webview_exact.dispatch_command =
      [](void* inst, uint32_t, std::string_view, const nve::NvncDecoder& dec) -> bool {
    nve::WebViewCommand cmd{};
    if (nve::decode_web_view_command(dec, cmd) != nve::NativeViewCodecError::Ok) return false;
    auto* m = static_cast<MockInstance*>(inst);
    if (cmd.kind == nve::WebViewCommandKind::Navigate) {
      m->last_url = std::string(cmd.text);
      return true;
    }
    if (cmd.kind == nve::WebViewCommandKind::Reload) {
      ++m->reloads;
      return true;
    }
    return false;
  };
  webview_exact.unmount = [](void* inst, uint32_t) {
    static_cast<MockInstance*>(inst)->unmounted = true;
  };

  reg.register_adapter("web.*", web_wild);
  reg.register_adapter(nve::kNativeKindWebView, webview_exact);

  NVE_CHECK(reg.supports("web.video"));
  NVE_CHECK(reg.supports("web.my-chart"));
  NVE_CHECK(!reg.supports("web."));
  NVE_CHECK(reg.supports(nve::kNativeKindWebView));
  NVE_CHECK(!reg.supports("android.map"));

  NvnpBuilder wv_props;
  wv_props.add_str(1u, "https://initial.nve.dev");
  wv_props.add_bool(2u, true);
  const auto wv_prop_bytes = wv_props.finish();

  // Exact match wins over the `web.*` wildcard; a mount with props runs create + update_props.
  nve::NativeViewCodecError err = nve::NativeViewCodecError::TrailingBytes;
  void* mounted = reg.mount_or_update(10u, nve::kNativeKindWebView, wv_prop_bytes, &err);
  NVE_CHECK(err == nve::NativeViewCodecError::Ok);
  auto* wv_inst = static_cast<MockInstance*>(reg.instance_for(10u));
  NVE_CHECK(mounted == wv_inst && wv_inst != nullptr && exact_creates == 1 && wildcard_creates == 0);
  NVE_CHECK(wv_inst->prop_updates == 1 && wv_inst->last_url == "https://initial.nve.dev");
  NVE_CHECK(reg.kind_for(10u) == nve::kNativeKindWebView);

  // Re-mounting the same kind updates in place (no second create).
  NVE_CHECK(reg.mount_or_update(10u, nve::kNativeKindWebView, wv_prop_bytes) == wv_inst);
  NVE_CHECK(exact_creates == 1 && wv_inst->prop_updates == 2);

  // Corrupt props are rejected with the codec error and never reach the adapter.
  const std::vector<uint8_t> bad_magic = {'N', 'V', 'X', 'P', 1u, 0u, 0u};
  NVE_CHECK(reg.mount_or_update(10u, nve::kNativeKindWebView, bad_magic, &err) == nullptr);
  NVE_CHECK(err == nve::NativeViewCodecError::InvalidMagic && wv_inst->prop_updates == 2);

  // NVNC Navigate + Reload commands reach the mounted instance.
  NvncBuilder nav(1u);
  nav.arg_str("https://navigated.nve.dev");
  const auto nav_bytes = nav.finish();
  NvncBuilder rel(2u);
  const auto rel_bytes = rel.finish();
  NVE_CHECK(reg.dispatch_command(10u, "navigate", nav_bytes, &err) && err == nve::NativeViewCodecError::Ok);
  NVE_CHECK(reg.dispatch_command(10u, "reload", rel_bytes));
  NVE_CHECK(wv_inst->last_url == "https://navigated.nve.dev" && wv_inst->reloads == 1);
  // Unknown node: no dispatch; truncated command: codec error.
  NVE_CHECK(!reg.dispatch_command(99u, "reload", rel_bytes));
  NVE_CHECK(!reg.dispatch_command(10u, "reload", std::span<const uint8_t>(rel_bytes.data(), 3u), &err));
  NVE_CHECK(err == nve::NativeViewCodecError::Truncated);

  // Wildcard mount + unmount check.
  void* winst = reg.mount_or_update(11u, "web.canvas", {});
  NVE_CHECK(winst != nullptr && wildcard_creates == 1 && reg.mounted_count() == 2u);
  NVE_CHECK(reg.unmount(10u) && wv_inst->unmounted && reg.mounted_count() == 1u);
  reg.unmount_all();
  NVE_CHECK(reg.mounted_count() == 0u);

  for (auto* m : owned) {
    NVE_CHECK(m->unmounted);
    delete m;
  }
}

} // namespace

int main() {
  test_nvnp_all_value_types_and_specs();
  test_corrupt_and_adversarial_wire_buffers();
  test_registry_wildcards_mount_and_commands();
  std::cout << "nve_native_view_test passed\n";
  return 0;
}
