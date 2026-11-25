#pragma once

#include "nve_rn_camera_jsi_convert.hpp"

#include <functional>
#include <memory>
#include <optional>
#include <span>
#include <string>
#include <utility>
#include <vector>

namespace nve::rn::camera {

using BackendFactory = std::function<std::unique_ptr<nve::NveCameraBackend>()>;

// =============================================================================
// Portable Standalone C++20 Bridge (Works with `NVE_WITH_RN_JSI=0` and `=1`)
// =============================================================================

/// Thin binary-codec facade over `nve::camera::HybridCameraSession` with zero duplicated camera logic.
class RnCameraBridge final {
 public:
  explicit RnCameraBridge(std::shared_ptr<nve::camera::HybridCameraSession> session)
      : session_(std::move(session)) {}

  std::shared_ptr<nve::camera::HybridCameraSession> session() const noexcept { return session_; }

  std::vector<nve::CameraDeviceInfo> list_devices() const {
    return session_ ? session_->get_available_devices() : std::vector<nve::CameraDeviceInfo>{};
  }

  bool apply_props(const nve::CameraPreviewProps& props) const {
    return session_ && session_->configure(props);
  }

  nve::NativeViewCodecError apply_encoded_props(std::span<const uint8_t> nvnp_bytes) const {
    return session_ ? session_->apply_encoded_props(nvnp_bytes) : nve::NativeViewCodecError::InvalidEnum;
  }

  bool dispatch_command(const nve::CameraPreviewCommand& cmd) const {
    if (!session_ || session_->is_disposed()) return false;
    std::vector<uint8_t> nvnc;
    if (nve::encode_camera_command(cmd, nvnc) != nve::NativeViewCodecError::Ok) return false;
    return session_->dispatch_encoded_command(nvnc) == nve::NativeViewCodecError::Ok;
  }

  nve::NativeViewCodecError dispatch_encoded_command(std::span<const uint8_t> nvnc_bytes) const {
    return session_ ? session_->dispatch_encoded_command(nvnc_bytes) : nve::NativeViewCodecError::InvalidEnum;
  }

  bool start() const { return session_ && session_->start(); }
  void stop() const {
    if (session_) session_->stop();
  }

  nve::nitro::Promise<nve::CapturedPhoto> take_photo(uint8_t quality = 90u, uint8_t flash = 0u) const {
    if (!session_) return nve::nitro::Promise<nve::CapturedPhoto>::rejected("SESSION_DISPOSED");
    return session_->take_photo(quality, flash);
  }

  nve::nitro::Promise<void> start_recording(std::string output_path) const {
    if (!session_) return nve::nitro::Promise<void>::rejected("SESSION_DISPOSED");
    return session_->start_recording(std::move(output_path));
  }

  nve::nitro::Promise<nve::RecordedVideo> stop_recording() const {
    if (!session_) return nve::nitro::Promise<nve::RecordedVideo>::rejected("SESSION_DISPOSED");
    return session_->stop_recording();
  }

  std::optional<std::array<uint8_t, nve::kCameraFrameDescriptorSize>> last_frame() const {
    return session_ ? session_->last_frame_nvcf() : std::nullopt;
  }

  nve::CameraSessionState state() const noexcept {
    return session_ ? session_->state() : nve::CameraSessionState::Released;
  }

  nve::CameraPreviewProps active_props() const {
    return session_ ? session_->active_props() : nve::CameraPreviewProps{};
  }

  uint64_t frame_count() const noexcept { return session_ ? session_->frame_count() : 0u; }
  bool is_disposed() const noexcept { return !session_ || session_->is_disposed(); }
  void dispose() const {
    if (session_) session_->dispose();
  }

 private:
  std::shared_ptr<nve::camera::HybridCameraSession> session_;
};

/// Registers `HybridCameraSession` in `registry` without requiring React Native JSI headers.
inline void install_nve_camera_standalone(
    nve::nitro::HybridObjectRegistry& registry,
    BackendFactory factory) {
  nve::camera::register_vision_camera_hybrid(registry, std::move(factory));
}

// =============================================================================
// React Native JSI `HostObject` / `NativeState` Bridge (`NVE_WITH_RN_JSI=1`)
// =============================================================================

#if defined(NVE_WITH_RN_JSI) && NVE_WITH_RN_JSI

/// Binary-buffer `facebook::jsi::HostObject` wrapping `nve::camera::HybridCameraSession`.
class RnCameraSessionHostObject final : public facebook::jsi::HostObject {
 public:
  explicit RnCameraSessionHostObject(std::shared_ptr<nve::camera::HybridCameraSession> session)
      : session_(std::move(session)) {}

