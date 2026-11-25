#import "NveAppleNativeViews.h"

#include "command_encoder.hpp"

#include <cmath>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <iostream>
#include <limits>
#include <optional>
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

#if NVE_APPLE_HAS_BLUR || NVE_APPLE_HAS_WEBVIEW || NVE_APPLE_HAS_VIDEO
void push_u16(std::vector<uint8_t> &b, uint16_t v) {
  b.push_back(static_cast<uint8_t>(v & 0xFFu));
  b.push_back(static_cast<uint8_t>((v >> 8u) & 0xFFu));
}

void push_u32(std::vector<uint8_t> &b, uint32_t v) {
  for (int i = 0; i < 4; ++i) {
    b.push_back(static_cast<uint8_t>((v >> (i * 8)) & 0xFFu));
  }
}
#endif

#if NVE_APPLE_HAS_VIDEO
void push_u64(std::vector<uint8_t> &b, uint64_t v) {
  for (int i = 0; i < 8; ++i) {
    b.push_back(static_cast<uint8_t>((v >> (i * 8)) & 0xFFu));
  }
}
#endif

#if NVE_APPLE_HAS_BLUR || NVE_APPLE_HAS_VIDEO
void push_f32(std::vector<uint8_t> &b, float v) {
  uint32_t bits = 0u;
  std::memcpy(&bits, &v, sizeof(bits));
  push_u32(b, bits);
}
#endif

#if NVE_APPLE_HAS_BLUR
std::vector<uint8_t> encode_blur_props(
    nve::BlurMaterial mat,
    float intensity,
    std::optional<uint32_t> tint) {
  std::vector<uint8_t> buf = {'N', 'V', 'N', 'P', 1u, 3u, 0u};
  push_u16(buf, 1u);
  buf.push_back(2u);
  push_u32(buf, static_cast<uint32_t>(mat));
  push_u16(buf, 2u);
  buf.push_back(5u);
  push_f32(buf, intensity);
  push_u16(buf, 3u);
  if (tint.has_value()) {
    buf.push_back(2u);
    push_u32(buf, *tint);
  } else {
    buf.push_back(0u);
  }
  return buf;
}
#endif

#if NVE_APPLE_HAS_WEBVIEW
std::vector<uint8_t> encode_webview_props(std::string_view url, bool allows_bf) {
  std::vector<uint8_t> buf = {'N', 'V', 'N', 'P', 1u, 2u, 0u};
  push_u16(buf, 1u);
  buf.push_back(7u);
  push_u32(buf, static_cast<uint32_t>(url.size()));
  buf.insert(buf.end(), url.begin(), url.end());
  push_u16(buf, 2u);
  buf.push_back(1u);
  buf.push_back(allows_bf ? 1u : 0u);
  return buf;
}

std::vector<uint8_t> encode_cmd_str(uint16_t op, std::string_view arg) {
  std::vector<uint8_t> buf = {'N', 'V', 'N', 'C', 1u};
  push_u16(buf, op);
  buf.push_back(1u);
  buf.push_back(7u);
  push_u32(buf, static_cast<uint32_t>(arg.size()));
  buf.insert(buf.end(), arg.begin(), arg.end());
  return buf;
}
#endif

#if NVE_APPLE_HAS_WEBVIEW || NVE_APPLE_HAS_VIDEO
std::vector<uint8_t> encode_cmd_no_args(uint16_t op) {
  std::vector<uint8_t> buf = {'N', 'V', 'N', 'C', 1u};
  push_u16(buf, op);
  buf.push_back(0u);
  return buf;
}
#endif

#if NVE_APPLE_HAS_VIDEO
std::vector<uint8_t> encode_video_props(
    std::string_view uri,
    bool autoplay,
    bool muted,
    bool looping,
    bool controls) {
  std::vector<uint8_t> buf = {'N', 'V', 'N', 'P', 1u, 5u, 0u};
  push_u16(buf, 1u);
  buf.push_back(7u);
  push_u32(buf, static_cast<uint32_t>(uri.size()));
  buf.insert(buf.end(), uri.begin(), uri.end());
  push_u16(buf, 2u);
  buf.push_back(1u);
  buf.push_back(autoplay ? 1u : 0u);
  push_u16(buf, 3u);
  buf.push_back(1u);
  buf.push_back(muted ? 1u : 0u);
  push_u16(buf, 4u);
  buf.push_back(1u);
  buf.push_back(looping ? 1u : 0u);
  push_u16(buf, 5u);
  buf.push_back(1u);
  buf.push_back(controls ? 1u : 0u);
  return buf;
}

