#include "nve_rn_camera_jsi.hpp"

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

class TestCameraBackend final : public nve::NveCameraBackend {
 public:
  TestCameraBackend()
      : devices_{
            nve::CameraDeviceInfo{"vc-back-0", "Back Wide", nve::CameraFacing::Back, true, 0.5f, 8.0f, 120u, true,
                                  {nve::CameraStabilizationMode::Off, nve::CameraStabilizationMode::Standard,
                                   nve::CameraStabilizationMode::Cinematic}},
            nve::CameraDeviceInfo{"vc-front-1", "Front", nve::CameraFacing::Front, false, 1.0f, 4.0f, 60u, false,
                                  {nve::CameraStabilizationMode::Off}},
        } {}

  nve::CameraBackendKind backend_kind() const noexcept override { return nve::CameraBackendKind::VisionCameraV5; }
  std::string_view backend_name() const noexcept override { return "test-vision-camera"; }
  nve::CameraSessionState state() const noexcept override { return state_; }
  const nve::CameraPreviewProps& active_props() const noexcept override { return props_; }
  std::optional<nve::CameraDeviceInfo> selected_device() const override { return selected_; }
  std::vector<nve::CameraDeviceInfo> list_devices() override { return devices_; }
  void attach_listener(nve::CameraEventListener* listener) override { listener_ = listener; }

  bool apply_props(const nve::CameraPreviewProps& props) override {
    if (state_ == nve::CameraSessionState::Released) return false;
    auto dev = nve::select_camera_device(devices_, props);
    if (!dev) return false;
    auto neg = nve::negotiate_camera_settings(*dev, props);
    selected_ = neg.device;
    props_ = props;
    props_.device_id = neg.device.id;
    props_.facing = neg.device.facing;
    props_.fps = neg.effective_fps;
    props_.hdr = neg.effective_hdr;
    props_.stabilization = neg.effective_stabilization;
    props_.torch = neg.effective_torch;
    props_.zoom = neg.effective_zoom;
    state_ = props.active ? nve::CameraSessionState::Running : nve::CameraSessionState::Configured;
    return true;
  }

  bool dispatch_command(const nve::CameraPreviewCommand& cmd) override {
    if (state_ == nve::CameraSessionState::Released) return false;
    switch (cmd.kind) {
      case nve::CameraCommandKind::SwitchFacing: {
        auto next = props_;
        next.device_id = std::nullopt;
        next.facing = cmd.facing;
        return apply_props(next);
      }
      case nve::CameraCommandKind::SetTorch:
        props_.torch = cmd.torch;
        return true;
      case nve::CameraCommandKind::SetZoom:
        props_.zoom = cmd.zoom;
        return true;
      case nve::CameraCommandKind::FocusPoint:
        return true;
      case nve::CameraCommandKind::CapturePhoto:
        if (listener_) {
          listener_->on_photo_captured(nve::CapturedPhoto{"/tmp/photo.jpg", 1920u, 1080u, cmd.quality, cmd.flash, 1ull});
        }
        return true;
      case nve::CameraCommandKind::StartRecording:
        if (cmd.output_path.empty() || state_ == nve::CameraSessionState::Recording) return false;
        rec_path_ = cmd.output_path;
        state_ = nve::CameraSessionState::Recording;
        if (listener_) listener_->on_recording_started(rec_path_);
        return true;
      case nve::CameraCommandKind::StopRecording:
        if (state_ != nve::CameraSessionState::Recording) return false;
        state_ = nve::CameraSessionState::Running;
        if (listener_) {
          listener_->on_recording_finished(nve::RecordedVideo{rec_path_, 1000ull, 1920u, 1080u, props_.audio_enabled});
        }
        rec_path_.clear();
        return true;
    }
    return false;
  }

  bool start() override {
    if (state_ == nve::CameraSessionState::Released) return false;
    state_ = nve::CameraSessionState::Running;
    return true;
  }
  void stop() override {
    if (state_ != nve::CameraSessionState::Released) state_ = nve::CameraSessionState::Stopped;
  }
  void release() override { state_ = nve::CameraSessionState::Released; }

  void emit_frame(const nve::CameraFrameDescriptor& desc) {
    if (!listener_) return;
    std::array<uint8_t, nve::kCameraFrameDescriptorSize> nvcf{};
    NVE_CHECK(nve::encode_camera_frame(desc, nvcf) == nve::NativeViewCodecError::Ok);
    listener_->on_frame(desc, nvcf);
  }

