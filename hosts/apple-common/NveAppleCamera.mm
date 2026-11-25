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

CameraPixelFormat map_cv_pixel_format(OSType fmt) noexcept {
  switch (fmt) {
    case kCVPixelFormatType_32RGBA: return CameraPixelFormat::Rgba8888;
    case kCVPixelFormatType_32BGRA: return CameraPixelFormat::Bgra8888;
    case kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange:
    case kCVPixelFormatType_420YpCbCr8BiPlanarFullRange: return CameraPixelFormat::Nv12;
    case kCVPixelFormatType_420YpCbCr8Planar:
    case kCVPixelFormatType_420YpCbCr8PlanarFullRange: return CameraPixelFormat::Yuv420_888;
    default: return CameraPixelFormat::Bgra8888;
  }
}

AVCaptureTorchMode map_av_torch(TorchMode m) noexcept {
  return m == TorchMode::On ? AVCaptureTorchModeOn : (m == TorchMode::Auto ? AVCaptureTorchModeAuto : AVCaptureTorchModeOff);
}
AVCaptureFlashMode map_av_flash(uint8_t f) noexcept {
  return f == 1u ? AVCaptureFlashModeOn : (f == 2u ? AVCaptureFlashModeAuto : AVCaptureFlashModeOff);
}
AVCaptureDevicePosition map_av_position(CameraPreviewFacing f) noexcept {
  return f == CameraPreviewFacing::Front ? AVCaptureDevicePositionFront
                                         : (f == CameraPreviewFacing::External ? AVCaptureDevicePositionUnspecified : AVCaptureDevicePositionBack);
}

BOOL can_open_media_without_tcc_crash(AVMediaType mediaType, NSString *usageKey) {
  const AVAuthorizationStatus status = [AVCaptureDevice authorizationStatusForMediaType:mediaType];
  if (status == AVAuthorizationStatusAuthorized) return YES;
  if (status == AVAuthorizationStatusDenied || status == AVAuthorizationStatusRestricted) return NO;
  id desc = [[NSBundle mainBundle] objectForInfoDictionaryKey:usageKey];
  return [desc isKindOfClass:[NSString class]] && [(NSString *)desc length] > 0u;
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

} // namespace nve::apple

@interface NveAppleCameraView () <
    AVCaptureVideoDataOutputSampleBufferDelegate,
    AVCapturePhotoCaptureDelegate,
    AVCaptureFileOutputRecordingDelegate>
@property (nonatomic, readwrite) AVCaptureSession *nveCaptureSession;
@property (nonatomic, readwrite) AVCaptureVideoPreviewLayer *nvePreviewLayer;
@property (nonatomic, readwrite) nve::apple::CameraPreviewFacing nveFacing;
@property (nonatomic, readwrite) nve::apple::TorchMode nveTorchMode;
@property (nonatomic, readwrite) float nveZoom;
@property (nonatomic, readwrite) uint16_t nveFps;
@property (nonatomic, readwrite) BOOL nveHdr;
@property (nonatomic, readwrite) nve::apple::StabilizationMode nveStabilization;
@property (nonatomic, readwrite) BOOL nveFrameProcessorEnabled;
@property (nonatomic, readwrite) BOOL nveAudioEnabled;
@property (nonatomic, readwrite) BOOL nveActive;
@property (nonatomic, readwrite) BOOL nveHasHardwareDevice;
@property (nonatomic, readwrite) BOOL nveIsRecording;
@property (nonatomic, readwrite) CGPoint nveLastFocusPoint;
@property (nonatomic, readwrite) NSUInteger nvePhotoCaptureCount;
@property (nonatomic, readwrite) NSUInteger nveFrameCount;
@property (nonatomic, readwrite, copy, nullable) NSString *nveDeviceId;
@property (nonatomic, readwrite, copy, nullable) NSString *nveLastRecordingPath;
@property (nonatomic, readwrite, copy, nullable) NSData *nveLastFrameDescriptor;
@end

@implementation NveAppleCameraView {
  AVCaptureDevice *_videoDevice;
  AVCaptureDeviceInput *_videoInput;
  AVCaptureDeviceInput *_audioInput;
  AVCaptureVideoDataOutput *_videoDataOutput;
  AVCapturePhotoOutput *_photoOutput;
  AVCaptureMovieFileOutput *_movieOutput;
  dispatch_queue_t _sessionQueue;
  dispatch_queue_t _frameQueue;
}

