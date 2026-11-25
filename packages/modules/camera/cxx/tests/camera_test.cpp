#include "nve_camera.hpp"
#include "nve_vision_camera_hybrid.hpp"

#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <limits>
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

using namespace nve;

namespace {

class MockCameraBackend final : public NveCameraBackend {
 public:
  explicit MockCameraBackend(CameraBackendKind kind = CameraBackendKind::VisionCameraV5)
      : kind_(kind),
        devices_{
            CameraDeviceInfo{"mock-back-0", "Mock Back", CameraFacing::Back, true, 0.5f, 8.0f, 120u, true,
                             {CameraStabilizationMode::Off, CameraStabilizationMode::Standard,
                              CameraStabilizationMode::Cinematic}},
            CameraDeviceInfo{"mock-front-1", "Mock Front", CameraFacing::Front, false, 1.0f, 4.0f, 60u, false,
                             {CameraStabilizationMode::Off}},
        } {}

  CameraBackendKind backend_kind() const noexcept override { return kind_; }
  std::string_view backend_name() const noexcept override { return "mock-vision-camera"; }
  CameraSessionState state() const noexcept override { return state_; }
  const CameraPreviewProps& active_props() const noexcept override { return props_; }
  std::optional<CameraDeviceInfo> selected_device() const override { return selected_; }
  std::vector<CameraDeviceInfo> list_devices() override { return devices_; }
  void attach_listener(CameraEventListener* listener) override { listener_ = listener; }

  bool apply_props(const CameraPreviewProps& props) override {
    if (state_ == CameraSessionState::Released) return false;
    auto dev = select_camera_device(devices_, props);
    if (!dev) return false;
    auto neg = negotiate_camera_settings(*dev, props);
    selected_ = neg.device;
    props_ = props;
    props_.device_id = neg.device.id;
    props_.facing = neg.device.facing;
    props_.fps = neg.effective_fps;
    props_.hdr = neg.effective_hdr;
    props_.stabilization = neg.effective_stabilization;
    props_.torch = neg.effective_torch;
    props_.zoom = neg.effective_zoom;
    state_ = props.active ? CameraSessionState::Running : CameraSessionState::Configured;
    return true;
  }

  bool dispatch_command(const CameraPreviewCommand& cmd) override {
    if (state_ == CameraSessionState::Released) return false;
    switch (cmd.kind) {
      case CameraCommandKind::SwitchFacing: {
        auto next = props_;
        next.device_id = std::nullopt;
        next.facing = cmd.facing;
        return apply_props(next);
      }
      case CameraCommandKind::SetTorch:
        props_.torch = cmd.torch;
        return true;
      case CameraCommandKind::SetZoom:
        props_.zoom = cmd.zoom;
        return true;
      case CameraCommandKind::FocusPoint:
        return true;
      case CameraCommandKind::CapturePhoto:
        if (listener_) {
          listener_->on_photo_captured(CapturedPhoto{"/tmp/mock.jpg", 1920u, 1080u, cmd.quality, cmd.flash, 1ull});
        }
        return true;
      case CameraCommandKind::StartRecording:
        if (cmd.output_path.empty() || state_ == CameraSessionState::Recording) return false;
        recording_path_ = cmd.output_path;
        state_ = CameraSessionState::Recording;
        if (listener_) listener_->on_recording_started(recording_path_);
        return true;
      case CameraCommandKind::StopRecording:
        if (state_ != CameraSessionState::Recording) return false;
        state_ = CameraSessionState::Running;
        if (listener_) {
          listener_->on_recording_finished(RecordedVideo{recording_path_, 1000ull, 1920u, 1080u, props_.audio_enabled});
        }
        recording_path_.clear();
        return true;
    }
    return false;
  }

  bool start() override {
    if (state_ == CameraSessionState::Released) return false;
    state_ = CameraSessionState::Running;
    return true;
  }
  void stop() override {
    if (state_ != CameraSessionState::Released) state_ = CameraSessionState::Stopped;
  }
  void release() override { state_ = CameraSessionState::Released; }

  void emit_frame(const CameraFrameDescriptor& desc) {
    if (!listener_) return;
    std::array<uint8_t, kCameraFrameDescriptorSize> nvcf{};
    NVE_CHECK(encode_camera_frame(desc, nvcf) == NativeViewCodecError::Ok);
    listener_->on_frame(desc, nvcf);
  }

