/**
 * `@nve/camera` — Optional React Native VisionCamera v5 adapter and typed `CameraPreview`
 * binary wire codec (`NVNP`/`NVNC`/`NVCF`) for NVE.
 *
 * Routes directly to the installed native C++20 JSI `HostObject` / Nitro `HybridObjectRegistry`
 * (`global.__nveCameraInstall` / `global.__nveCamera` / `NitroModules.createHybridObject("CameraSession")`).
 */

import {
  CAMERA_FRAME_DESCRIPTOR_SIZE,
  CAMERA_PREVIEW_KIND,
  CameraCodecError,
  CameraCommandOp,
  CameraFacing,
  CameraPixelFormat,
  CameraPropField,
  StabilizationMode,
  TorchMode,
  decodeCameraCommand,
  decodeCameraFrame,
  decodeCameraProps,
  defaultCameraPreviewProps,
  encodeCameraCommand,
  encodeCameraFrame,
  encodeCameraProps,
  facingToName,
  parseFacing,
  parseStabilizationMode,
  parseTorchMode,
  validateCameraProps,
  type CameraFacingCode,
  type CameraFacingName,
  type CameraFrameDescriptor,
  type CameraPixelFormatCode,
  type CameraPreviewCommand,
  type CameraPreviewProps,
  type StabilizationModeCode,
  type StabilizationModeName,
  type TorchModeCode,
  type TorchModeName,
} from "./nve_camera_wire.ts";

export {
  CAMERA_FRAME_DESCRIPTOR_SIZE,
  CAMERA_PREVIEW_KIND,
  CameraCodecError,
  CameraCommandOp,
  CameraFacing,
  CameraPixelFormat,
  CameraPropField,
  StabilizationMode,
  TorchMode,
  decodeCameraCommand,
  decodeCameraFrame,
  decodeCameraProps,
  defaultCameraPreviewProps,
  encodeCameraCommand,
  encodeCameraFrame,
  encodeCameraProps,
  facingToName,
  parseFacing,
  parseStabilizationMode,
  parseTorchMode,
  validateCameraProps,
  type CameraFacingCode,
  type CameraFacingName,
  type CameraFrameDescriptor,
  type CameraPixelFormatCode,
  type CameraPreviewCommand,
  type CameraPreviewProps,
  type StabilizationModeCode,
  type StabilizationModeName,
  type TorchModeCode,
  type TorchModeName,
};

export interface CameraDevice {
  id: string;
  name: string;
  facing: CameraFacingName;
  hasTorch: boolean;
  minZoom: number;
  maxZoom: number;
  maxFps: number;
  supportsHdr: boolean;
  supportedStabilization: StabilizationModeName[];
}

export interface CapturedPhoto {
  filePath: string;
  width: number;
  height: number;
  quality: number;
  flashMode: number;
  timestampNs: bigint | number;
  mimeType?: string;
  bytes?: Uint8Array;
}

export interface RecordedVideo {
  outputPath: string;
  durationMs: number;
  width: number;
  height: number;
  hasAudio: boolean;
  mimeType?: string;
  byteLength?: number;
}

