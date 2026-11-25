#pragma once

#include "nve_camera.hpp"
#include "nve_nitro_core.hpp"

#include <array>
#include <cstdint>
#include <functional>
#include <memory>
#include <mutex>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

namespace nve::camera {

inline constexpr std::string_view kHybridCameraSessionName = "CameraSession";

/// VisionCamera v5 Nitro `HybridCameraSession` adapter inheriting `nve::nitro::HybridObject`.
///
/// Wraps an injected host `nve::NveCameraBackend` (such as a VisionCamera v5 hardware bridge)
/// and plugs into RN Fabric / Expo via `HybridObject::as_jsi_native_state()` when
/// `NVE_WITH_RN_JSI=1`.
class HybridCameraSession final : public nve::nitro::HybridObject, private nve::CameraEventListener {
 public:
  using FrameCallback = std::function<void(
      const nve::CameraFrameDescriptor&,
      std::span<const uint8_t, nve::kCameraFrameDescriptorSize>,
      std::shared_ptr<nve::nitro::ArrayBuffer>)>;

  explicit HybridCameraSession(std::unique_ptr<nve::NveCameraBackend> backend)
      : nve::nitro::HybridObject(std::string(kHybridCameraSessionName)),
        backend_(std::move(backend)) {
    if (backend_) {
      backend_->attach_listener(this);
    }
  }

  ~HybridCameraSession() override { dispose(); }

  size_t memory_size() const noexcept override {
    return sizeof(*this) + sizeof(nve::CameraPreviewProps);
  }

  void dispose() override {
    std::lock_guard<std::mutex> lock(mu_);
    if (is_disposed()) return;
    if (backend_) {
      backend_->attach_listener(nullptr);
      backend_->release();
    }
    frame_cb_ = nullptr;
    if (pending_photo_) {
      pending_photo_->reject("SESSION_DISPOSED");
      pending_photo_.reset();
    }
    if (pending_video_) {
      pending_video_->reject("SESSION_DISPOSED");
      pending_video_.reset();
    }
    mark_disposed();
  }

  nve::CameraSessionState state() const noexcept {
    std::lock_guard<std::mutex> lock(mu_);
    return backend_ ? backend_->state() : nve::CameraSessionState::Released;
  }

  nve::CameraPreviewProps active_props() const {
    std::lock_guard<std::mutex> lock(mu_);
    return backend_ ? backend_->active_props() : nve::CameraPreviewProps{};
  }

  std::vector<nve::CameraDeviceInfo> get_available_devices() {
    std::lock_guard<std::mutex> lock(mu_);
    if (is_disposed() || !backend_) return {};
    return backend_->list_devices();
  }

  bool configure(const nve::CameraPreviewProps& props) {
    std::lock_guard<std::mutex> lock(mu_);
    if (is_disposed() || !backend_) return false;
    return backend_->apply_props(props);
  }

  nve::NativeViewCodecError apply_encoded_props(std::span<const uint8_t> nvnp_bytes) {
    std::lock_guard<std::mutex> lock(mu_);
    if (is_disposed() || !backend_) return nve::NativeViewCodecError::InvalidEnum;
    return backend_->apply_encoded_props(nvnp_bytes);
  }

  nve::NativeViewCodecError dispatch_encoded_command(std::span<const uint8_t> nvnc_bytes) {
    std::lock_guard<std::mutex> lock(mu_);
    if (is_disposed() || !backend_) return nve::NativeViewCodecError::InvalidEnum;
    return backend_->dispatch_encoded_command(nvnc_bytes);
  }

  bool start() {
    std::lock_guard<std::mutex> lock(mu_);
    if (is_disposed() || !backend_) return false;
    return backend_->start();
  }

  void stop() {
    std::lock_guard<std::mutex> lock(mu_);
    if (is_disposed() || !backend_) return;
    backend_->stop();
  }

  bool switch_facing(nve::CameraFacing facing) {
    std::lock_guard<std::mutex> lock(mu_);
    if (is_disposed() || !backend_) return false;
    return backend_->dispatch_command(nve::CameraPreviewCommand::switch_facing(facing));
  }

  bool set_torch(nve::CameraTorchMode torch) {
    std::lock_guard<std::mutex> lock(mu_);
    if (is_disposed() || !backend_) return false;
    return backend_->dispatch_command(nve::CameraPreviewCommand::set_torch(torch));
  }