- (instancetype)initWithFrame:(CGRect)frame {
  if ((self = [super initWithFrame:frame])) {
    _nveCaptureSession = [[AVCaptureSession alloc] init];
    _nvePreviewLayer = [AVCaptureVideoPreviewLayer layerWithSession:_nveCaptureSession];
    _nvePreviewLayer.videoGravity = AVLayerVideoGravityResizeAspectFill;
    _nvePreviewLayer.frame = self.bounds;
    _videoDataOutput = [[AVCaptureVideoDataOutput alloc] init];
    _videoDataOutput.alwaysDiscardsLateVideoFrames = YES;
    _videoDataOutput.videoSettings = @{(id)kCVPixelBufferPixelFormatTypeKey : @(kCVPixelFormatType_32BGRA)};
    _photoOutput = [[AVCapturePhotoOutput alloc] init];
    _movieOutput = [[AVCaptureMovieFileOutput alloc] init];
    _sessionQueue = dispatch_queue_create("dev.nve.camera.session", DISPATCH_QUEUE_SERIAL);
    _frameQueue = dispatch_queue_create("dev.nve.camera.frames", DISPATCH_QUEUE_SERIAL);
    _nveFacing = nve::apple::CameraPreviewFacing::Back;
    _nveTorchMode = nve::apple::TorchMode::Off;
    _nveZoom = 1.0f; _nveFps = 30u; _nveActive = YES; _nveLastFocusPoint = CGPointMake(0.5, 0.5);
#if TARGET_OS_OSX
    self.wantsLayer = YES;
    self.layer.backgroundColor = [NSColor blackColor].CGColor;
#else
    self.backgroundColor = [UIColor blackColor];
    self.clipsToBounds = YES;
#endif
    [self.layer addSublayer:_nvePreviewLayer];
  }
  return self;
}

- (void)dealloc {
  if (_movieOutput.isRecording) [_movieOutput stopRecording];
  if (_nveCaptureSession.isRunning) [_nveCaptureSession stopRunning];
}

#if TARGET_OS_OSX
- (BOOL)isFlipped { return YES; }
- (void)layout { [super layout]; _nvePreviewLayer.frame = self.bounds; }
#else
- (void)layoutSubviews { [super layoutSubviews]; _nvePreviewLayer.frame = self.bounds; }
#endif

- (nullable AVCaptureDevice *)_discoverVideoDevice {
  if (_nveDeviceId.length > 0u) {
    if (AVCaptureDevice *exact = [AVCaptureDevice deviceWithUniqueID:_nveDeviceId]) return exact;
  }
  NSMutableArray<AVCaptureDeviceType> *types = [NSMutableArray arrayWithObject:AVCaptureDeviceTypeBuiltInWideAngleCamera];
  if (@available(macOS 14.0, iOS 17.0, *)) [types addObject:AVCaptureDeviceTypeExternal];
  auto findPos = ^(AVCaptureDevicePosition pos) {
    return [AVCaptureDeviceDiscoverySession discoverySessionWithDeviceTypes:types mediaType:AVMediaTypeVideo position:pos].devices.firstObject;
  };
  AVCaptureDevice *dev = findPos(nve::apple::map_av_position(_nveFacing));
  return dev != nil ? dev : findPos(AVCaptureDevicePositionUnspecified);
}

- (void)_applyDeviceControls {
  AVCaptureDevice *device = _videoDevice;
  if (device == nil) {
#if TARGET_OS_OSX
    _nvePreviewLayer.transform = _nveZoom > 1.0f ? CATransform3DMakeScale(_nveZoom, _nveZoom, 1.0) : CATransform3DIdentity;
#endif
    return;
  }
  NSError *err = nil;
  if (![device lockForConfiguration:&err]) return;
  const AVCaptureTorchMode avTorch = nve::apple::map_av_torch(_nveTorchMode);
  if (device.hasTorch && [device isTorchModeSupported:avTorch]) device.torchMode = avTorch;
  const double targetFps = static_cast<double>(_nveFps);
  for (AVFrameRateRange *r in device.activeFormat.videoSupportedFrameRateRanges) {
    if (targetFps >= r.minFrameRate && targetFps <= r.maxFrameRate) {
      const CMTime dur = CMTimeMake(1, static_cast<int32_t>(_nveFps));
      device.activeVideoMinFrameDuration = dur; device.activeVideoMaxFrameDuration = dur; break;
    }
  }
#if !TARGET_OS_OSX
  const CGFloat maxZoom = MAX(1.0, MIN(device.activeFormat.videoMaxZoomFactor, 16.0));
  device.videoZoomFactor = std::clamp(static_cast<CGFloat>(_nveZoom), 1.0, maxZoom);
  if (device.activeFormat.isVideoHDRSupported) {
    device.automaticallyAdjustsVideoHDREnabled = NO; device.videoHDREnabled = _nveHdr;
  }
#else
  _nvePreviewLayer.transform = _nveZoom > 1.0f ? CATransform3DMakeScale(_nveZoom, _nveZoom, 1.0) : CATransform3DIdentity;
#endif
  [device unlockForConfiguration];
#if !TARGET_OS_OSX
  AVCaptureConnection *conn = [_videoDataOutput connectionWithMediaType:AVMediaTypeVideo];
  if (conn != nil && conn.isVideoStabilizationSupported) {
    conn.preferredVideoStabilizationMode =
        _nveStabilization == nve::apple::StabilizationMode::Cinematic ? AVCaptureVideoStabilizationModeCinematic
        : (_nveStabilization == nve::apple::StabilizationMode::Standard ? AVCaptureVideoStabilizationModeStandard
                                                                        : AVCaptureVideoStabilizationModeOff);
  }
#endif
}

