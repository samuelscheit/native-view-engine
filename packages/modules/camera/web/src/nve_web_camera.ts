/**
 * Zero-dependency Web `getUserMedia` camera session usable by BOTH NVE Web and React Native Web.
 *
 * Matches the VisionCamera v5 / `HybridCameraSession` surface:
 * `listDevices()`, `applyProps(props)`, `dispatchCommand(cmd)`, `takePhoto()`,
 * `startRecording()`, `stopRecording()`, and `dispose()`.
 */

import {
  CameraCodecError,
  CameraPixelFormat,
  TorchMode,
  decodeCameraCommand,
  decodeCameraProps,
  defaultCameraPreviewProps,
  encodeCameraFrame,
  facingToName,
  parseFacing,
  parseStabilizationMode,
  parseTorchMode,
  validateCameraProps,
  type CameraFacingCode,
  type CameraFrameDescriptor,
  type CameraPreviewCommand,
  type CameraPreviewProps,
} from "./nve_camera_wire.ts";
import {
  dataUrlToBytes,
  defaultWebCameraEnv,
  facingToWebFacingMode,
  inferFacingFromLabel,
  parseFlashOption,
  type MediaDevicesLike,
  type MediaRecorderLike,
  type MediaStreamLike,
  type VideoElementLike,
  type WebCameraCommandInput,
  type WebCameraDevice,
  type WebCameraEnvironment,
  type WebCameraPropsInput,
  type WebCameraSessionState,
  type WebCapturedPhoto,
  type WebFrameProcessorCallback,
  type WebRecordedVideo,
  type WebStartRecordingOptions,
  type WebTakePhotoOptions,
} from "./nve_web_camera_env.ts";

export * from "./nve_camera_wire.ts";
export * from "./nve_web_camera_env.ts";

function isBinaryBuffer(value: unknown): value is ArrayBuffer | ArrayBufferView {
  return value instanceof ArrayBuffer || ArrayBuffer.isView(value);
}

function clamp(v: number, lo: number, hi: number): number {
  return Math.min(Math.max(v, lo), hi);
}

interface StreamSignature {
  deviceId: string | null;
  facing: CameraFacingCode;
  fps: number;
  audioEnabled: boolean;
}

export class NveWebCameraSession {
  private readonly env: Required<Pick<WebCameraEnvironment, "nowMs" | "nowNs">> & WebCameraEnvironment;
  private stateValue: WebCameraSessionState = "idle";
  private propsValue: CameraPreviewProps = defaultCameraPreviewProps();
  private stream: MediaStreamLike | null = null;
  private streamSig: StreamSignature | null = null;
  private videoEl: VideoElementLike | null = null;
  private recorder: MediaRecorderLike | null = null;
  private recordedChunks: Array<Blob | Uint8Array> = [];
  private recordingPath = "";
  private recordingStartMs = 0;
  private recordingMimeType = "video/webm";
  private focusPoint: { x: number; y: number } = { x: 0.5, y: 0.5 };
  private frameProcessor: WebFrameProcessorCallback | null = null;
  private lastFrameBytes: Uint8Array | null = null;
  private frameSeq = 0;
  private photoSeq = 0;
  private disposed = false;

  constructor(env: WebCameraEnvironment = {}, videoElement?: VideoElementLike) {
    this.env = { ...defaultWebCameraEnv(), ...env };
    if (videoElement) this.attachVideoElement(videoElement);
  }

  get state(): WebCameraSessionState { return this.stateValue; }
  get isDisposed(): boolean { return this.disposed; }
  get activeProps(): CameraPreviewProps { return { ...this.propsValue }; }
  get frameCount(): number { return this.frameSeq; }
  get lastFocusPoint(): { x: number; y: number } { return { ...this.focusPoint }; }
  getVideoElement(): VideoElementLike | null { return this.videoEl; }

  attachVideoElement(video: VideoElementLike | null): void {
    this.videoEl = video;
    if (video) {
      video.autoplay = true;
      video.playsInline = true;
      video.muted = true;
      if (this.stream) video.srcObject = this.stream;
    }
  }

  setFrameProcessor(callback: WebFrameProcessorCallback | null): void {
    this.frameProcessor = callback;
  }

