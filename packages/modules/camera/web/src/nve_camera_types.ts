/**
 * Shared types, wire constants, enum parsers, and validators aligned 1:1 with
 * `packages/modules/camera/src/{spec,wire}.rs` and `packages/modules/camera/cxx/include/nve_camera_codec.hpp`.
 */

export const CAMERA_PREVIEW_KIND = "nve.camera.preview" as const;

export const NATIVE_PROPS_MAGIC = [0x4e, 0x56, 0x4e, 0x50] as const; // "NVNP"
export const NATIVE_PROPS_VERSION = 1;
export const NATIVE_PROPS_HEADER_SIZE = 7;
export const MAX_NATIVE_PROP_FIELDS = 1024;

export const NATIVE_CMD_MAGIC = [0x4e, 0x56, 0x4e, 0x43] as const; // "NVNC"
export const NATIVE_CMD_VERSION = 1;
export const NATIVE_CMD_HEADER_SIZE = 8;
export const MAX_NATIVE_CMD_ARGS = 64;

export const CAMERA_FRAME_MAGIC = [0x4e, 0x56, 0x43, 0x46] as const; // "NVCF"
export const CAMERA_FRAME_VERSION = 1;
export const CAMERA_FRAME_DESCRIPTOR_SIZE = 40;
export const MAX_CAMERA_FPS = 240;

export const NativeValueTag = {
  Null: 0,
  Bool: 1,
  U32: 2,
  I64: 3,
  U64: 4,
  F32: 5,
  F64: 6,
  Str: 7,
  Bytes: 8,
} as const;
export type NativeValueTagCode = (typeof NativeValueTag)[keyof typeof NativeValueTag];

export const CameraFacing = { Back: 1, Front: 2, External: 3 } as const;
export type CameraFacingCode = (typeof CameraFacing)[keyof typeof CameraFacing];
export type CameraFacingName = "back" | "front" | "external";

export const TorchMode = { Off: 0, On: 1, Auto: 2 } as const;
export type TorchModeCode = (typeof TorchMode)[keyof typeof TorchMode];
export type TorchModeName = "off" | "on" | "auto";

export const StabilizationMode = { Off: 0, Standard: 1, Cinematic: 2 } as const;
export type StabilizationModeCode = (typeof StabilizationMode)[keyof typeof StabilizationMode];
export type StabilizationModeName = "off" | "standard" | "cinematic";

export const CameraPixelFormat = { Rgba8888: 0, Bgra8888: 1, Nv12: 2, Yuv420_888: 3 } as const;
export type CameraPixelFormatCode = (typeof CameraPixelFormat)[keyof typeof CameraPixelFormat];

export const CameraPropField = {
  DeviceId: 1,
  Facing: 2,
  Torch: 3,
  Zoom: 4,
  Fps: 5,
  Hdr: 6,
  Stabilization: 7,
  EnableFrameProcessor: 8,
  AudioEnabled: 9,
  Active: 10,
} as const;

export const CameraCommandOp = {
  SwitchFacing: 1,
  SetTorch: 2,
  SetZoom: 3,
  FocusPoint: 4,
  CapturePhoto: 5,
  StartRecording: 6,
  StopRecording: 7,
} as const;
export type CameraCommandOpCode = (typeof CameraCommandOp)[keyof typeof CameraCommandOp];

export interface CameraPreviewProps {
  deviceId: string | null;
  facing: CameraFacingCode;
  torch: TorchModeCode;
  zoom: number;
  fps: number;
  hdr: boolean;
  stabilization: StabilizationModeCode;
  enableFrameProcessor: boolean;
  audioEnabled: boolean;
  active: boolean;
}

export type CameraPreviewCommand =
  | { kind: "switch_facing"; facing: CameraFacingCode }
  | { kind: "set_torch"; torch: TorchModeCode }
  | { kind: "set_zoom"; zoom: number }
  | { kind: "focus_point"; x: number; y: number }
  | { kind: "capture_photo"; quality: number; flash: number }
  | { kind: "start_recording"; outputPath: string }
  | { kind: "stop_recording" };

export interface CameraFrameDescriptor {
  width: number;
  height: number;
  strideBytes: number;
  pixelFormat: CameraPixelFormatCode;
  rotationDeg: 0 | 90 | 180 | 270;
  timestampNs: bigint;
  nativeHandleId: bigint;
}

export type CameraCodecErrorCode =
  | "TRUNCATED"
  | "INVALID_MAGIC"
  | "UNSUPPORTED_VERSION"
  | "INVALID_TAG"
  | "INVALID_BOOL"
  | "INVALID_ENUM"
  | "INVALID_UTF8"
  | "MISSING_FIELD"
  | "MISSING_ARG"
  | "UNEXPECTED_ARG_COUNT"
  | "TYPE_MISMATCH"
  | "TRAILING_BYTES"
  | "UNKNOWN_COMMAND_OPCODE"
  | "TOO_MANY_FIELDS"
  | "PAYLOAD_TOO_LARGE";

export class CameraCodecError extends Error {
  readonly code: CameraCodecErrorCode;

  constructor(code: CameraCodecErrorCode, message: string) {
    super(`${code}: ${message}`);
    this.name = "CameraCodecError";
    this.code = code;
  }
}

