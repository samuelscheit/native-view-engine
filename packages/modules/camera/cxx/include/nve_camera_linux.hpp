#pragma once

#include "nve_camera.hpp"

#include <algorithm>
#include <array>
#include <cstdint>
#include <memory>
#include <optional>
#include <string>
#include <utility>
#include <vector>

#if defined(__linux__)
#  include <fcntl.h>
#  include <linux/videodev2.h>
#  include <sys/ioctl.h>
#  include <unistd.h>
#endif

namespace nve {

/// Linux camera transport backend mode: PipeWire portal/daemon (`pw_stream` + DMA-BUF)
/// or direct V4L2 (`/dev/video*` `VIDIOC_*` + `V4L2_MEMORY_DMABUF`).
enum class LinuxCameraTransport : uint8_t {
  PipeWire = 0,
  V4l2 = 1,
};

struct LinuxCameraStreamConfig {
  std::string device_id;
  LinuxCameraTransport transport{LinuxCameraTransport::PipeWire};
  uint16_t target_fps{30u};
  bool hdr_enabled{false};
  CameraStabilizationMode stabilization{CameraStabilizationMode::Off};
  bool enable_dmabuf_frames{false};
  bool enable_audio{false};
  CameraPixelFormat pixel_format{CameraPixelFormat::Nv12};

  bool operator==(const LinuxCameraStreamConfig&) const = default;
};

/// Pluggable Linux PipeWire / V4L2 camera driver bridge.
class LinuxCameraBridge {
 public:
  virtual ~LinuxCameraBridge() = default;
  virtual LinuxCameraTransport transport() const noexcept = 0;
  virtual std::vector<CameraDeviceInfo> enumerate_devices() = 0;
  virtual bool open_and_configure(const LinuxCameraStreamConfig& config) = 0;
  virtual bool start_stream() = 0;
  virtual void stop_stream() = 0;
  virtual void set_torch(CameraTorchMode mode) = 0;
  virtual void set_zoom(float zoom) = 0;
  virtual void set_focus_point(float x, float y) = 0;
  virtual CapturedPhoto capture_photo(uint8_t quality, uint8_t flash_mode) = 0;
  virtual bool start_recording(std::string_view output_path, bool audio_enabled) = 0;
  virtual RecordedVideo stop_recording() = 0;
  virtual void set_frame_callback(std::function<void(const CameraFrameDescriptor&)> cb) = 0;
  virtual void close() = 0;
};

#if defined(__linux__)
/// Probes real Linux V4L2 video capture nodes (`/dev/video0..15`) via `VIDIOC_QUERYCAP`.
inline std::vector<CameraDeviceInfo> probe_linux_v4l2_devices() {
  std::vector<CameraDeviceInfo> found;
  for (int idx = 0; idx < 16; ++idx) {
    const std::string path = "/dev/video" + std::to_string(idx);
    const int fd = ::open(path.c_str(), O_RDWR | O_NONBLOCK);
    if (fd < 0) continue;
    v4l2_capability cap{};
    if (::ioctl(fd, VIDIOC_QUERYCAP, &cap) == 0) {
      const uint32_t caps = (cap.capabilities & V4L2_CAP_DEVICE_CAPS) ? cap.device_caps : cap.capabilities;
      if ((caps & V4L2_CAP_VIDEO_CAPTURE) != 0u) {
        found.push_back(CameraDeviceInfo{
            .id = path,
            .name = std::string(reinterpret_cast<const char*>(cap.card)),
            .facing = CameraFacing::External,
            .has_torch = false,
            .min_zoom = 1.0f,
            .max_zoom = 4.0f,
            .max_fps = 60u,
            .supports_hdr = false,
            .supported_stabilization = {CameraStabilizationMode::Off},
        });
      }
    }
    ::close(fd);
  }
  return found;
}
#endif

/// Deterministic in-memory PipeWire / V4L2 bridge for cross-platform tests & headless Linux CI.
class InMemoryLinuxCameraBridge final : public LinuxCameraBridge {
 public:
  explicit InMemoryLinuxCameraBridge(
      LinuxCameraTransport transport = LinuxCameraTransport::PipeWire,
      std::vector<CameraDeviceInfo> devices = {})
      : transport_(transport),
        devices_(devices.empty() ? default_devices(transport) : std::move(devices)) {}

