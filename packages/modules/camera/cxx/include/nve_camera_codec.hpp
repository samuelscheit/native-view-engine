#pragma once

#include "nve_native_view.hpp"

#include <array>
#include <cmath>
#include <cstddef>
#include <cstdint>
#include <cstring>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

namespace nve {

/// Canonical native view kind string for `CameraPreview`.
inline constexpr std::string_view kNativeKindCameraPreview = "nve.camera.preview";

/// Zero-copy binary camera frame descriptor (`NVCF`) header constants.
inline constexpr std::array<uint8_t, 4> kCameraFrameMagic = {'N', 'V', 'C', 'F'};
inline constexpr uint8_t kCameraFrameVersion = 1u;
inline constexpr size_t kCameraFrameDescriptorSize = 40u;
inline constexpr uint16_t kMaxCameraFps = 240u;

// =============================================================================
// Wire Enums (Discriminants aligned 1:1 with packages/core/interop & nve-sdk)
// =============================================================================

enum class CameraFacing : uint8_t { Back = 1, Front = 2, External = 3 };
enum class CameraTorchMode : uint8_t { Off = 0, On = 1, Auto = 2 };
enum class CameraStabilizationMode : uint8_t { Off = 0, Standard = 1, Cinematic = 2 };
enum class CameraPixelFormat : uint8_t { Rgba8888 = 0, Bgra8888 = 1, Nv12 = 2, Yuv420_888 = 3 };

inline constexpr uint32_t camera_pixel_format_min_bpp(CameraPixelFormat fmt) noexcept {
  switch (fmt) {
    case CameraPixelFormat::Rgba8888:
    case CameraPixelFormat::Bgra8888:
      return 4u;
    case CameraPixelFormat::Nv12:
    case CameraPixelFormat::Yuv420_888:
      return 1u;
  }
  return 0u;
}

inline std::optional<CameraFacing> camera_facing_from_u32(uint32_t v) noexcept {
  switch (v) {
    case 1u: return CameraFacing::Back;
    case 2u: return CameraFacing::Front;
    case 3u: return CameraFacing::External;
    default: return std::nullopt;
  }
}

inline std::optional<CameraTorchMode> camera_torch_from_u32(uint32_t v) noexcept {
  switch (v) {
    case 0u: return CameraTorchMode::Off;
    case 1u: return CameraTorchMode::On;
    case 2u: return CameraTorchMode::Auto;
    default: return std::nullopt;
  }
}

inline std::optional<CameraStabilizationMode> camera_stabilization_from_u32(uint32_t v) noexcept {
  switch (v) {
    case 0u: return CameraStabilizationMode::Off;
    case 1u: return CameraStabilizationMode::Standard;
    case 2u: return CameraStabilizationMode::Cinematic;
    default: return std::nullopt;
  }
}

inline std::optional<CameraPixelFormat> camera_pixel_format_from_u32(uint32_t v) noexcept {
  switch (v) {
    case 0u: return CameraPixelFormat::Rgba8888;
    case 1u: return CameraPixelFormat::Bgra8888;
    case 2u: return CameraPixelFormat::Nv12;
    case 3u: return CameraPixelFormat::Yuv420_888;
    default: return std::nullopt;
  }
}

// =============================================================================
// CameraPreviewProps (`NVNP`), CameraPreviewCommand (`NVNC`), NVCF Descriptor
// =============================================================================

struct CameraPreviewProps {
  static constexpr uint16_t kFieldDeviceId = 1u;
  static constexpr uint16_t kFieldFacing = 2u;
  static constexpr uint16_t kFieldTorch = 3u;
  static constexpr uint16_t kFieldZoom = 4u;
  static constexpr uint16_t kFieldFps = 5u;
  static constexpr uint16_t kFieldHdr = 6u;
  static constexpr uint16_t kFieldStabilization = 7u;
  static constexpr uint16_t kFieldEnableFrameProcessor = 8u;
  static constexpr uint16_t kFieldAudioEnabled = 9u;
  static constexpr uint16_t kFieldActive = 10u;

