#pragma once

#include "nve_vision_camera_hybrid.hpp"

#include <cmath>
#include <cstdint>
#include <memory>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

#if defined(NVE_WITH_RN_JSI) && NVE_WITH_RN_JSI
#  include <jsi/jsi.h>
#endif

namespace nve::rn::camera {

// =============================================================================
// Shared Binary Enum & Error Mapping Helpers
// =============================================================================

inline constexpr std::string_view facing_to_string(nve::CameraFacing f) noexcept {
  switch (f) {
    case nve::CameraFacing::Front: return "front";
    case nve::CameraFacing::External: return "external";
    case nve::CameraFacing::Back: return "back";
  }
  return "back";
}

inline std::optional<nve::CameraFacing> parse_facing_string(std::string_view s) noexcept {
  if (s == "back") return nve::CameraFacing::Back;
  if (s == "front") return nve::CameraFacing::Front;
  if (s == "external") return nve::CameraFacing::External;
  return std::nullopt;
}

inline constexpr std::string_view torch_to_string(nve::CameraTorchMode m) noexcept {
  switch (m) {
    case nve::CameraTorchMode::On: return "on";
    case nve::CameraTorchMode::Auto: return "auto";
    case nve::CameraTorchMode::Off: return "off";
  }
  return "off";
}

inline std::optional<nve::CameraTorchMode> parse_torch_string(std::string_view s) noexcept {
  if (s == "off") return nve::CameraTorchMode::Off;
  if (s == "on") return nve::CameraTorchMode::On;
  if (s == "auto") return nve::CameraTorchMode::Auto;
  return std::nullopt;
}

inline constexpr std::string_view stabilization_to_string(nve::CameraStabilizationMode m) noexcept {
  switch (m) {
    case nve::CameraStabilizationMode::Standard: return "standard";
    case nve::CameraStabilizationMode::Cinematic: return "cinematic";
    case nve::CameraStabilizationMode::Off: return "off";
  }
  return "off";
}

inline std::optional<nve::CameraStabilizationMode> parse_stabilization_string(std::string_view s) noexcept {
  if (s == "off") return nve::CameraStabilizationMode::Off;
  if (s == "standard") return nve::CameraStabilizationMode::Standard;
  if (s == "cinematic") return nve::CameraStabilizationMode::Cinematic;
  return std::nullopt;
}

inline constexpr std::string_view codec_error_name(nve::NativeViewCodecError err) noexcept {
  switch (err) {
    case nve::NativeViewCodecError::Ok: return "OK";
    case nve::NativeViewCodecError::Truncated: return "TRUNCATED";
    case nve::NativeViewCodecError::InvalidMagic: return "INVALID_MAGIC";
    case nve::NativeViewCodecError::UnsupportedVersion: return "UNSUPPORTED_VERSION";
    case nve::NativeViewCodecError::InvalidTag: return "INVALID_TAG";
    case nve::NativeViewCodecError::InvalidBool: return "INVALID_BOOL";
    case nve::NativeViewCodecError::InvalidEnum: return "INVALID_ENUM";
    case nve::NativeViewCodecError::InvalidUtf8: return "INVALID_UTF8";
    case nve::NativeViewCodecError::MissingField: return "MISSING_FIELD";
    case nve::NativeViewCodecError::MissingArg: return "MISSING_ARG";
    case nve::NativeViewCodecError::UnexpectedArgCount: return "UNEXPECTED_ARG_COUNT";
    case nve::NativeViewCodecError::TypeMismatch: return "TYPE_MISMATCH";
    case nve::NativeViewCodecError::TrailingBytes: return "TRAILING_BYTES";
    case nve::NativeViewCodecError::UnknownCommandOpcode: return "UNKNOWN_COMMAND_OPCODE";
    case nve::NativeViewCodecError::TooManyFields: return "TOO_MANY_FIELDS";
  }
  return "UNKNOWN_CODEC_ERROR";
}

#if defined(NVE_WITH_RN_JSI) && NVE_WITH_RN_JSI

namespace detail {

class OwnedMutableBuffer final : public facebook::jsi::MutableBuffer {
 public:
  explicit OwnedMutableBuffer(std::vector<uint8_t> bytes) : bytes_(std::move(bytes)) {}
  size_t size() const override { return bytes_.size(); }
  uint8_t* data() override { return bytes_.empty() ? nullptr : bytes_.data(); }