- (void)_configureHardwareSession {
  if (!nve::apple::can_open_media_without_tcc_crash(AVMediaTypeVideo, @"NSCameraUsageDescription")) {
    _nveHasHardwareDevice = NO; return;
  }
  if ([AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeVideo] == AVAuthorizationStatusNotDetermined) {
    __weak NveAppleCameraView *weakSelf = self;
    [AVCaptureDevice requestAccessForMediaType:AVMediaTypeVideo completionHandler:^(BOOL granted) {
      if (granted) dispatch_async(dispatch_get_main_queue(), ^{ [weakSelf _configureHardwareSession]; });
    }];
    return;
  }
  AVCaptureDevice *device = [self _discoverVideoDevice];
  NSError *inputErr = nil;
  AVCaptureDeviceInput *newIn = device != nil ? [AVCaptureDeviceInput deviceInputWithDevice:device error:&inputErr] : nil;
  if (newIn == nil || inputErr != nil) { _nveHasHardwareDevice = NO; return; }

  [_nveCaptureSession beginConfiguration];
  if (_videoInput != nil) { [_nveCaptureSession removeInput:_videoInput]; _videoInput = nil; }
  _nveHasHardwareDevice = [_nveCaptureSession canAddInput:newIn];
  if (_nveHasHardwareDevice) { [_nveCaptureSession addInput:newIn]; _videoInput = newIn; _videoDevice = device; }
  if (_audioInput != nil && !_nveAudioEnabled) {
    [_nveCaptureSession removeInput:_audioInput]; _audioInput = nil;
  } else if (_nveAudioEnabled && _audioInput == nil &&
             nve::apple::can_open_media_without_tcc_crash(AVMediaTypeAudio, @"NSMicrophoneUsageDescription")) {
    if (AVCaptureDevice *mic = [AVCaptureDevice defaultDeviceWithMediaType:AVMediaTypeAudio]) {
      NSError *micErr = nil;
      if (AVCaptureDeviceInput *micIn = [AVCaptureDeviceInput deviceInputWithDevice:mic error:&micErr];
          micIn != nil && [_nveCaptureSession canAddInput:micIn]) {
        [_nveCaptureSession addInput:micIn]; _audioInput = micIn;
      }
    }
  }
  for (AVCaptureOutput *out in @[_videoDataOutput, _photoOutput, _movieOutput]) {
    if (![_nveCaptureSession.outputs containsObject:out] && [_nveCaptureSession canAddOutput:out]) [_nveCaptureSession addOutput:out];
  }
  [_videoDataOutput setSampleBufferDelegate:(_nveFrameProcessorEnabled ? self : nil) queue:_frameQueue];
  [_nveCaptureSession commitConfiguration];
  [self _applyDeviceControls];

  AVCaptureSession *session = _nveCaptureSession;
  const BOOL shouldRun = _nveActive && _nveHasHardwareDevice;
  dispatch_async(_sessionQueue, ^{
    if (shouldRun && !session.isRunning) [session startRunning];
    else if (!shouldRun && session.isRunning) [session stopRunning];
  });
}

- (void)applyCameraProps:(const nve::apple::CameraPreviewProps &)props {
  _nveDeviceId = props.device_id.has_value() ? [nve::apple::nsstring_from_sv(*props.device_id) copy] : nil;
  _nveFacing = props.facing; _nveTorchMode = props.torch; _nveZoom = props.zoom;
  _nveFps = props.fps; _nveHdr = props.hdr ? YES : NO; _nveStabilization = props.stabilization;
  _nveFrameProcessorEnabled = props.enable_frame_processor ? YES : NO;
  _nveAudioEnabled = props.audio_enabled ? YES : NO; _nveActive = props.active ? YES : NO;
  [self _configureHardwareSession];
  if (!_nveHasHardwareDevice) [self _applyDeviceControls];
}