  lastFrame(): Uint8Array | null {
    return this.lastFrameBytes ? this.lastFrameBytes.slice() : null;
  }

  private assertAlive(): void {
    if (this.disposed) throw new Error("SESSION_DISPOSED");
  }

  private requireMediaDevices(): MediaDevicesLike {
    const md = this.env.mediaDevices;
    if (!md) throw new Error("MEDIA_DEVICES_UNAVAILABLE");
    return md;
  }

  async listDevices(): Promise<WebCameraDevice[]> {
    this.assertAlive();
    const raw = await this.requireMediaDevices().enumerateDevices();
    const activeTrack = this.stream?.getVideoTracks()[0];
    const activeCaps = activeTrack?.getCapabilities?.();
    const activeId = activeTrack?.getSettings?.().deviceId;

    return raw
      .filter((d) => d.kind === "videoinput")
      .map((d, index) => {
        const isCurrent = Boolean(activeId && activeId === d.deviceId);
        const facingCode = inferFacingFromLabel(d.label || "");
        return {
          id: d.deviceId || `web-cam-${index}`,
          name: d.label || `Web Camera ${index + 1}`,
          facing: facingToName(facingCode),
          facingCode,
          hasTorch: Boolean(isCurrent && activeCaps?.torch),
          minZoom: isCurrent && activeCaps?.zoom?.min ? activeCaps.zoom.min : 1.0,
          maxZoom: isCurrent && activeCaps?.zoom?.max ? activeCaps.zoom.max : 4.0,
          maxFps: isCurrent && activeCaps?.frameRate?.max ? Math.min(240, Math.round(activeCaps.frameRate.max)) : 60,
          supportsHdr: false,
          supportedStabilization: ["off"],
        };
      });
  }

  private normalizePropsInput(
    input: CameraPreviewProps | WebCameraPropsInput | ArrayBuffer | ArrayBufferView,
  ): CameraPreviewProps {
    if (isBinaryBuffer(input)) return decodeCameraProps(input);
    const base = this.stateValue === "idle" ? defaultCameraPreviewProps() : this.propsValue;
    const next: CameraPreviewProps = {
      deviceId: input.deviceId !== undefined ? input.deviceId : base.deviceId,
      facing: input.facing !== undefined ? parseFacing(input.facing) : base.facing,
      torch: input.torch !== undefined ? parseTorchMode(input.torch) : base.torch,
      zoom: input.zoom !== undefined ? input.zoom : base.zoom,
      fps: input.fps !== undefined ? input.fps : base.fps,
      hdr: input.hdr !== undefined ? Boolean(input.hdr) : base.hdr,
      stabilization:
        input.stabilization !== undefined ? parseStabilizationMode(input.stabilization) : base.stabilization,
      enableFrameProcessor:
        input.enableFrameProcessor !== undefined ? Boolean(input.enableFrameProcessor) : base.enableFrameProcessor,
      audioEnabled: input.audioEnabled !== undefined ? Boolean(input.audioEnabled) : base.audioEnabled,
      active: input.active !== undefined ? Boolean(input.active) : base.active,
    };
    validateCameraProps(next);
    return next;
  }

  async applyEncodedProps(nvnp: ArrayBuffer | ArrayBufferView): Promise<CameraPreviewProps> {
    return this.applyProps(decodeCameraProps(nvnp));
  }

  async applyProps(
    propsInput: CameraPreviewProps | WebCameraPropsInput | ArrayBuffer | ArrayBufferView,
  ): Promise<CameraPreviewProps> {
    this.assertAlive();
    const next = this.normalizePropsInput(propsInput);

    if (!next.active) {
      this.propsValue = next;
      await this.stopStreamInternal();
      this.stateValue = "stopped";
      return this.activeProps;
    }

    const desiredSig: StreamSignature = {
      deviceId: next.deviceId,
      facing: next.facing,
      fps: next.fps,
      audioEnabled: next.audioEnabled,
    };
    const needsReopen =
      !this.stream ||
      !this.streamSig ||
      this.streamSig.deviceId !== desiredSig.deviceId ||
      this.streamSig.facing !== desiredSig.facing ||
      this.streamSig.fps !== desiredSig.fps ||
      this.streamSig.audioEnabled !== desiredSig.audioEnabled;

    if (needsReopen) await this.openStream(desiredSig);
    this.propsValue = await this.applyTrackControls(next);
    if (this.stateValue !== "recording") this.stateValue = "running";
    return this.activeProps;
  }