 private:
  CameraBackendKind kind_;
  std::vector<CameraDeviceInfo> devices_;
  CameraSessionState state_{CameraSessionState::Idle};
  CameraPreviewProps props_{};
  std::optional<CameraDeviceInfo> selected_{};
  CameraEventListener* listener_{nullptr};
  std::string recording_path_{};
};

void test_golden_wire_parity_with_rust_and_kotlin() {
  const std::vector<uint8_t> expected_default_props = {
      'N', 'V', 'N', 'P', 1, 10, 0,
      1, 0, 0,
      2, 0, 2, 1, 0, 0, 0,
      3, 0, 2, 0, 0, 0, 0,
      4, 0, 5, 0x00, 0x00, 0x80, 0x3F,
      5, 0, 2, 30, 0, 0, 0,
      6, 0, 1, 0,
      7, 0, 2, 0, 0, 0, 0,
      8, 0, 1, 0,
      9, 0, 1, 0,
      10, 0, 1, 1,
  };
  std::vector<uint8_t> encoded_props;
  NVE_CHECK(encode_camera_props(CameraPreviewProps{}, encoded_props) == NativeViewCodecError::Ok);
  NVE_CHECK(encoded_props == expected_default_props);
  CameraPreviewProps decoded_props{};
  NVE_CHECK(decode_camera_props(expected_default_props, decoded_props) == NativeViewCodecError::Ok);
  NVE_CHECK(decoded_props == CameraPreviewProps{});

  const std::vector<uint8_t> expected_focus = {
      'N', 'V', 'N', 'C', 1, 4, 0, 2,
      5, 0x00, 0x00, 0x80, 0x3E,
      5, 0x00, 0x00, 0x40, 0x3F,
  };
  std::vector<uint8_t> encoded_focus;
  NVE_CHECK(encode_camera_command(CameraPreviewCommand::focus_point(0.25f, 0.75f), encoded_focus) ==
            NativeViewCodecError::Ok);
  NVE_CHECK(encoded_focus == expected_focus);
  CameraPreviewCommand decoded_cmd{};
  NVE_CHECK(decode_camera_command(expected_focus, decoded_cmd) == NativeViewCodecError::Ok);
  NVE_CHECK(decoded_cmd == CameraPreviewCommand::focus_point(0.25f, 0.75f));

  const std::array<uint8_t, 40> expected_nvcf = {
      'N', 'V', 'C', 'F', 1, 3, 0x0E, 0x01,
      0x80, 0x07, 0x00, 0x00,
      0x38, 0x04, 0x00, 0x00,
      0x80, 0x07, 0x00, 0x00,
      0x00, 0x00, 0x00, 0x00,
      0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01,
      0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11,
  };
  const CameraFrameDescriptor desc{
      .width = 1920u,
      .height = 1080u,
      .stride_bytes = 1920u,
      .pixel_format = CameraPixelFormat::Yuv420_888,
      .rotation_deg = 270u,
      .timestamp_ns = 0x0102030405060708ull,
      .native_handle_id = 0x1122334455667788ull,
  };
  std::array<uint8_t, kCameraFrameDescriptorSize> encoded_nvcf{};
  NVE_CHECK(encode_camera_frame(desc, encoded_nvcf) == NativeViewCodecError::Ok);
  NVE_CHECK(encoded_nvcf == expected_nvcf);
  CameraFrameDescriptor decoded_nvcf{};
  NVE_CHECK(decode_camera_frame(expected_nvcf, decoded_nvcf) == NativeViewCodecError::Ok);
  NVE_CHECK(decoded_nvcf == desc);
}

void test_corrupt_nan_truncated_and_fuzz_rejection() {
  std::vector<uint8_t> buf;
  CameraPreviewProps bad_zoom{};
  bad_zoom.zoom = std::numeric_limits<float>::quiet_NaN();
  NVE_CHECK(encode_camera_props(bad_zoom, buf) == NativeViewCodecError::TypeMismatch);
  bad_zoom.zoom = -1.0f;
  NVE_CHECK(encode_camera_props(bad_zoom, buf) == NativeViewCodecError::TypeMismatch);

  CameraPreviewProps bad_fps{};
  bad_fps.fps = 241u;
  NVE_CHECK(encode_camera_props(bad_fps, buf) == NativeViewCodecError::InvalidEnum);

  NVE_CHECK(encode_camera_command(CameraPreviewCommand::focus_point(1.2f, 0.5f), buf) ==
            NativeViewCodecError::TypeMismatch);
  NVE_CHECK(encode_camera_command(CameraPreviewCommand::capture_photo(101u, 0u), buf) ==
            NativeViewCodecError::InvalidEnum);
  NVE_CHECK(encode_camera_command(CameraPreviewCommand::capture_photo(80u, 3u), buf) ==
            NativeViewCodecError::InvalidEnum);

  const CameraFrameDescriptor valid_desc{1280u, 720u, 1280u * 4u, CameraPixelFormat::Rgba8888, 90u, 100ull, 200ull};
  std::array<uint8_t, kCameraFrameDescriptorSize> valid_nvcf{};
  NVE_CHECK(encode_camera_frame(valid_desc, valid_nvcf) == NativeViewCodecError::Ok);
  CameraFrameDescriptor out_desc{};
  for (size_t cut = 0u; cut < kCameraFrameDescriptorSize; ++cut) {
    NVE_CHECK(decode_camera_frame(std::span<const uint8_t>(valid_nvcf.data(), cut), out_desc) ==
              NativeViewCodecError::Truncated);
  }
  std::vector<uint8_t> extra(valid_nvcf.begin(), valid_nvcf.end());
  extra.push_back(0xAA);
  NVE_CHECK(decode_camera_frame(extra, out_desc) == NativeViewCodecError::TrailingBytes);

  auto bad_rot = valid_nvcf;
  bad_rot[6] = 45u;
  NVE_CHECK(decode_camera_frame(bad_rot, out_desc) == NativeViewCodecError::InvalidEnum);
  auto bad_res = valid_nvcf;
  bad_res[20] = 1u;
  NVE_CHECK(decode_camera_frame(bad_res, out_desc) == NativeViewCodecError::InvalidEnum);

  uint64_t state = 0xCAFEBABE12345678ull;
  auto next_u8 = [&]() -> uint8_t {
    state = state * 6364136223846793005ull + 1442695040888963407ull;
    return static_cast<uint8_t>(state >> 33u);
  };
  for (int i = 0; i < 1000; ++i) {
    const size_t len = static_cast<size_t>((i * 11) % 72);
    std::vector<uint8_t> fuzz(len);
    for (auto& b : fuzz) b = next_u8();
    if (len >= 8u) {
      if (i % 3 == 0) { std::memcpy(fuzz.data(), kNativePropsMagic.data(), 4u); fuzz[4] = 1u; }
      else if (i % 3 == 1) { std::memcpy(fuzz.data(), kNativeCmdMagic.data(), 4u); fuzz[4] = 1u; }
      else { std::memcpy(fuzz.data(), kCameraFrameMagic.data(), 4u); fuzz[4] = 1u; }
    }
    CameraPreviewProps p{};
    CameraPreviewCommand c{};
    CameraFrameDescriptor f{};
    (void)decode_camera_props(fuzz, p);
    (void)decode_camera_command(fuzz, c);
    (void)decode_camera_frame(fuzz, f);
  }
}

void test_registry_and_vision_camera_v5_nitro_hybrid() {
  NveCameraRegistry cam_reg{};
  NVE_CHECK(!cam_reg.resolve_backend_kind().has_value());
  NVE_CHECK(cam_reg.create_backend(1u, 1u) == nullptr);
  cam_reg.register_backend(CameraBackendKind::VisionCameraV5, [](uint32_t, uint32_t) {
    return std::make_unique<MockCameraBackend>(CameraBackendKind::VisionCameraV5);
  });
  NVE_CHECK(cam_reg.resolve_backend_kind() == CameraBackendKind::VisionCameraV5);
  NVE_CHECK(cam_reg.create_backend(1u, 1u) != nullptr);

  nve::nitro::HybridObjectRegistry registry{};
  MockCameraBackend* captured = nullptr;
  nve::camera::register_vision_camera_hybrid(registry, [&]() -> std::unique_ptr<NveCameraBackend> {
    auto b = std::make_unique<MockCameraBackend>();
    captured = b.get();
    return b;
  });

  NVE_CHECK(registry.has_hybrid_object(nve::camera::kHybridCameraSessionName));
  auto session = registry.create<nve::camera::HybridCameraSession>(nve::camera::kHybridCameraSessionName);
  NVE_CHECK(session != nullptr);
  NVE_CHECK(session->name() == "CameraSession");
  NVE_CHECK(session->memory_size() >= sizeof(nve::camera::HybridCameraSession));
  NVE_CHECK(session->get_available_devices().size() == 2u);

  std::vector<CameraFrameDescriptor> received_frames;
  bool saw_borrowed_array_buffer = false;
  session->set_frame_processor(
      [&](const CameraFrameDescriptor& desc,
          std::span<const uint8_t, kCameraFrameDescriptorSize> nvcf,
          std::shared_ptr<nve::nitro::ArrayBuffer> ab) {
        CameraFrameDescriptor parsed{};
        NVE_CHECK(decode_camera_frame(nvcf, parsed) == NativeViewCodecError::Ok);
        NVE_CHECK(parsed == desc);
        NVE_CHECK(ab != nullptr && !ab->is_owning() && ab->size() == kCameraFrameDescriptorSize);
        saw_borrowed_array_buffer = true;
        received_frames.push_back(desc);
      });

  CameraPreviewProps props{};
  props.facing = CameraFacing::Back;
  props.torch = CameraTorchMode::On;
  props.zoom = 2.25f;
  props.fps = 60u;
  props.hdr = true;
  props.enable_frame_processor = true;
  props.audio_enabled = true;
  props.active = true;
  std::vector<uint8_t> nvnp;
  NVE_CHECK(encode_camera_props(props, nvnp) == NativeViewCodecError::Ok);
  NVE_CHECK(session->apply_encoded_props(nvnp) == NativeViewCodecError::Ok);
  NVE_CHECK(session->state() == CameraSessionState::Running);

  const CameraFrameDescriptor frame{1920u, 1080u, 1920u, CameraPixelFormat::Nv12, 90u, 424242ull, 0xABCD1234ull};
  NVE_CHECK(captured != nullptr);
  captured->emit_frame(frame);
  NVE_CHECK(received_frames.size() == 1u && received_frames[0] == frame);
  NVE_CHECK(saw_borrowed_array_buffer);
  NVE_CHECK(session->frame_count() == 1u);
  NVE_CHECK(session->last_frame_nvcf().has_value());

  auto bad_photo = session->take_photo(101u, 0u).await_result();
  NVE_CHECK(bad_photo.is_err() && bad_photo.error() == "INVALID_CAPTURE_OPTIONS");
  auto photo_res = session->take_photo(92u, 1u).await_result();
  NVE_CHECK(photo_res.is_ok());
  NVE_CHECK(photo_res.value().quality == 92u && photo_res.value().flash_mode == 1u);

  NVE_CHECK(session->start_recording("").await_result().is_err());
  NVE_CHECK(session->stop_recording().await_result().is_err());
  NVE_CHECK(session->start_recording("/tmp/nitro_vc_clip.mp4").await_result().is_ok());
  NVE_CHECK(session->state() == CameraSessionState::Recording);
  auto rec_res = session->stop_recording().await_result();
  NVE_CHECK(rec_res.is_ok());
  NVE_CHECK(rec_res.value().output_path == "/tmp/nitro_vc_clip.mp4");
  NVE_CHECK(rec_res.value().has_audio);
  NVE_CHECK(session->state() == CameraSessionState::Running);

  auto& global_reg = nve::nitro::HybridObjectRegistry::global();
  nve::camera::register_vision_camera_hybrid(global_reg, []() {
    return std::make_unique<MockCameraBackend>();
  });
  NveNitroHandle handle = nve_nitro_create("CameraSession");
  NVE_CHECK(handle != nullptr);
  NVE_CHECK(std::string_view(nve_nitro_get_name(handle)) == "CameraSession");
  uint8_t out_buf[64]{};
  size_t out_len = 0u;
  NVE_CHECK(nve_nitro_invoke_bytes(handle, "applyProps", nvnp.data(), nvnp.size(), out_buf, sizeof(out_buf), &out_len) ==
            NVE_NITRO_OK);
  NVE_CHECK(out_len == 1u && out_buf[0] == 1u);

  std::vector<uint8_t> zoom_cmd;
  NVE_CHECK(encode_camera_command(CameraPreviewCommand::set_zoom(3.0f), zoom_cmd) == NativeViewCodecError::Ok);
  NVE_CHECK(nve_nitro_invoke_bytes(handle, "dispatchCommand", zoom_cmd.data(), zoom_cmd.size(), out_buf, sizeof(out_buf), &out_len) ==
            NVE_NITRO_OK);
  NVE_CHECK(out_len == 1u && out_buf[0] == 1u);

  nve_nitro_dispose(handle);
  NVE_CHECK(nve_nitro_is_disposed(handle) == 1);
  NVE_CHECK(nve_nitro_invoke_bytes(handle, "start", nullptr, 0u, out_buf, sizeof(out_buf), &out_len) ==
            NVE_NITRO_ERR_DISPOSED);
  nve_nitro_release(handle);
  global_reg.unregister_hybrid_object("CameraSession");

  session->dispose();
  NVE_CHECK(session->is_disposed());
  NVE_CHECK(session->state() == CameraSessionState::Released);
  NVE_CHECK(session->take_photo().await_result().is_err());
}

} // namespace

int main() {
  test_golden_wire_parity_with_rust_and_kotlin();
  test_corrupt_nan_truncated_and_fuzz_rejection();
  test_registry_and_vision_camera_v5_nitro_hybrid();
  std::puts("nve_camera_test: ALL PASSED");
  return 0;
}