  std::optional<std::string> device_id{};
  CameraFacing facing{CameraFacing::Back};
  CameraTorchMode torch{CameraTorchMode::Off};
  float zoom{1.0f};
  uint16_t fps{30u};
  bool hdr{false};
  CameraStabilizationMode stabilization{CameraStabilizationMode::Off};
  bool enable_frame_processor{false};
  bool audio_enabled{false};
  bool active{true};

  bool operator==(const CameraPreviewProps&) const = default;
};

enum class CameraCommandKind : uint16_t {
  SwitchFacing = 1u,
  SetTorch = 2u,
  SetZoom = 3u,
  FocusPoint = 4u,
  CapturePhoto = 5u,
  StartRecording = 6u,
  StopRecording = 7u,
};

struct CameraPreviewCommand {
  CameraCommandKind kind{CameraCommandKind::StopRecording};
  CameraFacing facing{CameraFacing::Back};
  CameraTorchMode torch{CameraTorchMode::Off};
  float zoom{1.0f};
  float focus_x{0.5f};
  float focus_y{0.5f};
  uint8_t quality{90u};
  uint8_t flash{0u};
  std::string output_path{};

  static CameraPreviewCommand switch_facing(CameraFacing f) {
    CameraPreviewCommand c{}; c.kind = CameraCommandKind::SwitchFacing; c.facing = f; return c;
  }
  static CameraPreviewCommand set_torch(CameraTorchMode m) {
    CameraPreviewCommand c{}; c.kind = CameraCommandKind::SetTorch; c.torch = m; return c;
  }
  static CameraPreviewCommand set_zoom(float z) {
    CameraPreviewCommand c{}; c.kind = CameraCommandKind::SetZoom; c.zoom = z; return c;
  }
  static CameraPreviewCommand focus_point(float x, float y) {
    CameraPreviewCommand c{}; c.kind = CameraCommandKind::FocusPoint; c.focus_x = x; c.focus_y = y; return c;
  }
  static CameraPreviewCommand capture_photo(uint8_t q, uint8_t fl) {
    CameraPreviewCommand c{}; c.kind = CameraCommandKind::CapturePhoto; c.quality = q; c.flash = fl; return c;
  }
  static CameraPreviewCommand start_recording(std::string path) {
    CameraPreviewCommand c{}; c.kind = CameraCommandKind::StartRecording; c.output_path = std::move(path); return c;
  }
  static CameraPreviewCommand stop_recording() {
    CameraPreviewCommand c{}; c.kind = CameraCommandKind::StopRecording; return c;
  }

  bool operator==(const CameraPreviewCommand&) const = default;
};

struct CameraFrameWireLayout {
  uint8_t magic[4];
  uint8_t version;
  uint8_t pixel_format;
  uint16_t rotation_deg;
  uint32_t width;
  uint32_t height;
  uint32_t stride_bytes;
  uint32_t reserved;
  uint64_t timestamp_ns;
  uint64_t native_handle_id;
};

static_assert(sizeof(CameraFrameWireLayout) == kCameraFrameDescriptorSize, "NVCF wire size must be 40 bytes");
static_assert(offsetof(CameraFrameWireLayout, magic) == 0u);
static_assert(offsetof(CameraFrameWireLayout, version) == 4u);
static_assert(offsetof(CameraFrameWireLayout, pixel_format) == 5u);
static_assert(offsetof(CameraFrameWireLayout, rotation_deg) == 6u);
static_assert(offsetof(CameraFrameWireLayout, width) == 8u);
static_assert(offsetof(CameraFrameWireLayout, height) == 12u);
static_assert(offsetof(CameraFrameWireLayout, stride_bytes) == 16u);
static_assert(offsetof(CameraFrameWireLayout, reserved) == 20u);
static_assert(offsetof(CameraFrameWireLayout, timestamp_ns) == 24u);
static_assert(offsetof(CameraFrameWireLayout, native_handle_id) == 32u);

struct CameraFrameDescriptor {
  uint32_t width{0u};
  uint32_t height{0u};
  uint32_t stride_bytes{0u};
  CameraPixelFormat pixel_format{CameraPixelFormat::Rgba8888};
  uint16_t rotation_deg{0u};
  uint64_t timestamp_ns{0u};
  uint64_t native_handle_id{0u};