  std::shared_ptr<nve::camera::HybridCameraSession> session() const noexcept { return session_; }

  std::vector<facebook::jsi::PropNameID> getPropertyNames(facebook::jsi::Runtime& rt) override {
    static constexpr const char* kNames[] = {
        "listDevices", "applyProps", "applyEncodedProps", "dispatchCommand",
        "dispatchEncodedCommand", "start", "stop", "switchFacing", "setTorch",
        "setZoom", "focus", "takePhoto", "startRecording", "stopRecording",
        "lastFrame", "asNativeState", "dispose", "state", "frameCount",
        "isDisposed", "name", "memorySize",
    };
    std::vector<facebook::jsi::PropNameID> out;
    out.reserve(std::size(kNames));
    for (const char* n : kNames) out.push_back(facebook::jsi::PropNameID::forAscii(rt, n));
    return out;
  }

  facebook::jsi::Value get(facebook::jsi::Runtime& rt, const facebook::jsi::PropNameID& prop) override {
    const std::string key = prop.utf8(rt);
    auto self = session_;

    if (key == "state") return facebook::jsi::Value(static_cast<int>(self ? self->state() : nve::CameraSessionState::Released));
    if (key == "frameCount") return facebook::jsi::Value(static_cast<double>(self ? self->frame_count() : 0u));
    if (key == "isDisposed") return facebook::jsi::Value(!self || self->is_disposed());
    if (key == "name") return facebook::jsi::String::createFromUtf8(rt, std::string(self ? self->name() : ""));
    if (key == "memorySize") return facebook::jsi::Value(static_cast<double>(self ? self->memory_size() : 0u));

    auto make_fn = [&](unsigned argc, facebook::jsi::HostFunctionType fn) {
      return facebook::jsi::Function::createFromHostFunction(rt, prop, argc, std::move(fn));
    };

    if (key == "listDevices") {
      return make_fn(0u, [self](facebook::jsi::Runtime& r, const facebook::jsi::Value&,
                                const facebook::jsi::Value*, size_t) -> facebook::jsi::Value {
        if (!self || self->is_disposed()) throw facebook::jsi::JSError(r, "SESSION_DISPOSED");
        return detail::devices_to_jsi(r, self->get_available_devices());
      });
    }

    if (key == "applyEncodedProps" || key == "applyProps") {
      const bool binary_only = (key == "applyEncodedProps");
      return make_fn(1u, [self, binary_only](facebook::jsi::Runtime& r, const facebook::jsi::Value&,
                                             const facebook::jsi::Value* args, size_t count) -> facebook::jsi::Value {
        if (!self || self->is_disposed()) throw facebook::jsi::JSError(r, "SESSION_DISPOSED");
        if (count < 1u || !args[0].isObject()) throw facebook::jsi::JSError(r, "EXPECTED_PROPS_OR_ARRAY_BUFFER");
        std::span<const uint8_t> bytes{};
        if (detail::extract_jsi_bytes(r, args[0], bytes)) {
          const auto err = self->apply_encoded_props(bytes);
          if (err != nve::NativeViewCodecError::Ok) {
            throw facebook::jsi::JSError(r, "NVNP_APPLY_FAILED: " + std::string(codec_error_name(err)));
          }
          return facebook::jsi::Value(true);
        }
        if (binary_only) throw facebook::jsi::JSError(r, "EXPECTED_ARRAY_BUFFER");
        return facebook::jsi::Value(self->configure(detail::jsi_object_to_props(r, args[0].getObject(r), self->active_props())));
      });
    }

    if (key == "dispatchEncodedCommand" || key == "dispatchCommand") {
      const bool binary_only = (key == "dispatchEncodedCommand");
      return make_fn(1u, [self, binary_only](facebook::jsi::Runtime& r, const facebook::jsi::Value&,
                                             const facebook::jsi::Value* args, size_t count) -> facebook::jsi::Value {
        if (!self || self->is_disposed()) throw facebook::jsi::JSError(r, "SESSION_DISPOSED");
        if (count < 1u || !args[0].isObject()) throw facebook::jsi::JSError(r, "EXPECTED_COMMAND_OR_ARRAY_BUFFER");
        std::span<const uint8_t> bytes{};
        if (detail::extract_jsi_bytes(r, args[0], bytes)) {
          const auto err = self->dispatch_encoded_command(bytes);
          if (err != nve::NativeViewCodecError::Ok) {
            throw facebook::jsi::JSError(r, "NVNC_DISPATCH_FAILED: " + std::string(codec_error_name(err)));
          }
          return facebook::jsi::Value(true);
        }
        if (binary_only) throw facebook::jsi::JSError(r, "EXPECTED_ARRAY_BUFFER");
        std::vector<uint8_t> encoded;
        const auto enc_err = nve::encode_camera_command(detail::jsi_object_to_command(r, args[0].getObject(r)), encoded);
        if (enc_err != nve::NativeViewCodecError::Ok) {
          throw facebook::jsi::JSError(r, "NVNC_ENCODE_FAILED: " + std::string(codec_error_name(enc_err)));
        }
        const auto err = self->dispatch_encoded_command(encoded);
        if (err != nve::NativeViewCodecError::Ok) {
          throw facebook::jsi::JSError(r, "NVNC_DISPATCH_FAILED: " + std::string(codec_error_name(err)));
        }
        return facebook::jsi::Value(true);
      });
    }

    if (key == "start") {
      return make_fn(0u, [self](facebook::jsi::Runtime& r, const facebook::jsi::Value&,
                                const facebook::jsi::Value*, size_t) -> facebook::jsi::Value {
        if (!self || self->is_disposed()) throw facebook::jsi::JSError(r, "SESSION_DISPOSED");
        return facebook::jsi::Value(self->start());
      });
    }
    if (key == "stop") {
      return make_fn(0u, [self](facebook::jsi::Runtime&, const facebook::jsi::Value&,
                                const facebook::jsi::Value*, size_t) -> facebook::jsi::Value {
        if (self) self->stop();
        return facebook::jsi::Value::undefined();
      });
    }
    if (key == "switchFacing") {
      return make_fn(1u, [self](facebook::jsi::Runtime& r, const facebook::jsi::Value&,
                                const facebook::jsi::Value* args, size_t count) -> facebook::jsi::Value {
        if (!self || self->is_disposed()) throw facebook::jsi::JSError(r, "SESSION_DISPOSED");
        if (count < 1u) throw facebook::jsi::JSError(r, "MISSING_ARG: facing");
        return facebook::jsi::Value(self->switch_facing(detail::jsi_to_facing(r, args[0])));
      });
    }
    if (key == "setTorch") {
      return make_fn(1u, [self](facebook::jsi::Runtime& r, const facebook::jsi::Value&,
                                const facebook::jsi::Value* args, size_t count) -> facebook::jsi::Value {
        if (!self || self->is_disposed()) throw facebook::jsi::JSError(r, "SESSION_DISPOSED");
        if (count < 1u) throw facebook::jsi::JSError(r, "MISSING_ARG: torch");
        return facebook::jsi::Value(self->set_torch(detail::jsi_to_torch(r, args[0])));
      });
    }
    if (key == "setZoom") {
      return make_fn(1u, [self](facebook::jsi::Runtime& r, const facebook::jsi::Value&,
                                const facebook::jsi::Value* args, size_t count) -> facebook::jsi::Value {
        if (!self || self->is_disposed()) throw facebook::jsi::JSError(r, "SESSION_DISPOSED");
        if (count < 1u) throw facebook::jsi::JSError(r, "MISSING_ARG: zoom");
        const float z = static_cast<float>(args[0].asNumber());
        if (!nve::camera_detail::valid_zoom(z)) throw facebook::jsi::JSError(r, "TYPE_MISMATCH: zoom");
        return facebook::jsi::Value(self->set_zoom(z));
      });
    }
    if (key == "focus") {
      return make_fn(2u, [self](facebook::jsi::Runtime& r, const facebook::jsi::Value&,
                                const facebook::jsi::Value* args, size_t count) -> facebook::jsi::Value {
        if (!self || self->is_disposed()) throw facebook::jsi::JSError(r, "SESSION_DISPOSED");
        if (count < 2u) throw facebook::jsi::JSError(r, "MISSING_ARG: x, y");
        const float x = static_cast<float>(args[0].asNumber()), y = static_cast<float>(args[1].asNumber());
        if (!nve::camera_detail::valid_unit_coord(x) || !nve::camera_detail::valid_unit_coord(y)) {
          throw facebook::jsi::JSError(r, "TYPE_MISMATCH: focus");
        }
        return facebook::jsi::Value(self->focus(x, y));
      });
    }
    if (key == "takePhoto") {
      return make_fn(1u, [self](facebook::jsi::Runtime& r, const facebook::jsi::Value&,
                                const facebook::jsi::Value* args, size_t count) -> facebook::jsi::Value {
        if (!self || self->is_disposed()) throw facebook::jsi::JSError(r, "SESSION_DISPOSED");
        uint8_t quality = 90u, flash = 0u;
        if (count >= 1u && args[0].isObject()) {
          auto opts = args[0].getObject(r);
          if (opts.hasProperty(r, "quality")) {
            const double q = opts.getProperty(r, "quality").asNumber();
            if (!std::isfinite(q) || q < 0.0 || q > 100.0) throw facebook::jsi::JSError(r, "INVALID_CAPTURE_OPTIONS");
            quality = static_cast<uint8_t>(q);
          }
          if (opts.hasProperty(r, "flash")) flash = detail::jsi_to_flash(r, opts.getProperty(r, "flash"));
        } else if (count >= 1u && args[0].isNumber()) {
          const double q = args[0].getNumber();
          if (!std::isfinite(q) || q < 0.0 || q > 100.0) throw facebook::jsi::JSError(r, "INVALID_CAPTURE_OPTIONS");
          quality = static_cast<uint8_t>(q);
          if (count >= 2u) flash = detail::jsi_to_flash(r, args[1]);
        }
        const auto res = self->take_photo(quality, flash).await_result();
        if (res.is_err()) throw facebook::jsi::JSError(r, res.error());
        return detail::photo_to_jsi(r, res.value());
      });
    }
    if (key == "startRecording") {
      return make_fn(1u, [self](facebook::jsi::Runtime& r, const facebook::jsi::Value&,
                                const facebook::jsi::Value* args, size_t count) -> facebook::jsi::Value {
        if (!self || self->is_disposed()) throw facebook::jsi::JSError(r, "SESSION_DISPOSED");
        if (count < 1u) throw facebook::jsi::JSError(r, "EMPTY_OUTPUT_PATH");
        std::string output_path;
        if (args[0].isString()) output_path = args[0].getString(r).utf8(r);
        else if (args[0].isObject() && args[0].getObject(r).hasProperty(r, "outputPath")) {
          output_path = args[0].getObject(r).getProperty(r, "outputPath").asString(r).utf8(r);
        }
        const auto res = self->start_recording(std::move(output_path)).await_result();
        if (res.is_err()) throw facebook::jsi::JSError(r, res.error());
        return facebook::jsi::Value(true);
      });
    }
    if (key == "stopRecording") {
      return make_fn(0u, [self](facebook::jsi::Runtime& r, const facebook::jsi::Value&,
                                const facebook::jsi::Value*, size_t) -> facebook::jsi::Value {
        if (!self || self->is_disposed()) throw facebook::jsi::JSError(r, "SESSION_DISPOSED");
        const auto res = self->stop_recording().await_result();
        if (res.is_err()) throw facebook::jsi::JSError(r, res.error());
        return detail::video_to_jsi(r, res.value());
      });
    }
    if (key == "lastFrame") {
      return make_fn(0u, [self](facebook::jsi::Runtime& r, const facebook::jsi::Value&,
                                const facebook::jsi::Value*, size_t) -> facebook::jsi::Value {
        if (!self) return facebook::jsi::Value::null();
        const auto frame = self->last_frame_nvcf();
        if (!frame) return facebook::jsi::Value::null();
        return facebook::jsi::ArrayBuffer(
            r, std::make_shared<detail::OwnedMutableBuffer>(std::vector<uint8_t>(frame->begin(), frame->end())));
      });
    }
    if (key == "asNativeState") {
      return make_fn(0u, [self](facebook::jsi::Runtime& r, const facebook::jsi::Value&,
                                const facebook::jsi::Value*, size_t) -> facebook::jsi::Value {
        if (!self) return facebook::jsi::Value::null();
        facebook::jsi::Object holder(r);
        holder.setNativeState(r, self->as_jsi_native_state());
        return holder;
      });
    }
    if (key == "dispose") {
      return make_fn(0u, [self](facebook::jsi::Runtime&, const facebook::jsi::Value&,
                                const facebook::jsi::Value*, size_t) -> facebook::jsi::Value {
        if (self) self->dispose();
        return facebook::jsi::Value::undefined();
      });
    }
    return facebook::jsi::Value::undefined();
  }