std::vector<uint8_t> encode_cmd_u64(uint16_t op, uint64_t val) {
  std::vector<uint8_t> buf = {'N', 'V', 'N', 'C', 1u};
  push_u16(buf, op);
  buf.push_back(1u);
  buf.push_back(4u);
  push_u64(buf, val);
  return buf;
}

std::vector<uint8_t> encode_cmd_f32(uint16_t op, float val) {
  std::vector<uint8_t> buf = {'N', 'V', 'N', 'C', 1u};
  push_u16(buf, op);
  buf.push_back(1u);
  buf.push_back(5u);
  push_f32(buf, val);
  return buf;
}
#endif

void test_apple_native_views_registry_and_toggles() {
  @autoreleasepool {
    nve::NativeViewRegistry registry;
    nve::apple::register_apple_native_views(registry);

    // 1. Compile-time per-view toggle registration invariants.
    NVE_CHECK(registry.supports(nve::kNativeKindBlurSurface) == (NVE_APPLE_HAS_BLUR != 0));
    NVE_CHECK(registry.supports(nve::kNativeKindBlurAlias) == (NVE_APPLE_HAS_BLUR != 0));
    NVE_CHECK(registry.supports(nve::kNativeKindWebView) == (NVE_APPLE_HAS_WEBVIEW != 0));
    NVE_CHECK(registry.supports(nve::kNativeKindWebViewAlias) == (NVE_APPLE_HAS_WEBVIEW != 0));
    NVE_CHECK(registry.supports(nve::kNativeKindVideoPlayer) == (NVE_APPLE_HAS_VIDEO != 0));
    NVE_CHECK(registry.supports(nve::kNativeKindVideoAlias) == (NVE_APPLE_HAS_VIDEO != 0));

    // Null host bind and unknown node lookups must be safe no-ops.
    nve::apple::bind_apple_native_views_to_fabric_host(nullptr, registry);
    NVE_CHECK(nve::apple::native_view_for_node(registry, 9999u) == nil);

    NveSurfaceConfig cfg{.surface_id = 1u, .width = 480.0f, .height = 320.0f, .scale_factor = 2.0f};
    NveFabricHostHandle host = nve_fabric_host_create(cfg);
    NVE_CHECK(host != nullptr);
    nve::apple::bind_apple_native_views_to_fabric_host(host, registry);

    // 2. BlurSurface tests (when enabled).
#if NVE_APPLE_HAS_BLUR
    {
      const auto blur1 = encode_blur_props(nve::BlurMaterial::Thick, 0.75f, 0x204060CCu);
      const auto blurAlias = encode_blur_props(
          nve::BlurMaterial::UltraThin, std::numeric_limits<float>::quiet_NaN(), std::nullopt);
      nve::CommandBufferEncoder enc(1u, 1u);
      enc.create_node(10u, NVE_COMP_CUSTOM, nve::kNativeKindBlurSurface);
      enc.set_raw_prop_string(10u, nve::kNativeViewKindPropKey, nve::kNativeKindBlurSurface);
      enc.set_raw_prop_bytes(10u, nve::kNativeViewPropsPropKey, blur1);
      enc.create_node(11u, NVE_COMP_CUSTOM, nve::kNativeKindBlurAlias);
      enc.set_raw_prop_string(11u, nve::kNativeViewKindPropKey, nve::kNativeKindBlurAlias);
      enc.set_raw_prop_bytes(11u, nve::kNativeViewPropsPropKey, blurAlias);
      enc.set_children(NVE_ROOT_NODE_ID, {10u, 11u});
      enc.commit(true);
      NVE_CHECK(nve_fabric_host_submit_frame(host, enc.finish(), nullptr) == 0);

      NveAppleBlurSurfaceView *blurView = nve::apple::blur_surface_for_node(registry, 10u);
      NVE_CHECK(blurView != nil);
      NVE_CHECK([blurView isKindOfClass:[NSVisualEffectView class]]);
      NVE_CHECK(blurView.nveBlurMaterial == nve::BlurMaterial::Thick);
      NVE_CHECK(blurView.material == NSVisualEffectMaterialHeaderView);
      NVE_CHECK(std::fabs(blurView.nveBlurIntensity - 0.75f) < 1e-4f);
      NVE_CHECK(blurView.nveHasTint == YES && blurView.nveTintRgba == 0x204060CCu);
      NVE_CHECK(blurView.nveTintOverlayView != nil);

      NveAppleBlurSurfaceView *aliasView = nve::apple::blur_surface_for_node(registry, 11u);
      NVE_CHECK(aliasView != nil);
      NVE_CHECK(aliasView.nveBlurMaterial == nve::BlurMaterial::UltraThin);
      NVE_CHECK(aliasView.material == NSVisualEffectMaterialUnderPageBackground);
      NVE_CHECK(std::fabs(aliasView.nveBlurIntensity - 1.0f) < 1e-4f);
      NVE_CHECK(aliasView.nveHasTint == NO);

      // Update intensity clamping + clear tint + Chrome material.
      const auto blur2 = encode_blur_props(nve::BlurMaterial::Chrome, -0.5f, std::nullopt);
      nve::CommandBufferEncoder enc2(1u, 2u);
      enc2.set_raw_prop_bytes(10u, nve::kNativeViewPropsPropKey, blur2);
      enc2.commit(true);
      NVE_CHECK(nve_fabric_host_submit_frame(host, enc2.finish(), nullptr) == 0);
      NVE_CHECK(blurView.nveBlurMaterial == nve::BlurMaterial::Chrome);
      NVE_CHECK(blurView.material == NSVisualEffectMaterialTitlebar);
      NVE_CHECK(std::fabs(blurView.nveBlurIntensity - 0.0f) < 1e-4f);
      NVE_CHECK(blurView.nveHasTint == NO);

      NVE_CHECK(registry.unmount(10u));
      NVE_CHECK(registry.unmount(11u));
      NVE_CHECK(nve::apple::blur_surface_for_node(registry, 10u) == nil);
    }
#else
    NVE_CHECK(registry.mount_or_update(10u, nve::kNativeKindBlurSurface, {}) == nullptr);
#endif

    // 3. WebView tests (when enabled).
#if NVE_APPLE_HAS_WEBVIEW
    {
      NVE_CHECK(nve::apple::is_safe_apple_webview_url("https://nve.dev/docs"));
      NVE_CHECK(nve::apple::is_safe_apple_webview_url("  HTTP://localhost:3000/test "));
      NVE_CHECK(nve::apple::is_safe_apple_webview_url("about:blank"));
      NVE_CHECK(!nve::apple::is_safe_apple_webview_url(""));
      NVE_CHECK(!nve::apple::is_safe_apple_webview_url("javascript:alert(1)"));
      NVE_CHECK(!nve::apple::is_safe_apple_webview_url("java\nscript:alert(1)"));
      NVE_CHECK(!nve::apple::is_safe_apple_webview_url("data:text/html,<h1>hi</h1>"));
      NVE_CHECK(!nve::apple::is_safe_apple_webview_url("file:///etc/passwd"));

      const auto wv1 = encode_webview_props("https://nve.dev/home", true);
      nve::CommandBufferEncoder enc(1u, 3u);
      enc.create_node(20u, NVE_COMP_CUSTOM, nve::kNativeKindWebView);
      enc.set_raw_prop_string(20u, nve::kNativeViewKindPropKey, nve::kNativeKindWebView);
      enc.set_raw_prop_bytes(20u, nve::kNativeViewPropsPropKey, wv1);
      enc.set_children(NVE_ROOT_NODE_ID, {20u});
      enc.commit(true);
      NVE_CHECK(nve_fabric_host_submit_frame(host, enc.finish(), nullptr) == 0);

      NveAppleWebView *webView = nve::apple::web_view_for_node(registry, 20u);
      NVE_CHECK(webView != nil);
      NVE_CHECK([webView isKindOfClass:[WKWebView class]]);
      NVE_CHECK([webView.nveLastRequestedUrl isEqualToString:@"https://nve.dev/home"]);
      NVE_CHECK(webView.nveAllowsBackForward == YES);
      NVE_CHECK(webView.nveNavigationCount == 1u);
#if NVE_APPLE_HAS_BLUR
      NVE_CHECK(nve::apple::blur_surface_for_node(registry, 20u) == nil);
#endif

      // Re-applying identical URL must not trigger duplicate navigation.
      nve::CommandBufferEncoder encDup(1u, 4u);
      encDup.set_raw_prop_bytes(20u, nve::kNativeViewPropsPropKey, wv1);
      encDup.commit(true);
      NVE_CHECK(nve_fabric_host_submit_frame(host, encDup.finish(), nullptr) == 0);
      NVE_CHECK(webView.nveNavigationCount == 1u);

      // Dispatch NVNC commands: reject unsafe navigate, accept safe navigate, reload, back, forward, eval.
      nve::CommandBufferEncoder encCmd(1u, 5u);
      encCmd.dispatch_command(20u, "navigate", encode_cmd_str(1u, "javascript:alert(1)"));
      encCmd.dispatch_command(20u, "navigate", encode_cmd_str(1u, "https://nve.dev/api"));
      encCmd.dispatch_command(20u, "reload", encode_cmd_no_args(2u));
      encCmd.dispatch_command(20u, "go_back", encode_cmd_no_args(3u));
      encCmd.dispatch_command(20u, "go_forward", encode_cmd_no_args(4u));
      encCmd.dispatch_command(20u, "evaluate_script", encode_cmd_str(5u, "window.__nve = 42;"));
      encCmd.commit(true);
      NVE_CHECK(nve_fabric_host_submit_frame(host, encCmd.finish(), nullptr) == 0);

      NVE_CHECK([webView.nveLastRequestedUrl isEqualToString:@"https://nve.dev/api"]);
      NVE_CHECK(webView.nveNavigationCount == 2u);
      NVE_CHECK(webView.nveReloadCount == 1u);
      NVE_CHECK(webView.nveGoBackCount == 1u);
      NVE_CHECK(webView.nveGoForwardCount == 1u);
      NVE_CHECK([webView.nveLastEvaluatedScript isEqualToString:@"window.__nve = 42;"]);

      // Disable back/forward gestures in one frame, then verify go_back/go_forward are gated.
      const auto wv2 = encode_webview_props("https://nve.dev/api", false);
      nve::CommandBufferEncoder encNoBfProps(1u, 6u);
      encNoBfProps.set_raw_prop_bytes(20u, nve::kNativeViewPropsPropKey, wv2);
      encNoBfProps.commit(true);
      NVE_CHECK(nve_fabric_host_submit_frame(host, encNoBfProps.finish(), nullptr) == 0);
      NVE_CHECK(webView.nveAllowsBackForward == NO);

      nve::CommandBufferEncoder encNoBfCmd(1u, 7u);
      encNoBfCmd.dispatch_command(20u, "go_back", encode_cmd_no_args(3u));
      encNoBfCmd.dispatch_command(20u, "go_forward", encode_cmd_no_args(4u));
      encNoBfCmd.commit(true);
      NVE_CHECK(nve_fabric_host_submit_frame(host, encNoBfCmd.finish(), nullptr) == 0);
      NVE_CHECK(webView.nveGoBackCount == 1u);
      NVE_CHECK(webView.nveGoForwardCount == 1u);

      NVE_CHECK(registry.unmount(20u));
      NVE_CHECK(nve::apple::web_view_for_node(registry, 20u) == nil);
    }
#else
    NVE_CHECK(registry.mount_or_update(20u, nve::kNativeKindWebView, {}) == nullptr);
#endif

    // 4. VideoPlayer tests (when enabled).
#if NVE_APPLE_HAS_VIDEO
    {
      NVE_CHECK(nve::apple::is_safe_apple_media_uri("https://cdn.nve.dev/demo.mp4"));
      NVE_CHECK(nve::apple::is_safe_apple_media_uri("http://localhost:8080/clip.m3u8"));
      NVE_CHECK(nve::apple::is_safe_apple_media_uri("file:///tmp/sample.mov"));
      NVE_CHECK(!nve::apple::is_safe_apple_media_uri(""));
      NVE_CHECK(!nve::apple::is_safe_apple_media_uri("javascript:alert(1)"));
      NVE_CHECK(!nve::apple::is_safe_apple_media_uri("data:video/mp4;base64,AAAA"));
      NVE_CHECK(!nve::apple::is_safe_apple_media_uri("rtsp://example.com/stream"));

      const auto vid1 = encode_video_props(
          "https://cdn.nve.dev/demo.mp4", /*autoplay=*/true, /*muted=*/true, /*looping=*/true,
          /*controls=*/false);
      nve::CommandBufferEncoder enc(1u, 7u);
      enc.create_node(30u, NVE_COMP_CUSTOM, nve::kNativeKindVideoPlayer);
      enc.set_raw_prop_string(30u, nve::kNativeViewKindPropKey, nve::kNativeKindVideoPlayer);
      enc.set_raw_prop_bytes(30u, nve::kNativeViewPropsPropKey, vid1);
      enc.set_children(NVE_ROOT_NODE_ID, {30u});
      enc.commit(true);
      NVE_CHECK(nve_fabric_host_submit_frame(host, enc.finish(), nullptr) == 0);

      NveAppleVideoPlayerView *videoView = nve::apple::video_player_for_node(registry, 30u);
      NVE_CHECK(videoView != nil);
      NVE_CHECK(videoView.nvePlayer != nil);
      NVE_CHECK(videoView.nvePlayerLayer != nil);
      NVE_CHECK(videoView.nvePlayerLayer.player == videoView.nvePlayer);
      NVE_CHECK([videoView.nveSourceUri isEqualToString:@"https://cdn.nve.dev/demo.mp4"]);
      NVE_CHECK(videoView.nveAutoplay == YES);
      NVE_CHECK(videoView.nveMuted == YES && videoView.nvePlayer.muted == YES);
      NVE_CHECK(videoView.nveLooping == YES);
      NVE_CHECK(videoView.nveControls == NO);
      NVE_CHECK(videoView.nveIsPlaying == YES && videoView.nvePlayCount == 1u);
#if NVE_APPLE_HAS_BLUR
      NVE_CHECK(nve::apple::blur_surface_for_node(registry, 30u) == nil);
#endif
#if NVE_APPLE_HAS_WEBVIEW
      NVE_CHECK(nve::apple::web_view_for_node(registry, 30u) == nil);
#endif

      // Unsafe URI prop update must keep the previous safe URI.
      const auto vidUnsafe = encode_video_props(
          "javascript:evil()", /*autoplay=*/false, /*muted=*/false, /*looping=*/false,
          /*controls=*/true);
      nve::CommandBufferEncoder encCmd(1u, 8u);
      encCmd.set_raw_prop_bytes(30u, nve::kNativeViewPropsPropKey, vidUnsafe);
      encCmd.dispatch_command(30u, "pause", encode_cmd_no_args(2u));
      encCmd.dispatch_command(30u, "seek_ms", encode_cmd_u64(3u, 12500u));
      encCmd.dispatch_command(30u, "set_volume", encode_cmd_f32(4u, 0.4f));
      encCmd.dispatch_command(30u, "set_volume", encode_cmd_f32(4u, std::numeric_limits<float>::quiet_NaN()));
      encCmd.dispatch_command(30u, "play", encode_cmd_no_args(1u));
      encCmd.commit(true);
      NVE_CHECK(nve_fabric_host_submit_frame(host, encCmd.finish(), nullptr) == 0);

      NVE_CHECK([videoView.nveSourceUri isEqualToString:@"https://cdn.nve.dev/demo.mp4"]);
      NVE_CHECK(videoView.nveMuted == NO && videoView.nvePlayer.muted == NO);
      NVE_CHECK(videoView.nveLooping == NO);
      NVE_CHECK(videoView.nveControls == YES);
      NVE_CHECK(videoView.nveIsPlaying == YES);
      NVE_CHECK(videoView.nvePlayCount == 2u);
      NVE_CHECK(videoView.nvePauseCount == 1u);
      NVE_CHECK(videoView.nvePositionMs == 12500u && videoView.nveSeekCount == 1u);
      NVE_CHECK(std::fabs(videoView.nveVolume - 0.4f) < 1e-4f);
      NVE_CHECK(videoView.nveVolumeChangeCount == 1u);

      // Volume clamping.
      nve::CommandBufferEncoder encClamp(1u, 9u);
      encClamp.dispatch_command(30u, "set_volume", encode_cmd_f32(4u, 2.5f));
      encClamp.commit(true);
      NVE_CHECK(nve_fabric_host_submit_frame(host, encClamp.finish(), nullptr) == 0);
      NVE_CHECK(std::fabs(videoView.nveVolume - 1.0f) < 1e-4f);
      NVE_CHECK(videoView.nveVolumeChangeCount == 2u);

      NVE_CHECK(registry.unmount(30u));
      NVE_CHECK(nve::apple::video_player_for_node(registry, 30u) == nil);
    }
#else
    NVE_CHECK(registry.mount_or_update(30u, nve::kNativeKindVideoPlayer, {}) == nullptr);
#endif

    NVE_CHECK(registry.mounted_count() == 0u);
    nve_fabric_host_destroy(host);
  }
}

} // namespace

int main() {
  test_apple_native_views_registry_and_toggles();
  std::cout << "nve_apple_native_views_test passed (blur=" << NVE_APPLE_HAS_BLUR
            << ", webview=" << NVE_APPLE_HAS_WEBVIEW << ", video=" << NVE_APPLE_HAS_VIDEO << ")\n";
  return 0;
}