 private:
  std::vector<uint8_t> bytes_;
};

inline bool extract_jsi_bytes(
    facebook::jsi::Runtime& rt,
    const facebook::jsi::Value& value,
    std::span<const uint8_t>& out) {
  if (!value.isObject()) return false;
  facebook::jsi::Object obj = value.getObject(rt);
  if (obj.isArrayBuffer(rt)) {
    facebook::jsi::ArrayBuffer ab = obj.getArrayBuffer(rt);
    out = std::span<const uint8_t>(ab.data(rt), ab.size(rt));
    return true;
  }
  if (obj.hasProperty(rt, "buffer") && obj.hasProperty(rt, "byteLength")) {
    facebook::jsi::Value buf_val = obj.getProperty(rt, "buffer");
    if (buf_val.isObject()) {
      facebook::jsi::Object buf_obj = buf_val.getObject(rt);
      if (buf_obj.isArrayBuffer(rt)) {
        facebook::jsi::ArrayBuffer ab = buf_obj.getArrayBuffer(rt);
        const size_t total = ab.size(rt);
        const size_t offset = obj.hasProperty(rt, "byteOffset")
                                  ? static_cast<size_t>(obj.getProperty(rt, "byteOffset").asNumber())
                                  : 0u;
        const size_t len = static_cast<size_t>(obj.getProperty(rt, "byteLength").asNumber());
        if (offset <= total && len <= total - offset) {
          out = std::span<const uint8_t>(ab.data(rt) + offset, len);
          return true;
        }
      }
    }
  }
  return false;
}

inline nve::CameraFacing jsi_to_facing(facebook::jsi::Runtime& rt, const facebook::jsi::Value& v) {
  if (v.isString()) {
    if (auto parsed = parse_facing_string(v.getString(rt).utf8(rt))) return *parsed;
  } else if (v.isNumber()) {
    const double n = v.getNumber();
    if (std::isfinite(n) && n >= 0.0) {
      if (auto parsed = nve::camera_facing_from_u32(static_cast<uint32_t>(n))) return *parsed;
    }
  }
  throw facebook::jsi::JSError(rt, "INVALID_ENUM: CameraFacing");
}

inline nve::CameraTorchMode jsi_to_torch(facebook::jsi::Runtime& rt, const facebook::jsi::Value& v) {
  if (v.isString()) {
    if (auto parsed = parse_torch_string(v.getString(rt).utf8(rt))) return *parsed;
  } else if (v.isNumber()) {
    const double n = v.getNumber();
    if (std::isfinite(n) && n >= 0.0) {
      if (auto parsed = nve::camera_torch_from_u32(static_cast<uint32_t>(n))) return *parsed;
    }
  }
  throw facebook::jsi::JSError(rt, "INVALID_ENUM: CameraTorchMode");
}

inline nve::CameraStabilizationMode jsi_to_stabilization(
    facebook::jsi::Runtime& rt,
    const facebook::jsi::Value& v) {
  if (v.isString()) {
    if (auto parsed = parse_stabilization_string(v.getString(rt).utf8(rt))) return *parsed;
  } else if (v.isNumber()) {
    const double n = v.getNumber();
    if (std::isfinite(n) && n >= 0.0) {
      if (auto parsed = nve::camera_stabilization_from_u32(static_cast<uint32_t>(n))) return *parsed;
    }
  }
  throw facebook::jsi::JSError(rt, "INVALID_ENUM: CameraStabilizationMode");
}

inline uint8_t jsi_to_flash(facebook::jsi::Runtime& rt, const facebook::jsi::Value& v) {
  if (v.isUndefined() || v.isNull()) return 0u;
  if (v.isString()) {
    const std::string s = v.getString(rt).utf8(rt);
    if (s == "off") return 0u;
    if (s == "on") return 1u;
    if (s == "auto") return 2u;
    throw facebook::jsi::JSError(rt, "INVALID_CAPTURE_OPTIONS: flash");
  }
  if (v.isNumber()) {
    const double n = v.getNumber();
    if (std::isfinite(n) && n >= 0.0 && n <= 2.0) return static_cast<uint8_t>(n);
  }
  throw facebook::jsi::JSError(rt, "INVALID_CAPTURE_OPTIONS: flash");
}

inline nve::CameraPreviewProps jsi_object_to_props(
    facebook::jsi::Runtime& rt,
    const facebook::jsi::Object& obj,
    const nve::CameraPreviewProps& base) {
  nve::CameraPreviewProps p = base;
  if (obj.hasProperty(rt, "deviceId")) {
    auto v = obj.getProperty(rt, "deviceId");
    if (v.isNull() || v.isUndefined()) p.device_id = std::nullopt;
    else if (v.isString()) p.device_id = v.getString(rt).utf8(rt);
    else throw facebook::jsi::JSError(rt, "TYPE_MISMATCH: deviceId");
  }
  if (obj.hasProperty(rt, "facing")) {
    auto v = obj.getProperty(rt, "facing");
    if (!v.isUndefined()) p.facing = jsi_to_facing(rt, v);
  }
  if (obj.hasProperty(rt, "torch")) {
    auto v = obj.getProperty(rt, "torch");
    if (!v.isUndefined()) p.torch = jsi_to_torch(rt, v);
  }
  if (obj.hasProperty(rt, "zoom")) {
    auto v = obj.getProperty(rt, "zoom");
    if (!v.isUndefined()) {
      const float z = static_cast<float>(v.asNumber());
      if (!nve::camera_detail::valid_zoom(z)) throw facebook::jsi::JSError(rt, "TYPE_MISMATCH: zoom");
      p.zoom = z;
    }
  }
  if (obj.hasProperty(rt, "fps")) {
    auto v = obj.getProperty(rt, "fps");
    if (!v.isUndefined()) {
      const double fps_d = v.asNumber();
      if (!std::isfinite(fps_d) || fps_d < 1.0 || fps_d > static_cast<double>(nve::kMaxCameraFps)) {
        throw facebook::jsi::JSError(rt, "INVALID_ENUM: fps");
      }
      p.fps = static_cast<uint16_t>(fps_d);
    }
  }
  if (obj.hasProperty(rt, "hdr")) {
    auto v = obj.getProperty(rt, "hdr");
    if (!v.isUndefined()) p.hdr = v.asBool();
  }
  if (obj.hasProperty(rt, "stabilization")) {
    auto v = obj.getProperty(rt, "stabilization");
    if (!v.isUndefined()) p.stabilization = jsi_to_stabilization(rt, v);
  }
  if (obj.hasProperty(rt, "enableFrameProcessor")) {
    auto v = obj.getProperty(rt, "enableFrameProcessor");
    if (!v.isUndefined()) p.enable_frame_processor = v.asBool();
  }
  if (obj.hasProperty(rt, "audioEnabled")) {
    auto v = obj.getProperty(rt, "audioEnabled");
    if (!v.isUndefined()) p.audio_enabled = v.asBool();
  }
  if (obj.hasProperty(rt, "active")) {
    auto v = obj.getProperty(rt, "active");
    if (!v.isUndefined()) p.active = v.asBool();
  }
  return p;
}

inline nve::CameraPreviewCommand jsi_object_to_command(
    facebook::jsi::Runtime& rt,
    const facebook::jsi::Object& obj) {
  if (!obj.hasProperty(rt, "kind")) throw facebook::jsi::JSError(rt, "MISSING_FIELD: command.kind");
  const std::string kind = obj.getProperty(rt, "kind").asString(rt).utf8(rt);
  if (kind == "switch_facing") {
    return nve::CameraPreviewCommand::switch_facing(jsi_to_facing(rt, obj.getProperty(rt, "facing")));
  }
  if (kind == "set_torch") {
    return nve::CameraPreviewCommand::set_torch(jsi_to_torch(rt, obj.getProperty(rt, "torch")));
  }
  if (kind == "set_zoom") {
    const float z = static_cast<float>(obj.getProperty(rt, "zoom").asNumber());
    if (!nve::camera_detail::valid_zoom(z)) throw facebook::jsi::JSError(rt, "TYPE_MISMATCH: zoom");
    return nve::CameraPreviewCommand::set_zoom(z);
  }
  if (kind == "focus_point") {
    const float x = static_cast<float>(obj.getProperty(rt, "x").asNumber());
    const float y = static_cast<float>(obj.getProperty(rt, "y").asNumber());
    if (!nve::camera_detail::valid_unit_coord(x) || !nve::camera_detail::valid_unit_coord(y)) {
      throw facebook::jsi::JSError(rt, "TYPE_MISMATCH: focus_point");
    }
    return nve::CameraPreviewCommand::focus_point(x, y);
  }
  if (kind == "capture_photo") {
    const double q = obj.hasProperty(rt, "quality") ? obj.getProperty(rt, "quality").asNumber() : 90.0;
    if (!std::isfinite(q) || q < 0.0 || q > 100.0) throw facebook::jsi::JSError(rt, "INVALID_ENUM: quality");
    const uint8_t fl = obj.hasProperty(rt, "flash") ? jsi_to_flash(rt, obj.getProperty(rt, "flash")) : 0u;
    return nve::CameraPreviewCommand::capture_photo(static_cast<uint8_t>(q), fl);
  }
  if (kind == "start_recording") {
    return nve::CameraPreviewCommand::start_recording(obj.getProperty(rt, "outputPath").asString(rt).utf8(rt));
  }
  if (kind == "stop_recording") return nve::CameraPreviewCommand::stop_recording();
  throw facebook::jsi::JSError(rt, "UNKNOWN_COMMAND_OPCODE: " + kind);
}

inline facebook::jsi::Array devices_to_jsi(
    facebook::jsi::Runtime& rt,
    const std::vector<nve::CameraDeviceInfo>& devices) {
  facebook::jsi::Array arr(rt, devices.size());
  for (size_t i = 0u; i < devices.size(); ++i) {
    const auto& d = devices[i];
    facebook::jsi::Object obj(rt);
    obj.setProperty(rt, "id", facebook::jsi::String::createFromUtf8(rt, d.id));
    obj.setProperty(rt, "name", facebook::jsi::String::createFromUtf8(rt, d.name));
    obj.setProperty(rt, "facing", facebook::jsi::String::createFromUtf8(rt, std::string(facing_to_string(d.facing))));
    obj.setProperty(rt, "hasTorch", facebook::jsi::Value(d.has_torch));
    obj.setProperty(rt, "minZoom", facebook::jsi::Value(static_cast<double>(d.min_zoom)));
    obj.setProperty(rt, "maxZoom", facebook::jsi::Value(static_cast<double>(d.max_zoom)));
    obj.setProperty(rt, "maxFps", facebook::jsi::Value(static_cast<double>(d.max_fps)));
    obj.setProperty(rt, "supportsHdr", facebook::jsi::Value(d.supports_hdr));
    facebook::jsi::Array stab(rt, d.supported_stabilization.size());
    for (size_t k = 0u; k < d.supported_stabilization.size(); ++k) {
      stab.setValueAtIndex(
          rt, k,
          facebook::jsi::String::createFromUtf8(rt, std::string(stabilization_to_string(d.supported_stabilization[k]))));
    }
    obj.setProperty(rt, "supportedStabilization", std::move(stab));
    arr.setValueAtIndex(rt, i, std::move(obj));
  }
  return arr;
}

inline facebook::jsi::Object photo_to_jsi(facebook::jsi::Runtime& rt, const nve::CapturedPhoto& p) {
  facebook::jsi::Object obj(rt);
  obj.setProperty(rt, "filePath", facebook::jsi::String::createFromUtf8(rt, p.file_path));
  obj.setProperty(rt, "width", facebook::jsi::Value(static_cast<double>(p.width)));
  obj.setProperty(rt, "height", facebook::jsi::Value(static_cast<double>(p.height)));
  obj.setProperty(rt, "quality", facebook::jsi::Value(static_cast<double>(p.quality)));
  obj.setProperty(rt, "flashMode", facebook::jsi::Value(static_cast<double>(p.flash_mode)));
  obj.setProperty(rt, "timestampNs", facebook::jsi::Value(static_cast<double>(p.timestamp_ns)));
  return obj;
}

inline facebook::jsi::Object video_to_jsi(facebook::jsi::Runtime& rt, const nve::RecordedVideo& v) {
  facebook::jsi::Object obj(rt);
  obj.setProperty(rt, "outputPath", facebook::jsi::String::createFromUtf8(rt, v.output_path));
  obj.setProperty(rt, "durationMs", facebook::jsi::Value(static_cast<double>(v.duration_ms)));
  obj.setProperty(rt, "width", facebook::jsi::Value(static_cast<double>(v.width)));
  obj.setProperty(rt, "height", facebook::jsi::Value(static_cast<double>(v.height)));
  obj.setProperty(rt, "hasAudio", facebook::jsi::Value(v.has_audio));
  return obj;
}

} // namespace detail
#endif // defined(NVE_WITH_RN_JSI) && NVE_WITH_RN_JSI

} // namespace nve::rn::camera