  async start(): Promise<boolean> {
    this.assertAlive();
    await this.applyProps({ ...this.propsValue, active: true });
    return this.stateValue === "running" || this.stateValue === "recording";
  }

  async stop(): Promise<void> {
    if (this.disposed) return;
    this.propsValue = { ...this.propsValue, active: false };
    await this.stopStreamInternal();
    this.stateValue = "stopped";
  }

  private async openStream(sig: StreamSignature): Promise<void> {
    const md = this.requireMediaDevices();
    await this.stopStreamInternal();

    const videoConstraints: Record<string, unknown> = { frameRate: { ideal: sig.fps, max: sig.fps } };
    if (sig.deviceId) {
      videoConstraints.deviceId = { exact: sig.deviceId };
    } else {
      const facingMode = facingToWebFacingMode(sig.facing);
      if (facingMode) videoConstraints.facingMode = { ideal: facingMode };
    }

    const stream = await md.getUserMedia({ video: videoConstraints, audio: sig.audioEnabled });
    if (this.disposed) {
      for (const track of stream.getTracks()) track.stop();
      throw new Error("SESSION_DISPOSED");
    }

    this.stream = stream;
    this.streamSig = { ...sig };
    this.stateValue = "configured";

    if (!this.videoEl && this.env.createVideoElement) {
      try { this.videoEl = this.env.createVideoElement(); } catch { /* Headless non-DOM environment */ }
    }
    if (this.videoEl) {
      this.videoEl.autoplay = true;
      this.videoEl.playsInline = true;
      this.videoEl.muted = true;
      this.videoEl.srcObject = stream;
      if (this.videoEl.play) await this.videoEl.play();
    }
  }

  private async applyTrackControls(props: CameraPreviewProps): Promise<CameraPreviewProps> {
    const out: CameraPreviewProps = { ...props };
    const track = this.stream?.getVideoTracks()[0];
    if (!track) return out;

    const caps = track.getCapabilities?.();
    const advanced: Record<string, unknown> = {};
    let hasAdvanced = false;
    if (caps?.zoom) {
      const minZ = typeof caps.zoom.min === "number" ? caps.zoom.min : 1.0;
      const maxZ = typeof caps.zoom.max === "number" ? caps.zoom.max : Math.max(minZ, out.zoom);
      out.zoom = clamp(out.zoom, minZ, maxZ);
      advanced.zoom = out.zoom;
      hasAdvanced = true;
    }
    if (caps && "torch" in caps) {
      if (caps.torch) {
        advanced.torch = out.torch !== TorchMode.Off;
        hasAdvanced = true;
      } else {
        out.torch = TorchMode.Off;
      }
    }
    if (hasAdvanced && track.applyConstraints) {
      await track.applyConstraints({ advanced: [advanced] });
    }
    return out;
  }

  async dispatchCommand(cmdInput: WebCameraCommandInput | ArrayBuffer | ArrayBufferView): Promise<unknown> {
    this.assertAlive();
    const cmd: CameraPreviewCommand = isBinaryBuffer(cmdInput)
      ? decodeCameraCommand(cmdInput)
      : this.normalizeCommand(cmdInput);

    switch (cmd.kind) {
      case "switch_facing":
        return this.applyProps({ ...this.propsValue, deviceId: null, facing: cmd.facing, active: true });
      case "set_torch":
        return this.applyProps({ ...this.propsValue, torch: cmd.torch });
      case "set_zoom":
        return this.applyProps({ ...this.propsValue, zoom: cmd.zoom });
      case "focus_point": {
        if (!Number.isFinite(cmd.x) || cmd.x < 0 || cmd.x > 1 || !Number.isFinite(cmd.y) || cmd.y < 0 || cmd.y > 1) {
          throw new CameraCodecError("TYPE_MISMATCH", "focus_point coordinates must be in 0.0..=1.0");
        }
        this.focusPoint = { x: cmd.x, y: cmd.y };
        const track = this.stream?.getVideoTracks()[0];
        if (track?.applyConstraints) {
          try { await track.applyConstraints({ advanced: [{ pointsOfInterest: [{ x: cmd.x, y: cmd.y }] }] }); }
          catch { /* Ignore unsupported browser constraint */ }
        }
        return true;
      }
      case "capture_photo":
        return this.takePhoto({ quality: cmd.quality, flash: cmd.flash });
      case "start_recording":
        await this.startRecording(cmd.outputPath);
        return true;
      case "stop_recording":
        return this.stopRecording();
    }
  }