- (BOOL)dispatchCameraCommand:(const nve::apple::CameraPreviewCommand &)cmd {
  switch (cmd.kind) {
    case nve::apple::CameraPreviewCommandKind::SwitchFacing:
      _nveFacing = cmd.facing; [self _configureHardwareSession]; return YES;
    case nve::apple::CameraPreviewCommandKind::SetTorch:
      _nveTorchMode = cmd.torch; [self _applyDeviceControls]; return YES;
    case nve::apple::CameraPreviewCommandKind::SetZoom:
      _nveZoom = cmd.zoom; [self _applyDeviceControls]; return YES;
    case nve::apple::CameraPreviewCommandKind::FocusPoint:
      _nveLastFocusPoint = CGPointMake(cmd.focus_x, cmd.focus_y);
      if (_videoDevice != nil && _videoDevice.isFocusPointOfInterestSupported) {
        NSError *err = nil;
        if ([_videoDevice lockForConfiguration:&err]) {
          _videoDevice.focusPointOfInterest = _nveLastFocusPoint;
          if ([_videoDevice isFocusModeSupported:AVCaptureFocusModeAutoFocus]) _videoDevice.focusMode = AVCaptureFocusModeAutoFocus;
          [_videoDevice unlockForConfiguration];
        }
      }
      return YES;
    case nve::apple::CameraPreviewCommandKind::CapturePhoto:
      ++_nvePhotoCaptureCount;
      if (_nveHasHardwareDevice && _nveCaptureSession.isRunning) {
        AVCapturePhotoSettings *settings = [AVCapturePhotoSettings photoSettings];
        const AVCaptureFlashMode avFlash = nve::apple::map_av_flash(cmd.flash);
        if ([_photoOutput.supportedFlashModes containsObject:@(avFlash)]) settings.flashMode = avFlash;
        [_photoOutput capturePhotoWithSettings:settings delegate:self];
      }
      return YES;
    case nve::apple::CameraPreviewCommandKind::StartRecording:
      if (cmd.output_path.empty()) return NO;
      _nveLastRecordingPath = [nve::apple::nsstring_from_sv(cmd.output_path) copy];
      _nveIsRecording = YES;
      if (_nveHasHardwareDevice && _nveCaptureSession.isRunning && !_movieOutput.isRecording) {
        [_movieOutput startRecordingToOutputFileURL:[NSURL fileURLWithPath:_nveLastRecordingPath] recordingDelegate:self];
      }
      return YES;
    case nve::apple::CameraPreviewCommandKind::StopRecording:
      _nveIsRecording = NO;
      if (_movieOutput.isRecording) [_movieOutput stopRecording];
      return YES;
  }
  return NO;
}

- (BOOL)emitFrameFromPixelBuffer:(CVPixelBufferRef)pb timestampNs:(uint64_t)ts rotationDeg:(uint16_t)rot {
  if (pb == nullptr) return NO;
  const uint32_t stride = CVPixelBufferIsPlanar(pb)
                              ? static_cast<uint32_t>(CVPixelBufferGetBytesPerRowOfPlane(pb, 0u))
                              : static_cast<uint32_t>(CVPixelBufferGetBytesPerRow(pb));
  nve::apple::CameraFrameDescriptor desc{
      .width = static_cast<uint32_t>(CVPixelBufferGetWidth(pb)),
      .height = static_cast<uint32_t>(CVPixelBufferGetHeight(pb)),
      .stride_bytes = stride,
      .pixel_format = nve::apple::map_cv_pixel_format(CVPixelBufferGetPixelFormatType(pb)),
      .rotation_deg = rot, .timestamp_ns = ts,
      .native_handle_id = static_cast<uint64_t>(reinterpret_cast<uintptr_t>(pb)),
  };
  std::array<uint8_t, nve::apple::kCameraFrameDescriptorSize> wire{};
  if (nve::apple::encode_camera_frame_descriptor(desc, wire) != nve::NativeViewCodecError::Ok) return NO;
  _nveLastFrameDescriptor = [NSData dataWithBytes:wire.data() length:wire.size()];
  ++_nveFrameCount;
  if (_nveFrameCallback != nil) _nveFrameCallback(wire.data(), wire.size());
  return YES;
}