 private:
  std::shared_ptr<nve::camera::HybridCameraSession> session_;
};

/// Top-level `global.__nveCamera` JSI `HostObject` factory module.
class RnCameraModuleHostObject final : public facebook::jsi::HostObject {
 public:
  explicit RnCameraModuleHostObject(BackendFactory factory) : factory_(std::move(factory)) {}

  std::vector<facebook::jsi::PropNameID> getPropertyNames(facebook::jsi::Runtime& rt) override {
    std::vector<facebook::jsi::PropNameID> names;
    names.reserve(4);
    names.push_back(facebook::jsi::PropNameID::forAscii(rt, "createSession"));
    names.push_back(facebook::jsi::PropNameID::forAscii(rt, "listDevices"));
    names.push_back(facebook::jsi::PropNameID::forAscii(rt, "isAvailable"));
    names.push_back(facebook::jsi::PropNameID::forAscii(rt, "hybridObjectName"));
    return names;
  }

  facebook::jsi::Value get(facebook::jsi::Runtime& rt, const facebook::jsi::PropNameID& prop) override {
    const std::string key = prop.utf8(rt);
    if (key == "isAvailable") return facebook::jsi::Value(true);
    if (key == "hybridObjectName") {
      return facebook::jsi::String::createFromUtf8(rt, std::string(nve::camera::kHybridCameraSessionName));
    }
    if (key == "createSession") {
      auto factory = factory_;
      return facebook::jsi::Function::createFromHostFunction(
          rt, prop, 0u,
          [factory](facebook::jsi::Runtime& r, const facebook::jsi::Value&,
                    const facebook::jsi::Value*, size_t) -> facebook::jsi::Value {
            auto session = factory
                               ? std::make_shared<nve::camera::HybridCameraSession>(factory())
                               : nve::nitro::HybridObjectRegistry::global().create<nve::camera::HybridCameraSession>(
                                     nve::camera::kHybridCameraSessionName);
            if (!session) throw facebook::jsi::JSError(r, "CAMERA_BACKEND_UNAVAILABLE");
            return facebook::jsi::Object::createFromHostObject(
                r, std::make_shared<RnCameraSessionHostObject>(std::move(session)));
          });
    }
    if (key == "listDevices") {
      auto factory = factory_;
      return facebook::jsi::Function::createFromHostFunction(
          rt, prop, 0u,
          [factory](facebook::jsi::Runtime& r, const facebook::jsi::Value&,
                    const facebook::jsi::Value*, size_t) -> facebook::jsi::Value {
            if (factory) {
              auto backend = factory();
              auto devices = backend ? backend->list_devices() : std::vector<nve::CameraDeviceInfo>{};
              if (backend) backend->release();
              return detail::devices_to_jsi(r, devices);
            }
            auto session = nve::nitro::HybridObjectRegistry::global().create<nve::camera::HybridCameraSession>(
                nve::camera::kHybridCameraSessionName);
            if (!session) return detail::devices_to_jsi(r, {});
            auto devices = session->get_available_devices();
            session->dispose();
            return detail::devices_to_jsi(r, devices);
          });
    }
    return facebook::jsi::Value::undefined();
  }