  static std::vector<CameraDeviceInfo> default_devices(LinuxCameraTransport transport) {
    const std::string prefix = transport == LinuxCameraTransport::PipeWire ? "pipewire-node:" : "/dev/video";
    return {
        CameraDeviceInfo{
            .id = prefix + "0",
            .name = transport == LinuxCameraTransport::PipeWire ? "PipeWire Integrated Webcam" : "V4L2 Integrated Webcam",
            .facing = CameraFacing::Front,
            .has_torch = false,
            .min_zoom = 1.0f,
            .max_zoom = 4.0f,
            .max_fps = 60u,
            .supports_hdr = false,
            .supported_stabilization = {CameraStabilizationMode::Off, CameraStabilizationMode::Standard},
        },
        CameraDeviceInfo{
            .id = prefix + "2",
            .name = transport == LinuxCameraTransport::PipeWire ? "PipeWire USB 4K UVC" : "V4L2 USB 4K UVC",
            .facing = CameraFacing::External,
            .has_torch = true,
            .min_zoom = 1.0f,
            .max_zoom = 6.0f,
            .max_fps = 60u,
            .supports_hdr = true,
            .supported_stabilization = {CameraStabilizationMode::Off, CameraStabilizationMode::Standard},
        },
    };
  }

  LinuxCameraTransport transport() const noexcept override { return transport_; }
  std::vector<CameraDeviceInfo> enumerate_devices() override { return devices_; }

  bool open_and_configure(const LinuxCameraStreamConfig& config) override {
    last_config_ = config;
    ++configure_count_;
    return true;
  }

  bool start_stream() override { streaming_ = true; return true; }
  void stop_stream() override { streaming_ = false; }
  void set_torch(CameraTorchMode mode) override { torch_ = mode; }
  void set_zoom(float zoom) override { zoom_ = zoom; }
  void set_focus_point(float x, float y) override { last_focus_ = {x, y}; }

  CapturedPhoto capture_photo(uint8_t quality, uint8_t flash_mode) override {
    const uint32_t seq = ++capture_seq_;
    return CapturedPhoto{
        .file_path = "/tmp/nve_linux_capture_" + std::to_string(seq) + ".jpg",
        .width = 1920u,
        .height = 1080u,
        .quality = quality,
        .flash_mode = flash_mode,
        .timestamp_ns = static_cast<uint64_t>(seq) * 33'333'333ull,
    };
  }

  bool start_recording(std::string_view output_path, bool audio_enabled) override {
    recording_path_ = std::string(output_path);
    recording_audio_ = audio_enabled;
    return true;
  }

  RecordedVideo stop_recording() override {
    RecordedVideo v{};
    v.output_path = recording_path_.value_or("");
    v.duration_ms = 1500u;
    v.width = 1920u;
    v.height = 1080u;
    v.has_audio = recording_audio_;
    recording_path_.reset();
    recording_audio_ = false;
    return v;
  }

  void set_frame_callback(std::function<void(const CameraFrameDescriptor&)> cb) override {
    frame_cb_ = std::move(cb);
  }

  void close() override {
    streaming_ = false;
    frame_cb_ = nullptr;
    closed_ = true;
  }

  void push_dmabuf_frame(const CameraFrameDescriptor& desc) const {
    if (frame_cb_) frame_cb_(desc);
  }

  const std::optional<LinuxCameraStreamConfig>& last_config() const noexcept { return last_config_; }
  uint32_t configure_count() const noexcept { return configure_count_; }
  bool streaming() const noexcept { return streaming_; }
  CameraTorchMode torch() const noexcept { return torch_; }
  float zoom() const noexcept { return zoom_; }
  std::pair<float, float> last_focus() const noexcept { return last_focus_; }
  bool closed() const noexcept { return closed_; }

 private:
  LinuxCameraTransport transport_;
  std::vector<CameraDeviceInfo> devices_;
  std::optional<LinuxCameraStreamConfig> last_config_{};
  uint32_t configure_count_{0u};
  bool streaming_{false};
  CameraTorchMode torch_{CameraTorchMode::Off};
  float zoom_{1.0f};
  std::pair<float, float> last_focus_{0.5f, 0.5f};
  std::optional<std::string> recording_path_{};
  bool recording_audio_{false};
  uint32_t capture_seq_{0u};
  std::function<void(const CameraFrameDescriptor&)> frame_cb_{};
  bool closed_{false};
};

/// Linux PipeWire & V4L2 implementation of [NveCameraBackend].
class NveLinuxCameraBackend final : public NveCameraBackend {
 public:
  explicit NveLinuxCameraBackend(
      std::unique_ptr<LinuxCameraBridge> bridge = std::make_unique<InMemoryLinuxCameraBridge>(LinuxCameraTransport::PipeWire),
      CameraPixelFormat preferred_format = CameraPixelFormat::Nv12)
      : bridge_(std::move(bridge)), preferred_format_(preferred_format) {}