- (void)stopSession {
  _nveActive = NO; _nveIsRecording = NO;
  [_videoDataOutput setSampleBufferDelegate:nil queue:nil];
  if (_movieOutput.isRecording) [_movieOutput stopRecording];
  AVCaptureSession *session = _nveCaptureSession;
  dispatch_async(_sessionQueue, ^{ if (session.isRunning) [session stopRunning]; });
}

- (void)captureOutput:(AVCaptureOutput *)out didOutputSampleBuffer:(CMSampleBufferRef)sb fromConnection:(AVCaptureConnection *)c {
  (void)out; (void)c;
  if (!_nveFrameProcessorEnabled || sb == nullptr) return;
  CVImageBufferRef img = CMSampleBufferGetImageBuffer(sb);
  if (img == nullptr) return;
  const CMTime pts = CMSampleBufferGetPresentationTimeStamp(sb);
  const CMTime ns = CMTIME_IS_NUMERIC(pts) ? CMTimeConvertScale(pts, 1000000000, kCMTimeRoundingMethod_Default) : kCMTimeZero;
  [self emitFrameFromPixelBuffer:img timestampNs:(ns.value > 0 ? static_cast<uint64_t>(ns.value) : 0ull) rotationDeg:0u];
}

- (void)captureOutput:(AVCapturePhotoOutput *)out didFinishProcessingPhoto:(AVCapturePhoto *)photo error:(nullable NSError *)err {
  (void)out;
  if (_nvePhotoCallback != nil) _nvePhotoCallback(err == nil ? [photo fileDataRepresentation] : nil, err);
}

- (void)captureOutput:(AVCaptureFileOutput *)out didFinishRecordingToOutputFileAtURL:(NSURL *)u
      fromConnections:(NSArray<AVCaptureConnection *> *)c error:(nullable NSError *)err {
  (void)out; (void)u; (void)c; (void)err; _nveIsRecording = NO;
}

@end

namespace nve::apple {

namespace {

nve::NativeViewAdapter make_camera_adapter() {
  nve::NativeViewAdapter a{};
  a.create = [](uint32_t node_id, std::string_view, const nve::NvnpDecoder *) -> void * {
    if (id<NveVisionCameraAppleBridge> b = get_vision_camera_apple_bridge(); b != nil) {
      NvePlatformView *v = [b createVisionCameraViewForNodeId:node_id frame:CGRectZero];
      objc_setAssociatedObject(v, &kVisionBridgeAssocKey, b, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
      return (__bridge_retained void *)v;
    }
    return (__bridge_retained void *)[[NveAppleCameraView alloc] initWithFrame:CGRectZero];
  };
  a.update_props = [](void *inst, uint32_t, const nve::NvnpDecoder &dec) {
    CameraPreviewProps props{};
    if (inst == nullptr || decode_camera_preview_props(dec, props) != nve::NativeViewCodecError::Ok) return;
    NvePlatformView *v = (__bridge NvePlatformView *)inst;
    if (id<NveVisionCameraAppleBridge> b = objc_getAssociatedObject(v, &kVisionBridgeAssocKey)) [b updateVisionCameraView:v props:props];
    else if ([v isKindOfClass:[NveAppleCameraView class]]) [(NveAppleCameraView *)v applyCameraProps:props];
  };
  a.dispatch_command = [](void *inst, uint32_t, std::string_view, const nve::NvncDecoder &dec) -> bool {
    CameraPreviewCommand cmd{};
    if (inst == nullptr || decode_camera_preview_command(dec, cmd) != nve::NativeViewCodecError::Ok) return false;
    NvePlatformView *v = (__bridge NvePlatformView *)inst;
    if (id<NveVisionCameraAppleBridge> b = objc_getAssociatedObject(v, &kVisionBridgeAssocKey)) return [b dispatchVisionCameraCommand:v command:cmd] ? true : false;
    return [v isKindOfClass:[NveAppleCameraView class]] && [(NveAppleCameraView *)v dispatchCameraCommand:cmd];
  };
  a.unmount = [](void *inst, uint32_t) {
    if (inst == nullptr) return;
    NvePlatformView *v = (__bridge_transfer NvePlatformView *)inst;
    if (id<NveVisionCameraAppleBridge> b = objc_getAssociatedObject(v, &kVisionBridgeAssocKey)) {
      [b unmountVisionCameraView:v];
      objc_setAssociatedObject(v, &kVisionBridgeAssocKey, nil, OBJC_ASSOCIATION_ASSIGN);
    } else if ([v isKindOfClass:[NveAppleCameraView class]]) {
      [(NveAppleCameraView *)v stopSession];
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
