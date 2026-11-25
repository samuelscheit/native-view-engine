#include "NveGtkNativeViews.hpp"
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

#if NVE_NV_ENABLE_BLUR || NVE_NV_ENABLE_VIDEO
bool approx_eq(double a, double b, double eps = 1.5) {
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

void test_pure_helpers() {
#if NVE_NV_ENABLE_WEBVIEW
  NVE_CHECK(nve::gtk::is_safe_gtk_webview_url("https://gtk.nve.dev"));
  NVE_CHECK(nve::gtk::is_safe_gtk_webview_url("http://localhost:9000"));
  NVE_CHECK(nve::gtk::is_safe_gtk_webview_url("about:blank"));
  NVE_CHECK(!nve::gtk::is_safe_gtk_webview_url("javascript:alert(1)"));
  NVE_CHECK(!nve::gtk::is_safe_gtk_webview_url("vbscript:msgbox(1)"));
  NVE_CHECK(!nve::gtk::is_safe_gtk_webview_url("data:text/html,hi"));
#endif
#if NVE_NV_ENABLE_VIDEO
  NVE_CHECK(nve::gtk::is_safe_gtk_media_uri("https://cdn.nve.dev/gtk.mp4"));
  NVE_CHECK(nve::gtk::is_safe_gtk_media_uri("file:///tmp/gtk.mp4"));
  NVE_CHECK(!nve::gtk::is_safe_gtk_media_uri("javascript:alert(1)"));
#endif
#if NVE_NV_ENABLE_BLUR
  const auto c = nve::gtk::unpack_gtk_tint_rrggbbaa(0x336699AAu);
  NVE_CHECK(c.r == 0x33u && c.g == 0x66u && c.b == 0x99u && c.a == 0xAAu);
  NVE_CHECK(approx_eq(nve::gtk::base_gtk_blur_radius(nve::BlurMaterial::Chrome), 24.0, 0.01));
#endif
}

void test_gtk_native_views_registry_and_toggles() {
#if NVE_NV_ENABLE_WEBVIEW
  struct SpyGtkWebBackend : nve::gtk::NveGtkWebViewBackend {
    GtkWidget* inner{gtk_label_new("webview")};
    std::string url;
    std::string script;
    int reloads{0};
    bool* closed{nullptr};
    GtkWidget* widget() override { return inner; }
    void set_allows_back_forward(bool) override {}
    bool navigate(std::string_view u) override {
      url.assign(u);
      return true;
    }
    void reload() override { ++reloads; }
    bool go_back() override { return true; }
    bool go_forward() override { return true; }
    void evaluate_script(std::string_view js) override { script.assign(js); }
    void stop_and_close() override {
      if (closed) *closed = true;
    }
  };
  bool web_closed = false;
#endif

#if NVE_NV_ENABLE_VIDEO
  struct SpyGtkVideoBackend : nve::gtk::NveGtkVideoBackend {
    GtkWidget* inner{gtk_label_new("video")};
    std::string uri;
    uint64_t seek{0u};
    float vol{1.0f};
    bool playing{false};
    bool* closed{nullptr};
    GtkWidget* widget() override { return inner; }
    void set_source_uri(std::string_view u) override { uri.assign(u); }
    void set_muted(bool) override {}
    void set_looping(bool) override {}
    void set_controls(bool) override {}
    void play() override { playing = true; }
    void pause() override { playing = false; }
    void seek_ms(uint64_t ms) override { seek = ms; }
    void set_volume(float v) override { vol = v; }
    void stop_and_close() override {
      if (closed) *closed = true;
    }
  };
  bool vid_closed = false;
#endif

  nve::NativeViewRegistry registry;
  nve::gtk::NveGtkNativeViewHooks hooks{};
#if NVE_NV_ENABLE_WEBVIEW
  hooks.create_webview_backend = [&](uint32_t) {
    auto b = std::make_unique<SpyGtkWebBackend>();
    b->closed = &web_closed;
    return b;
  };
#endif
#if NVE_NV_ENABLE_VIDEO
  hooks.create_video_backend = [&](uint32_t) {
    auto b = std::make_unique<SpyGtkVideoBackend>();
    b->closed = &vid_closed;
    return b;
  };
#endif
  nve::gtk::register_gtk_native_views(registry, std::move(hooks));

  NVE_CHECK(registry.supports(nve::kNativeKindBlurSurface) == (NVE_NV_ENABLE_BLUR != 0));
  NVE_CHECK(registry.supports(nve::kNativeKindBlurAlias) == (NVE_NV_ENABLE_BLUR != 0));
  NVE_CHECK(registry.supports(nve::kNativeKindWebView) == (NVE_NV_ENABLE_WEBVIEW != 0));
  NVE_CHECK(registry.supports(nve::kNativeKindWebViewAlias) == (NVE_NV_ENABLE_WEBVIEW != 0));
  NVE_CHECK(registry.supports(nve::kNativeKindVideoPlayer) == (NVE_NV_ENABLE_VIDEO != 0));
  NVE_CHECK(registry.supports(nve::kNativeKindVideoAlias) == (NVE_NV_ENABLE_VIDEO != 0));

  NveSurfaceConfig cfg{.surface_id = 1u, .width = 640.0f, .height = 420.0f, .scale_factor = 1.0f};
  NveFabricHostHandle host = nve_fabric_host_create(cfg);
  NVE_CHECK(host != nullptr);
  nve::gtk::bind_gtk_native_views_to_fabric_host(host, registry);

  nve::CommandBufferEncoder enc(1u, 1u);
  std::vector<uint32_t> children;

#if NVE_NV_ENABLE_BLUR
  NvnpBuilder blur_p;
  blur_p.add_u32(1u, static_cast<uint32_t>(nve::BlurMaterial::Chrome));
  blur_p.add_f32(2u, 0.75f);
  blur_p.add_u32(3u, 0x336699AAu);
  enc.create_node(30u, NVE_COMP_CUSTOM, nve::kNativeKindBlurSurface);
  enc.set_raw_prop_string(30u, nve::kNativeViewKindPropKey, nve::kNativeKindBlurSurface);
  enc.set_raw_prop_bytes(30u, nve::kNativeViewPropsPropKey, blur_p.finish());
  children.push_back(30u);
#endif

#if NVE_NV_ENABLE_WEBVIEW
  NvnpBuilder web_p;
  web_p.add_str(1u, "https://gtk-init.nve.dev");
  web_p.add_bool(2u, true);
  enc.create_node(31u, NVE_COMP_CUSTOM, nve::kNativeKindWebViewAlias);
  enc.set_raw_prop_string(31u, nve::kNativeViewKindPropKey, nve::kNativeKindWebViewAlias);
  enc.set_raw_prop_bytes(31u, nve::kNativeViewPropsPropKey, web_p.finish());
  children.push_back(31u);
#endif

#if NVE_NV_ENABLE_VIDEO
  NvnpBuilder vid_p;
  vid_p.add_str(1u, "https://cdn.nve.dev/gtk.mp4");
  vid_p.add_bool(2u, true);
  vid_p.add_bool(3u, false);
  vid_p.add_bool(4u, true);
  vid_p.add_bool(5u, true);
  enc.create_node(32u, NVE_COMP_CUSTOM, nve::kNativeKindVideoPlayer);
  enc.set_raw_prop_string(32u, nve::kNativeViewKindPropKey, nve::kNativeKindVideoPlayer);
  enc.set_raw_prop_bytes(32u, nve::kNativeViewPropsPropKey, vid_p.finish());
  children.push_back(32u);
#endif

  enc.set_children(NVE_ROOT_NODE_ID, children);
  enc.commit(true);
  NVE_CHECK(nve_fabric_host_submit_frame(host, enc.finish(), nullptr) == 0);

#if NVE_NV_ENABLE_BLUR
  auto* blur = nve::gtk::blur_surface_for_node(registry, 30u);
  NVE_CHECK(blur != nullptr && blur->widget() != nullptr && GTK_IS_BOX(blur->widget()));
  NVE_CHECK(nve::gtk::native_widget_for_node(registry, 30u) == blur->widget());
  NVE_CHECK(blur->material() == nve::BlurMaterial::Chrome);
  NVE_CHECK(approx_eq(blur->blur_radius(), 18.0, 0.1));
  NVE_CHECK(blur->active_css().find("rgba(51, 102, 153,") != std::string::npos);
  NVE_CHECK(blur->active_css().find("filter: blur") == std::string::npos);
#endif

#if NVE_NV_ENABLE_WEBVIEW
  auto* wv = nve::gtk::web_view_for_node(registry, 31u);
  NVE_CHECK(wv != nullptr && wv->widget() != nullptr && GTK_IS_BOX(wv->widget()));
  NVE_CHECK(wv->last_requested_url() == "https://gtk-init.nve.dev");

  NvncBuilder bad_nav(1u);
  bad_nav.arg_str("data:text/html,<svg>");
  NVE_CHECK(!registry.dispatch_command(31u, "navigate", bad_nav.finish()));
#endif

#if NVE_NV_ENABLE_VIDEO
  auto* vid = nve::gtk::video_player_for_node(registry, 32u);
  NVE_CHECK(vid != nullptr && vid->widget() != nullptr && GTK_IS_BOX(vid->widget()));
  NVE_CHECK(vid->source_uri() == "https://cdn.nve.dev/gtk.mp4" && vid->is_playing());
#endif

#if NVE_NV_ENABLE_WEBVIEW || NVE_NV_ENABLE_VIDEO
  nve::CommandBufferEncoder enc2(1u, 2u);
#if NVE_NV_ENABLE_WEBVIEW
  NvncBuilder ok_nav(1u);
  ok_nav.arg_str("https://gtk-next.nve.dev");
  NvncBuilder rel_c(2u);
  NvncBuilder eval_c(5u);
  eval_c.arg_str("window.ok = 1");
  enc2.dispatch_command(31u, "navigate", ok_nav.finish());
  enc2.dispatch_command(31u, "reload", rel_c.finish());
  enc2.dispatch_command(31u, "evaluate_script", eval_c.finish());
#endif
#if NVE_NV_ENABLE_VIDEO
  NvncBuilder pause_c(2u);
  NvncBuilder seek_c(3u);
  seek_c.arg_u64(7777ULL);
  NvncBuilder vol_c(4u);
  vol_c.arg_f32(0.6f);
  enc2.dispatch_command(32u, "pause", pause_c.finish());
  enc2.dispatch_command(32u, "seek_ms", seek_c.finish());
  enc2.dispatch_command(32u, "set_volume", vol_c.finish());
#endif
  enc2.commit(false);
  NVE_CHECK(nve_fabric_host_submit_frame(host, enc2.finish(), nullptr) == 0);
#endif

#if NVE_NV_ENABLE_WEBVIEW
  auto* spy_web = static_cast<SpyGtkWebBackend*>(wv->backend());
  NVE_CHECK(wv->last_requested_url() == "https://gtk-next.nve.dev" && spy_web->reloads == 1);
  NVE_CHECK(spy_web->script == "window.ok = 1");

  nve::NativeViewRegistry empty_reg;
  nve::gtk::register_gtk_native_views(empty_reg);
  NVE_CHECK(empty_reg.mount_or_update(99u, nve::kNativeKindWebView, web_p.finish()) == nullptr);
#endif
#if NVE_NV_ENABLE_VIDEO
  auto* spy_vid = static_cast<SpyGtkVideoBackend*>(vid->backend());
  NVE_CHECK(!vid->is_playing() && spy_vid->seek == 7777ULL && approx_eq(spy_vid->vol, 0.6, 1e-4));
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

int main() {
  test_pure_helpers();
  if (!gtk_init_check()) {
    std::cout << "nve_gtk_native_views_test partial pass: pure helpers OK, GTK4 display unavailable\n";
    return 0;
  }
  test_gtk_native_views_registry_and_toggles();
  std::cout << "nve_gtk_native_views_test passed (blur=" << NVE_NV_ENABLE_BLUR
            << ", webview=" << NVE_NV_ENABLE_WEBVIEW << ", video=" << NVE_NV_ENABLE_VIDEO << ")\n";
  return 0;
}
