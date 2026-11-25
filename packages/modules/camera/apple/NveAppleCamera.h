#pragma once

#import "NveAppleSupport.h"

#include "nve_native_view.hpp"

#include <array>
#include <cstddef>
#include <cstdint>
#include <optional>
#include <span>
#include <string_view>

namespace nve::apple {

inline constexpr std::string_view kNativeKindCameraPreview = "nve.camera.preview";
inline constexpr std::string_view kNativeKindCameraAlias = "nve.camera";
inline constexpr std::array<uint8_t, 4> kCameraFrameMagic = {'N', 'V', 'C', 'F'};
inline constexpr uint8_t kCameraFrameVersion = 1u;
inline constexpr size_t kCameraFrameDescriptorSize = 40u;
inline constexpr uint16_t kMaxCameraFps = 240u;

enum class CameraPreviewFacing : uint8_t { Back = 1, Front = 2, External = 3 };
enum class TorchMode : uint8_t { Off = 0, On = 1, Auto = 2 };
enum class StabilizationMode : uint8_t { Off = 0, Standard = 1, Cinematic = 2 };
enum class CameraPixelFormat : uint8_t { Rgba8888 = 0, Bgra8888 = 1, Nv12 = 2, Yuv420_888 = 3 };

struct CameraPreviewProps {
  std::optional<std::string_view> device_id{};
  CameraPreviewFacing facing{CameraPreviewFacing::Back};
  TorchMode torch{TorchMode::Off};
  float zoom{1.0f};
  uint16_t fps{30u};
  bool hdr{false};
  StabilizationMode stabilization{StabilizationMode::Off};
  bool enable_frame_processor{false};
  bool audio_enabled{false};
  bool active{true};
};

enum class CameraPreviewCommandKind : uint16_t {
  SwitchFacing = 1, SetTorch = 2, SetZoom = 3, FocusPoint = 4,
  CapturePhoto = 5, StartRecording = 6, StopRecording = 7,
};

struct CameraPreviewCommand {
  CameraPreviewCommandKind kind{CameraPreviewCommandKind::StopRecording};
  CameraPreviewFacing facing{CameraPreviewFacing::Back};
  TorchMode torch{TorchMode::Off};
  float zoom{1.0f};
  float focus_x{0.5f};
  float focus_y{0.5f};
  uint8_t quality{90u};
  uint8_t flash{0u};
  std::string_view output_path{};
};

struct CameraFrameDescriptor {
  uint32_t width{0u};
  uint32_t height{0u};
  uint32_t stride_bytes{0u};
  CameraPixelFormat pixel_format{CameraPixelFormat::Bgra8888};
  uint16_t rotation_deg{0u};
  uint64_t timestamp_ns{0u};
  uint64_t native_handle_id{0u};
};

nve::NativeViewCodecError decode_camera_preview_props(
    const nve::NvnpDecoder &dec, CameraPreviewProps &out) noexcept;
nve::NativeViewCodecError decode_camera_preview_command(
    const nve::NvncDecoder &dec, CameraPreviewCommand &out) noexcept;
nve::NativeViewCodecError encode_camera_frame_descriptor(
    const CameraFrameDescriptor &desc,
    std::array<uint8_t, kCameraFrameDescriptorSize> &out) noexcept;
nve::NativeViewCodecError decode_camera_frame_descriptor(
    std::span<const uint8_t> bytes, CameraFrameDescriptor &out) noexcept;

} // namespace nve::apple

NS_ASSUME_NONNULL_BEGIN

/// Pluggable bridge for `react-native-vision-camera` v5 (`HybridCameraSession` /
/// `HybridCameraView`) on Apple hosts. When no bridge is registered, mounting
/// `"nve.camera.preview"` explicitly returns `nullptr` (unsupported).
@protocol NveVisionCameraAppleBridge <NSObject>
- (NvePlatformView *)createVisionCameraViewForNodeId:(uint32_t)nodeId frame:(CGRect)frame;
- (void)updateVisionCameraView:(NvePlatformView *)view props:(const nve::apple::CameraPreviewProps &)props;
- (BOOL)dispatchVisionCameraCommand:(NvePlatformView *)view command:(const nve::apple::CameraPreviewCommand &)cmd;
- (void)unmountVisionCameraView:(NvePlatformView *)view;
@end

NS_ASSUME_NONNULL_END

namespace nve::apple {

void set_vision_camera_apple_bridge(id<NveVisionCameraAppleBridge> _Nullable bridge);
id<NveVisionCameraAppleBridge> _Nullable get_vision_camera_apple_bridge();
void register_apple_camera_native_view(nve::NativeViewRegistry &registry);
void bind_apple_camera_native_view_to_fabric_host(
    NveFabricHostHandle _Nullable host, nve::NativeViewRegistry &registry);

} // namespace nve::apple