  CameraBackendKind backend_kind() const noexcept override {
    return bridge_->transport() == LinuxCameraTransport::PipeWire
               ? CameraBackendKind::LinuxPipeWire
               : CameraBackendKind::LinuxV4l2;
  }

  std::string_view backend_name() const noexcept override {
    return bridge_->transport() == LinuxCameraTransport::PipeWire ? "linux-pipewire" : "linux-v4l2";
  }

  CameraSessionState state() const noexcept override { return state_; }
  const CameraPreviewProps& active_props() const noexcept override { return active_props_; }
  std::optional<CameraDeviceInfo> selected_device() const override { return selected_device_; }

  std::vector<CameraDeviceInfo> list_devices() override {
    if (state_ == CameraSessionState::Released) return {};
    return bridge_->enumerate_devices();
  }

  void attach_listener(CameraEventListener* listener) override { listener_ = listener; }

  bool apply_props(const CameraPreviewProps& props) override {
    if (state_ == CameraSessionState::Released) return false;
    if (validate_camera_props(props) != NativeViewCodecError::Ok) return false;
    const auto devices = bridge_->enumerate_devices();
    const auto dev = select_camera_device(devices, props);
    if (!dev) {
      if (listener_) listener_->on_error("DEVICE_NOT_FOUND", "No matching Linux camera device found");
      return false;
    }
    const auto neg = negotiate_camera_settings(*dev, props);
    const LinuxCameraStreamConfig cfg{
        .device_id = neg.device.id,
        .transport = bridge_->transport(),
        .target_fps = neg.effective_fps,
        .hdr_enabled = neg.effective_hdr,
        .stabilization = neg.effective_stabilization,
        .enable_dmabuf_frames = neg.enable_frame_processor,
        .enable_audio = neg.audio_enabled,
        .pixel_format = preferred_format_,
    };

    const bool needs_reconfig = !selected_device_ || selected_device_->id != dev->id || last_config_ != cfg;
    if (needs_reconfig) {
      if (!bridge_->open_and_configure(cfg)) return false;
      selected_device_ = *dev;
      last_config_ = cfg;
      if (state_ == CameraSessionState::Idle) transition_to(CameraSessionState::Configured);
    }

    sync_frame_callback(neg.enable_frame_processor);
    bridge_->set_torch(neg.effective_torch);
    bridge_->set_zoom(neg.effective_zoom);

    active_props_ = props;
    active_props_.device_id = dev->id;
    active_props_.facing = dev->facing;
    active_props_.torch = neg.effective_torch;
    active_props_.zoom = neg.effective_zoom;
    active_props_.fps = neg.effective_fps;
    active_props_.hdr = neg.effective_hdr;
    active_props_.stabilization = neg.effective_stabilization;

    if (props.active && state_ != CameraSessionState::Running && state_ != CameraSessionState::Recording) {
      if (!bridge_->start_stream()) return false;
      transition_to(CameraSessionState::Running);
    } else if (!props.active && (state_ == CameraSessionState::Running || state_ == CameraSessionState::Recording)) {
      stop_internal();
    }
    return true;
  }

  bool dispatch_command(const CameraPreviewCommand& cmd) override {
    if (state_ == CameraSessionState::Released) return false;
    switch (cmd.kind) {
      case CameraCommandKind::SwitchFacing: {
        CameraPreviewProps next = active_props_;
        next.device_id = std::nullopt;
        next.facing = cmd.facing;
        return apply_props(next);
      }
      case CameraCommandKind::SetTorch: {
        if (!ensure_configured()) return false;
        const auto eff = selected_device_->has_torch ? cmd.torch : CameraTorchMode::Off;
        bridge_->set_torch(eff);
        active_props_.torch = eff;
        return true;
      }
      case CameraCommandKind::SetZoom: {
        if (!camera_detail::valid_zoom(cmd.zoom) || !ensure_configured()) return false;
        const float clamped = std::clamp(cmd.zoom, selected_device_->min_zoom, selected_device_->max_zoom);
        bridge_->set_zoom(clamped);
        active_props_.zoom = clamped;
        return true;
      }
      case CameraCommandKind::FocusPoint:
        if (!camera_detail::valid_unit_coord(cmd.focus_x) || !camera_detail::valid_unit_coord(cmd.focus_y)) return false;
        if (!ensure_configured()) return false;
        bridge_->set_focus_point(cmd.focus_x, cmd.focus_y);
        return true;
      case CameraCommandKind::CapturePhoto: {
        if (cmd.quality > 100u || cmd.flash > 2u || !ensure_configured()) return false;
        const auto photo = bridge_->capture_photo(cmd.quality, cmd.flash);
        if (listener_) listener_->on_photo_captured(photo);
        return true;
      }
      case CameraCommandKind::StartRecording:
        if (cmd.output_path.empty() || state_ == CameraSessionState::Recording || !ensure_configured()) return false;
        if (state_ != CameraSessionState::Running && !bridge_->start_stream()) return false;
        if (!bridge_->start_recording(cmd.output_path, active_props_.audio_enabled)) return false;
        transition_to(CameraSessionState::Recording);
        if (listener_) listener_->on_recording_started(cmd.output_path);
        return true;
      case CameraCommandKind::StopRecording: {
        if (state_ != CameraSessionState::Recording) return false;
        const auto video = bridge_->stop_recording();
        transition_to(CameraSessionState::Running);
        if (listener_) listener_->on_recording_finished(video);
        return true;
      }
    }
    return false;
  }