  private normalizeCommand(cmd: WebCameraCommandInput): CameraPreviewCommand {
    switch (cmd.kind) {
      case "switch_facing": return { kind: "switch_facing", facing: parseFacing(cmd.facing) };
      case "set_torch": return { kind: "set_torch", torch: parseTorchMode(cmd.torch) };
      case "set_zoom":
        if (!Number.isFinite(cmd.zoom) || cmd.zoom <= 0) throw new CameraCodecError("TYPE_MISMATCH", "zoom");
        return { kind: "set_zoom", zoom: cmd.zoom };
      default: return cmd;
    }
  }

  async takePhoto(options: WebTakePhotoOptions = {}): Promise<WebCapturedPhoto> {
    this.assertAlive();
    const quality = options.quality ?? 90;
    const flashMode = parseFlashOption(options.flash);
    if (!Number.isInteger(quality) || quality < 0 || quality > 100) {
      throw new CameraCodecError("INVALID_ENUM", "INVALID_CAPTURE_OPTIONS");
    }
    if (!this.stream) await this.applyProps({ ...this.propsValue, active: true });

    const video = this.videoEl;
    const trackSettings = this.stream?.getVideoTracks()[0]?.getSettings?.();
    const width = video?.videoWidth || trackSettings?.width || 1280;
    const height = video?.videoHeight || trackSettings?.height || 720;
    const mimeType = options.mimeType ?? "image/jpeg";
    const qualityUnit = quality / 100;

    if (!this.env.createCanvas) throw new Error("NO_CANVAS_PROVIDER");
    const canvas = this.env.createCanvas(width, height);
    canvas.width = width;
    canvas.height = height;
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("CANVAS_2D_CONTEXT_UNAVAILABLE");
    if (video) ctx.drawImage(video, 0, 0, width, height);

    let blob: Blob | undefined;
    let bytes: Uint8Array;
    if (canvas.convertToBlob) {
      blob = await canvas.convertToBlob({ type: mimeType, quality: qualityUnit });
      bytes = new Uint8Array(await blob.arrayBuffer());
    } else if (canvas.toBlob) {
      blob = await new Promise<Blob>((resolve, reject) => {
        canvas.toBlob!((b) => (b ? resolve(b) : reject(new Error("CANVAS_TO_BLOB_FAILED"))), mimeType, qualityUnit);
      });
      bytes = new Uint8Array(await blob.arrayBuffer());
    } else if (canvas.toDataURL) {
      bytes = dataUrlToBytes(canvas.toDataURL(mimeType, qualityUnit));
    } else {
      throw new Error("CANVAS_EXPORT_UNSUPPORTED");
    }

    const seq = ++this.photoSeq;
    return {
      filePath: `memory://nve-web-photo-${seq}.jpg`,
      width,
      height,
      quality,
      flashMode,
      timestampNs: this.env.nowNs(),
      mimeType,
      bytes,
      blob,
    };
  }

