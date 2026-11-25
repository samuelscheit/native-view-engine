#include "NveQtNativeViews.hpp"
#include "command_encoder.hpp"

#include <QApplication>
#if NVE_NV_ENABLE_BLUR
#include <QColor>
#endif
#include <QWidget>

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

#if NVE_NV_ENABLE_BLUR
bool approx_eq(double a, double b, double eps = 2.0) {
  return std::fabs(a - b) <= eps;
}
#endif

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

void test_qt_native_views_registry_and_toggles() {
#if NVE_NV_ENABLE_WEBVIEW
  NVE_CHECK(nve::qt::is_safe_qt_webview_url("https://qt.nve.dev/app"));
  NVE_CHECK(nve::qt::is_safe_qt_webview_url("http://127.0.0.1:3000"));
  NVE_CHECK(nve::qt::is_safe_qt_webview_url("about:blank"));
  NVE_CHECK(!nve::qt::is_safe_qt_webview_url("javascript:alert(1)"));
  NVE_CHECK(!nve::qt::is_safe_qt_webview_url("  vbscript:msgbox(1) "));
  NVE_CHECK(!nve::qt::is_safe_qt_webview_url("data:text/html,<b>x</b>"));

  struct SpyQtWebBackend : nve::qt::NveQtWebViewBackend {
    QWidget* inner{new QWidget()};
    std::string url;
    std::string script;
    int reloads{0};
    bool* closed{nullptr};
    QWidget* widget() override { return inner; }
    void setAllowsBackForward(bool) override {}
    bool navigate(std::string_view u) override {
      url.assign(u);
      return true;
    }
    void reload() override { ++reloads; }
    bool goBack() override { return true; }
    bool goForward() override { return true; }
    void evaluateScript(std::string_view js) override { script.assign(js); }
    void stopAndClose() override {
      if (closed) *closed = true;
    }
  };
  bool web_closed = false;
#endif

#if NVE_NV_ENABLE_VIDEO
  NVE_CHECK(nve::qt::is_safe_qt_media_uri("https://cdn.nve.dev/qt.mp4"));
  NVE_CHECK(!nve::qt::is_safe_qt_media_uri("javascript:alert(1)"));

  struct SpyQtVideoBackend : nve::qt::NveQtVideoBackend {
    QWidget* inner{new QWidget()};
    std::string uri;
    uint64_t seek{0u};
    float vol{1.0f};
    bool playing{false};
    bool* closed{nullptr};
    QWidget* widget() override { return inner; }
    void setSourceUri(std::string_view u) override { uri.assign(u); }
    void setMuted(bool) override {}
    void setLooping(bool) override {}
    void setControls(bool) override {}
    void play() override { playing = true; }
    void pause() override { playing = false; }
    void seekMs(uint64_t ms) override { seek = ms; }
    void setVolume(float v) override { vol = v; }
    void stopAndClose() override {
      if (closed) *closed = true;
    }
  };
  bool vid_closed = false;
#endif

  nve::NativeViewRegistry registry;
  nve::qt::NveQtNativeViewHooks hooks{};
#if NVE_NV_ENABLE_WEBVIEW
  hooks.create_webview_backend = [&](uint32_t) {
    auto b = std::make_unique<SpyQtWebBackend>();
    b->closed = &web_closed;
    return b;
  };
#endif
#if NVE_NV_ENABLE_VIDEO
  hooks.create_video_backend = [&](uint32_t) {
    auto b = std::make_unique<SpyQtVideoBackend>();
    b->closed = &vid_closed;
    return b;
  };
#endif
  nve::qt::register_qt_native_views(registry, std::move(hooks));

  NVE_CHECK(registry.supports(nve::kNativeKindBlurSurface) == (NVE_NV_ENABLE_BLUR != 0));
  NVE_CHECK(registry.supports(nve::kNativeKindBlurAlias) == (NVE_NV_ENABLE_BLUR != 0));
  NVE_CHECK(registry.supports(nve::kNativeKindWebView) == (NVE_NV_ENABLE_WEBVIEW != 0));
  NVE_CHECK(registry.supports(nve::kNativeKindWebViewAlias) == (NVE_NV_ENABLE_WEBVIEW != 0));
  NVE_CHECK(registry.supports(nve::kNativeKindVideoPlayer) == (NVE_NV_ENABLE_VIDEO != 0));
  NVE_CHECK(registry.supports(nve::kNativeKindVideoAlias) == (NVE_NV_ENABLE_VIDEO != 0));

  NveSurfaceConfig cfg{.surface_id = 1u, .width = 640.0f, .height = 420.0f, .scale_factor = 1.0f};
  NveFabricHostHandle host = nve_fabric_host_create(cfg);
  NVE_CHECK(host != nullptr);
  nve::qt::bind_qt_native_views_to_fabric_host(host, registry);

  nve::CommandBufferEncoder enc(1u, 1u);
  std::vector<uint32_t> children;

#if NVE_NV_ENABLE_BLUR
  NvnpBuilder blur_p;
  blur_p.add_u32(1u, static_cast<uint32_t>(nve::BlurMaterial::Thick));
  blur_p.add_f32(2u, 0.5f);
  blur_p.add_u32(3u, 0x224466CCu);
  enc.create_node(20u, NVE_COMP_CUSTOM, nve::kNativeKindBlurAlias);
  enc.set_raw_prop_string(20u, nve::kNativeViewKindPropKey, nve::kNativeKindBlurAlias);
  enc.set_raw_prop_bytes(20u, nve::kNativeViewPropsPropKey, blur_p.finish());
  children.push_back(20u);
#endif

#if NVE_NV_ENABLE_WEBVIEW
  NvnpBuilder web_p;
  web_p.add_str(1u, "https://qt-initial.nve.dev");
  web_p.add_bool(2u, true);
  enc.create_node(21u, NVE_COMP_CUSTOM, nve::kNativeKindWebView);
  enc.set_raw_prop_string(21u, nve::kNativeViewKindPropKey, nve::kNativeKindWebView);
  enc.set_raw_prop_bytes(21u, nve::kNativeViewPropsPropKey, web_p.finish());
  children.push_back(21u);
#endif

#if NVE_NV_ENABLE_VIDEO
  NvnpBuilder vid_p;
  vid_p.add_str(1u, "https://cdn.nve.dev/qt.mp4");
  vid_p.add_bool(2u, false);
  vid_p.add_bool(3u, true);
  vid_p.add_bool(4u, true);
  vid_p.add_bool(5u, true);
  enc.create_node(22u, NVE_COMP_CUSTOM, nve::kNativeKindVideoPlayer);
  enc.set_raw_prop_string(22u, nve::kNativeViewKindPropKey, nve::kNativeKindVideoPlayer);
  enc.set_raw_prop_bytes(22u, nve::kNativeViewPropsPropKey, vid_p.finish());
  children.push_back(22u);
#endif

  enc.set_children(NVE_ROOT_NODE_ID, children);
  enc.commit(true);
  NVE_CHECK(nve_fabric_host_submit_frame(host, enc.finish(), nullptr) == 0);

#if NVE_NV_ENABLE_BLUR
  auto* blur = nve::qt::blur_surface_for_node(registry, 20u);
  NVE_CHECK(blur != nullptr && nve::qt::native_widget_for_node(registry, 20u) == blur);
  NVE_CHECK(blur->blurMaterial() == nve::BlurMaterial::Thick);
  NVE_CHECK(approx_eq(blur->blurRadius(), 15.0, 0.1));
  NVE_CHECK(blur->graphicsEffect() == nullptr);
  NVE_CHECK(blur->hasTint() && blur->effectiveTintColor() == QColor(0x22, 0x44, 0x66, 0xCC));

  nve::NativeViewCodecError codec_err = nve::NativeViewCodecError::Ok;
  const std::vector<uint8_t> bad_magic = {'X', 'X', 'X', 'X', 1u, 0u, 0u};
  NVE_CHECK(registry.mount_or_update(20u, nve::kNativeKindBlurSurface, bad_magic, &codec_err) == nullptr);
  NVE_CHECK(codec_err == nve::NativeViewCodecError::InvalidMagic);
#endif

#if NVE_NV_ENABLE_WEBVIEW
  auto* wv = nve::qt::web_view_for_node(registry, 21u);
  NVE_CHECK(wv != nullptr && wv->lastRequestedUrl() == "https://qt-initial.nve.dev");

  NvncBuilder nav_bad(1u);
  nav_bad.arg_str("javascript:alert(1)");
  NVE_CHECK(!registry.dispatch_command(21u, "navigate", nav_bad.finish()));
#endif

#if NVE_NV_ENABLE_VIDEO
  auto* vid = nve::qt::video_player_for_node(registry, 22u);
  NVE_CHECK(vid != nullptr && vid->sourceUri() == "https://cdn.nve.dev/qt.mp4");
  NVE_CHECK(!vid->isPlaying() && vid->muted() && vid->looping() && vid->controls());
#endif

#if NVE_NV_ENABLE_WEBVIEW || NVE_NV_ENABLE_VIDEO
  nve::CommandBufferEncoder enc2(1u, 2u);
#if NVE_NV_ENABLE_WEBVIEW
  NvncBuilder nav_ok(1u);
  nav_ok.arg_str("https://qt-next.nve.dev");
  NvncBuilder reload_c(2u);
  NvncBuilder eval_c(5u);
  eval_c.arg_str("1 + 1");
  enc2.dispatch_command(21u, "navigate", nav_ok.finish());
  enc2.dispatch_command(21u, "reload", reload_c.finish());
  enc2.dispatch_command(21u, "evaluate_script", eval_c.finish());
#endif
#if NVE_NV_ENABLE_VIDEO
  NvncBuilder play_c(1u);
  NvncBuilder seek_c(3u);
  seek_c.arg_u64(9000ULL);
  NvncBuilder vol_c(4u);
  vol_c.arg_f32(0.75f);
  enc2.dispatch_command(22u, "play", play_c.finish());
  enc2.dispatch_command(22u, "seek_ms", seek_c.finish());
  enc2.dispatch_command(22u, "set_volume", vol_c.finish());
#endif
  enc2.commit(false);
  NVE_CHECK(nve_fabric_host_submit_frame(host, enc2.finish(), nullptr) == 0);
#endif

#if NVE_NV_ENABLE_WEBVIEW
  auto* spy_web = static_cast<SpyQtWebBackend*>(wv->backend());
  NVE_CHECK(wv->lastRequestedUrl() == "https://qt-next.nve.dev" && spy_web->reloads == 1);
  NVE_CHECK(spy_web->script == "1 + 1");

  nve::NativeViewRegistry empty_reg;
  nve::qt::register_qt_native_views(empty_reg);
  NVE_CHECK(empty_reg.mount_or_update(99u, nve::kNativeKindWebView, web_p.finish()) == nullptr);
#endif
#if NVE_NV_ENABLE_VIDEO
  auto* spy_vid = static_cast<SpyQtVideoBackend*>(vid->backend());
  NVE_CHECK(vid->isPlaying() && spy_vid->playing && spy_vid->seek == 9000ULL);
  NVE_CHECK(std::fabs(spy_vid->vol - 0.75f) < 1e-5f);
#endif

  registry.unmount_all();
  NVE_CHECK(registry.mounted_count() == 0u);
#if NVE_NV_ENABLE_WEBVIEW
  NVE_CHECK(web_closed);
#endif
#if NVE_NV_ENABLE_VIDEO
  NVE_CHECK(vid_closed);
#endif
  nve_fabric_host_destroy(host);
}

} // namespace

int main(int argc, char* argv[]) {
  QApplication app(argc, argv);
  test_qt_native_views_registry_and_toggles();
  std::cout << "nve_qt_native_views_test passed (blur=" << NVE_NV_ENABLE_BLUR
            << ", webview=" << NVE_NV_ENABLE_WEBVIEW << ", video=" << NVE_NV_ENABLE_VIDEO << ")\n";
  return 0;
}
