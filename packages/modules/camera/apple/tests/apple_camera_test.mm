#import <AppKit/AppKit.h>
#import "NveAppleCamera.h"

#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <optional>
#include <string_view>
#include <vector>

#define NVE_CHECK(expr)                                                               \
  do {                                                                                \
    if (!(expr)) {                                                                    \
      std::fprintf(stderr, "NVE_CHECK failed at %s:%d: %s\n", __FILE__, __LINE__,     \
                   #expr);                                                            \
      std::abort();                                                                   \
    }                                                                                 \
  } while (false)

@interface MockVisionCameraBridge : NSObject <NveVisionCameraAppleBridge>
@property (nonatomic) NSUInteger createCount;
@property (nonatomic) NSUInteger updateCount;
@property (nonatomic) NSUInteger commandCount;
@property (nonatomic) NSUInteger unmountCount;
@property (nonatomic) nve::apple::CameraPreviewFacing lastFacing;
@property (nonatomic) nve::apple::TorchMode lastTorch;
@property (nonatomic) float lastZoom;
@property (nonatomic) nve::apple::CameraPreviewCommandKind lastCommandKind;
@property (nonatomic, copy, nullable) NSString *lastRecordingPath;
@end

@implementation MockVisionCameraBridge
- (NvePlatformView *)createVisionCameraViewForNodeId:(uint32_t)nodeId frame:(CGRect)frame {
  (void)nodeId;
  ++self.createCount;
  return [[NSView alloc] initWithFrame:frame];
}
- (void)updateVisionCameraView:(NvePlatformView *)view
                         props:(const nve::apple::CameraPreviewProps &)props {
  (void)view;
  ++self.updateCount;
  self.lastFacing = props.facing;
  self.lastTorch = props.torch;
  self.lastZoom = props.zoom;
}
- (BOOL)dispatchVisionCameraCommand:(NvePlatformView *)view
                            command:(const nve::apple::CameraPreviewCommand &)cmd {
  (void)view;
  ++self.commandCount;
  self.lastCommandKind = cmd.kind;
  if (cmd.kind == nve::apple::CameraPreviewCommandKind::SwitchFacing) {
    self.lastFacing = cmd.facing;
  } else if (cmd.kind == nve::apple::CameraPreviewCommandKind::StartRecording) {
    self.lastRecordingPath = nve::apple::nsstring_from_sv(cmd.output_path);
  }
  return YES;
}
- (void)unmountVisionCameraView:(NvePlatformView *)view {
  (void)view;
  ++self.unmountCount;
}
@end

namespace {

NvePlatformView * _Nullable view_for_node(const nve::NativeViewRegistry &registry, uint32_t node_id) {
  void *raw = registry.instance_for(node_id);
  return raw != nullptr ? (__bridge NvePlatformView *)raw : nil;
}

void push_u16(std::vector<uint8_t> &b, uint16_t v) {
  b.push_back(static_cast<uint8_t>(v & 0xFFu));
  b.push_back(static_cast<uint8_t>((v >> 8u) & 0xFFu));
}

void push_u32(std::vector<uint8_t> &b, uint32_t v) {
  for (int i = 0; i < 4; ++i) b.push_back(static_cast<uint8_t>((v >> (i * 8)) & 0xFFu));
}

void push_f32(std::vector<uint8_t> &b, float v) {
  uint32_t bits = 0u;
  std::memcpy(&bits, &v, sizeof(bits));
  for (int i = 0; i < 4; ++i) b.push_back(static_cast<uint8_t>((bits >> (i * 8)) & 0xFFu));
}

std::vector<uint8_t> encode_camera_props(
    std::optional<std::string_view> device_id,
    uint32_t facing,
    uint32_t torch,
    float zoom,
    uint32_t fps,
    bool hdr,
    uint32_t stabilization,
    bool enable_fp,
    bool audio,
    bool active) {
  std::vector<uint8_t> buf = {'N', 'V', 'N', 'P', 1u, 10u, 0u};
  push_u16(buf, 1u);
  if (device_id.has_value()) {
    buf.push_back(7u);
    push_u32(buf, static_cast<uint32_t>(device_id->size()));
    buf.insert(buf.end(), device_id->begin(), device_id->end());
  } else {
    buf.push_back(0u);
  }
  push_u16(buf, 2u); buf.push_back(2u); push_u32(buf, facing);
  push_u16(buf, 3u); buf.push_back(2u); push_u32(buf, torch);
  push_u16(buf, 4u); buf.push_back(5u); push_f32(buf, zoom);
  push_u16(buf, 5u); buf.push_back(2u); push_u32(buf, fps);
  push_u16(buf, 6u); buf.push_back(1u); buf.push_back(hdr ? 1u : 0u);
  push_u16(buf, 7u); buf.push_back(2u); push_u32(buf, stabilization);
  push_u16(buf, 8u); buf.push_back(1u); buf.push_back(enable_fp ? 1u : 0u);
  push_u16(buf, 9u); buf.push_back(1u); buf.push_back(audio ? 1u : 0u);
  push_u16(buf, 10u); buf.push_back(1u); buf.push_back(active ? 1u : 0u);
  return buf;
}

std::vector<uint8_t> encode_cmd_u32(uint16_t op, uint32_t a0) {
  std::vector<uint8_t> buf = {'N', 'V', 'N', 'C', 1u};
  push_u16(buf, op); buf.push_back(1u); buf.push_back(2u); push_u32(buf, a0);
  return buf;
}

std::vector<uint8_t> encode_cmd_u32_u32(uint16_t op, uint32_t a0, uint32_t a1) {
  std::vector<uint8_t> buf = {'N', 'V', 'N', 'C', 1u};
  push_u16(buf, op); buf.push_back(2u);
  buf.push_back(2u); push_u32(buf, a0);
  buf.push_back(2u); push_u32(buf, a1);
  return buf;
}

std::vector<uint8_t> encode_cmd_f32(uint16_t op, float a0) {
  std::vector<uint8_t> buf = {'N', 'V', 'N', 'C', 1u};
  push_u16(buf, op); buf.push_back(1u); buf.push_back(5u); push_f32(buf, a0);
  return buf;
}

std::vector<uint8_t> encode_cmd_f32_f32(uint16_t op, float a0, float a1) {
  std::vector<uint8_t> buf = {'N', 'V', 'N', 'C', 1u};
  push_u16(buf, op); buf.push_back(2u);
  buf.push_back(5u); push_f32(buf, a0);
  buf.push_back(5u); push_f32(buf, a1);
  return buf;
}

std::vector<uint8_t> encode_cmd_str(uint16_t op, std::string_view s) {
  std::vector<uint8_t> buf = {'N', 'V', 'N', 'C', 1u};
  push_u16(buf, op); buf.push_back(1u); buf.push_back(7u);
  push_u32(buf, static_cast<uint32_t>(s.size()));
  buf.insert(buf.end(), s.begin(), s.end());
  return buf;
}

std::vector<uint8_t> encode_cmd_empty(uint16_t op) {
  std::vector<uint8_t> buf = {'N', 'V', 'N', 'C', 1u};
  push_u16(buf, op); buf.push_back(0u);
  return buf;
}

} // namespace

int main() {
  @autoreleasepool {
    // 1. NVCF 40-byte wire layout + validation edge cases.
    nve::apple::CameraFrameDescriptor fd{};
    fd.width = 1920u;
    fd.height = 1080u;
    fd.stride_bytes = 7680u;
    fd.pixel_format = nve::apple::CameraPixelFormat::Bgra8888;
    fd.rotation_deg = 90u;
    fd.timestamp_ns = 0x1122334455667788ULL;
    fd.native_handle_id = 0xCAFEBABE12345678ULL;
    std::array<uint8_t, nve::apple::kCameraFrameDescriptorSize> wire{};
    NVE_CHECK(nve::apple::encode_camera_frame_descriptor(fd, wire) == nve::NativeViewCodecError::Ok);
    NVE_CHECK(wire[0] == 'N' && wire[1] == 'V' && wire[2] == 'C' && wire[3] == 'F');
    NVE_CHECK(wire[4] == 1u && wire[5] == 1u && wire[6] == 90u && wire[7] == 0u);
    nve::apple::CameraFrameDescriptor parsedFd{};
    NVE_CHECK(nve::apple::decode_camera_frame_descriptor(wire, parsedFd) == nve::NativeViewCodecError::Ok);
    NVE_CHECK(parsedFd.width == 1920u && parsedFd.height == 1080u && parsedFd.stride_bytes == 7680u);
    NVE_CHECK(parsedFd.pixel_format == nve::apple::CameraPixelFormat::Bgra8888);
    NVE_CHECK(parsedFd.rotation_deg == 90u && parsedFd.timestamp_ns == fd.timestamp_ns);
    NVE_CHECK(parsedFd.native_handle_id == fd.native_handle_id);

    auto badMagic = wire; badMagic[0] = 'X';
    NVE_CHECK(nve::apple::decode_camera_frame_descriptor(badMagic, parsedFd) == nve::NativeViewCodecError::InvalidMagic);
    auto badReserved = wire; badReserved[20] = 1u;
    NVE_CHECK(nve::apple::decode_camera_frame_descriptor(badReserved, parsedFd) == nve::NativeViewCodecError::InvalidEnum);
    auto badRot = wire; badRot[6] = 45u;
    NVE_CHECK(nve::apple::decode_camera_frame_descriptor(badRot, parsedFd) == nve::NativeViewCodecError::InvalidEnum);
    NVE_CHECK(nve::apple::decode_camera_frame_descriptor(std::span<const uint8_t>(wire.data(), 39u), parsedFd) ==
              nve::NativeViewCodecError::Truncated);

    // Invalid NVNP props (facing=0, fps=241, zoom=0.0f) rejected cleanly.
    {
      nve::NvnpDecoder badDec{};
      nve::apple::CameraPreviewProps badProps{};
      const auto badFacing = encode_camera_props(std::nullopt, 0u, 0u, 1.0f, 30u, false, 0u, false, false, true);
      NVE_CHECK(nve::NvnpDecoder::parse(badFacing, badDec) == nve::NativeViewCodecError::Ok);
      NVE_CHECK(nve::apple::decode_camera_preview_props(badDec, badProps) == nve::NativeViewCodecError::InvalidEnum);
      const auto badFps = encode_camera_props(std::nullopt, 1u, 0u, 1.0f, 241u, false, 0u, false, false, true);
      NVE_CHECK(nve::NvnpDecoder::parse(badFps, badDec) == nve::NativeViewCodecError::Ok);
      NVE_CHECK(nve::apple::decode_camera_preview_props(badDec, badProps) == nve::NativeViewCodecError::InvalidEnum);
      const auto badZoom = encode_camera_props(std::nullopt, 1u, 0u, 0.0f, 30u, false, 0u, false, false, true);
      NVE_CHECK(nve::NvnpDecoder::parse(badZoom, badDec) == nve::NativeViewCodecError::Ok);
      NVE_CHECK(nve::apple::decode_camera_preview_props(badDec, badProps) == nve::NativeViewCodecError::TypeMismatch);
    }

    // 2. Without a registered VisionCamera bridge, mounting returns nullptr (unsupported).
    nve::apple::set_vision_camera_apple_bridge(nil);
    nve::NativeViewRegistry registry;
    nve::apple::register_apple_camera_native_view(registry);
    NVE_CHECK(registry.supports(nve::apple::kNativeKindCameraPreview));
    NVE_CHECK(registry.supports(nve::apple::kNativeKindCameraAlias));

    const auto vcProps = encode_camera_props(
        std::nullopt, 2u, 1u, 1.75f, 30u, false, 1u, false, true, true);
    NVE_CHECK(registry.mount_or_update(30u, nve::apple::kNativeKindCameraPreview, vcProps) == nullptr);
    NVE_CHECK(view_for_node(registry, 30u) == nil);

    // 3. Pluggable VisionCamera v5 bridge delegates create, update_props, dispatch_command, unmount.
    MockVisionCameraBridge *mockBridge = [[MockVisionCameraBridge alloc] init];
    nve::apple::set_vision_camera_apple_bridge(mockBridge);
    NVE_CHECK(nve::apple::get_vision_camera_apple_bridge() == mockBridge);

    NVE_CHECK(registry.mount_or_update(40u, nve::apple::kNativeKindCameraPreview, vcProps) != nullptr);
    // The alias kind resolves to the same adapter.
    NVE_CHECK(registry.mount_or_update(42u, nve::apple::kNativeKindCameraAlias, vcProps) != nullptr);
    NVE_CHECK(registry.unmount(42u));

    const std::vector<uint8_t> cmds[] = {
        encode_cmd_u32(1u, 3u), encode_cmd_u32(2u, 1u), encode_cmd_f32(3u, 3.25f), encode_cmd_f32(3u, -2.0f),
        encode_cmd_f32_f32(4u, 0.25f, 0.75f), encode_cmd_u32_u32(5u, 85u, 2u),
        encode_cmd_str(6u, "/tmp/vc.mp4"), encode_cmd_empty(7u),
    };
    const char *names[] = {"switch_facing", "set_torch", "set_zoom", "set_zoom",
                           "focus_point", "capture_photo", "start_recording", "stop_recording"};
    for (size_t i = 0; i < std::size(cmds); ++i) {
      nve::NativeViewCodecError err = nve::NativeViewCodecError::TrailingBytes;
      (void)registry.dispatch_command(40u, names[i], cmds[i], &err);
      NVE_CHECK(err == nve::NativeViewCodecError::Ok);
    }

    NSView *vcRaw = view_for_node(registry, 40u);
    NVE_CHECK(vcRaw != nil);
    // Two mounts (preview + alias), seven accepted commands (the negative zoom is rejected).
    NVE_CHECK(mockBridge.createCount == 2u && mockBridge.updateCount == 2u && mockBridge.commandCount == 7u);
    NVE_CHECK(mockBridge.lastFacing == nve::apple::CameraPreviewFacing::External);
    NVE_CHECK(mockBridge.lastTorch == nve::apple::TorchMode::On);
    NVE_CHECK(std::fabs(mockBridge.lastZoom - 1.75f) < 1e-4f);
    NVE_CHECK([mockBridge.lastRecordingPath isEqualToString:@"/tmp/vc.mp4"]);
    NVE_CHECK(registry.unmount(40u));
    NVE_CHECK(mockBridge.unmountCount == 2u);

    // Clear bridge -> mounting returns nullptr again.
    nve::apple::set_vision_camera_apple_bridge(nil);
    NVE_CHECK(registry.mount_or_update(41u, nve::apple::kNativeKindCameraPreview, vcProps) == nullptr);
    NVE_CHECK(registry.mounted_count() == 0u);

    std::puts("nve_apple_camera_test: ALL PASSED");
  }
  return 0;
}