export interface CameraPropsInput {
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

export interface TakePhotoOptions {
  quality?: number;
  flash?: number | "off" | "on" | "auto";
}

export interface StartRecordingOptions {
  outputPath: string;
}

export type CameraCommandInput =
  | { kind: "switch_facing"; facing: CameraFacingCode | CameraFacingName }
  | { kind: "set_torch"; torch: TorchModeCode | TorchModeName }
  | { kind: "set_zoom"; zoom: number }
  | { kind: "focus_point"; x: number; y: number }
  | { kind: "capture_photo"; quality: number; flash: number }
  | { kind: "start_recording"; outputPath: string }
  | { kind: "stop_recording" };

export interface CameraProps extends CameraPropsInput {
  device?: CameraDevice | null;
  isActive?: boolean;
  photo?: boolean;
  video?: boolean;
  audio?: boolean;
  frameProcessor?: (frame: CameraFrameDescriptor, nvcfBytes: Uint8Array) => void;
  style?: Record<string, unknown>;
  onError?: (error: Error) => void;
}

export interface NativeJsiCameraSessionHost {
  readonly state: number;
  readonly frameCount: number;
  readonly isDisposed: boolean;
  listDevices(): CameraDevice[];
  applyProps(props: CameraPreviewProps | CameraPropsInput | ArrayBuffer): boolean;
  applyEncodedProps(nvnp: ArrayBuffer): boolean;
  dispatchCommand(cmd: CameraCommandInput | ArrayBuffer): boolean;
  dispatchEncodedCommand?(nvnc: ArrayBuffer): boolean;
  start(): boolean;
  stop(): void;
  takePhoto(options?: TakePhotoOptions): CapturedPhoto | Promise<CapturedPhoto>;
  startRecording(outputPathOrOptions: string | StartRecordingOptions): boolean | Promise<void>;
  stopRecording(): RecordedVideo | Promise<RecordedVideo>;
  lastFrame(): ArrayBuffer | null;
  dispose(): void;
}

export interface NativeJsiCameraModuleHost {
  readonly isAvailable?: boolean;
  createSession(): NativeJsiCameraSessionHost;
  listDevices(): CameraDevice[];
}

interface NveCameraGlobalScope {
  __nveCameraInstall?: () => NativeJsiCameraModuleHost | null | undefined;
  __nveCamera?: NativeJsiCameraModuleHost;
  NitroModules?: {
    createHybridObject?: (name: string) => NativeJsiCameraSessionHost | null | undefined;
  };
}

function getGlobalScope(): NveCameraGlobalScope {
  return globalThis as unknown as NveCameraGlobalScope;
}

function resolveNativeJsiModule(): NativeJsiCameraModuleHost | null {
  const g = getGlobalScope();
  if (g.__nveCamera) return g.__nveCamera;
  if (typeof g.__nveCameraInstall === "function") {
    const installed = g.__nveCameraInstall();
    if (installed) {
      g.__nveCamera = installed;
      return installed;
    }
  }
  return null;
}

function resolveNitroSession(): NativeJsiCameraSessionHost | null {
  const g = getGlobalScope();
  if (typeof g.NitroModules?.createHybridObject === "function") {
    return g.NitroModules.createHybridObject("CameraSession") ?? null;
  }
  return null;
}

function toArrayBufferCopy(bytes: Uint8Array): ArrayBuffer {
  const copy = new Uint8Array(bytes.byteLength);
  copy.set(bytes);
  return copy.buffer;
}

function normalizeCameraProps(
  input: CameraProps | CameraPreviewProps | CameraPropsInput,
  base: CameraPreviewProps,
): CameraPreviewProps {
  const raw = input as CameraProps;
  const active =
    raw.isActive !== undefined
      ? Boolean(raw.isActive)
      : raw.active !== undefined
        ? Boolean(raw.active)
        : base.active;
  const audioEnabled =
    raw.audio !== undefined
      ? Boolean(raw.audio)
      : raw.audioEnabled !== undefined
        ? Boolean(raw.audioEnabled)
        : base.audioEnabled;
  const deviceId =
    raw.device !== undefined
      ? raw.device?.id ?? null
      : raw.deviceId !== undefined
        ? raw.deviceId
        : base.deviceId;
  const facing =
    raw.facing !== undefined
      ? parseFacing(raw.facing)
      : raw.device?.facing !== undefined
        ? parseFacing(raw.device.facing)
        : base.facing;
  const next: CameraPreviewProps = {
    deviceId,
    facing,
    torch: raw.torch !== undefined ? parseTorchMode(raw.torch) : base.torch,
    zoom: raw.zoom !== undefined ? raw.zoom : base.zoom,
    fps: raw.fps !== undefined ? raw.fps : base.fps,
    hdr: raw.hdr !== undefined ? Boolean(raw.hdr) : base.hdr,
    stabilization:
      raw.stabilization !== undefined
        ? parseStabilizationMode(raw.stabilization)
        : base.stabilization,
    enableFrameProcessor:
      raw.enableFrameProcessor !== undefined
        ? Boolean(raw.enableFrameProcessor)
        : Boolean(raw.frameProcessor) || base.enableFrameProcessor,
    audioEnabled,
    active,
  };
  validateCameraProps(next);
  return next;
}

/**
 * VisionCamera v5 adapter session wrapping the installed native JSI/Nitro `HybridCameraSession`.
 */
export class CameraSession {
  readonly backend = "native" as const;
  private readonly nativeHost: NativeJsiCameraSessionHost;
  private currentProps: CameraPreviewProps = defaultCameraPreviewProps();

  constructor() {
    const mod = resolveNativeJsiModule();
    const host = mod ? mod.createSession() : resolveNitroSession();
    if (!host) {
      throw new Error(
        "NVE_CAMERA_NATIVE_UNAVAILABLE: global.__nveCameraInstall / NitroModules CameraSession not installed",
      );
    }
    this.nativeHost = host;
  }

  get isDisposed(): boolean {
    return this.nativeHost.isDisposed;
  }

  get activeProps(): CameraPreviewProps {
    return { ...this.currentProps };
  }

  async listDevices(): Promise<CameraDevice[]> {
    return this.nativeHost.listDevices();
  }

  async applyProps(
    props: CameraProps | CameraPreviewProps | CameraPropsInput | ArrayBuffer | ArrayBufferView,
  ): Promise<CameraPreviewProps> {
    if (props instanceof ArrayBuffer || ArrayBuffer.isView(props)) {
      const decoded = decodeCameraProps(props);
      const ab =
        props instanceof ArrayBuffer
          ? props
          : toArrayBufferCopy(new Uint8Array(props.buffer, props.byteOffset, props.byteLength));
      const ok = this.nativeHost.applyEncodedProps(ab);
      if (!ok) throw new Error("NVNP_APPLY_FAILED");
      this.currentProps = decoded;
      return { ...decoded };
    }

    const normalized = normalizeCameraProps(props, this.currentProps);
    const nvnp = encodeCameraProps(normalized);
    const ok = this.nativeHost.applyEncodedProps(toArrayBufferCopy(nvnp));
    if (!ok) throw new Error("NVNP_APPLY_FAILED");
    this.currentProps = normalized;
    return { ...normalized };
  }

