#include "nve_camera.hpp"
#include "nve_camera_linux.hpp"
#include "nve_camera_windows.hpp"
#include "nve_vision_camera_hybrid.hpp"

#include <cmath>
#include <cstdio>
#include <cstdlib>
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

struct RecordingCameraListener final : CameraEventListener {
  std::vector<CameraSessionState> states;
  std::vector<std::pair<CameraFrameDescriptor, std::array<uint8_t, kCameraFrameDescriptorSize>>> frames;
  std::vector<CapturedPhoto> photos;
  std::vector<std::string> rec_starts;
  std::vector<RecordedVideo> videos;
  std::vector<std::pair<std::string, std::string>> errors;

  void on_state_changed(CameraSessionState s) override { states.push_back(s); }
  void on_frame(const CameraFrameDescriptor& d,
                std::span<const uint8_t, kCameraFrameDescriptorSize> raw) override {
    std::array<uint8_t, kCameraFrameDescriptorSize> copy{};
    std::memcpy(copy.data(), raw.data(), kCameraFrameDescriptorSize);
    frames.emplace_back(d, copy);
  }
  void on_photo_captured(const CapturedPhoto& p) override { photos.push_back(p); }
  void on_recording_started(std::string_view p) override { rec_starts.emplace_back(p); }
  void on_recording_finished(const RecordedVideo& v) override { videos.push_back(v); }
  void on_error(std::string_view c, std::string_view m) override {
    errors.emplace_back(std::string(c), std::string(m));
  }
};

