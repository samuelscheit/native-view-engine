#pragma once

#include "nve_camera.hpp"

#include <atomic>
#include <chrono>
#include <cstdint>
#include <memory>
#include <optional>
#include <string>
#include <utility>
#include <vector>

#if defined(_WIN32) && defined(NVE_ENABLE_WIN32_MF)
#  include <mfapi.h>
#  include <mfidl.h>
#  include <mfreadwrite.h>
#  include <strmif.h>
#endif

namespace nve {

/// Stream configuration mapped onto Windows Media Foundation / WinRT `MediaCapture`.
struct WindowsCaptureStreamConfig {
  std::string device_id;
  uint16_t target_fps{30u};
  bool hdr_enabled{false};
  CameraStabilizationMode stabilization{CameraStabilizationMode::Off};
  bool enable_frame_reader{false};
  bool enable_audio{false};
  CameraPixelFormat pixel_format{CameraPixelFormat::Nv12};

  bool operator==(const WindowsCaptureStreamConfig&) const = default;
};

/// Pluggable bridge connecting [NveWindowsCameraBackend] to Windows Media Foundation
/// (`IMFMediaSource`/`IMFSourceReader`/`IMFDXGIBuffer`) or WinRT `MediaCapture`.
class WindowsMediaCaptureBridge {
 public:
  virtual ~WindowsMediaCaptureBridge() = default;
  virtual std::vector<CameraDeviceInfo> enumerate_devices() = 0;
  virtual bool configure_stream(const WindowsCaptureStreamConfig& config) = 0;
  virtual bool start_preview() = 0;
  virtual void stop_preview() = 0;
  virtual void set_torch(CameraTorchMode mode) = 0;
  virtual void set_zoom(float zoom) = 0;
  virtual void set_focus_point(float x, float y) = 0;
  virtual CapturedPhoto capture_photo(uint8_t quality, uint8_t flash_mode) = 0;
  virtual bool start_recording(std::string_view output_path, bool audio_enabled) = 0;
  virtual RecordedVideo stop_recording() = 0;
  virtual void set_frame_callback(std::function<void(const CameraFrameDescriptor&)> cb) = 0;
  virtual void release() = 0;
};

/// Deterministic in-memory Windows `MediaCapture` bridge for cross-platform C++ tests
/// and headless CI where no physical UVC/MIPI webcam is attached.
class InMemoryWindowsMediaCaptureBridge final : public WindowsMediaCaptureBridge {
 public:
  explicit InMemoryWindowsMediaCaptureBridge(std::vector<CameraDeviceInfo> devices = default_devices())
      : devices_(std::move(devices)) {}

  static std::vector<CameraDeviceInfo> default_devices() {
    return {
        CameraDeviceInfo{
            .id = R"(\\?\usb#vid_045e&pid_075d#win-front-0)",
            .name = "Surface Studio Camera Front",
            .facing = CameraFacing::Front,
            .has_torch = false,
            .min_zoom = 1.0f,
            .max_zoom = 4.0f,
            .max_fps = 60u,
            .supports_hdr = true,
            .supported_stabilization = {CameraStabilizationMode::Off, CameraStabilizationMode::Standard},
        },
        CameraDeviceInfo{
            .id = R"(\\?\usb#vid_045e&pid_075e#win-back-1)",
            .name = "Surface Rear 4K Camera",
            .facing = CameraFacing::Back,
            .has_torch = true,
            .min_zoom = 0.5f,
            .max_zoom = 8.0f,
            .max_fps = 60u,
            .supports_hdr = true,
            .supported_stabilization = {CameraStabilizationMode::Off, CameraStabilizationMode::Standard,
                                        CameraStabilizationMode::Cinematic},
        },
    };
  }

  std::vector<CameraDeviceInfo> enumerate_devices() override { return devices_; }

  bool configure_stream(const WindowsCaptureStreamConfig& config) override {
    last_config_ = config;
    ++configure_count_;
    return true;
  }

  bool start_preview() override { preview_running_ = true; return true; }
  void stop_preview() override { preview_running_ = false; }
  void set_torch(CameraTorchMode mode) override { torch_ = mode; }
  void set_zoom(float zoom) override { zoom_ = zoom; }
  void set_focus_point(float x, float y) override { last_focus_ = {x, y}; }