  bool operator==(const CameraFrameDescriptor&) const = default;
};

namespace camera_detail {

inline void append_u16_le(std::vector<uint8_t>& out, uint16_t v) {
  out.push_back(static_cast<uint8_t>(v & 0xFFu));
  out.push_back(static_cast<uint8_t>((v >> 8u) & 0xFFu));
}

inline void append_u32_le(std::vector<uint8_t>& out, uint32_t v) {
  out.push_back(static_cast<uint8_t>(v & 0xFFu));
  out.push_back(static_cast<uint8_t>((v >> 8u) & 0xFFu));
  out.push_back(static_cast<uint8_t>((v >> 16u) & 0xFFu));
  out.push_back(static_cast<uint8_t>((v >> 24u) & 0xFFu));
}

inline void write_u16_le(uint8_t* dst, uint16_t v) noexcept {
  dst[0] = static_cast<uint8_t>(v & 0xFFu);
  dst[1] = static_cast<uint8_t>((v >> 8u) & 0xFFu);
}

inline void write_u32_le(uint8_t* dst, uint32_t v) noexcept {
  dst[0] = static_cast<uint8_t>(v & 0xFFu);
  dst[1] = static_cast<uint8_t>((v >> 8u) & 0xFFu);
  dst[2] = static_cast<uint8_t>((v >> 16u) & 0xFFu);
  dst[3] = static_cast<uint8_t>((v >> 24u) & 0xFFu);
}

inline void write_u64_le(uint8_t* dst, uint64_t v) noexcept {
  write_u32_le(dst, static_cast<uint32_t>(v & 0xFFFF'FFFFull));
  write_u32_le(dst + 4, static_cast<uint32_t>((v >> 32u) & 0xFFFF'FFFFull));
}

inline void append_f32_le(std::vector<uint8_t>& out, float v) {
  uint32_t bits = 0u;
  std::memcpy(&bits, &v, sizeof(bits));
  append_u32_le(out, bits);
}

inline void append_str(std::vector<uint8_t>& out, std::string_view s) {
  append_u32_le(out, static_cast<uint32_t>(s.size()));
  out.insert(out.end(), reinterpret_cast<const uint8_t*>(s.data()),
             reinterpret_cast<const uint8_t*>(s.data() + s.size()));
}

inline bool valid_zoom(float z) noexcept { return std::isfinite(z) && z > 0.0f; }
inline bool valid_unit_coord(float c) noexcept { return std::isfinite(c) && c >= 0.0f && c <= 1.0f; }
inline bool valid_rotation(uint16_t deg) noexcept { return deg == 0u || deg == 90u || deg == 180u || deg == 270u; }

inline NativeViewCodecError require_cmd_u32(const NvncDecoder& dec, size_t idx, uint32_t& out) noexcept {
  const auto v = dec.arg(idx);
  if (!v) return NativeViewCodecError::MissingArg;
  const auto n = v->as_u32();
  if (!n) return NativeViewCodecError::TypeMismatch;
  out = *n;
  return NativeViewCodecError::Ok;
}

} // namespace camera_detail

// =============================================================================
// NVNP / NVNC / NVCF Binary Codecs
// =============================================================================

inline NativeViewCodecError validate_camera_props(const CameraPreviewProps& p) noexcept {
  if (!camera_detail::valid_zoom(p.zoom)) return NativeViewCodecError::TypeMismatch;
  if (p.fps == 0u || p.fps > kMaxCameraFps) return NativeViewCodecError::InvalidEnum;
  if (p.device_id && !nve_utf8_valid(reinterpret_cast<const uint8_t*>(p.device_id->data()), p.device_id->size())) {
    return NativeViewCodecError::InvalidUtf8;
  }
  return NativeViewCodecError::Ok;
}

inline NativeViewCodecError encode_camera_props(const CameraPreviewProps& p, std::vector<uint8_t>& out) {
  if (auto err = validate_camera_props(p); err != NativeViewCodecError::Ok) return err;
  out.clear();
  out.reserve(96u);
  out.insert(out.end(), kNativePropsMagic.begin(), kNativePropsMagic.end());
  out.push_back(kNativePropsVersion);
  camera_detail::append_u16_le(out, 10u);

  auto field_hdr = [&](uint16_t id, NativeValueTag tag) {
    camera_detail::append_u16_le(out, id);
    out.push_back(static_cast<uint8_t>(tag));
  };

  if (p.device_id) {
    field_hdr(CameraPreviewProps::kFieldDeviceId, NativeValueTag::Str);
    camera_detail::append_str(out, *p.device_id);
  } else {
    field_hdr(CameraPreviewProps::kFieldDeviceId, NativeValueTag::Null);
  }
  field_hdr(CameraPreviewProps::kFieldFacing, NativeValueTag::U32);
  camera_detail::append_u32_le(out, static_cast<uint32_t>(p.facing));
  field_hdr(CameraPreviewProps::kFieldTorch, NativeValueTag::U32);
  camera_detail::append_u32_le(out, static_cast<uint32_t>(p.torch));
  field_hdr(CameraPreviewProps::kFieldZoom, NativeValueTag::F32);
  camera_detail::append_f32_le(out, p.zoom);
  field_hdr(CameraPreviewProps::kFieldFps, NativeValueTag::U32);
  camera_detail::append_u32_le(out, static_cast<uint32_t>(p.fps));
  field_hdr(CameraPreviewProps::kFieldHdr, NativeValueTag::Bool);
  out.push_back(p.hdr ? 1u : 0u);
  field_hdr(CameraPreviewProps::kFieldStabilization, NativeValueTag::U32);
  camera_detail::append_u32_le(out, static_cast<uint32_t>(p.stabilization));
  field_hdr(CameraPreviewProps::kFieldEnableFrameProcessor, NativeValueTag::Bool);
  out.push_back(p.enable_frame_processor ? 1u : 0u);
  field_hdr(CameraPreviewProps::kFieldAudioEnabled, NativeValueTag::Bool);
  out.push_back(p.audio_enabled ? 1u : 0u);
  field_hdr(CameraPreviewProps::kFieldActive, NativeValueTag::Bool);
  out.push_back(p.active ? 1u : 0u);
  return NativeViewCodecError::Ok;
}

inline NativeViewCodecError decode_camera_props(std::span<const uint8_t> bytes, CameraPreviewProps& out) {
  NvnpDecoder dec{};
  if (auto err = NvnpDecoder::parse(bytes, dec); err != NativeViewCodecError::Ok) return err;
  CameraPreviewProps p{};
  const auto dev = dec.find(CameraPreviewProps::kFieldDeviceId);
  if (!dev) return NativeViewCodecError::MissingField;
  if (dev->is_null()) p.device_id = std::nullopt;
  else if (auto s = dev->as_str()) p.device_id = std::string(*s);
  else return NativeViewCodecError::TypeMismatch;

  uint32_t facing_raw = 0u, torch_raw = 0u, fps_raw = 0u, stab_raw = 0u;
  if (auto e = dec.require_u32(CameraPreviewProps::kFieldFacing, facing_raw); e != NativeViewCodecError::Ok) return e;
  if (auto e = dec.require_u32(CameraPreviewProps::kFieldTorch, torch_raw); e != NativeViewCodecError::Ok) return e;
  if (auto e = dec.require_f32(CameraPreviewProps::kFieldZoom, p.zoom); e != NativeViewCodecError::Ok) return e;
  if (auto e = dec.require_u32(CameraPreviewProps::kFieldFps, fps_raw); e != NativeViewCodecError::Ok) return e;
  if (auto e = dec.require_bool(CameraPreviewProps::kFieldHdr, p.hdr); e != NativeViewCodecError::Ok) return e;
  if (auto e = dec.require_u32(CameraPreviewProps::kFieldStabilization, stab_raw); e != NativeViewCodecError::Ok) return e;
  if (auto e = dec.require_bool(CameraPreviewProps::kFieldEnableFrameProcessor, p.enable_frame_processor); e != NativeViewCodecError::Ok) return e;
  if (auto e = dec.require_bool(CameraPreviewProps::kFieldAudioEnabled, p.audio_enabled); e != NativeViewCodecError::Ok) return e;
  if (auto e = dec.require_bool(CameraPreviewProps::kFieldActive, p.active); e != NativeViewCodecError::Ok) return e;

  auto facing = camera_facing_from_u32(facing_raw);
  auto torch = camera_torch_from_u32(torch_raw);
  auto stab = camera_stabilization_from_u32(stab_raw);
  if (!facing || !torch || !stab) return NativeViewCodecError::InvalidEnum;
  if (!camera_detail::valid_zoom(p.zoom)) return NativeViewCodecError::TypeMismatch;
  if (fps_raw == 0u || fps_raw > kMaxCameraFps) return NativeViewCodecError::InvalidEnum;
  p.facing = *facing; p.torch = *torch; p.fps = static_cast<uint16_t>(fps_raw); p.stabilization = *stab;
  out = std::move(p);
  return NativeViewCodecError::Ok;
}

inline NativeViewCodecError encode_camera_command(const CameraPreviewCommand& cmd, std::vector<uint8_t>& out) {
  out.clear();
  out.reserve(32u);
  out.insert(out.end(), kNativeCmdMagic.begin(), kNativeCmdMagic.end());
  out.push_back(kNativeCmdVersion);
  camera_detail::append_u16_le(out, static_cast<uint16_t>(cmd.kind));
  switch (cmd.kind) {
    case CameraCommandKind::SwitchFacing:
      out.push_back(1u); out.push_back(static_cast<uint8_t>(NativeValueTag::U32));
      camera_detail::append_u32_le(out, static_cast<uint32_t>(cmd.facing));
      return NativeViewCodecError::Ok;
    case CameraCommandKind::SetTorch:
      out.push_back(1u); out.push_back(static_cast<uint8_t>(NativeValueTag::U32));
      camera_detail::append_u32_le(out, static_cast<uint32_t>(cmd.torch));
      return NativeViewCodecError::Ok;
    case CameraCommandKind::SetZoom:
      if (!camera_detail::valid_zoom(cmd.zoom)) return NativeViewCodecError::TypeMismatch;
      out.push_back(1u); out.push_back(static_cast<uint8_t>(NativeValueTag::F32));
      camera_detail::append_f32_le(out, cmd.zoom);
      return NativeViewCodecError::Ok;
    case CameraCommandKind::FocusPoint:
      if (!camera_detail::valid_unit_coord(cmd.focus_x) || !camera_detail::valid_unit_coord(cmd.focus_y)) {
        return NativeViewCodecError::TypeMismatch;
      }
      out.push_back(2u);
      out.push_back(static_cast<uint8_t>(NativeValueTag::F32)); camera_detail::append_f32_le(out, cmd.focus_x);
      out.push_back(static_cast<uint8_t>(NativeValueTag::F32)); camera_detail::append_f32_le(out, cmd.focus_y);
      return NativeViewCodecError::Ok;
    case CameraCommandKind::CapturePhoto:
      if (cmd.quality > 100u || cmd.flash > 2u) return NativeViewCodecError::InvalidEnum;
      out.push_back(2u);
      out.push_back(static_cast<uint8_t>(NativeValueTag::U32)); camera_detail::append_u32_le(out, cmd.quality);
      out.push_back(static_cast<uint8_t>(NativeValueTag::U32)); camera_detail::append_u32_le(out, cmd.flash);
      return NativeViewCodecError::Ok;
    case CameraCommandKind::StartRecording:
      if (!nve_utf8_valid(reinterpret_cast<const uint8_t*>(cmd.output_path.data()), cmd.output_path.size())) {
        return NativeViewCodecError::InvalidUtf8;
      }
      out.push_back(1u); out.push_back(static_cast<uint8_t>(NativeValueTag::Str));
      camera_detail::append_str(out, cmd.output_path);
      return NativeViewCodecError::Ok;
    case CameraCommandKind::StopRecording:
      out.push_back(0u);
      return NativeViewCodecError::Ok;
  }
  return NativeViewCodecError::UnknownCommandOpcode;
}

inline NativeViewCodecError decode_camera_command(std::span<const uint8_t> bytes, CameraPreviewCommand& out) {
  NvncDecoder dec{};
  if (auto err = NvncDecoder::parse(bytes, dec); err != NativeViewCodecError::Ok) return err;
  switch (dec.opcode()) {
    case static_cast<uint16_t>(CameraCommandKind::SwitchFacing): {
      if (dec.arg_count() != 1u) return NativeViewCodecError::UnexpectedArgCount;
      uint32_t raw = 0u;
      if (auto e = camera_detail::require_cmd_u32(dec, 0u, raw); e != NativeViewCodecError::Ok) return e;
      auto f = camera_facing_from_u32(raw);
      if (!f) return NativeViewCodecError::InvalidEnum;
      out = CameraPreviewCommand::switch_facing(*f);
      return NativeViewCodecError::Ok;
    }
    case static_cast<uint16_t>(CameraCommandKind::SetTorch): {
      if (dec.arg_count() != 1u) return NativeViewCodecError::UnexpectedArgCount;
      uint32_t raw = 0u;
      if (auto e = camera_detail::require_cmd_u32(dec, 0u, raw); e != NativeViewCodecError::Ok) return e;
      auto m = camera_torch_from_u32(raw);
      if (!m) return NativeViewCodecError::InvalidEnum;
      out = CameraPreviewCommand::set_torch(*m);
      return NativeViewCodecError::Ok;
    }
    case static_cast<uint16_t>(CameraCommandKind::SetZoom): {
      if (dec.arg_count() != 1u) return NativeViewCodecError::UnexpectedArgCount;
      float z = 0.0f;
      if (auto e = dec.require_f32(0u, z); e != NativeViewCodecError::Ok) return e;
      if (!camera_detail::valid_zoom(z)) return NativeViewCodecError::TypeMismatch;
      out = CameraPreviewCommand::set_zoom(z);
      return NativeViewCodecError::Ok;
    }
    case static_cast<uint16_t>(CameraCommandKind::FocusPoint): {
      if (dec.arg_count() != 2u) return NativeViewCodecError::UnexpectedArgCount;
      float x = 0.0f, y = 0.0f;
      if (auto e = dec.require_f32(0u, x); e != NativeViewCodecError::Ok) return e;
      if (auto e = dec.require_f32(1u, y); e != NativeViewCodecError::Ok) return e;
      if (!camera_detail::valid_unit_coord(x) || !camera_detail::valid_unit_coord(y)) return NativeViewCodecError::TypeMismatch;
      out = CameraPreviewCommand::focus_point(x, y);
      return NativeViewCodecError::Ok;
    }
    case static_cast<uint16_t>(CameraCommandKind::CapturePhoto): {
      if (dec.arg_count() != 2u) return NativeViewCodecError::UnexpectedArgCount;
      uint32_t q = 0u, fl = 0u;
      if (auto e = camera_detail::require_cmd_u32(dec, 0u, q); e != NativeViewCodecError::Ok) return e;
      if (auto e = camera_detail::require_cmd_u32(dec, 1u, fl); e != NativeViewCodecError::Ok) return e;
      if (q > 100u || fl > 2u) return NativeViewCodecError::InvalidEnum;
      out = CameraPreviewCommand::capture_photo(static_cast<uint8_t>(q), static_cast<uint8_t>(fl));
      return NativeViewCodecError::Ok;
    }
    case static_cast<uint16_t>(CameraCommandKind::StartRecording): {
      if (dec.arg_count() != 1u) return NativeViewCodecError::UnexpectedArgCount;
      std::string_view path{};
      if (auto e = dec.require_str(0u, path); e != NativeViewCodecError::Ok) return e;
      out = CameraPreviewCommand::start_recording(std::string(path));
      return NativeViewCodecError::Ok;
    }
    case static_cast<uint16_t>(CameraCommandKind::StopRecording):
      if (dec.arg_count() != 0u) return NativeViewCodecError::UnexpectedArgCount;
      out = CameraPreviewCommand::stop_recording();
      return NativeViewCodecError::Ok;
    default:
      return NativeViewCodecError::UnknownCommandOpcode;
  }
}

inline NativeViewCodecError validate_camera_frame(const CameraFrameDescriptor& d) noexcept {
  if (d.width == 0u || d.height == 0u) return NativeViewCodecError::TypeMismatch;
  if (!camera_detail::valid_rotation(d.rotation_deg)) return NativeViewCodecError::InvalidEnum;
  const uint64_t min_stride =
      static_cast<uint64_t>(d.width) * static_cast<uint64_t>(camera_pixel_format_min_bpp(d.pixel_format));
  if (min_stride > 0xFFFF'FFFFull || d.stride_bytes < min_stride) return NativeViewCodecError::TypeMismatch;
  return NativeViewCodecError::Ok;
}

inline NativeViewCodecError encode_camera_frame(
    const CameraFrameDescriptor& d,
    std::array<uint8_t, kCameraFrameDescriptorSize>& out) noexcept {
  if (auto err = validate_camera_frame(d); err != NativeViewCodecError::Ok) return err;
  std::memcpy(out.data(), kCameraFrameMagic.data(), 4u);
  out[4] = kCameraFrameVersion;
  out[5] = static_cast<uint8_t>(d.pixel_format);
  camera_detail::write_u16_le(out.data() + 6u, d.rotation_deg);
  camera_detail::write_u32_le(out.data() + 8u, d.width);
  camera_detail::write_u32_le(out.data() + 12u, d.height);
  camera_detail::write_u32_le(out.data() + 16u, d.stride_bytes);
  camera_detail::write_u32_le(out.data() + 20u, 0u);
  camera_detail::write_u64_le(out.data() + 24u, d.timestamp_ns);
  camera_detail::write_u64_le(out.data() + 32u, d.native_handle_id);
  return NativeViewCodecError::Ok;
}

inline NativeViewCodecError decode_camera_frame(
    std::span<const uint8_t> bytes,
    CameraFrameDescriptor& out) noexcept {
  if (bytes.size() < kCameraFrameDescriptorSize) return NativeViewCodecError::Truncated;
  if (bytes.size() > kCameraFrameDescriptorSize) return NativeViewCodecError::TrailingBytes;
  if (std::memcmp(bytes.data(), kCameraFrameMagic.data(), 4u) != 0) return NativeViewCodecError::InvalidMagic;
  if (bytes[4] != kCameraFrameVersion) return NativeViewCodecError::UnsupportedVersion;
  auto fmt = camera_pixel_format_from_u32(bytes[5]);
  if (!fmt) return NativeViewCodecError::InvalidEnum;
  if (detail::read_u32_le(bytes.data() + 20u) != 0u) return NativeViewCodecError::InvalidEnum;

  CameraFrameDescriptor d{};
  d.pixel_format = *fmt;
  d.rotation_deg = detail::read_u16_le(bytes.data() + 6u);
  d.width = detail::read_u32_le(bytes.data() + 8u);
  d.height = detail::read_u32_le(bytes.data() + 12u);
  d.stride_bytes = detail::read_u32_le(bytes.data() + 16u);
  d.timestamp_ns = detail::read_u64_le(bytes.data() + 24u);
  d.native_handle_id = detail::read_u64_le(bytes.data() + 32u);
  if (auto err = validate_camera_frame(d); err != NativeViewCodecError::Ok) return err;
  out = d;
  return NativeViewCodecError::Ok;
}

} // namespace nve