  bool set_zoom(float zoom) {
    std::lock_guard<std::mutex> lock(mu_);
    if (is_disposed() || !backend_) return false;
    return backend_->dispatch_command(nve::CameraPreviewCommand::set_zoom(zoom));
  }

  bool focus(float x, float y) {
    std::lock_guard<std::mutex> lock(mu_);
    if (is_disposed() || !backend_) return false;
    return backend_->dispatch_command(nve::CameraPreviewCommand::focus_point(x, y));
  }

  nve::nitro::Promise<nve::CapturedPhoto> take_photo(uint8_t quality = 90u, uint8_t flash_mode = 0u) {
    auto promise = nve::nitro::Promise<nve::CapturedPhoto>::create();
    if (quality > 100u || flash_mode > 2u) {
      promise.reject("INVALID_CAPTURE_OPTIONS");
      return promise;
    }
    std::lock_guard<std::mutex> lock(mu_);
    if (is_disposed() || !backend_) {
      promise.reject("SESSION_DISPOSED");
      return promise;
    }
    pending_photo_ = promise;
    const bool ok = backend_->dispatch_command(nve::CameraPreviewCommand::capture_photo(quality, flash_mode));
    if (!ok) {
      if (pending_photo_) {
        pending_photo_->reject("PHOTO_CAPTURE_FAILED");
        pending_photo_.reset();
      }
    } else if (pending_photo_ && pending_photo_->is_pending()) {
      // Backend completed synchronously without firing listener callback -> reject cleanly.
      pending_photo_->reject("NO_PHOTO_DELIVERED");
      pending_photo_.reset();
    }
    return promise;
  }

  nve::nitro::Promise<void> start_recording(std::string output_path) {
    auto promise = nve::nitro::Promise<void>::create();
    if (output_path.empty()) {
      promise.reject("EMPTY_OUTPUT_PATH");
      return promise;
    }
    std::lock_guard<std::mutex> lock(mu_);
    if (is_disposed() || !backend_) {
      promise.reject("SESSION_DISPOSED");
      return promise;
    }
    if (!backend_->dispatch_command(nve::CameraPreviewCommand::start_recording(std::move(output_path)))) {
      promise.reject("START_RECORDING_FAILED");
      return promise;
    }
    promise.resolve();
    return promise;
  }

  nve::nitro::Promise<nve::RecordedVideo> stop_recording() {
    auto promise = nve::nitro::Promise<nve::RecordedVideo>::create();
    std::lock_guard<std::mutex> lock(mu_);
    if (is_disposed() || !backend_) {
      promise.reject("SESSION_DISPOSED");
      return promise;
    }
    pending_video_ = promise;
    const bool ok = backend_->dispatch_command(nve::CameraPreviewCommand::stop_recording());
    if (!ok) {
      if (pending_video_) {
        pending_video_->reject("NOT_RECORDING");
        pending_video_.reset();
      }
    } else if (pending_video_ && pending_video_->is_pending()) {
      pending_video_->reject("NO_VIDEO_DELIVERED");
      pending_video_.reset();
    }
    return promise;
  }

  void set_frame_processor(FrameCallback cb) {
    std::lock_guard<std::mutex> lock(mu_);
    frame_cb_ = std::move(cb);
  }

  uint64_t frame_count() const noexcept {
    std::lock_guard<std::mutex> lock(mu_);
    return frame_count_;
  }

  std::optional<std::array<uint8_t, nve::kCameraFrameDescriptorSize>> last_frame_nvcf() const {
    std::lock_guard<std::mutex> lock(mu_);
    return last_frame_nvcf_;
  }

  nve::NveCameraBackend* raw_backend() noexcept { return backend_.get(); }

