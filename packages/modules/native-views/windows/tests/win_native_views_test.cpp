#include "NveWinNativeViews.hpp"
#include "command_encoder.hpp"

#include <cmath>
#include <cstdlib>
#include <iostream>
#include <memory>
#include <string>
#include <string_view>
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
  void add_f32(uint16_t id, float v) {
    push_le<uint16_t>(buf, id);
    buf.push_back(5u);
    push_le<float>(buf, v);
    ++count;
  }
  void add_str(uint16_t id, std::string_view s) {
    push_le<uint16_t>(buf, id);
    buf.push_back(7u);
    push_le<uint32_t>(buf, static_cast<uint32_t>(s.size()));
    buf.insert(buf.end(), s.begin(), s.end());
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
  explicit NvncBuilder(uint16_t op) {
    buf.insert(buf.end(), nve::kNativeCmdMagic.begin(), nve::kNativeCmdMagic.end());
    buf.push_back(nve::kNativeCmdVersion);
    push_le<uint16_t>(buf, op);
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

void test_windows_native_views_registry_and_toggles() {
#if NVE_NV_ENABLE_WEBVIEW
  NVE_CHECK(nve::win::is_safe_win_webview_url("https://nve.dev/docs"));
  NVE_CHECK(nve::win::is_safe_win_webview_url("http://localhost:8080"));
  NVE_CHECK(nve::win::is_safe_win_webview_url("  ABOUT:BLANK \r\n"));
  NVE_CHECK(!nve::win::is_safe_win_webview_url("javascript:alert(1)"));
  NVE_CHECK(!nve::win::is_safe_win_webview_url("  JaVa\tScRiPt:alert(1)"));
  NVE_CHECK(!nve::win::is_safe_win_webview_url("vbscript:msgbox(1)"));
  NVE_CHECK(!nve::win::is_safe_win_webview_url("data:text/html,<script>alert(1)</script>"));
  NVE_CHECK(!nve::win::is_safe_win_webview_url(""));

  struct SpyWebBackend : nve::win::NveWinWebViewBackend {
    std::string last_url;
    std::string last_script;
    int reloads{0};
    int backs{0};
    int forwards{0};
    bool allows_bf{true};
    bool* closed_flag{nullptr};
    void set_allows_back_forward(bool a) override { allows_bf = a; }
    bool navigate(std::string_view u) override {
      last_url.assign(u);
      return true;
    }
    void reload() override { ++reloads; }
    bool go_back() override {
      ++backs;
      return true;
    }
    bool go_forward() override {
      ++forwards;
      return true;
    }
    void evaluate_script(std::string_view js) override { last_script.assign(js); }
    void close() override {
      if (closed_flag) *closed_flag = true;
    }
  };
  bool web_closed = false;
#endif

#if NVE_NV_ENABLE_VIDEO
  NVE_CHECK(nve::win::is_safe_win_media_uri("https://cdn.nve.dev/a.mp4"));
  NVE_CHECK(nve::win::is_safe_win_media_uri("file:///tmp/a.mp4"));
  NVE_CHECK(!nve::win::is_safe_win_media_uri("javascript:alert(1)"));

  struct SpyVideoBackend : nve::win::NveWinVideoBackend {
    std::string uri;
    bool muted{false};
    bool looping{false};
    bool controls{true};
    bool playing{false};
    uint64_t seek_target{0u};
    float vol{1.0f};
    bool* closed_flag{nullptr};
    void set_source_uri(std::string_view u) override { uri.assign(u); }
    void set_muted(bool m) override { muted = m; }
    void set_looping(bool l) override { looping = l; }
    void set_controls(bool c) override { controls = c; }
    void play() override { playing = true; }
    void pause() override { playing = false; }
    void seek_ms(uint64_t ms) override { seek_target = ms; }
    void set_volume(float v) override { vol = v; }
    void close() override {
      if (closed_flag) *closed_flag = true;
    }
  };
  bool video_closed = false;
#endif

  nve::NativeViewRegistry registry;
  nve::win::NveWinNativeViewHooks hooks{};
#if NVE_NV_ENABLE_WEBVIEW
  hooks.create_webview_backend = [&](uint32_t) {
    auto b = std::make_unique<SpyWebBackend>();
    b->closed_flag = &web_closed;
    return b;
  };
#endif
#if NVE_NV_ENABLE_VIDEO
  hooks.create_video_backend = [&](uint32_t) {
    auto b = std::make_unique<SpyVideoBackend>();
    b->closed_flag = &video_closed;
    return b;
  };
#endif
  nve::win::register_win_native_views(registry, std::move(hooks));

  // Verify compile-time per-view toggle registration invariants.
  NVE_CHECK(registry.supports(nve::kNativeKindBlurSurface) == (NVE_NV_ENABLE_BLUR != 0));
  NVE_CHECK(registry.supports(nve::kNativeKindBlurAlias) == (NVE_NV_ENABLE_BLUR != 0));
  NVE_CHECK(registry.supports(nve::kNativeKindWebView) == (NVE_NV_ENABLE_WEBVIEW != 0));
  NVE_CHECK(registry.supports(nve::kNativeKindWebViewAlias) == (NVE_NV_ENABLE_WEBVIEW != 0));
  NVE_CHECK(registry.supports(nve::kNativeKindVideoPlayer) == (NVE_NV_ENABLE_VIDEO != 0));
  NVE_CHECK(registry.supports(nve::kNativeKindVideoAlias) == (NVE_NV_ENABLE_VIDEO != 0));

  NveSurfaceConfig cfg{.surface_id = 1u, .width = 640.0f, .height = 420.0f, .scale_factor = 1.0f};
  NveFabricHostHandle host = nve_fabric_host_create(cfg);
  NVE_CHECK(host != nullptr);
  nve::win::bind_win_native_views_to_fabric_host(host, registry);

  nve::CommandBufferEncoder enc(1u, 1u);
  std::vector<uint32_t> children;

#if NVE_NV_ENABLE_BLUR
  NvnpBuilder blur_props;
  blur_props.add_u32(1u, static_cast<uint32_t>(nve::BlurMaterial::Thin));
  blur_props.add_f32(2u, 0.8f);
  blur_props.add_u32(3u, 0x10203080u);
  enc.create_node(10u, NVE_COMP_CUSTOM, nve::kNativeKindBlurSurface);
  enc.set_raw_prop_string(10u, nve::kNativeViewKindPropKey, nve::kNativeKindBlurSurface);
  enc.set_raw_prop_bytes(10u, nve::kNativeViewPropsPropKey, blur_props.finish());
  children.push_back(10u);
#endif

#if NVE_NV_ENABLE_WEBVIEW
  NvnpBuilder wv_props;
  wv_props.add_str(1u, "https://initial.nve.dev");
  wv_props.add_bool(2u, true);
  enc.create_node(11u, NVE_COMP_CUSTOM, nve::kNativeKindWebView);
  enc.set_raw_prop_string(11u, nve::kNativeViewKindPropKey, nve::kNativeKindWebView);
  enc.set_raw_prop_bytes(11u, nve::kNativeViewPropsPropKey, wv_props.finish());
  children.push_back(11u);
#endif

#if NVE_NV_ENABLE_VIDEO
  NvnpBuilder vid_props;
  vid_props.add_str(1u, "https://cdn.nve.dev/demo.mp4");
  vid_props.add_bool(2u, true);
  vid_props.add_bool(3u, true);
  vid_props.add_bool(4u, true);
  vid_props.add_bool(5u, false);
  enc.create_node(12u, NVE_COMP_CUSTOM, nve::kNativeKindVideoAlias);
  enc.set_raw_prop_string(12u, nve::kNativeViewKindPropKey, nve::kNativeKindVideoAlias);
  enc.set_raw_prop_bytes(12u, nve::kNativeViewPropsPropKey, vid_props.finish());
  children.push_back(12u);
#endif

  enc.set_children(NVE_ROOT_NODE_ID, children);
  enc.commit(true);
  NVE_CHECK(nve_fabric_host_submit_frame(host, enc.finish(), nullptr) == 0);

#if NVE_NV_ENABLE_BLUR
  auto* blur = nve::win::blur_surface_for_node(registry, 10u);
  NVE_CHECK(blur != nullptr);
  NVE_CHECK(blur->material() == nve::BlurMaterial::Thin);
  NVE_CHECK(blur->backdrop_type() == nve::win::WinSystemBackdropType::TransientWindow);
  NVE_CHECK(std::fabs(blur->intensity() - 0.8f) < 1e-5f);
  NVE_CHECK(blur->has_tint() && blur->tint_rgba() == 0x10203080u);
  NVE_CHECK(blur->tint_color().r8 == 0x10u && blur->tint_color().a8 == 0x80u);
#endif

#if NVE_NV_ENABLE_WEBVIEW
  auto* wv = nve::win::web_view_for_node(registry, 11u);
  NVE_CHECK(wv != nullptr && wv->last_requested_url() == "https://initial.nve.dev");
  NVE_CHECK(wv->allows_back_forward());

  NvncBuilder nav_bad(1u);
  nav_bad.arg_str("javascript:evil()");
  NVE_CHECK(!registry.dispatch_command(11u, "navigate", nav_bad.finish()));
  NVE_CHECK(wv->last_requested_url() == "https://initial.nve.dev");
#endif

#if NVE_NV_ENABLE_VIDEO
  auto* vid = nve::win::video_player_for_node(registry, 12u);
  NVE_CHECK(vid != nullptr && vid->source_uri() == "https://cdn.nve.dev/demo.mp4");
  NVE_CHECK(vid->is_playing() && vid->muted() && vid->looping() && !vid->controls());
#endif

#if NVE_NV_ENABLE_WEBVIEW || NVE_NV_ENABLE_VIDEO
  nve::CommandBufferEncoder enc2(1u, 2u);
#if NVE_NV_ENABLE_WEBVIEW
  NvncBuilder nav_ok(1u);
  nav_ok.arg_str("https://docs.nve.dev");
  NvncBuilder rel(2u);
  NvncBuilder back(3u);
  NvncBuilder fwd(4u);
  NvncBuilder eval(5u);
  eval.arg_str("window.__nveReady = true;");
  enc2.dispatch_command(11u, "navigate", nav_ok.finish());
  enc2.dispatch_command(11u, "reload", rel.finish());
  enc2.dispatch_command(11u, "go_back", back.finish());
  enc2.dispatch_command(11u, "go_forward", fwd.finish());
  enc2.dispatch_command(11u, "evaluate_script", eval.finish());
#endif
#if NVE_NV_ENABLE_VIDEO
  NvncBuilder pause_cmd(2u);
  NvncBuilder seek_cmd(3u);
  seek_cmd.arg_u64(12345ULL);
  NvncBuilder vol_cmd(4u);
  vol_cmd.arg_f32(0.35f);
  enc2.dispatch_command(12u, "pause", pause_cmd.finish());
  enc2.dispatch_command(12u, "seek_ms", seek_cmd.finish());
  enc2.dispatch_command(12u, "set_volume", vol_cmd.finish());
#endif
  enc2.commit(false);
  NVE_CHECK(nve_fabric_host_submit_frame(host, enc2.finish(), nullptr) == 0);
#endif

#if NVE_NV_ENABLE_WEBVIEW
  auto* spy_web = static_cast<SpyWebBackend*>(wv->backend());
  NVE_CHECK(wv->last_requested_url() == "https://docs.nve.dev");
  NVE_CHECK(spy_web->reloads == 1 && spy_web->backs == 1 && spy_web->forwards == 1);
  NVE_CHECK(spy_web->last_script == "window.__nveReady = true;");

  NvnpBuilder wv_no_bf;
  wv_no_bf.add_str(1u, "https://docs.nve.dev");
  wv_no_bf.add_bool(2u, false);
  NVE_CHECK(registry.mount_or_update(11u, nve::kNativeKindWebView, wv_no_bf.finish()) != nullptr);
  NVE_CHECK(!registry.dispatch_command(11u, "go_back", back.finish()));
  NVE_CHECK(!registry.dispatch_command(11u, "go_forward", fwd.finish()));

  // Without an injected backend factory, mounting WebView explicitly returns nullptr.
  nve::NativeViewRegistry empty_reg;
  nve::win::register_win_native_views(empty_reg);
  NVE_CHECK(empty_reg.mount_or_update(99u, nve::kNativeKindWebView, wv_no_bf.finish()) == nullptr);
#endif

#if NVE_NV_ENABLE_VIDEO
  auto* spy_vid = static_cast<SpyVideoBackend*>(vid->backend());
  NVE_CHECK(!vid->is_playing() && !spy_vid->playing);
  NVE_CHECK(spy_vid->seek_target == 12345ULL && std::fabs(spy_vid->vol - 0.35f) < 1e-5f);
#endif

  registry.unmount_all();
  NVE_CHECK(registry.mounted_count() == 0u);
#if NVE_NV_ENABLE_WEBVIEW
  NVE_CHECK(web_closed);
#endif
#if NVE_NV_ENABLE_VIDEO
  NVE_CHECK(video_closed);
#endif
  nve_fabric_host_destroy(host);
}

} // namespace

int main() {
  test_windows_native_views_registry_and_toggles();
  std::cout << "nve_win_native_views_test passed (blur=" << NVE_NV_ENABLE_BLUR
            << ", webview=" << NVE_NV_ENABLE_WEBVIEW << ", video=" << NVE_NV_ENABLE_VIDEO << ")\n";
  return 0;
}