  async dispatchCommand(cmd: CameraCommandInput | ArrayBuffer | ArrayBufferView): Promise<boolean> {
    if (cmd instanceof ArrayBuffer || ArrayBuffer.isView(cmd)) {
      const ab =
        cmd instanceof ArrayBuffer
          ? cmd
          : toArrayBufferCopy(new Uint8Array(cmd.buffer, cmd.byteOffset, cmd.byteLength));
      const ok = this.nativeHost.dispatchEncodedCommand
        ? this.nativeHost.dispatchEncodedCommand(ab)
        : this.nativeHost.dispatchCommand(ab);
      if (!ok) throw new Error("NVNC_DISPATCH_FAILED");
      return true;
    }
    const normalized: CameraPreviewCommand =
      cmd.kind === "switch_facing"
        ? { kind: "switch_facing", facing: parseFacing(cmd.facing) }
        : cmd.kind === "set_torch"
          ? { kind: "set_torch", torch: parseTorchMode(cmd.torch) }
          : cmd;
    const nvnc = encodeCameraCommand(normalized);
    const ok = this.nativeHost.dispatchEncodedCommand
      ? this.nativeHost.dispatchEncodedCommand(toArrayBufferCopy(nvnc))
      : this.nativeHost.dispatchCommand(toArrayBufferCopy(nvnc));
    if (!ok) throw new Error("NVNC_DISPATCH_FAILED");
    return true;
  }

  async start(): Promise<boolean> {
    return this.nativeHost.start();
  }

  async stop(): Promise<void> {
    this.nativeHost.stop();
  }

  async takePhoto(options?: TakePhotoOptions): Promise<CapturedPhoto> {
    return await this.nativeHost.takePhoto(options);
  }

  async startRecording(outputPathOrOptions: string | StartRecordingOptions): Promise<void> {
    await this.nativeHost.startRecording(outputPathOrOptions);
  }

  async stopRecording(): Promise<RecordedVideo> {
    return await this.nativeHost.stopRecording();
  }

  lastFrame(): CameraFrameDescriptor | null {
    const raw = this.nativeHost.lastFrame();
    return raw ? decodeCameraFrame(raw) : null;
  }

  async dispose(): Promise<void> {
    this.nativeHost.dispose();
  }
}

export function createCameraSession(): CameraSession {
  return new CameraSession();
}

export async function getAvailableCameraDevices(): Promise<CameraDevice[]> {
  const nativeMod = resolveNativeJsiModule();
  if (nativeMod) {
    return nativeMod.listDevices();
  }
  const session = new CameraSession();
  try {
    return await session.listDevices();
  } finally {
    await session.dispose();
  }
}

export function useCameraDevice(
  position: CameraFacingName | CameraFacingCode,
  devices: readonly CameraDevice[] = [],
): CameraDevice | undefined {
  const target = facingToName(parseFacing(position));
  return devices.find((d) => d.facing === target) ?? devices[0];
}

export interface ReactLikeHooks {
  useRef<T>(initial: T): { current: T };
  useEffect(effect: () => void | (() => void), deps?: readonly unknown[]): void;
  createElement?(
    type: string,
    props: Record<string, unknown> | null,
    ...children: unknown[]
  ): unknown;
}

export function useCameraSession(
  props?: CameraProps,
  react?: ReactLikeHooks,
): CameraSession {
  if (!react) {
    const session = new CameraSession();
    if (props) {
      void session.applyProps(props);
    }
    return session;
  }
  const ref = react.useRef<CameraSession | null>(null);
  if (!ref.current) {
    ref.current = new CameraSession();
  }
  const session = ref.current;
  react.useEffect(() => {
    if (props) {
      session.applyProps(props).catch((err) => {
        props.onError?.(err instanceof Error ? err : new Error(String(err)));
      });
    }
  }, [
    props?.device?.id,
    props?.deviceId,
    props?.facing,
    props?.torch,
    props?.zoom,
    props?.fps,
    props?.hdr,
    props?.stabilization,
    props?.enableFrameProcessor,
    props?.audio,
    props?.audioEnabled,
    props?.isActive,
    props?.active,
  ]);
  react.useEffect(() => {
    return () => {
      void session.dispose();
    };
  }, [session]);
  return session;
}

/**
 * React Native `<Camera />` element descriptor encoding declarative props into binary `NVNP` bytes
 * for `"NveCameraPreview"` backed by VisionCamera v5.
 */
export function Camera(
  props: CameraProps & { react?: ReactLikeHooks },
): unknown {
  const normalized = normalizeCameraProps(props, defaultCameraPreviewProps());
  const nvnpProps = encodeCameraProps(normalized);
  const elementProps: Record<string, unknown> = {
    nativeViewKind: CAMERA_PREVIEW_KIND,
    nvnpProps,
    style: props.style,
  };
  if (props.react?.createElement) {
    return props.react.createElement("NveCameraPreview", elementProps);
  }
  return { type: "NveCameraPreview", props: elementProps };
}