 private:
  BackendFactory factory_;
};

/// Installs `global.__nveCamera` and `global.__nveCameraInstall` on `rt` and registers
/// `HybridCameraSession` in `nve::nitro::HybridObjectRegistry::global()`.
inline void install_nve_camera_jsi(facebook::jsi::Runtime& rt, BackendFactory factory) {
  if (factory) {
    nve::camera::register_vision_camera_hybrid(nve::nitro::HybridObjectRegistry::global(), factory);
  }
  auto mod_host = std::make_shared<RnCameraModuleHostObject>(std::move(factory));
  rt.global().setProperty(rt, "__nveCamera", facebook::jsi::Object::createFromHostObject(rt, mod_host));
  auto install_fn = facebook::jsi::Function::createFromHostFunction(
      rt, facebook::jsi::PropNameID::forAscii(rt, "__nveCameraInstall"), 0u,
      [mod_host](facebook::jsi::Runtime& r, const facebook::jsi::Value&,
                 const facebook::jsi::Value*, size_t) -> facebook::jsi::Value {
        auto obj = facebook::jsi::Object::createFromHostObject(r, mod_host);
        r.global().setProperty(r, "__nveCamera", obj);
        return obj;
      });
  rt.global().setProperty(rt, "__nveCameraInstall", std::move(install_fn));
}

#endif // defined(NVE_WITH_RN_JSI) && NVE_WITH_RN_JSI

} // namespace nve::rn::camera