void test_golden_wire_parity_with_rust_and_kotlin() {
  // Golden NVNP bytes for default CameraPreviewProps
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

  // Golden NVNC bytes for FocusPoint(0.25f, 0.75f)
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

  // Golden NVCF 40-byte frame descriptor
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

  // Truncated and trailing-byte NVCF rejection
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

  // Deterministic 1,000-case fuzz loop across all 3 decoders
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

void test_windows_media_capture_backend() {
  auto bridge_ptr = std::make_unique<InMemoryWindowsMediaCaptureBridge>();
  auto* raw_bridge = bridge_ptr.get();
  NveWindowsCameraBackend backend(std::move(bridge_ptr));
  RecordingCameraListener listener{};
  backend.attach_listener(&listener);

  NVE_CHECK(backend.backend_kind() == CameraBackendKind::WindowsMediaCapture);
  NVE_CHECK(backend.list_devices().size() == 2u);

  CameraPreviewProps props{};
  props.facing = CameraFacing::Front;
  props.torch = CameraTorchMode::On;        // Front has no torch -> clamped Off
  props.zoom = 20.0f;                       // Front max_zoom=4.0 -> clamped 4.0
  props.fps = 240u;                         // Front max_fps=60 -> clamped 60
  props.hdr = true;
  props.stabilization = CameraStabilizationMode::Cinematic; // Unsupported on front -> Off
  props.enable_frame_processor = true;
  props.audio_enabled = true;
  props.active = true;

  std::vector<uint8_t> nvnp;
  NVE_CHECK(encode_camera_props(props, nvnp) == NativeViewCodecError::Ok);
  NVE_CHECK(backend.apply_encoded_props(nvnp) == NativeViewCodecError::Ok);
  NVE_CHECK(backend.state() == CameraSessionState::Running);
  NVE_CHECK(raw_bridge->configure_count() == 1u);
  NVE_CHECK(raw_bridge->torch() == CameraTorchMode::Off);
  NVE_CHECK(raw_bridge->zoom() == 4.0f);
  NVE_CHECK(backend.active_props().fps == 60u);
  NVE_CHECK(backend.active_props().stabilization == CameraStabilizationMode::Off);

  // Diff-aware zoom update must NOT reconfigure stream
  auto next = backend.active_props();
  next.zoom = 2.0f;
  NVE_CHECK(backend.apply_props(next));
  NVE_CHECK(raw_bridge->configure_count() == 1u);
  NVE_CHECK(raw_bridge->zoom() == 2.0f);

  // Zero-copy NVCF frame emission
  const CameraFrameDescriptor frame{1920u, 1080u, 1920u, CameraPixelFormat::Nv12, 0u, 777ull, 0xD3D11BEEFull};
  raw_bridge->push_simulated_frame(frame);
  NVE_CHECK(listener.frames.size() == 1u);
  CameraFrameDescriptor decoded{};
  NVE_CHECK(decode_camera_frame(listener.frames[0].second, decoded) == NativeViewCodecError::Ok);
  NVE_CHECK(decoded == frame);

  // Encoded NVNC commands: switch to Back, torch, focus, photo, record
  std::vector<uint8_t> cmd_bytes;
  NVE_CHECK(encode_camera_command(CameraPreviewCommand::switch_facing(CameraFacing::Back), cmd_bytes) == NativeViewCodecError::Ok);
  NVE_CHECK(backend.dispatch_encoded_command(cmd_bytes) == NativeViewCodecError::Ok);
  NVE_CHECK(raw_bridge->configure_count() == 2u);

  NVE_CHECK(encode_camera_command(CameraPreviewCommand::set_torch(CameraTorchMode::Auto), cmd_bytes) == NativeViewCodecError::Ok);
  NVE_CHECK(backend.dispatch_encoded_command(cmd_bytes) == NativeViewCodecError::Ok);
  NVE_CHECK(raw_bridge->torch() == CameraTorchMode::Auto);

  NVE_CHECK(encode_camera_command(CameraPreviewCommand::focus_point(0.3f, 0.7f), cmd_bytes) == NativeViewCodecError::Ok);
  NVE_CHECK(backend.dispatch_encoded_command(cmd_bytes) == NativeViewCodecError::Ok);
  NVE_CHECK(std::abs(raw_bridge->last_focus().first - 0.3f) < 1e-5f);

  NVE_CHECK(encode_camera_command(CameraPreviewCommand::capture_photo(95u, 1u), cmd_bytes) == NativeViewCodecError::Ok);
  NVE_CHECK(backend.dispatch_encoded_command(cmd_bytes) == NativeViewCodecError::Ok);
  NVE_CHECK(listener.photos.size() == 1u);

  NVE_CHECK(encode_camera_command(CameraPreviewCommand::start_recording("C:\\temp\\win_clip.mp4"), cmd_bytes) == NativeViewCodecError::Ok);
  NVE_CHECK(backend.dispatch_encoded_command(cmd_bytes) == NativeViewCodecError::Ok);
  NVE_CHECK(backend.state() == CameraSessionState::Recording);

  NVE_CHECK(encode_camera_command(CameraPreviewCommand::stop_recording(), cmd_bytes) == NativeViewCodecError::Ok);
  NVE_CHECK(backend.dispatch_encoded_command(cmd_bytes) == NativeViewCodecError::Ok);
  NVE_CHECK(backend.state() == CameraSessionState::Running);
  NVE_CHECK(listener.videos.size() == 1u);
  NVE_CHECK(listener.videos[0].output_path == "C:\\temp\\win_clip.mp4");

  backend.release();
  NVE_CHECK(backend.state() == CameraSessionState::Released);
  NVE_CHECK(raw_bridge->released());
  NVE_CHECK(!backend.dispatch_command(CameraPreviewCommand::set_zoom(2.0f)));
}

void test_linux_pipewire_v4l2_and_registry_precedence() {
  NveCameraRegistry registry{};
  register_linux_camera_backends(registry);
  NVE_CHECK(registry.resolve_backend_kind() == CameraBackendKind::LinuxPipeWire);

  // PipeWire preferred over V4L2
  auto pw = registry.create_backend(1u, 10u);
  NVE_CHECK(pw != nullptr);
  NVE_CHECK(pw->backend_kind() == CameraBackendKind::LinuxPipeWire);
  NVE_CHECK(pw->backend_name() == "linux-pipewire");

  // Explicit V4L2 fallback
  auto v4l2 = registry.create_backend(1u, 11u, CameraBackendKind::LinuxV4l2);
  NVE_CHECK(v4l2 != nullptr);
  NVE_CHECK(v4l2->backend_kind() == CameraBackendKind::LinuxV4l2);
  NVE_CHECK(v4l2->backend_name() == "linux-v4l2");

  // Unregister PipeWire -> falls back to V4L2 automatically
  NVE_CHECK(registry.unregister_backend(CameraBackendKind::LinuxPipeWire));
  NVE_CHECK(registry.resolve_backend_kind() == CameraBackendKind::LinuxV4l2);

  // Exercise full Linux PipeWire DMA-BUF + recording lifecycle
  auto bridge_ptr = std::make_unique<InMemoryLinuxCameraBridge>(LinuxCameraTransport::PipeWire);
  auto* raw_bridge = bridge_ptr.get();
  NveLinuxCameraBackend linux_cam(std::move(bridge_ptr));
  RecordingCameraListener listener{};
  linux_cam.attach_listener(&listener);

  CameraPreviewProps props{};
  props.facing = CameraFacing::External;
  props.torch = CameraTorchMode::On;
  props.zoom = 3.0f;
  props.fps = 60u;
  props.hdr = true;
  props.enable_frame_processor = true;
  props.active = true;
  NVE_CHECK(linux_cam.apply_props(props));
  NVE_CHECK(linux_cam.state() == CameraSessionState::Running);
  NVE_CHECK(raw_bridge->streaming());
  NVE_CHECK(raw_bridge->last_config()->device_id == "pipewire-node:2");
  NVE_CHECK(raw_bridge->last_config()->enable_dmabuf_frames);

  const CameraFrameDescriptor dmabuf_frame{1280u, 720u, 1280u, CameraPixelFormat::Nv12, 0u, 123456ull, 42ull};
  raw_bridge->push_dmabuf_frame(dmabuf_frame);
  NVE_CHECK(listener.frames.size() == 1u);
  NVE_CHECK(listener.frames[0].first == dmabuf_frame);

  NVE_CHECK(linux_cam.dispatch_command(CameraPreviewCommand::start_recording("/tmp/pw_clip.mkv")));
  NVE_CHECK(linux_cam.state() == CameraSessionState::Recording);
  linux_cam.stop();
  NVE_CHECK(linux_cam.state() == CameraSessionState::Stopped);
  NVE_CHECK(listener.videos.size() == 1u);
  NVE_CHECK(!raw_bridge->streaming());

  linux_cam.release();
  NVE_CHECK(linux_cam.state() == CameraSessionState::Released);
  NVE_CHECK(raw_bridge->closed());
}

void test_vision_camera_v5_nitro_hybrid_standalone() {
  // Register VisionCamera v5 HybridCameraSession in standalone Nitro registry (zero RN / zero JSI)
  nve::nitro::HybridObjectRegistry registry{};
  InMemoryLinuxCameraBridge* captured_bridge = nullptr;
  nve::camera::register_vision_camera_hybrid(registry, [&]() -> std::unique_ptr<NveCameraBackend> {
    auto bridge = std::make_unique<InMemoryLinuxCameraBridge>(LinuxCameraTransport::PipeWire);
    captured_bridge = bridge.get();
    return std::make_unique<NveLinuxCameraBackend>(std::move(bridge));
  });

  NVE_CHECK(registry.has_hybrid_object(nve::camera::kHybridCameraSessionName));
  auto session = registry.create<nve::camera::HybridCameraSession>(nve::camera::kHybridCameraSessionName);
  NVE_CHECK(session != nullptr);
  NVE_CHECK(session->name() == "CameraSession");
  NVE_CHECK(session->memory_size() >= sizeof(nve::camera::HybridCameraSession));
  NVE_CHECK(session->get_available_devices().size() == 2u);

  // Configure via binary NVNP props and attach zero-copy NVCF frame processor
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
  props.facing = CameraFacing::External;
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

  // Zero-copy NVCF frame delivery through HybridCameraSession
  const CameraFrameDescriptor frame{1920u, 1080u, 1920u, CameraPixelFormat::Nv12, 90u, 424242ull, 0xABCD1234ull};
  NVE_CHECK(captured_bridge != nullptr);
  captured_bridge->push_dmabuf_frame(frame);
  NVE_CHECK(received_frames.size() == 1u && received_frames[0] == frame);
  NVE_CHECK(saw_borrowed_array_buffer);
  NVE_CHECK(session->frame_count() == 1u);
  NVE_CHECK(session->last_frame_nvcf().has_value());

  // VisionCamera v5 Promise-returning take_photo() + invalid option rejection
  auto bad_photo = session->take_photo(101u, 0u).await_result();
  NVE_CHECK(bad_photo.is_err() && bad_photo.error() == "INVALID_CAPTURE_OPTIONS");
  auto photo_res = session->take_photo(92u, 1u).await_result();
  NVE_CHECK(photo_res.is_ok());
  NVE_CHECK(photo_res.value().quality == 92u && photo_res.value().flash_mode == 1u);

  // VisionCamera v5 Promise-returning start_recording() / stop_recording()
  NVE_CHECK(session->start_recording("").await_result().is_err());
  NVE_CHECK(session->stop_recording().await_result().is_err());
  NVE_CHECK(session->start_recording("/tmp/nitro_vc_clip.mp4").await_result().is_ok());
  NVE_CHECK(session->state() == CameraSessionState::Recording);
  auto rec_res = session->stop_recording().await_result();
  NVE_CHECK(rec_res.is_ok());
  NVE_CHECK(rec_res.value().output_path == "/tmp/nitro_vc_clip.mp4");
  NVE_CHECK(rec_res.value().has_audio);
  NVE_CHECK(session->state() == CameraSessionState::Running);

  // Direct C-ABI NveNitroHandle zero-JS-VM bridge test against global registry
  auto& global_reg = nve::nitro::HybridObjectRegistry::global();
  nve::camera::register_vision_camera_hybrid(global_reg, []() {
    return std::make_unique<NveWindowsCameraBackend>();
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

  // Disposing session cleans up backend & rejects subsequent captures
  session->dispose();
  NVE_CHECK(session->is_disposed());
  NVE_CHECK(session->state() == CameraSessionState::Released);
  NVE_CHECK(session->take_photo().await_result().is_err());
}

} // namespace

int main() {
  test_golden_wire_parity_with_rust_and_kotlin();
  test_corrupt_nan_truncated_and_fuzz_rejection();
  test_windows_media_capture_backend();
  test_linux_pipewire_v4l2_and_registry_precedence();
  test_vision_camera_v5_nitro_hybrid_standalone();
  std::puts("nve_camera_test: ALL PASSED");
  return 0;
}
