/**
 * `@nve/camera` — Standalone React Native / RN-Windows (0.88) / RN-macOS / RN-Web camera package.
 *
 * Zero duplicated camera logic:
 * - Native (iOS / macOS / Android / Windows): routes directly to the C++20 JSI `HostObject` /
 *   Nitro `HybridObjectRegistry` (`global.__nveCameraInstall` / `global.__nveCamera`) with zero-JSON
 *   binary `NVNP`/`NVNC`/`NVCF` buffers.
 * - Web / React Native Web: routes directly to `../../web/src/nve_web_camera.ts`.
 */

import {
  CAMERA_FRAME_DESCRIPTOR_SIZE,
  CAMERA_PREVIEW_KIND,
  CameraCodecError,
  CameraCommandOp,
  CameraFacing,
  CameraPixelFormat,
  CameraPropField,
  NveWebCameraSession,
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
  type VideoElementLike,
  type WebCameraCommandInput,
  type WebCameraEnvironment,
  type WebCameraPropsInput,
  type WebCameraSessionState,
  type WebStartRecordingOptions,
  type WebTakePhotoOptions,
} from "../../web/src/nve_web_camera.ts";

export {
  CAMERA_FRAME_DESCRIPTOR_SIZE,
  CAMERA_PREVIEW_KIND,
  CameraCodecError,
  CameraCommandOp,
  CameraFacing,
  CameraPixelFormat,
  CameraPropField,
  NveWebCameraSession,
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
  type WebCameraEnvironment,
  type WebCameraSessionState,
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

export interface CameraProps extends WebCameraPropsInput {
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
  applyProps(props: CameraPreviewProps | WebCameraPropsInput | ArrayBuffer): boolean;
  applyEncodedProps(nvnp: ArrayBuffer): boolean;
  dispatchCommand(cmd: WebCameraCommandInput | ArrayBuffer): boolean;
  dispatchEncodedCommand?(nvnc: ArrayBuffer): boolean;
  start(): boolean;
  stop(): void;
  takePhoto(options?: WebTakePhotoOptions): CapturedPhoto | Promise<CapturedPhoto>;
  startRecording(outputPathOrOptions: string | WebStartRecordingOptions): boolean | Promise<void>;
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
  navigator?: { mediaDevices?: unknown };
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
  input: CameraProps | CameraPreviewProps | WebCameraPropsInput,
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

export interface CameraSessionOptions {
  /** Force `'native'` (JSI/Nitro) or `'web'` (`getUserMedia`); defaults to `'auto'`. */
  backend?: "auto" | "native" | "web";
  webEnv?: WebCameraEnvironment;
  videoElement?: VideoElementLike;
}

/**
 * Unified VisionCamera v5-ergonomic `CameraSession` wrapping either the zero-JSON native JSI
 * `HybridCameraSession` host object (iOS / macOS / Android / Windows) or `NveWebCameraSession`
 * on Web / React Native Web.
 */
export class CameraSession {
  readonly backend: "native" | "web";
  private readonly nativeHost: NativeJsiCameraSessionHost | null;
  private readonly webSession: NveWebCameraSession | null;
  private currentProps: CameraPreviewProps = defaultCameraPreviewProps();

  constructor(options: CameraSessionOptions = {}) {
    const mode = options.backend ?? "auto";
    if (mode !== "web") {
      const mod = resolveNativeJsiModule();
      const host = mod ? mod.createSession() : resolveNitroSession();
      if (host) {
        this.backend = "native";
        this.nativeHost = host;
        this.webSession = null;
        return;
      }
      if (mode === "native") {
        throw new Error(
          "NVE_CAMERA_NATIVE_UNAVAILABLE: global.__nveCameraInstall / NitroModules CameraSession not installed",
        );
      }
    }
    this.backend = "web";
    this.nativeHost = null;
    this.webSession = new NveWebCameraSession(options.webEnv, options.videoElement);
  }

  get isDisposed(): boolean {
    return this.nativeHost ? this.nativeHost.isDisposed : this.webSession!.isDisposed;
  }

  get activeProps(): CameraPreviewProps {
    return this.webSession ? this.webSession.activeProps : { ...this.currentProps };
  }

  get web(): NveWebCameraSession | null {
    return this.webSession;
  }

  async listDevices(): Promise<CameraDevice[]> {
    if (this.nativeHost) {
      return this.nativeHost.listDevices();
    }
    return this.webSession!.listDevices();
  }

  async applyProps(
    props: CameraProps | CameraPreviewProps | WebCameraPropsInput | ArrayBuffer | ArrayBufferView,
  ): Promise<CameraPreviewProps> {
    if (props instanceof ArrayBuffer || ArrayBuffer.isView(props)) {
      const decoded = decodeCameraProps(props);
      if (this.nativeHost) {
        const ab =
          props instanceof ArrayBuffer
            ? props
            : toArrayBufferCopy(new Uint8Array(props.buffer, props.byteOffset, props.byteLength));
        const ok = this.nativeHost.applyEncodedProps(ab);
        if (!ok) throw new Error("NVNP_APPLY_FAILED");
        this.currentProps = decoded;
        return { ...decoded };
      }
      return this.webSession!.applyEncodedProps(props);
    }

    const normalized = normalizeCameraProps(props, this.currentProps);
    if (this.nativeHost) {
      const nvnp = encodeCameraProps(normalized);
      const ok = this.nativeHost.applyEncodedProps(toArrayBufferCopy(nvnp));
      if (!ok) throw new Error("NVNP_APPLY_FAILED");
      this.currentProps = normalized;
      return { ...normalized };
    }
    const applied = await this.webSession!.applyProps(normalized);
    this.currentProps = applied;
    return applied;
  }

  async dispatchCommand(
    cmd: WebCameraCommandInput | ArrayBuffer | ArrayBufferView,
  ): Promise<unknown> {
    if (this.nativeHost) {
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
    return this.webSession!.dispatchCommand(cmd);
  }

  async start(): Promise<boolean> {
    return this.nativeHost ? this.nativeHost.start() : this.webSession!.start();
  }

  async stop(): Promise<void> {
    if (this.nativeHost) {
      this.nativeHost.stop();
      return;
    }
    await this.webSession!.stop();
  }

  async takePhoto(options?: WebTakePhotoOptions): Promise<CapturedPhoto> {
    if (this.nativeHost) {
      return await this.nativeHost.takePhoto(options);
    }
    return this.webSession!.takePhoto(options);
  }

  async startRecording(outputPathOrOptions: string | WebStartRecordingOptions): Promise<void> {
    if (this.nativeHost) {
      await this.nativeHost.startRecording(outputPathOrOptions);
      return;
    }
    await this.webSession!.startRecording(outputPathOrOptions);
  }

  async stopRecording(): Promise<RecordedVideo> {
    if (this.nativeHost) {
      return await this.nativeHost.stopRecording();
    }
    return this.webSession!.stopRecording();
  }

  lastFrame(): CameraFrameDescriptor | null {
    const raw = this.nativeHost ? this.nativeHost.lastFrame() : this.webSession!.lastFrame();
    return raw ? decodeCameraFrame(raw) : null;
  }

  async dispose(): Promise<void> {
    if (this.nativeHost) {
      this.nativeHost.dispose();
      return;
    }
    await this.webSession!.dispose();
  }
}

export function createCameraSession(options?: CameraSessionOptions): CameraSession {
  return new CameraSession(options);
}

export async function getAvailableCameraDevices(
  options?: CameraSessionOptions,
): Promise<CameraDevice[]> {
  const nativeMod = options?.backend === "web" ? null : resolveNativeJsiModule();
  if (nativeMod) {
    return nativeMod.listDevices();
  }
  const session = new CameraSession(options);
  try {
    return await session.listDevices();
  } finally {
    await session.dispose();
  }
}

/**
 * VisionCamera v5-style device selector helper & hook.
 * Pass `devices` directly for pure/SSR usage, or call inside React with a preloaded device array.
 */
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

/**
 * Creates and manages a `CameraSession` lifecycle (auto-applies props and disposes on unmount
 * when given React hooks; also usable directly without React).
 */
export function useCameraSession(
  props?: CameraProps,
  options?: CameraSessionOptions,
  react?: ReactLikeHooks,
): CameraSession {
  if (!react) {
    const session = new CameraSession(options);
    if (props) {
      void session.applyProps(props);
    }
    return session;
  }
  const ref = react.useRef<CameraSession | null>(null);
  if (!ref.current) {
    ref.current = new CameraSession(options);
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
 * Cross-platform `<Camera />` element descriptor / React component adapter.
 * On Native hosts, encodes declarative props into zero-JSON binary `NVNP` bytes for
 * `"NveCameraPreview"`; on Web, binds a `<video autoplay playsInline muted>` element to
 * `NveWebCameraSession`.
 */
export function Camera(
  props: CameraProps & { session?: CameraSession; react?: ReactLikeHooks },
): unknown {
  const normalized = normalizeCameraProps(props, defaultCameraPreviewProps());
  const nvnpProps = encodeCameraProps(normalized);
  const isNative = Boolean(resolveNativeJsiModule() || resolveNitroSession());
  const elementType = isNative ? "NveCameraPreview" : "video";
  const elementProps: Record<string, unknown> = isNative
    ? {
        nativeViewKind: CAMERA_PREVIEW_KIND,
        nvnpProps,
        style: props.style,
      }
    : {
        autoPlay: true,
        playsInline: true,
        muted: true,
        style: props.style,
        ref: (el: VideoElementLike | null) => {
          props.session?.web?.attachVideoElement(el);
        },
      };

  if (props.react?.createElement) {
    return props.react.createElement(elementType, elementProps);
  }
  return { type: elementType, props: elementProps };
}