  async startRecording(outputPathOrOptions: string | WebStartRecordingOptions = ""): Promise<void> {
    this.assertAlive();
    const opts: WebStartRecordingOptions =
      typeof outputPathOrOptions === "string" ? { outputPath: outputPathOrOptions } : outputPathOrOptions;
    const outputPath = opts.outputPath ?? "";
    if (!outputPath) throw new Error("EMPTY_OUTPUT_PATH");
    if (this.stateValue === "recording" || this.recorder) throw new Error("ALREADY_RECORDING");
    if (!this.stream) await this.applyProps({ ...this.propsValue, active: true });
    if (!this.stream || !this.env.createMediaRecorder) throw new Error("MEDIA_RECORDER_UNAVAILABLE");

    const recorder = this.env.createMediaRecorder(
      this.stream,
      opts.mimeType ? { mimeType: opts.mimeType } : undefined,
    );
    this.recordedChunks = [];
    this.recordingPath = outputPath;
    this.recordingStartMs = this.env.nowMs();
    this.recordingMimeType = opts.mimeType || recorder.mimeType || "video/webm";
    recorder.ondataavailable = (ev) => { if (ev.data) this.recordedChunks.push(ev.data); };
    this.recorder = recorder;
    recorder.start();
    this.stateValue = "recording";
  }

  async stopRecording(): Promise<WebRecordedVideo> {
    this.assertAlive();
    if (this.stateValue !== "recording" || !this.recorder) throw new Error("NOT_RECORDING");
    const recorder = this.recorder;
    await new Promise<void>((resolve) => {
      const prevOnStop = recorder.onstop;
      recorder.onstop = () => { prevOnStop?.(); resolve(); };
      if (recorder.state !== "inactive") recorder.stop();
      else resolve();
    });

    const trackSettings = this.stream?.getVideoTracks()[0]?.getSettings?.();
    const width = this.videoEl?.videoWidth || trackSettings?.width || 1280;
    const height = this.videoEl?.videoHeight || trackSettings?.height || 720;
    const durationMs = Math.max(0, this.env.nowMs() - this.recordingStartMs);
    const hasAudio = Boolean(this.stream && this.stream.getAudioTracks().length > 0);
    const mimeType = recorder.mimeType || this.recordingMimeType;

    let byteLength = 0;
    for (const chunk of this.recordedChunks) {
      byteLength += chunk instanceof Uint8Array ? chunk.byteLength : chunk.size;
    }
    const blob = typeof Blob === "function" ? new Blob(this.recordedChunks as BlobPart[], { type: mimeType }) : undefined;
    const result: WebRecordedVideo = {
      outputPath: this.recordingPath,
      durationMs,
      width,
      height,
      hasAudio,
      mimeType,
      byteLength,
      blob,
    };

    this.recorder = null;
    this.recordedChunks = [];
    this.recordingPath = "";
    this.stateValue = this.stream ? "running" : "stopped";
    return result;
  }

  emitFrame(nativeHandleId = 0n): Uint8Array | null {
    if (this.disposed || !this.stream) return null;
    const settings = this.stream.getVideoTracks()[0]?.getSettings?.();
    const width = this.videoEl?.videoWidth || settings?.width || 1280;
    const height = this.videoEl?.videoHeight || settings?.height || 720;
    const desc: CameraFrameDescriptor = {
      width,
      height,
      strideBytes: width * 4,
      pixelFormat: CameraPixelFormat.Rgba8888,
      rotationDeg: 0,
      timestampNs: this.env.nowNs(),
      nativeHandleId,
    };
    const nvcf = encodeCameraFrame(desc);
    this.frameSeq += 1;
    this.lastFrameBytes = nvcf;
    if (this.propsValue.enableFrameProcessor && this.frameProcessor) {
      this.frameProcessor(desc, nvcf);
    }
    return nvcf;
  }

  private async stopStreamInternal(): Promise<void> {
    if (this.recorder) {
      try { if (this.recorder.state !== "inactive") this.recorder.stop(); } catch { /* Best-effort */ }
      this.recorder = null;
      this.recordedChunks = [];
    }
    if (this.stream) {
      for (const track of this.stream.getTracks()) track.stop();
      this.stream = null;
      this.streamSig = null;
    }
    if (this.videoEl) {
      this.videoEl.pause?.();
      this.videoEl.srcObject = null;
    }
  }

  async dispose(): Promise<void> {
    if (this.disposed) return;
    this.disposed = true;
    this.frameProcessor = null;
    await this.stopStreamInternal();
    this.stateValue = "released";
  }
}

export function createWebCameraSession(
  env?: WebCameraEnvironment,
  videoElement?: VideoElementLike,
): NveWebCameraSession {
  return new NveWebCameraSession(env, videoElement);
}