 private:
  std::vector<nve::CameraDeviceInfo> devices_;
  nve::CameraSessionState state_{nve::CameraSessionState::Idle};
  nve::CameraPreviewProps props_{};
  std::optional<nve::CameraDeviceInfo> selected_{};
  nve::CameraEventListener* listener_{nullptr};
  std::string rec_path_{};
};

void test_standalone_hybrid_camera_bridge() {
  nve::nitro::HybridObjectRegistry registry{};
  TestCameraBackend* captured = nullptr;

  nve::rn::camera::install_nve_camera_standalone(registry, [&]() -> std::unique_ptr<nve::NveCameraBackend> {
    auto b = std::make_unique<TestCameraBackend>();
    captured = b.get();
    return b;
  });

  NVE_CHECK(registry.has_hybrid_object(nve::camera::kHybridCameraSessionName));
  auto session = registry.create<nve::camera::HybridCameraSession>(nve::camera::kHybridCameraSessionName);
  NVE_CHECK(session != nullptr);

  nve::rn::camera::RnCameraBridge bridge(session);
  NVE_CHECK(!bridge.is_disposed());
  NVE_CHECK(bridge.list_devices().size() == 2u);

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

  const std::vector<uint8_t> bad_nvnp = {'B', 'A', 'D', '!', 1, 0, 0};
  NVE_CHECK(bridge.apply_encoded_props(bad_nvnp) == nve::NativeViewCodecError::InvalidMagic);

  NVE_CHECK(captured != nullptr);
  const nve::CameraFrameDescriptor frame{1920u, 1080u, 1920u, nve::CameraPixelFormat::Nv12, 0u, 9001ull, 0xBEEF1234ull};
  captured->emit_frame(frame);
  NVE_CHECK(bridge.frame_count() == 1u);
  const auto last = bridge.last_frame();
  NVE_CHECK(last.has_value());
  nve::CameraFrameDescriptor decoded{};
  NVE_CHECK(nve::decode_camera_frame(*last, decoded) == nve::NativeViewCodecError::Ok);
  NVE_CHECK(decoded == frame);

  std::vector<uint8_t> nvnc;
  NVE_CHECK(nve::encode_camera_command(nve::CameraPreviewCommand::set_zoom(3.5f), nvnc) ==
            nve::NativeViewCodecError::Ok);
  NVE_CHECK(bridge.dispatch_encoded_command(nvnc) == nve::NativeViewCodecError::Ok);
  NVE_CHECK(std::abs(bridge.active_props().zoom - 3.5f) < 1e-5f);

  auto photo = bridge.take_photo(94u, 1u).await_result();
  NVE_CHECK(photo.is_ok());
  NVE_CHECK(photo.value().quality == 94u && photo.value().flash_mode == 1u);

  NVE_CHECK(bridge.start_recording("/tmp/rn_standalone.mp4").await_result().is_ok());
  NVE_CHECK(bridge.state() == nve::CameraSessionState::Recording);
  auto video = bridge.stop_recording().await_result();
  NVE_CHECK(video.is_ok());
  NVE_CHECK(video.value().output_path == "/tmp/rn_standalone.mp4");
  NVE_CHECK(video.value().has_audio);

  bridge.dispose();
  NVE_CHECK(bridge.is_disposed());
  NVE_CHECK(bridge.state() == nve::CameraSessionState::Released);
  NVE_CHECK(bridge.take_photo().await_result().is_err());
}