  CapturedPhoto capture_photo(uint8_t quality, uint8_t flash_mode) override {
    const uint32_t seq = ++capture_seq_;
    return CapturedPhoto{
        .file_path = "C:\\Users\\Public\\Pictures\\nve_win_capture_" + std::to_string(seq) + ".jpg",
        .width = 1920u,
        .height = 1080u,
        .quality = quality,
        .flash_mode = flash_mode,
        .timestamp_ns = static_cast<uint64_t>(seq) * 16'666'666ull,
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
    v.duration_ms = 1200u;
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

  void release() override {
    preview_running_ = false;
    frame_cb_ = nullptr;
    released_ = true;
  }

  void push_simulated_frame(const CameraFrameDescriptor& desc) const {
    if (frame_cb_) frame_cb_(desc);
  }

  const std::optional<WindowsCaptureStreamConfig>& last_config() const noexcept { return last_config_; }
  uint32_t configure_count() const noexcept { return configure_count_; }
  bool preview_running() const noexcept { return preview_running_; }
  CameraTorchMode torch() const noexcept { return torch_; }
  float zoom() const noexcept { return zoom_; }
  std::pair<float, float> last_focus() const noexcept { return last_focus_; }
  bool released() const noexcept { return released_; }

 private:
  std::vector<CameraDeviceInfo> devices_;
  std::optional<WindowsCaptureStreamConfig> last_config_{};
  uint32_t configure_count_{0u};
  bool preview_running_{false};
  CameraTorchMode torch_{CameraTorchMode::Off};
  float zoom_{1.0f};
  std::pair<float, float> last_focus_{0.5f, 0.5f};
  std::optional<std::string> recording_path_{};
  bool recording_audio_{false};
  uint32_t capture_seq_{0u};
  std::function<void(const CameraFrameDescriptor&)> frame_cb_{};
  bool released_{false};
};

#if defined(_WIN32) && defined(NVE_ENABLE_WIN32_MF)
/// Native Windows Media Foundation (`IMFMediaSource` / `IMFSourceReader`) device enumerator
/// helper compiled on Win32 hosts linking `mfplat.lib` + `mf.lib` + `mfreadwrite.lib`.
inline std::vector<CameraDeviceInfo> enumerate_windows_mf_devices() {
  std::vector<CameraDeviceInfo> result;
  if (FAILED(MFStartup(MF_VERSION, MFSTARTUP_LITE))) return result;
  IMFAttributes* attrs = nullptr;
  if (SUCCEEDED(MFCreateAttributes(&attrs, 1))) {
    attrs->SetGUID(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID);
    IMFActivate** devices = nullptr;
    UINT32 count = 0;
    if (SUCCEEDED(MFEnumDeviceSources(attrs, &devices, &count))) {
      for (UINT32 i = 0; i < count; ++i) {
        WCHAR* friendly = nullptr;
        WCHAR* symlink = nullptr;
        UINT32 flen = 0, slen = 0;
        devices[i]->GetAllocatedString(MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, &friendly, &flen);
        devices[i]->GetAllocatedString(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK, &symlink, &slen);
        auto narrow = [](const WCHAR* ws, UINT32 n) {
          std::string s;
          if (!ws) return s;
          s.reserve(n);
          for (UINT32 k = 0; k < n; ++k) s.push_back(ws[k] < 128 ? static_cast<char>(ws[k]) : '?');
          return s;
        };
        result.push_back(CameraDeviceInfo{
            .id = symlink ? narrow(symlink, slen) : ("win-mf-" + std::to_string(i)),
            .name = friendly ? narrow(friendly, flen) : ("Media Foundation Camera " + std::to_string(i)),
            .facing = CameraFacing::External,
            .has_torch = false,
            .min_zoom = 1.0f,
            .max_zoom = 4.0f,
            .max_fps = 60u,
            .supports_hdr = false,
            .supported_stabilization = {CameraStabilizationMode::Off},
        });
        if (friendly) CoTaskMemFree(friendly);
        if (symlink) CoTaskMemFree(symlink);
        devices[i]->Release();
      }
      CoTaskMemFree(devices);
    }
    attrs->Release();
  }
  MFShutdown();
  return result;
}
#endif

/// Windows Media Foundation / `MediaCapture` implementation of [NveCameraBackend].
class NveWindowsCameraBackend final : public NveCameraBackend {
 public:
  explicit NveWindowsCameraBackend(
      std::unique_ptr<WindowsMediaCaptureBridge> bridge = std::make_unique<InMemoryWindowsMediaCaptureBridge>(),
      CameraPixelFormat preferred_format = CameraPixelFormat::Nv12)
      : bridge_(std::move(bridge)), preferred_format_(preferred_format) {}

  CameraBackendKind backend_kind() const noexcept override { return CameraBackendKind::WindowsMediaCapture; }
  std::string_view backend_name() const noexcept override { return "windows-media-capture"; }
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
      if (listener_) listener_->on_error("DEVICE_NOT_FOUND", "No matching Windows MediaCapture camera found");
      return false;
    }
    const auto neg = negotiate_camera_settings(*dev, props);
    const WindowsCaptureStreamConfig cfg{
        .device_id = neg.device.id,
        .target_fps = neg.effective_fps,
        .hdr_enabled = neg.effective_hdr,
        .stabilization = neg.effective_stabilization,
        .enable_frame_reader = neg.enable_frame_processor,
        .enable_audio = neg.audio_enabled,
        .pixel_format = preferred_format_,
    };

    const bool needs_reconfig = !selected_device_ || selected_device_->id != dev->id || last_config_ != cfg;
    if (needs_reconfig) {
      if (!bridge_->configure_stream(cfg)) return false;
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
      if (!bridge_->start_preview()) return false;
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
        if (state_ != CameraSessionState::Running && !bridge_->start_preview()) return false;
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
    bridge_->stop_preview();
    bridge_->release();
    selected_device_.reset();
    last_config_.reset();
    transition_to(CameraSessionState::Released);
  }

  WindowsMediaCaptureBridge& bridge() noexcept { return *bridge_; }

 private:
  bool ensure_configured() { return selected_device_.has_value() || apply_props(active_props_); }

  void stop_internal() {
    if (state_ == CameraSessionState::Recording) {
      const auto video = bridge_->stop_recording();
      if (listener_) listener_->on_recording_finished(video);
    }
    bridge_->stop_preview();
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
          listener_->on_error("INVALID_FRAME_DESCRIPTOR", "Malformed Windows NVCF frame");
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

  std::unique_ptr<WindowsMediaCaptureBridge> bridge_;
  CameraPixelFormat preferred_format_{CameraPixelFormat::Nv12};
  CameraSessionState state_{CameraSessionState::Idle};
  CameraPreviewProps active_props_{};
  std::optional<CameraDeviceInfo> selected_device_{};
  std::optional<WindowsCaptureStreamConfig> last_config_{};
  CameraEventListener* listener_{nullptr};
  bool frame_cb_installed_{false};
};

} // namespace nve
