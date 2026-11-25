#import "NveAppleCamera.h"

#import <objc/runtime.h>

#include <algorithm>
#include <bit>
#include <cmath>
#include <cstring>
#include <mutex>

namespace nve::apple {

namespace {

static_assert(std::endian::native == std::endian::little, "NVCF requires little-endian layout");

struct CameraFrameWireLayout {
  uint8_t magic[4]; uint8_t version; uint8_t pixel_format; uint16_t rotation_deg;
  uint32_t width; uint32_t height; uint32_t stride_bytes; uint32_t reserved;
  uint64_t timestamp_ns; uint64_t native_handle_id;
};

static_assert(sizeof(CameraFrameWireLayout) == kCameraFrameDescriptorSize &&
              offsetof(CameraFrameWireLayout, magic) == 0u && offsetof(CameraFrameWireLayout, version) == 4u &&
              offsetof(CameraFrameWireLayout, pixel_format) == 5u && offsetof(CameraFrameWireLayout, rotation_deg) == 6u &&
              offsetof(CameraFrameWireLayout, width) == 8u && offsetof(CameraFrameWireLayout, height) == 12u &&
              offsetof(CameraFrameWireLayout, stride_bytes) == 16u && offsetof(CameraFrameWireLayout, reserved) == 20u &&
              offsetof(CameraFrameWireLayout, timestamp_ns) == 24u && offsetof(CameraFrameWireLayout, native_handle_id) == 32u);

inline bool valid_zoom(float z) noexcept { return std::isfinite(z) && z > 0.0f; }
inline bool valid_unit(float v) noexcept { return std::isfinite(v) && v >= 0.0f && v <= 1.0f; }

nve::NativeViewCodecError validate_frame_desc(const CameraFrameDescriptor &d) noexcept {
  if (d.width == 0u || d.height == 0u) return nve::NativeViewCodecError::TypeMismatch;
  if (d.rotation_deg != 0u && d.rotation_deg != 90u && d.rotation_deg != 180u && d.rotation_deg != 270u) {
    return nve::NativeViewCodecError::InvalidEnum;
  }
  const uint64_t bpp = (d.pixel_format == CameraPixelFormat::Rgba8888 || d.pixel_format == CameraPixelFormat::Bgra8888) ? 4ull : 1ull;
  const uint64_t min_stride = static_cast<uint64_t>(d.width) * bpp;
  return (min_stride <= UINT32_MAX && d.stride_bytes >= min_stride) ? nve::NativeViewCodecError::Ok : nve::NativeViewCodecError::TypeMismatch;
}

std::mutex &vision_bridge_mutex() { static std::mutex m; return m; }
id<NveVisionCameraAppleBridge> __strong &vision_bridge_slot() {
  static id<NveVisionCameraAppleBridge> __strong slot = nil;
  return slot;
}
char kVisionBridgeAssocKey = 0;

} // namespace

#define NVE_CAM_TRY(expr) do { if (const auto _e = (expr); _e != nve::NativeViewCodecError::Ok) return _e; } while (0)

nve::NativeViewCodecError decode_camera_preview_props(const nve::NvnpDecoder &dec, CameraPreviewProps &out) noexcept {
  out = CameraPreviewProps{};
  const auto dev = dec.find(1u);
  if (!dev) return nve::NativeViewCodecError::MissingField;
  if (dev->is_null()) out.device_id = std::nullopt;
  else if (const auto s = dev->as_str()) out.device_id = *s;
  else return nve::NativeViewCodecError::TypeMismatch;

  uint32_t facing = 0u, torch = 0u, fps = 0u, stab = 0u;
  NVE_CAM_TRY(dec.require_u32(2u, facing));
  if (facing < 1u || facing > 3u) return nve::NativeViewCodecError::InvalidEnum;
  out.facing = static_cast<CameraPreviewFacing>(facing);
  NVE_CAM_TRY(dec.require_u32(3u, torch));
  if (torch > 2u) return nve::NativeViewCodecError::InvalidEnum;
  out.torch = static_cast<TorchMode>(torch);
  NVE_CAM_TRY(dec.require_f32(4u, out.zoom));
  if (!valid_zoom(out.zoom)) return nve::NativeViewCodecError::TypeMismatch;
  NVE_CAM_TRY(dec.require_u32(5u, fps));
  if (fps < 1u || fps > kMaxCameraFps) return nve::NativeViewCodecError::InvalidEnum;
  out.fps = static_cast<uint16_t>(fps);
  NVE_CAM_TRY(dec.require_bool(6u, out.hdr));
  NVE_CAM_TRY(dec.require_u32(7u, stab));
  if (stab > 2u) return nve::NativeViewCodecError::InvalidEnum;
  out.stabilization = static_cast<StabilizationMode>(stab);
  NVE_CAM_TRY(dec.require_bool(8u, out.enable_frame_processor));
  NVE_CAM_TRY(dec.require_bool(9u, out.audio_enabled));
  return dec.require_bool(10u, out.active);
}

nve::NativeViewCodecError decode_camera_preview_command(const nve::NvncDecoder &dec, CameraPreviewCommand &out) noexcept {
  out = CameraPreviewCommand{};
  auto req_u32 = [&](size_t idx, uint32_t &v) -> nve::NativeViewCodecError {
    const auto a = dec.arg(idx);
    if (!a) return nve::NativeViewCodecError::MissingArg;
    const auto n = a->as_u32();
    if (!n) return nve::NativeViewCodecError::TypeMismatch;
    v = *n;
    return nve::NativeViewCodecError::Ok;
  };
  switch (dec.opcode()) {
    case 1u: {
      uint32_t raw = 0u;
      NVE_CAM_TRY(dec.expect_arg_count(1u)); NVE_CAM_TRY(req_u32(0u, raw));
      if (raw < 1u || raw > 3u) return nve::NativeViewCodecError::InvalidEnum;
      out.facing = static_cast<CameraPreviewFacing>(raw); out.kind = CameraPreviewCommandKind::SwitchFacing;
      return nve::NativeViewCodecError::Ok;
    }
    case 2u: {
      uint32_t raw = 0u;
      NVE_CAM_TRY(dec.expect_arg_count(1u)); NVE_CAM_TRY(req_u32(0u, raw));
      if (raw > 2u) return nve::NativeViewCodecError::InvalidEnum;
      out.torch = static_cast<TorchMode>(raw); out.kind = CameraPreviewCommandKind::SetTorch;
      return nve::NativeViewCodecError::Ok;
    }
    case 3u:
      NVE_CAM_TRY(dec.expect_arg_count(1u)); NVE_CAM_TRY(dec.require_f32(0u, out.zoom));
      if (!valid_zoom(out.zoom)) return nve::NativeViewCodecError::TypeMismatch;
      out.kind = CameraPreviewCommandKind::SetZoom; return nve::NativeViewCodecError::Ok;
    case 4u:
      NVE_CAM_TRY(dec.expect_arg_count(2u));
      NVE_CAM_TRY(dec.require_f32(0u, out.focus_x)); NVE_CAM_TRY(dec.require_f32(1u, out.focus_y));
      if (!valid_unit(out.focus_x) || !valid_unit(out.focus_y)) return nve::NativeViewCodecError::TypeMismatch;
      out.kind = CameraPreviewCommandKind::FocusPoint; return nve::NativeViewCodecError::Ok;
    case 5u: {
      uint32_t q = 0u, fl = 0u;
      NVE_CAM_TRY(dec.expect_arg_count(2u)); NVE_CAM_TRY(req_u32(0u, q)); NVE_CAM_TRY(req_u32(1u, fl));
      if (q > 100u || fl > 2u) return nve::NativeViewCodecError::InvalidEnum;
      out.quality = static_cast<uint8_t>(q); out.flash = static_cast<uint8_t>(fl);
      out.kind = CameraPreviewCommandKind::CapturePhoto; return nve::NativeViewCodecError::Ok;
    }
    case 6u:
      NVE_CAM_TRY(dec.expect_arg_count(1u)); NVE_CAM_TRY(dec.require_str(0u, out.output_path));
      out.kind = CameraPreviewCommandKind::StartRecording; return nve::NativeViewCodecError::Ok;
    case 7u:
      out.kind = CameraPreviewCommandKind::StopRecording; return dec.expect_arg_count(0u);
    default:
      return nve::NativeViewCodecError::UnknownCommandOpcode;
  }
}

#undef NVE_CAM_TRY

nve::NativeViewCodecError encode_camera_frame_descriptor(
    const CameraFrameDescriptor &desc, std::array<uint8_t, kCameraFrameDescriptorSize> &out) noexcept {
  if (auto e = validate_frame_desc(desc); e != nve::NativeViewCodecError::Ok) return e;
  CameraFrameWireLayout wire{
      .magic = {'N', 'V', 'C', 'F'}, .version = kCameraFrameVersion,
      .pixel_format = static_cast<uint8_t>(desc.pixel_format), .rotation_deg = desc.rotation_deg,
      .width = desc.width, .height = desc.height, .stride_bytes = desc.stride_bytes, .reserved = 0u,
      .timestamp_ns = desc.timestamp_ns, .native_handle_id = desc.native_handle_id,
  };
  std::memcpy(out.data(), &wire, sizeof(wire));
  return nve::NativeViewCodecError::Ok;
}

nve::NativeViewCodecError decode_camera_frame_descriptor(
    std::span<const uint8_t> bytes, CameraFrameDescriptor &out) noexcept {
  if (bytes.size() < kCameraFrameDescriptorSize) return nve::NativeViewCodecError::Truncated;
  if (bytes.size() > kCameraFrameDescriptorSize) return nve::NativeViewCodecError::TrailingBytes;
  CameraFrameWireLayout wire{};
  std::memcpy(&wire, bytes.data(), sizeof(wire));
  if (std::memcmp(wire.magic, kCameraFrameMagic.data(), 4u) != 0) return nve::NativeViewCodecError::InvalidMagic;
  if (wire.version != kCameraFrameVersion) return nve::NativeViewCodecError::UnsupportedVersion;
  if (wire.pixel_format > 3u || wire.reserved != 0u) return nve::NativeViewCodecError::InvalidEnum;
  CameraFrameDescriptor parsed{
      .width = wire.width, .height = wire.height, .stride_bytes = wire.stride_bytes,
      .pixel_format = static_cast<CameraPixelFormat>(wire.pixel_format), .rotation_deg = wire.rotation_deg,
      .timestamp_ns = wire.timestamp_ns, .native_handle_id = wire.native_handle_id,
  };
  if (auto e = validate_frame_desc(parsed); e != nve::NativeViewCodecError::Ok) return e;
  out = parsed;
  return nve::NativeViewCodecError::Ok;
}

void set_vision_camera_apple_bridge(id<NveVisionCameraAppleBridge> _Nullable bridge) {
  std::lock_guard<std::mutex> lock(vision_bridge_mutex());
  vision_bridge_slot() = bridge;
}

id<NveVisionCameraAppleBridge> _Nullable get_vision_camera_apple_bridge() {
  std::lock_guard<std::mutex> lock(vision_bridge_mutex());
  return vision_bridge_slot();
}

namespace {

nve::NativeViewAdapter make_camera_adapter() {
  nve::NativeViewAdapter a{};
  a.create = [](uint32_t node_id, std::string_view, const nve::NvnpDecoder *) -> void * {
    id<NveVisionCameraAppleBridge> b = get_vision_camera_apple_bridge();
    if (b == nil) return nullptr;
    NvePlatformView *v = [b createVisionCameraViewForNodeId:node_id frame:CGRectZero];
    if (v == nil) return nullptr;
    objc_setAssociatedObject(v, &kVisionBridgeAssocKey, b, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    return (__bridge_retained void *)v;
  };
  a.update_props = [](void *inst, uint32_t, const nve::NvnpDecoder &dec) {
    CameraPreviewProps props{};
    if (inst == nullptr || decode_camera_preview_props(dec, props) != nve::NativeViewCodecError::Ok) return;
    NvePlatformView *v = (__bridge NvePlatformView *)inst;
    if (id<NveVisionCameraAppleBridge> b = objc_getAssociatedObject(v, &kVisionBridgeAssocKey)) {
      [b updateVisionCameraView:v props:props];
    }
  };
  a.dispatch_command = [](void *inst, uint32_t, std::string_view, const nve::NvncDecoder &dec) -> bool {
    CameraPreviewCommand cmd{};
    if (inst == nullptr || decode_camera_preview_command(dec, cmd) != nve::NativeViewCodecError::Ok) return false;
    NvePlatformView *v = (__bridge NvePlatformView *)inst;
    id<NveVisionCameraAppleBridge> b = objc_getAssociatedObject(v, &kVisionBridgeAssocKey);
    return b != nil && [b dispatchVisionCameraCommand:v command:cmd] ? true : false;
  };
  a.unmount = [](void *inst, uint32_t) {
    if (inst == nullptr) return;
    NvePlatformView *v = (__bridge_transfer NvePlatformView *)inst;
    if (id<NveVisionCameraAppleBridge> b = objc_getAssociatedObject(v, &kVisionBridgeAssocKey)) {
      [b unmountVisionCameraView:v];
      objc_setAssociatedObject(v, &kVisionBridgeAssocKey, nil, OBJC_ASSOCIATION_ASSIGN);
    }
    [v removeFromSuperview];
  };
  return a;
}

} // namespace

void register_apple_camera_native_view(nve::NativeViewRegistry &registry) {
  registry.register_adapter(kNativeKindCameraPreview, make_camera_adapter());
  registry.register_adapter(kNativeKindCameraAlias, make_camera_adapter());
}

void bind_apple_camera_native_view_to_fabric_host(NveFabricHostHandle _Nullable host, nve::NativeViewRegistry &registry) {
  if (host == nullptr) return;
  registry.bind_to_fabric_host(host, kNativeKindCameraPreview);
  registry.bind_to_fabric_host(host, kNativeKindCameraAlias);
}

} // namespace nve::apple