  bool start() override {
    if (state_ == CameraSessionState::Released) return false;
    CameraPreviewProps next = active_props_;
    next.active = true;
    return apply_props(next);
  }

  void stop() override {
    if (state_ == CameraSessionState::Released) return;
    active_props_.active = false;
    stop_internal();
  }

  void release() override {
    if (state_ == CameraSessionState::Released) return;
    if (state_ == CameraSessionState::Recording) {
      const auto video = bridge_->stop_recording();
      if (listener_) listener_->on_recording_finished(video);
    }
    if (frame_cb_installed_) {
      bridge_->set_frame_callback(nullptr);
      frame_cb_installed_ = false;
    }
    bridge_->stop_stream();
    bridge_->close();
    selected_device_.reset();
    last_config_.reset();
    transition_to(CameraSessionState::Released);
  }

  LinuxCameraBridge& bridge() noexcept { return *bridge_; }

 private:
  bool ensure_configured() { return selected_device_.has_value() || apply_props(active_props_); }

  void stop_internal() {
    if (state_ == CameraSessionState::Recording) {
      const auto video = bridge_->stop_recording();
      if (listener_) listener_->on_recording_finished(video);
    }
    bridge_->stop_stream();
    if (state_ != CameraSessionState::Idle) transition_to(CameraSessionState::Stopped);
  }

  void sync_frame_callback(bool enable) {
    if (enable && !frame_cb_installed_) {
      bridge_->set_frame_callback([this](const CameraFrameDescriptor& desc) {
        if (!listener_) return;
        std::array<uint8_t, kCameraFrameDescriptorSize> nvcf{};
        if (encode_camera_frame(desc, nvcf) == NativeViewCodecError::Ok) {
          listener_->on_frame(desc, nvcf);
        } else {
          listener_->on_error("INVALID_FRAME_DESCRIPTOR", "Malformed Linux DMA-BUF NVCF frame");
        }
      });
      frame_cb_installed_ = true;
    } else if (!enable && frame_cb_installed_) {
      bridge_->set_frame_callback(nullptr);
      frame_cb_installed_ = false;
    }
  }

  void transition_to(CameraSessionState next) {
    if (state_ != next) {
      state_ = next;
      if (listener_) listener_->on_state_changed(next);
    }
  }

  std::unique_ptr<LinuxCameraBridge> bridge_;
  CameraPixelFormat preferred_format_{CameraPixelFormat::Nv12};
  CameraSessionState state_{CameraSessionState::Idle};
  CameraPreviewProps active_props_{};
  std::optional<CameraDeviceInfo> selected_device_{};
  std::optional<LinuxCameraStreamConfig> last_config_{};
  CameraEventListener* listener_{nullptr};
  bool frame_cb_installed_{false};
};

/// Registers Linux PipeWire (preferred) and V4L2 fallback into [registry].
inline void register_linux_camera_backends(NveCameraRegistry& registry) {
  registry.register_backend(CameraBackendKind::LinuxPipeWire, [](uint32_t, uint32_t) {
    return std::make_unique<NveLinuxCameraBackend>(
        std::make_unique<InMemoryLinuxCameraBridge>(LinuxCameraTransport::PipeWire));
  });
  registry.register_backend(CameraBackendKind::LinuxV4l2, [](uint32_t, uint32_t) {
    return std::make_unique<NveLinuxCameraBackend>(
        std::make_unique<InMemoryLinuxCameraBridge>(LinuxCameraTransport::V4l2));
  });
}

} // namespace nve
