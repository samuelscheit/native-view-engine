/**
 * Browser / DOM abstraction interfaces and helpers for `NveWebCameraSession`.
 * Allows zero-dependency execution in browsers AND deterministic mocking in Node tests.
 */

import {
  CameraCodecError,
  CameraFacing,
  type CameraFacingCode,
  type CameraFacingName,
  type CameraFrameDescriptor,
  type CameraPreviewCommand,
  type StabilizationModeCode,
  type StabilizationModeName,
  type TorchModeCode,
  type TorchModeName,
} from "./nve_camera_types.ts";

export type WebCameraSessionState =
  | "idle"
  | "configured"
  | "running"
  | "recording"
  | "stopped"
  | "released";

export interface WebCameraDevice {
  id: string;
  name: string;
  facing: CameraFacingName;
  facingCode: CameraFacingCode;
  hasTorch: boolean;
  minZoom: number;
  maxZoom: number;
  maxFps: number;
  supportsHdr: boolean;
  supportedStabilization: StabilizationModeName[];
}

export interface WebCameraPropsInput {
  deviceId?: string | null;
  facing?: CameraFacingCode | CameraFacingName;
  torch?: TorchModeCode | TorchModeName;
  zoom?: number;
  fps?: number;
  hdr?: boolean;
  stabilization?: StabilizationModeCode | StabilizationModeName;
  enableFrameProcessor?: boolean;
  audioEnabled?: boolean;
  active?: boolean;
}

export type WebCameraCommandInput =
  | CameraPreviewCommand
  | { kind: "switch_facing"; facing: CameraFacingCode | CameraFacingName }
  | { kind: "set_torch"; torch: TorchModeCode | TorchModeName };

export interface WebTakePhotoOptions {
  quality?: number;
  flash?: number | "off" | "on" | "auto";
  mimeType?: string;
}

export interface WebCapturedPhoto {
  filePath: string;
  width: number;
  height: number;
  quality: number;
  flashMode: number;
  timestampNs: bigint;
  mimeType: string;
  bytes: Uint8Array;
  blob?: Blob;
}

export interface WebStartRecordingOptions {
  outputPath?: string;
  mimeType?: string;
}

export interface WebRecordedVideo {
  outputPath: string;
  durationMs: number;
  width: number;
  height: number;
  hasAudio: boolean;
  mimeType: string;
  byteLength: number;
  blob?: Blob;
}

export type WebFrameProcessorCallback = (descriptor: CameraFrameDescriptor, nvcfBytes: Uint8Array) => void;

export interface MediaTrackLike {
  kind: string;
  id?: string;
  label?: string;
  readyState?: string;
  stop(): void;
  getCapabilities?: () => {
    torch?: boolean;
    zoom?: { min?: number; max?: number; step?: number };
    facingMode?: string[];
    frameRate?: { max?: number };
  };
  getSettings?: () => {
    deviceId?: string;
    width?: number;
    height?: number;
    frameRate?: number;
    facingMode?: string;
  };
  applyConstraints?: (constraints: Record<string, unknown>) => Promise<void> | void;
}

export interface MediaStreamLike {
  id?: string;
  getTracks(): MediaTrackLike[];
  getVideoTracks(): MediaTrackLike[];
  getAudioTracks(): MediaTrackLike[];
}

export interface MediaDeviceInfoLike {
  deviceId: string;
  kind: string;
  label: string;
  groupId?: string;
}

export interface MediaDevicesLike {
  enumerateDevices(): Promise<MediaDeviceInfoLike[]>;
  getUserMedia(constraints: Record<string, unknown>): Promise<MediaStreamLike>;
}

export interface VideoElementLike {
  autoplay: boolean;
  playsInline: boolean;
  muted: boolean;
  srcObject: MediaStreamLike | null;
  videoWidth: number;
  videoHeight: number;
  play?: () => Promise<void> | void;
  pause?: () => void;
}

export interface CanvasContext2DLike {
  drawImage(source: VideoElementLike, dx: number, dy: number, dw: number, dh: number): void;
}