#if defined(NVE_WITH_RN_JSI) && NVE_WITH_RN_JSI && defined(NVE_USING_MOCK_JSI)
void test_jsi_host_object_and_native_state() {
  facebook::jsi::Runtime rt;
  TestCameraBackend* captured = nullptr;

  nve::rn::camera::install_nve_camera_jsi(rt, [&]() -> std::unique_ptr<nve::NveCameraBackend> {
    auto b = std::make_unique<TestCameraBackend>();
    captured = b.get();
    return b;
  });

  NVE_CHECK(rt.global().hasProperty(rt, "__nveCameraInstall"));
  NVE_CHECK(rt.global().hasProperty(rt, "__nveCamera"));
  auto install_fn = rt.global().getProperty(rt, "__nveCameraInstall").asObject(rt).asFunction(rt);
  auto mod_obj = install_fn.call(rt).asObject(rt);
  NVE_CHECK(mod_obj.isHostObject(rt));
  NVE_CHECK(mod_obj.getProperty(rt, "isAvailable").asBool());
  NVE_CHECK(mod_obj.getProperty(rt, "hybridObjectName").asString(rt).utf8(rt) == "CameraSession");

  auto mod_devices = mod_obj.getProperty(rt, "listDevices").asObject(rt).asFunction(rt).call(rt).asObject(rt).getArray(rt);
  NVE_CHECK(mod_devices.size(rt) == 2u);

  auto session_obj = mod_obj.getProperty(rt, "createSession").asObject(rt).asFunction(rt).call(rt).asObject(rt);
  NVE_CHECK(session_obj.isHostObject(rt));
  NVE_CHECK(!session_obj.getProperty(rt, "isDisposed").asBool());
  NVE_CHECK(session_obj.getProperty(rt, "name").asString(rt).utf8(rt) == "CameraSession");
  NVE_CHECK(session_obj.getProperty(rt, "memorySize").asNumber() > 0.0);

  auto ns_holder = session_obj.getProperty(rt, "asNativeState").asObject(rt).asFunction(rt).call(rt).asObject(rt);
  NVE_CHECK(ns_holder.hasNativeState(rt));
  auto ns_state = std::dynamic_pointer_cast<nve::nitro::HybridJsiNativeState>(ns_holder.getNativeState(rt));
  NVE_CHECK(ns_state != nullptr && ns_state->target() != nullptr && ns_state->target()->name() == "CameraSession");

  facebook::jsi::Object js_props(rt);
  js_props.setProperty(rt, "facing", facebook::jsi::String::createFromUtf8(rt, "back"));
  js_props.setProperty(rt, "torch", facebook::jsi::String::createFromUtf8(rt, "on"));
  js_props.setProperty(rt, "zoom", facebook::jsi::Value(2.0));
  js_props.setProperty(rt, "fps", facebook::jsi::Value(60));
  js_props.setProperty(rt, "hdr", facebook::jsi::Value(true));
  js_props.setProperty(rt, "enableFrameProcessor", facebook::jsi::Value(true));
  js_props.setProperty(rt, "audioEnabled", facebook::jsi::Value(true));
  js_props.setProperty(rt, "active", facebook::jsi::Value(true));
  NVE_CHECK(session_obj.getProperty(rt, "applyProps").asObject(rt).asFunction(rt).call(rt, js_props).asBool());
  NVE_CHECK(static_cast<int>(session_obj.getProperty(rt, "state").asNumber()) ==
            static_cast<int>(nve::CameraSessionState::Running));

  NVE_CHECK(captured != nullptr);
  const nve::CameraFrameDescriptor frame{1280u, 720u, 1280u, nve::CameraPixelFormat::Nv12, 90u, 55555ull, 0xCAFE42ull};
  captured->emit_frame(frame);
  NVE_CHECK(static_cast<uint64_t>(session_obj.getProperty(rt, "frameCount").asNumber()) == 1u);
  auto last_frame_ab = session_obj.getProperty(rt, "lastFrame").asObject(rt).asFunction(rt).call(rt).asObject(rt).getArrayBuffer(rt);
  NVE_CHECK(last_frame_ab.size(rt) == nve::kCameraFrameDescriptorSize);

  (void)session_obj.getProperty(rt, "dispose").asObject(rt).asFunction(rt).call(rt);
  NVE_CHECK(session_obj.getProperty(rt, "isDisposed").asBool());
  nve::nitro::HybridObjectRegistry::global().unregister_hybrid_object(nve::camera::kHybridCameraSessionName);
}
#elif defined(NVE_WITH_RN_JSI) && NVE_WITH_RN_JSI
void test_real_jsi_host_object_signatures() {
  auto session = std::make_shared<nve::camera::HybridCameraSession>(std::make_unique<TestCameraBackend>());
  auto session_host = std::make_shared<nve::rn::camera::RnCameraSessionHostObject>(session);
  auto module_host = std::make_shared<nve::rn::camera::RnCameraModuleHostObject>(nullptr);
  auto installer = &nve::rn::camera::install_nve_camera_jsi;
  NVE_CHECK(session_host != nullptr && module_host != nullptr && installer != nullptr);
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
