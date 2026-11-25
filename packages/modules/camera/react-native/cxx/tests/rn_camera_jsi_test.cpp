#include "nve_rn_camera_jsi.hpp"
#include "nve_camera_linux.hpp"
#include "nve_camera_windows.hpp"

#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <memory>
#include <string>
#include <vector>

#define NVE_CHECK(expr)                                                               \
  do {                                                                                \
    if (!(expr)) {                                                                    \
      std::fprintf(stderr, "NVE_CHECK failed at %s:%d: %s\n", __FILE__, __LINE__,     \
                   #expr);                                                            \
      std::abort();                                                                   \
    }                                                                                 \
  } while (false)

namespace {

void test_standalone_hybrid_camera_bridge() {
  nve::nitro::HybridObjectRegistry registry{};
  nve::InMemoryWindowsMediaCaptureBridge* captured_bridge = nullptr;

  nve::rn::camera::install_nve_camera_standalone(registry, [&]() -> std::unique_ptr<nve::NveCameraBackend> {
    auto bridge = std::make_unique<nve::InMemoryWindowsMediaCaptureBridge>();
    captured_bridge = bridge.get();
    return std::make_unique<nve::NveWindowsCameraBackend>(std::move(bridge));
  });

  NVE_CHECK(registry.has_hybrid_object(nve::camera::kHybridCameraSessionName));
  auto session = registry.create<nve::camera::HybridCameraSession>(nve::camera::kHybridCameraSessionName);
  NVE_CHECK(session != nullptr);

  nve::rn::camera::RnCameraBridge bridge(session);
  NVE_CHECK(!bridge.is_disposed());
  NVE_CHECK(bridge.list_devices().size() == 2u);

  // Zero-JSON binary NVNP props apply
  nve::CameraPreviewProps props{};
  props.facing = nve::CameraFacing::Back;
  props.torch = nve::CameraTorchMode::On;
  props.zoom = 2.5f;
  props.fps = 60u;
  props.hdr = true;
  props.stabilization = nve::CameraStabilizationMode::Cinematic;
  props.enable_frame_processor = true;
  props.audio_enabled = true;
  props.active = true;

  std::vector<uint8_t> nvnp;
  NVE_CHECK(nve::encode_camera_props(props, nvnp) == nve::NativeViewCodecError::Ok);
  NVE_CHECK(bridge.apply_encoded_props(nvnp) == nve::NativeViewCodecError::Ok);
  NVE_CHECK(bridge.state() == nve::CameraSessionState::Running);
  NVE_CHECK(std::abs(bridge.active_props().zoom - 2.5f) < 1e-5f);

  // Corrupt NVNP rejected
  const std::vector<uint8_t> bad_nvnp = {'B', 'A', 'D', '!', 1, 0, 0};
  NVE_CHECK(bridge.apply_encoded_props(bad_nvnp) == nve::NativeViewCodecError::InvalidMagic);

  // Zero-copy NVCF frame delivery
  NVE_CHECK(captured_bridge != nullptr);
  const nve::CameraFrameDescriptor frame{1920u, 1080u, 1920u, nve::CameraPixelFormat::Nv12, 0u, 9001ull, 0xBEEF1234ull};
  captured_bridge->push_simulated_frame(frame);
  NVE_CHECK(bridge.frame_count() == 1u);
  const auto last = bridge.last_frame();
  NVE_CHECK(last.has_value());
  nve::CameraFrameDescriptor decoded{};
  NVE_CHECK(nve::decode_camera_frame(*last, decoded) == nve::NativeViewCodecError::Ok);
  NVE_CHECK(decoded == frame);

  // NVNC commands: zoom, focus, photo, recording lifecycle
  std::vector<uint8_t> nvnc;
  NVE_CHECK(nve::encode_camera_command(nve::CameraPreviewCommand::set_zoom(3.5f), nvnc) ==
            nve::NativeViewCodecError::Ok);
  NVE_CHECK(bridge.dispatch_encoded_command(nvnc) == nve::NativeViewCodecError::Ok);
  NVE_CHECK(std::abs(bridge.active_props().zoom - 3.5f) < 1e-5f);

  auto photo = bridge.take_photo(94u, 1u).await_result();
  NVE_CHECK(photo.is_ok());
  NVE_CHECK(photo.value().quality == 94u && photo.value().flash_mode == 1u);

  NVE_CHECK(bridge.start_recording("C:\\temp\\rn_standalone.mp4").await_result().is_ok());
  NVE_CHECK(bridge.state() == nve::CameraSessionState::Recording);
  auto video = bridge.stop_recording().await_result();
  NVE_CHECK(video.is_ok());
  NVE_CHECK(video.value().output_path == "C:\\temp\\rn_standalone.mp4");
  NVE_CHECK(video.value().has_audio);

  bridge.dispose();
  NVE_CHECK(bridge.is_disposed());
  NVE_CHECK(bridge.state() == nve::CameraSessionState::Released);
  NVE_CHECK(bridge.take_photo().await_result().is_err());
}

#if defined(NVE_WITH_RN_JSI) && NVE_WITH_RN_JSI && defined(NVE_USING_MOCK_JSI)
void test_jsi_host_object_and_native_state() {
  facebook::jsi::Runtime rt;
  nve::InMemoryLinuxCameraBridge* captured_bridge = nullptr;

  nve::rn::camera::install_nve_camera_jsi(rt, [&]() -> std::unique_ptr<nve::NveCameraBackend> {
    auto bridge = std::make_unique<nve::InMemoryLinuxCameraBridge>(nve::LinuxCameraTransport::PipeWire);
    captured_bridge = bridge.get();
    return std::make_unique<nve::NveLinuxCameraBackend>(std::move(bridge));
  });

  // Verify global.__nveCameraInstall returns the module HostObject
  NVE_CHECK(rt.global().hasProperty(rt, "__nveCameraInstall"));
  NVE_CHECK(rt.global().hasProperty(rt, "__nveCamera"));
  auto install_fn = rt.global().getProperty(rt, "__nveCameraInstall").asObject(rt).asFunction(rt);
  auto mod_obj = install_fn.call(rt).asObject(rt);
  NVE_CHECK(mod_obj.isHostObject(rt));
  NVE_CHECK(mod_obj.getProperty(rt, "isAvailable").asBool());
  NVE_CHECK(mod_obj.getProperty(rt, "hybridObjectName").asString(rt).utf8(rt) == "CameraSession");

  // Module-level listDevices()
  auto mod_devices = mod_obj.getProperty(rt, "listDevices").asObject(rt).asFunction(rt).call(rt).asObject(rt).getArray(rt);
  NVE_CHECK(mod_devices.size(rt) == 2u);

  // Create HybridCameraSession HostObject
  auto session_obj = mod_obj.getProperty(rt, "createSession").asObject(rt).asFunction(rt).call(rt).asObject(rt);
  NVE_CHECK(session_obj.isHostObject(rt));
  NVE_CHECK(!session_obj.getProperty(rt, "isDisposed").asBool());
  NVE_CHECK(session_obj.getProperty(rt, "name").asString(rt).utf8(rt) == "CameraSession");
  NVE_CHECK(session_obj.getProperty(rt, "memorySize").asNumber() > 0.0);

  auto host_ptr = session_obj.getHostObject(rt);
  NVE_CHECK(host_ptr != nullptr);
  NVE_CHECK(host_ptr->getPropertyNames(rt).size() >= 20u);

  // Verify NativeState interop wraps the exact same HybridCameraSession
  auto ns_holder = session_obj.getProperty(rt, "asNativeState").asObject(rt).asFunction(rt).call(rt).asObject(rt);
  NVE_CHECK(ns_holder.hasNativeState(rt));
  auto ns_state = std::dynamic_pointer_cast<nve::nitro::HybridJsiNativeState>(ns_holder.getNativeState(rt));
  NVE_CHECK(ns_state != nullptr);
  NVE_CHECK(ns_state->target() != nullptr && ns_state->target()->name() == "CameraSession");

  // 1. listDevices()
  auto devices = session_obj.getProperty(rt, "listDevices").asObject(rt).asFunction(rt).call(rt).asObject(rt).getArray(rt);
  NVE_CHECK(devices.size(rt) == 2u);
  auto first_dev = devices.getValueAtIndex(rt, 0u).asObject(rt);
  NVE_CHECK(!first_dev.getProperty(rt, "id").asString(rt).utf8(rt).empty());
  NVE_CHECK(!first_dev.getProperty(rt, "facing").asString(rt).utf8(rt).empty());

  // 2. applyProps(plain JS object) — zero JSON!
  facebook::jsi::Object js_props(rt);
  js_props.setProperty(rt, "facing", facebook::jsi::String::createFromUtf8(rt, "external"));
  js_props.setProperty(rt, "torch", facebook::jsi::String::createFromUtf8(rt, "on"));
  js_props.setProperty(rt, "zoom", facebook::jsi::Value(2.0));
  js_props.setProperty(rt, "fps", facebook::jsi::Value(60));
  js_props.setProperty(rt, "hdr", facebook::jsi::Value(true));
  js_props.setProperty(rt, "enableFrameProcessor", facebook::jsi::Value(true));
  js_props.setProperty(rt, "audioEnabled", facebook::jsi::Value(true));
  js_props.setProperty(rt, "active", facebook::jsi::Value(true));
  auto apply_ok = session_obj.getProperty(rt, "applyProps").asObject(rt).asFunction(rt).call(rt, js_props);
  NVE_CHECK(apply_ok.asBool());
  NVE_CHECK(static_cast<int>(session_obj.getProperty(rt, "state").asNumber()) ==
            static_cast<int>(nve::CameraSessionState::Running));

  // 3. applyEncodedProps(ArrayBuffer) with binary NVNP bytes
  nve::CameraPreviewProps bin_props{};
  bin_props.facing = nve::CameraFacing::External;
  bin_props.torch = nve::CameraTorchMode::On;
  bin_props.zoom = 3.25f;
  bin_props.fps = 60u;
  bin_props.enable_frame_processor = true;
  bin_props.audio_enabled = true;
  bin_props.active = true;
  std::vector<uint8_t> nvnp;
  NVE_CHECK(nve::encode_camera_props(bin_props, nvnp) == nve::NativeViewCodecError::Ok);
  facebook::jsi::ArrayBuffer nvnp_ab(rt, std::make_shared<nve::rn::camera::detail::OwnedMutableBuffer>(nvnp));
  auto bin_ok = session_obj.getProperty(rt, "applyEncodedProps").asObject(rt).asFunction(rt).call(rt, nvnp_ab);
  NVE_CHECK(bin_ok.asBool());

  // Corrupt ArrayBuffer must throw facebook::jsi::JSError
  bool threw_corrupt = false;
  try {
    facebook::jsi::ArrayBuffer bad_ab(
        rt, std::make_shared<nve::rn::camera::detail::OwnedMutableBuffer>(std::vector<uint8_t>{0x00, 0x01, 0x02}));
    (void)session_obj.getProperty(rt, "applyEncodedProps").asObject(rt).asFunction(rt).call(rt, bad_ab);
  } catch (const facebook::jsi::JSError&) {
    threw_corrupt = true;
  }
  NVE_CHECK(threw_corrupt);

  // 4. dispatchCommand with binary NVNC ArrayBuffer AND plain JS object
  std::vector<uint8_t> focus_nvnc;
  NVE_CHECK(nve::encode_camera_command(nve::CameraPreviewCommand::focus_point(0.25f, 0.75f), focus_nvnc) ==
            nve::NativeViewCodecError::Ok);
  facebook::jsi::ArrayBuffer focus_ab(rt, std::make_shared<nve::rn::camera::detail::OwnedMutableBuffer>(focus_nvnc));
  NVE_CHECK(session_obj.getProperty(rt, "dispatchCommand").asObject(rt).asFunction(rt).call(rt, focus_ab).asBool());

  facebook::jsi::Object zoom_cmd(rt);
  zoom_cmd.setProperty(rt, "kind", facebook::jsi::String::createFromUtf8(rt, "set_zoom"));
  zoom_cmd.setProperty(rt, "zoom", facebook::jsi::Value(1.75));
  NVE_CHECK(session_obj.getProperty(rt, "dispatchCommand").asObject(rt).asFunction(rt).call(rt, zoom_cmd).asBool());

  // 5. Direct helper methods: setZoom, focus, setTorch, switchFacing
  NVE_CHECK(session_obj.getProperty(rt, "setZoom").asObject(rt).asFunction(rt).call(rt, facebook::jsi::Value(2.5)).asBool());
  NVE_CHECK(session_obj.getProperty(rt, "focus").asObject(rt).asFunction(rt).call(rt, facebook::jsi::Value(0.4), facebook::jsi::Value(0.6)).asBool());
  NVE_CHECK(session_obj.getProperty(rt, "setTorch").asObject(rt).asFunction(rt).call(rt, facebook::jsi::String::createFromUtf8(rt, "auto")).asBool());

  // 6. Zero-copy 40-byte NVCF frame readback via lastFrame()
  NVE_CHECK(captured_bridge != nullptr);
  const nve::CameraFrameDescriptor frame{1280u, 720u, 1280u, nve::CameraPixelFormat::Nv12, 90u, 55555ull, 0xCAFE42ull};
  captured_bridge->push_dmabuf_frame(frame);
  NVE_CHECK(static_cast<uint64_t>(session_obj.getProperty(rt, "frameCount").asNumber()) == 1u);
  auto last_frame_ab = session_obj.getProperty(rt, "lastFrame").asObject(rt).asFunction(rt).call(rt).asObject(rt).getArrayBuffer(rt);
  NVE_CHECK(last_frame_ab.size(rt) == nve::kCameraFrameDescriptorSize);
  nve::CameraFrameDescriptor decoded_frame{};
  NVE_CHECK(nve::decode_camera_frame(std::span<const uint8_t>(last_frame_ab.data(rt), last_frame_ab.size(rt)), decoded_frame) ==
            nve::NativeViewCodecError::Ok);
  NVE_CHECK(decoded_frame == frame);

  // 7. takePhoto + invalid quality rejection
  facebook::jsi::Object photo_opts(rt);
  photo_opts.setProperty(rt, "quality", facebook::jsi::Value(88));
  photo_opts.setProperty(rt, "flash", facebook::jsi::String::createFromUtf8(rt, "on"));
  auto photo_obj = session_obj.getProperty(rt, "takePhoto").asObject(rt).asFunction(rt).call(rt, photo_opts).asObject(rt);
  NVE_CHECK(static_cast<int>(photo_obj.getProperty(rt, "quality").asNumber()) == 88);
  NVE_CHECK(static_cast<int>(photo_obj.getProperty(rt, "flashMode").asNumber()) == 1);

  bool threw_bad_photo = false;
  try {
    (void)session_obj.getProperty(rt, "takePhoto").asObject(rt).asFunction(rt).call(rt, facebook::jsi::Value(105));
  } catch (const facebook::jsi::JSError&) {
    threw_bad_photo = true;
  }
  NVE_CHECK(threw_bad_photo);

  // 8. startRecording / stopRecording
  NVE_CHECK(session_obj.getProperty(rt, "startRecording")
                .asObject(rt)
                .asFunction(rt)
                .call(rt, facebook::jsi::String::createFromUtf8(rt, "/tmp/jsi_clip.mp4"))
                .asBool());
  NVE_CHECK(static_cast<int>(session_obj.getProperty(rt, "state").asNumber()) ==
            static_cast<int>(nve::CameraSessionState::Recording));

  auto video_obj = session_obj.getProperty(rt, "stopRecording").asObject(rt).asFunction(rt).call(rt).asObject(rt);
  NVE_CHECK(video_obj.getProperty(rt, "outputPath").asString(rt).utf8(rt) == "/tmp/jsi_clip.mp4");
  NVE_CHECK(video_obj.getProperty(rt, "hasAudio").asBool());

  // 9. dispose() cleans up and subsequent calls throw JSError
  (void)session_obj.getProperty(rt, "dispose").asObject(rt).asFunction(rt).call(rt);
  NVE_CHECK(session_obj.getProperty(rt, "isDisposed").asBool());

  bool threw_disposed = false;
  try {
    (void)session_obj.getProperty(rt, "listDevices").asObject(rt).asFunction(rt).call(rt);
  } catch (const facebook::jsi::JSError&) {
    threw_disposed = true;
  }
  NVE_CHECK(threw_disposed);

  nve::nitro::HybridObjectRegistry::global().unregister_hybrid_object(nve::camera::kHybridCameraSessionName);
}
#elif defined(NVE_WITH_RN_JSI) && NVE_WITH_RN_JSI
void test_real_jsi_host_object_signatures() {
  // Real facebook::jsi::Runtime is abstract (Hermes/V8 provides the concrete implementation).
  // Verify all HostObject/NativeState types and install_nve_camera_jsi compile & link cleanly.
  auto bridge = std::make_unique<nve::InMemoryLinuxCameraBridge>(nve::LinuxCameraTransport::PipeWire);
  auto session = std::make_shared<nve::camera::HybridCameraSession>(
      std::make_unique<nve::NveLinuxCameraBackend>(std::move(bridge)));
  auto session_host = std::make_shared<nve::rn::camera::RnCameraSessionHostObject>(session);
  auto module_host = std::make_shared<nve::rn::camera::RnCameraModuleHostObject>(nullptr);
  auto installer = &nve::rn::camera::install_nve_camera_jsi;
  NVE_CHECK(session_host != nullptr);
  NVE_CHECK(module_host != nullptr);
  NVE_CHECK(installer != nullptr);
  session->dispose();
}
#endif

} // namespace

int main() {
  test_standalone_hybrid_camera_bridge();
#if defined(NVE_WITH_RN_JSI) && NVE_WITH_RN_JSI && defined(NVE_USING_MOCK_JSI)
  test_jsi_host_object_and_native_state();
  std::puts("nve_rn_camera_jsi_test (NVE_WITH_RN_JSI=1 mock): ALL PASSED");
#elif defined(NVE_WITH_RN_JSI) && NVE_WITH_RN_JSI
  test_real_jsi_host_object_signatures();
  std::puts("nve_rn_camera_jsi_test (NVE_WITH_RN_JSI=1 real JSI): ALL PASSED");
#else
  std::puts("nve_rn_camera_jsi_test (standalone): ALL PASSED");
#endif
  return 0;
}