export function defaultCameraPreviewProps(): CameraPreviewProps {
  return {
    deviceId: null,
    facing: CameraFacing.Back,
    torch: TorchMode.Off,
    zoom: 1.0,
    fps: 30,
    hdr: false,
    stabilization: StabilizationMode.Off,
    enableFrameProcessor: false,
    audioEnabled: false,
    active: true,
  };
}

export function toUint8Array(input: ArrayBuffer | ArrayBufferView): Uint8Array {
  if (input instanceof Uint8Array) return input;
  if (ArrayBuffer.isView(input)) return new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
  if (input instanceof ArrayBuffer) return new Uint8Array(input);
  throw new CameraCodecError("TYPE_MISMATCH", "Expected ArrayBuffer or ArrayBufferView");
}

export function parseFacing(value: CameraFacingCode | CameraFacingName | number): CameraFacingCode {
  if (value === 1 || value === "back") return CameraFacing.Back;
  if (value === 2 || value === "front") return CameraFacing.Front;
  if (value === 3 || value === "external") return CameraFacing.External;
  throw new CameraCodecError("INVALID_ENUM", `Invalid CameraFacing: ${String(value)}`);
}

export function facingToName(facing: CameraFacingCode): CameraFacingName {
  if (facing === CameraFacing.Front) return "front";
  if (facing === CameraFacing.External) return "external";
  return "back";
}

export function parseTorchMode(value: TorchModeCode | TorchModeName | number): TorchModeCode {
  if (value === 0 || value === "off") return TorchMode.Off;
  if (value === 1 || value === "on") return TorchMode.On;
  if (value === 2 || value === "auto") return TorchMode.Auto;
  throw new CameraCodecError("INVALID_ENUM", `Invalid TorchMode: ${String(value)}`);
}

export function parseStabilizationMode(
  value: StabilizationModeCode | StabilizationModeName | number,
): StabilizationModeCode {
  if (value === 0 || value === "off") return StabilizationMode.Off;
  if (value === 1 || value === "standard") return StabilizationMode.Standard;
  if (value === 2 || value === "cinematic") return StabilizationMode.Cinematic;
  throw new CameraCodecError("INVALID_ENUM", `Invalid StabilizationMode: ${String(value)}`);
}

export function parsePixelFormat(value: number): CameraPixelFormatCode {
  if (value === 0 || value === 1 || value === 2 || value === 3) return value;
  throw new CameraCodecError("INVALID_ENUM", `Invalid CameraPixelFormat: ${String(value)}`);
}

export function validateZoom(zoom: number): void {
  if (typeof zoom !== "number" || !Number.isFinite(zoom) || zoom <= 0) {
    throw new CameraCodecError("TYPE_MISMATCH", "zoom must be a finite positive float");
  }
}

export function validateFps(fps: number): number {
  if (!Number.isInteger(fps) || fps < 1 || fps > MAX_CAMERA_FPS) {
    throw new CameraCodecError("INVALID_ENUM", `fps must be in 1..=${MAX_CAMERA_FPS}, got ${fps}`);
  }
  return fps;
}

export function validateUnitCoord(value: number, label: string): void {
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0 || value > 1) {
    throw new CameraCodecError("TYPE_MISMATCH", `${label} must be finite in 0.0..=1.0`);
  }
}

export function validateQuality(q: number): number {
  if (!Number.isInteger(q) || q < 0 || q > 100) {
    throw new CameraCodecError("INVALID_ENUM", `photo quality must be in 0..=100, got ${q}`);
  }
  return q;
}

export function validateFlash(f: number): number {
  if (!Number.isInteger(f) || f < 0 || f > 2) {
    throw new CameraCodecError("INVALID_ENUM", `flash mode must be in 0..=2, got ${f}`);
  }
  return f;
}

export function validateCameraProps(props: CameraPreviewProps): void {
  parseFacing(props.facing);
  parseTorchMode(props.torch);
  validateZoom(props.zoom);
  validateFps(props.fps);
  parseStabilizationMode(props.stabilization);
}

export function validateCameraFrame(desc: CameraFrameDescriptor): void {
  if (!Number.isInteger(desc.width) || desc.width <= 0 || desc.width > 0xffffffff) {
    throw new CameraCodecError("TYPE_MISMATCH", "Frame width must be positive u32");
  }
  if (!Number.isInteger(desc.height) || desc.height <= 0 || desc.height > 0xffffffff) {
    throw new CameraCodecError("TYPE_MISMATCH", "Frame height must be positive u32");
  }
  parsePixelFormat(desc.pixelFormat);
  if (desc.rotationDeg !== 0 && desc.rotationDeg !== 90 && desc.rotationDeg !== 180 && desc.rotationDeg !== 270) {
    throw new CameraCodecError("INVALID_ENUM", `Invalid rotationDeg: ${desc.rotationDeg}`);
  }
  const bpp =
    desc.pixelFormat === CameraPixelFormat.Rgba8888 || desc.pixelFormat === CameraPixelFormat.Bgra8888 ? 4 : 1;
  const minStride = desc.width * bpp;
  if (minStride > 0xffffffff || !Number.isInteger(desc.strideBytes) || desc.strideBytes < minStride) {
    throw new CameraCodecError("TYPE_MISMATCH", `strideBytes must be >= ${minStride}`);
  }
}