  nve::nitro::NitroVariant invoke_method(
      std::string_view method,
      std::span<const nve::nitro::NitroVariant> args) override {
    if (is_disposed()) return nve::nitro::NitroVariant::from_error("SESSION_DISPOSED");
    if (method == "applyProps") {
      if (args.empty()) return nve::nitro::NitroVariant::from_error("MISSING_NVNP_BYTES");
      auto ab = args[0].as_array_buffer();
      if (!ab) return nve::nitro::NitroVariant::from_error("EXPECTED_ARRAY_BUFFER");
      const auto err = apply_encoded_props(ab->span());
      return err == nve::NativeViewCodecError::Ok
                 ? nve::nitro::NitroVariant::from_bool(true)
                 : nve::nitro::NitroVariant::from_error("NVNP_APPLY_FAILED");
    }
    if (method == "dispatchCommand") {
      if (args.empty()) return nve::nitro::NitroVariant::from_error("MISSING_NVNC_BYTES");
      auto ab = args[0].as_array_buffer();
      if (!ab) return nve::nitro::NitroVariant::from_error("EXPECTED_ARRAY_BUFFER");
      const auto err = dispatch_encoded_command(ab->span());
      return err == nve::NativeViewCodecError::Ok
                 ? nve::nitro::NitroVariant::from_bool(true)
                 : nve::nitro::NitroVariant::from_error("NVNC_DISPATCH_FAILED");
    }
    if (method == "start") {
      return nve::nitro::NitroVariant::from_bool(start());
    }
    if (method == "stop") {
      stop();
      return nve::nitro::NitroVariant::from_bool(true);
    }
    if (method == "state") {
      return nve::nitro::NitroVariant::from_i64(static_cast<int64_t>(state()));
    }
    if (method == "lastFrame") {
      const auto frame = last_frame_nvcf();
      if (!frame) return nve::nitro::NitroVariant::null();
      return nve::nitro::NitroVariant::from_bytes(nve::nitro::ArrayBuffer::copy(*frame));
    }
    return nve::nitro::HybridObject::invoke_method(method, args);
  }

  nve::nitro::NitroVariant get_property(std::string_view prop) const override {
    if (prop == "state") return nve::nitro::NitroVariant::from_i64(static_cast<int64_t>(state()));
    if (prop == "frameCount") return nve::nitro::NitroVariant::from_i64(static_cast<int64_t>(frame_count()));
    return nve::nitro::HybridObject::get_property(prop);
  }

 private:
  void on_frame(
      const nve::CameraFrameDescriptor& desc,
      std::span<const uint8_t, nve::kCameraFrameDescriptorSize> nvcf) override {
    // Note: invoked synchronously from backend while caller may hold mu_ or from capture thread.
    // Copy descriptor bytes into zero-copy ArrayBuffer view for Nitro frame processor listeners.
    std::array<uint8_t, nve::kCameraFrameDescriptorSize> copy{};
    std::memcpy(copy.data(), nvcf.data(), nve::kCameraFrameDescriptorSize);
    FrameCallback cb;
    {
      std::lock_guard<std::mutex> lock(frame_mu_);
      ++frame_count_;
      last_frame_nvcf_ = copy;
      cb = frame_cb_;
    }
    if (cb) {
      auto ab = nve::nitro::ArrayBuffer::wrap(copy.data(), copy.size());
      cb(desc, std::span<const uint8_t, nve::kCameraFrameDescriptorSize>(copy), std::move(ab));
    }
  }

  void on_photo_captured(const nve::CapturedPhoto& photo) override {
    if (pending_photo_) {
      pending_photo_->resolve(photo);
      pending_photo_.reset();
    }
  }

  void on_recording_finished(const nve::RecordedVideo& video) override {
    if (pending_video_) {
      pending_video_->resolve(video);
      pending_video_.reset();
    }
  }

  void on_error(std::string_view code, std::string_view message) override {
    const std::string err = std::string(code) + ": " + std::string(message);
    if (pending_photo_) {
      pending_photo_->reject(err);
      pending_photo_.reset();
    }
    if (pending_video_) {
      pending_video_->reject(err);
      pending_video_.reset();
    }
  }

  mutable std::mutex mu_;
  mutable std::mutex frame_mu_;
  std::unique_ptr<nve::NveCameraBackend> backend_;
  std::optional<nve::nitro::Promise<nve::CapturedPhoto>> pending_photo_{};
  std::optional<nve::nitro::Promise<nve::RecordedVideo>> pending_video_{};
  FrameCallback frame_cb_{};
  uint64_t frame_count_{0u};
  std::optional<std::array<uint8_t, nve::kCameraFrameDescriptorSize>> last_frame_nvcf_{};
};

/// Registers `HybridCameraSession` under `"CameraSession"` in `registry` using `backend_factory`.
inline void register_vision_camera_hybrid(
    nve::nitro::HybridObjectRegistry& registry,
    std::function<std::unique_ptr<nve::NveCameraBackend>()> backend_factory) {
  registry.register_hybrid_object(
      std::string(kHybridCameraSessionName),
      [factory = std::move(backend_factory)]() -> std::shared_ptr<nve::nitro::HybridObject> {
        return std::make_shared<HybridCameraSession>(factory ? factory() : nullptr);
      });
}

} // namespace nve::camera