export interface CanvasElementLike {
  width: number;
  height: number;
  getContext(type: "2d"): CanvasContext2DLike | null;
  toBlob?: (cb: (blob: Blob | null) => void, type?: string, quality?: number) => void;
  convertToBlob?: (options?: { type?: string; quality?: number }) => Promise<Blob>;
  toDataURL?: (type?: string, quality?: number) => string;
}

export interface MediaRecorderLike {
  readonly state: string;
  readonly mimeType?: string;
  ondataavailable: ((event: { data: Blob | Uint8Array | null }) => void) | null;
  onstop: (() => void) | null;
  onerror: ((event: unknown) => void) | null;
  start(timeslice?: number): void;
  stop(): void;
}

export interface WebCameraEnvironment {
  mediaDevices?: MediaDevicesLike;
  createVideoElement?: () => VideoElementLike;
  createCanvas?: (width: number, height: number) => CanvasElementLike;
  createMediaRecorder?: (stream: MediaStreamLike, options?: { mimeType?: string }) => MediaRecorderLike;
  nowMs?: () => number;
  nowNs?: () => bigint;
}

export function inferFacingFromLabel(label: string): CameraFacingCode {
  const lower = label.toLowerCase();
  if (lower.includes("front") || lower.includes("facetime") || lower.includes("user") || lower.includes("selfie")) {
    return CameraFacing.Front;
  }
  if (lower.includes("usb") || lower.includes("external") || lower.includes("hdmi") || lower.includes("capture")) {
    return CameraFacing.External;
  }
  return CameraFacing.Back;
}

export function facingToWebFacingMode(facing: CameraFacingCode): "user" | "environment" | undefined {
  if (facing === CameraFacing.Front) return "user";
  if (facing === CameraFacing.Back) return "environment";
  return undefined;
}

export function parseFlashOption(flash: number | "off" | "on" | "auto" | undefined): number {
  if (flash === undefined || flash === "off" || flash === 0) return 0;
  if (flash === "on" || flash === 1) return 1;
  if (flash === "auto" || flash === 2) return 2;
  throw new CameraCodecError("INVALID_ENUM", `Invalid flash mode: ${String(flash)}`);
}

export function dataUrlToBytes(dataUrl: string): Uint8Array {
  const comma = dataUrl.indexOf(",");
  if (comma < 0) return new Uint8Array(0);
  const payload = dataUrl.slice(comma + 1);
  if (typeof globalThis.atob === "function") {
    const raw = globalThis.atob(payload);
    const out = new Uint8Array(raw.length);
    for (let i = 0; i < raw.length; i++) out[i] = raw.charCodeAt(i);
    return out;
  }
  return new TextEncoder().encode(payload);
}

export function defaultWebCameraEnv(): Required<Pick<WebCameraEnvironment, "nowMs" | "nowNs">> & WebCameraEnvironment {
  return {
    mediaDevices: (globalThis as { navigator?: { mediaDevices?: MediaDevicesLike } }).navigator?.mediaDevices,
    createVideoElement: () => {
      const doc = (globalThis as { document?: { createElement(tag: string): unknown } }).document;
      if (!doc) throw new Error("NO_DOM_DOCUMENT");
      return doc.createElement("video") as VideoElementLike;
    },
    createCanvas: (width: number, height: number) => {
      const Offscreen = (globalThis as { OffscreenCanvas?: new (w: number, h: number) => CanvasElementLike })
        .OffscreenCanvas;
      if (typeof Offscreen === "function") return new Offscreen(width, height);
      const doc = (globalThis as { document?: { createElement(tag: string): unknown } }).document;
      if (!doc) throw new Error("NO_CANVAS_PROVIDER");
      const canvas = doc.createElement("canvas") as CanvasElementLike;
      canvas.width = width;
      canvas.height = height;
      return canvas;
    },
    createMediaRecorder: (stream, options) => {
      const RecorderCtor = (
        globalThis as { MediaRecorder?: new (s: MediaStreamLike, o?: { mimeType?: string }) => MediaRecorderLike }
      ).MediaRecorder;
      if (typeof RecorderCtor !== "function") throw new Error("MEDIA_RECORDER_UNAVAILABLE");
      return options?.mimeType ? new RecorderCtor(stream, options) : new RecorderCtor(stream);
    },
    nowMs: () => Date.now(),
    nowNs: () => BigInt(Date.now()) * 1_000_000n,
  };
}
