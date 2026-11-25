#pragma once

#include "nve_camera_codec.hpp"

#include <algorithm>
#include <cstdint>
#include <functional>
#include <memory>
#include <mutex>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <unordered_map>
#include <utility>
#include <vector>

namespace nve {

enum class CameraBackendKind : uint8_t {
  VisionCameraV5 = 0,
  Custom = 1,
};

enum class CameraSessionState : uint8_t {
  Idle = 0,
  Configured = 1,
  Running = 2,
  Recording = 3,
  Stopped = 4,
  Released = 5,
};

struct CameraDeviceInfo {
  std::string id;
  std::string name;
  CameraFacing facing{CameraFacing::Back};
  bool has_torch{false};
  float min_zoom{1.0f};
  float max_zoom{4.0f};
  uint16_t max_fps{60u};
  bool supports_hdr{false};
  std::vector<CameraStabilizationMode> supported_stabilization{CameraStabilizationMode::Off};
};

struct CapturedPhoto {
  std::string file_path;
  uint32_t width{0u};
  uint32_t height{0u};
  uint8_t quality{90u};
  uint8_t flash_mode{0u};
  uint64_t timestamp_ns{0u};
};

struct RecordedVideo {
  std::string output_path;
  uint64_t duration_ms{0u};
  uint32_t width{0u};
  uint32_t height{0u};
  bool has_audio{false};
};

struct CameraEventListener {
  virtual ~CameraEventListener() = default;
  virtual void on_state_changed(CameraSessionState) {}
  virtual void on_frame(const CameraFrameDescriptor&, std::span<const uint8_t, kCameraFrameDescriptorSize>) {}
  virtual void on_photo_captured(const CapturedPhoto&) {}
  virtual void on_recording_started(std::string_view) {}
  virtual void on_recording_finished(const RecordedVideo&) {}
  virtual void on_error(std::string_view, std::string_view) {}
};

struct NegotiatedCameraSettings {
  CameraDeviceInfo device;
  uint16_t effective_fps{30u};
  bool effective_hdr{false};
  CameraStabilizationMode effective_stabilization{CameraStabilizationMode::Off};
  CameraTorchMode effective_torch{CameraTorchMode::Off};
  float effective_zoom{1.0f};
  bool enable_frame_processor{false};
  bool audio_enabled{false};
};

inline std::optional<CameraDeviceInfo> select_camera_device(
    const std::vector<CameraDeviceInfo>& devices,
    const CameraPreviewProps& props) {
  if (devices.empty()) return std::nullopt;
  if (props.device_id && !props.device_id->empty()) {
    for (const auto& d : devices) {
      if (d.id == *props.device_id) return d;
    }
    return std::nullopt;
  }
  for (const auto& d : devices) {
    if (d.facing == props.facing) return d;
  }
  return devices.front();
}

inline NegotiatedCameraSettings negotiate_camera_settings(
    const CameraDeviceInfo& device,
    const CameraPreviewProps& props) {
  NegotiatedCameraSettings out{};
  out.device = device;
  const uint16_t cap_fps = device.max_fps == 0u ? uint16_t{1u} : device.max_fps;
  out.effective_fps = std::clamp<uint16_t>(props.fps, uint16_t{1u}, cap_fps);
  out.effective_hdr = props.hdr && device.supports_hdr;
  const bool stab_ok = std::find(device.supported_stabilization.begin(),
                                 device.supported_stabilization.end(),
                                 props.stabilization) != device.supported_stabilization.end();
  out.effective_stabilization = stab_ok ? props.stabilization : CameraStabilizationMode::Off;
  out.effective_torch = device.has_torch ? props.torch : CameraTorchMode::Off;
  out.effective_zoom = std::clamp(props.zoom, device.min_zoom, device.max_zoom);
  out.enable_frame_processor = props.enable_frame_processor;
  out.audio_enabled = props.audio_enabled;
  return out;
}

class NveCameraBackend {
 public:
  virtual ~NveCameraBackend() = default;
  virtual CameraBackendKind backend_kind() const noexcept = 0;
  virtual std::string_view backend_name() const noexcept = 0;
  virtual CameraSessionState state() const noexcept = 0;
  virtual const CameraPreviewProps& active_props() const noexcept = 0;
  virtual std::optional<CameraDeviceInfo> selected_device() const = 0;
  virtual std::vector<CameraDeviceInfo> list_devices() = 0;
  virtual void attach_listener(CameraEventListener* listener) = 0;
  virtual bool apply_props(const CameraPreviewProps& props) = 0;
  virtual bool dispatch_command(const CameraPreviewCommand& cmd) = 0;
  virtual bool start() = 0;
  virtual void stop() = 0;
  virtual void release() = 0;

  NativeViewCodecError apply_encoded_props(std::span<const uint8_t> nvnp_bytes) {
    CameraPreviewProps props{};
    if (auto err = decode_camera_props(nvnp_bytes, props); err != NativeViewCodecError::Ok) return err;
    return apply_props(props) ? NativeViewCodecError::Ok : NativeViewCodecError::InvalidEnum;
  }

  NativeViewCodecError dispatch_encoded_command(std::span<const uint8_t> nvnc_bytes) {
    CameraPreviewCommand cmd{};
    if (auto err = decode_camera_command(nvnc_bytes, cmd); err != NativeViewCodecError::Ok) return err;
    return dispatch_command(cmd) ? NativeViewCodecError::Ok : NativeViewCodecError::InvalidEnum;
  }
};

using CameraBackendFactory = std::function<std::unique_ptr<NveCameraBackend>(uint32_t surface_id, uint32_t node_id)>;

class NveCameraRegistry {
 public:
  void register_backend(CameraBackendKind kind, CameraBackendFactory factory) {
    std::lock_guard<std::mutex> lock(mu_);
    factories_[static_cast<uint8_t>(kind)] = std::move(factory);
  }

  bool unregister_backend(CameraBackendKind kind) {
    std::lock_guard<std::mutex> lock(mu_);
    return factories_.erase(static_cast<uint8_t>(kind)) > 0u;
  }

  void clear() {
    std::lock_guard<std::mutex> lock(mu_);
    factories_.clear();
  }

  std::optional<CameraBackendKind> resolve_backend_kind() const {
    std::lock_guard<std::mutex> lock(mu_);
    for (CameraBackendKind k : {CameraBackendKind::VisionCameraV5, CameraBackendKind::Custom}) {
      if (factories_.contains(static_cast<uint8_t>(k))) return k;
    }
    return std::nullopt;
  }

  std::unique_ptr<NveCameraBackend> create_backend(
      uint32_t surface_id,
      uint32_t node_id,
      std::optional<CameraBackendKind> preferred = std::nullopt) const {
    CameraBackendFactory factory{};
    {
      std::lock_guard<std::mutex> lock(mu_);
      if (preferred) {
        auto it = factories_.find(static_cast<uint8_t>(*preferred));
        if (it == factories_.end()) return nullptr;
        factory = it->second;
      } else {
        for (CameraBackendKind k : {CameraBackendKind::VisionCameraV5, CameraBackendKind::Custom}) {
          auto it = factories_.find(static_cast<uint8_t>(k));
          if (it != factories_.end()) { factory = it->second; break; }
        }
      }
    }
    return factory ? factory(surface_id, node_id) : nullptr;
  }

 private:
  mutable std::mutex mu_;
  std::unordered_map<uint8_t, CameraBackendFactory> factories_;
};

} // namespace nve
