import test from "node:test";
import assert from "node:assert/strict";

import {
  CAMERA_FRAME_DESCRIPTOR_SIZE,
  CameraCodecError,
  CameraFacing,
  CameraPixelFormat,
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
} from "../src/nve_web_camera.ts";

function createMockWebEnv() {
  const createdTracks = [];
  const getUserMediaCalls = [];
  const appliedConstraints = [];
  const canvasDrawCalls = [];
  let clockMs = 10_000;
  let streamSeq = 0;

  const devices = [
    { deviceId: "mic-1", kind: "audioinput", label: "Built-in Mic" },
    { deviceId: "cam-back", kind: "videoinput", label: "Back Wide Camera" },
    { deviceId: "cam-front", kind: "videoinput", label: "FaceTime HD Front Camera" },
    { deviceId: "cam-usb", kind: "videoinput", label: "USB Capture HDMI" },
  ];

  const makeTrack = (kind, facingMode, hasTorch) => {
    const track = {
      kind,
      id: `${kind}-${createdTracks.length + 1}`,
      stopped: false,
      stop() {
        this.stopped = true;
      },
      getCapabilities() {
        return {
          torch: hasTorch,
          zoom: { min: 1.0, max: hasTorch ? 8.0 : 4.0 },
          facingMode: [facingMode],
          frameRate: { max: 60 },
        };
      },
      getSettings() {
        return {
          deviceId: facingMode === "user" ? "cam-front" : "cam-back",
          width: 1920,
          height: 1080,
          frameRate: 30,
          facingMode,
        };
      },
      async applyConstraints(c) {
        appliedConstraints.push({ trackId: this.id, constraints: c });
      },
    };
    createdTracks.push(track);
    return track;
  };

  const videoEl = {
    autoplay: false,
    playsInline: false,
    muted: false,
    srcObject: null,
    videoWidth: 1920,
    videoHeight: 1080,
    playCalls: 0,
    pauseCalls: 0,
    async play() {
      this.playCalls += 1;
    },
    pause() {
      this.pauseCalls += 1;
    },
  };

  const recorders = [];

  const env = {
    mediaDevices: {
      async enumerateDevices() {
        return devices;
      },
      async getUserMedia(constraints) {
        getUserMediaCalls.push(constraints);
        streamSeq += 1;
        const videoSpec = constraints.video || {};
        const facing =
          videoSpec.facingMode?.ideal === "user" || videoSpec.deviceId?.exact === "cam-front"
            ? "user"
            : "environment";
        const hasTorch = facing === "environment";
        const vTrack = makeTrack("video", facing, hasTorch);
        const aTracks = constraints.audio ? [makeTrack("audio", facing, false)] : [];
        return {
          id: `stream-${streamSeq}`,
          getTracks: () => [vTrack, ...aTracks],
          getVideoTracks: () => [vTrack],
          getAudioTracks: () => aTracks,
        };
      },
    },
    createVideoElement: () => videoEl,
    createCanvas: (width, height) => ({
      width,
      height,
      getContext(type) {
        assert.equal(type, "2d");
        return {
          drawImage(src, dx, dy, dw, dh) {
            canvasDrawCalls.push({ src, dx, dy, dw, dh });
          },
        };
      },
      async convertToBlob({ type, quality }) {
        const header = new Uint8Array([0xff, 0xd8, 0xff, Math.round((quality ?? 0.9) * 100)]);
        return new Blob([header], { type: type ?? "image/jpeg" });
      },
    }),
    createMediaRecorder: (_stream, options) => {
      const rec = {
        state: "inactive",
        mimeType: options?.mimeType ?? "video/webm;codecs=vp9",
        ondataavailable: null,
        onstop: null,
        onerror: null,
        start() {
          this.state = "recording";
          this.ondataavailable?.({ data: new Uint8Array([1, 2, 3, 4, 5]) });
        },
        stop() {
          if (this.state === "inactive") return;
          this.state = "inactive";
          this.ondataavailable?.({ data: new Uint8Array([6, 7, 8]) });
          this.onstop?.();
        },
      };
      recorders.push(rec);
      return rec;
    },
    nowMs: () => clockMs,
    nowNs: () => BigInt(clockMs) * 1_000_000n,
  };

  return {
    env,
    videoEl,
    createdTracks,
    getUserMediaCalls,
    appliedConstraints,
    canvasDrawCalls,
    recorders,
    advanceMs: (delta) => {
      clockMs += delta;
    },
  };
}

test("NVNP / NVNC / NVCF golden byte parity with Rust (spec.rs) and C++ (camera_test.cpp)", () => {
  // Golden default CameraPreviewProps NVNP bytes (matches camera_test.cpp lines 51-63)
  const expectedDefaultNvnp = new Uint8Array([
    0x4e, 0x56, 0x4e, 0x50, 1, 10, 0,
    1, 0, 0,
    2, 0, 2, 1, 0, 0, 0,
    3, 0, 2, 0, 0, 0, 0,
    4, 0, 5, 0x00, 0x00, 0x80, 0x3f,
    5, 0, 2, 30, 0, 0, 0,
    6, 0, 1, 0,
    7, 0, 2, 0, 0, 0, 0,
    8, 0, 1, 0,
    9, 0, 1, 0,
    10, 0, 1, 1,
  ]);

  const encodedProps = encodeCameraProps(defaultCameraPreviewProps());
  assert.deepEqual(encodedProps, expectedDefaultNvnp);
  assert.deepEqual(decodeCameraProps(expectedDefaultNvnp), defaultCameraPreviewProps());

  // Golden NVNC FocusPoint(0.25, 0.75) bytes (matches camera_test.cpp lines 72-76)
  const expectedFocusNvnc = new Uint8Array([
    0x4e, 0x56, 0x4e, 0x43, 1, 4, 0, 2,
    5, 0x00, 0x00, 0x80, 0x3e,
    5, 0x00, 0x00, 0x40, 0x3f,
  ]);
  const encodedFocus = encodeCameraCommand({ kind: "focus_point", x: 0.25, y: 0.75 });
  assert.deepEqual(encodedFocus, expectedFocusNvnc);
  assert.deepEqual(decodeCameraCommand(expectedFocusNvnc), { kind: "focus_point", x: 0.25, y: 0.75 });

  // Golden 40-byte NVCF frame descriptor (matches camera_test.cpp lines 86-94)
  const expectedNvcf = new Uint8Array([
    0x4e, 0x56, 0x43, 0x46, 1, 3, 0x0e, 0x01,
    0x80, 0x07, 0x00, 0x00,
    0x38, 0x04, 0x00, 0x00,
    0x80, 0x07, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00,
    0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01,
    0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11,
  ]);
  const frameDesc = {
    width: 1920,
    height: 1080,
    strideBytes: 1920,
    pixelFormat: CameraPixelFormat.Yuv420_888,
    rotationDeg: 270,
    timestampNs: 0x0102030405060708n,
    nativeHandleId: 0x1122334455667788n,
  };
  const encodedFrame = encodeCameraFrame(frameDesc);
  assert.equal(encodedFrame.byteLength, CAMERA_FRAME_DESCRIPTOR_SIZE);
  assert.deepEqual(encodedFrame, expectedNvcf);
  assert.deepEqual(decodeCameraFrame(expectedNvcf), frameDesc);
});

test("Corrupt, truncated, trailing, NaN, and fuzz buffers are rejected cleanly", () => {
  // Invalid props
  assert.throws(
    () => encodeCameraProps({ ...defaultCameraPreviewProps(), zoom: Number.NaN }),
    (err) => err instanceof CameraCodecError && err.code === "TYPE_MISMATCH",
  );
  assert.throws(
    () => encodeCameraProps({ ...defaultCameraPreviewProps(), zoom: -2 }),
    (err) => err instanceof CameraCodecError && err.code === "TYPE_MISMATCH",
  );
  assert.throws(
    () => encodeCameraProps({ ...defaultCameraPreviewProps(), fps: 241 }),
    (err) => err instanceof CameraCodecError && err.code === "INVALID_ENUM",
  );

  // Invalid commands
  assert.throws(
    () => encodeCameraCommand({ kind: "focus_point", x: 1.5, y: 0.5 }),
    (err) => err instanceof CameraCodecError && err.code === "TYPE_MISMATCH",
  );
  assert.throws(
    () => encodeCameraCommand({ kind: "capture_photo", quality: 101, flash: 0 }),
    (err) => err instanceof CameraCodecError && err.code === "INVALID_ENUM",
  );
  assert.throws(
    () => encodeCameraCommand({ kind: "capture_photo", quality: 80, flash: 3 }),
    (err) => err instanceof CameraCodecError && err.code === "INVALID_ENUM",
  );

  // Bad magic, bad version, trailing byte, invalid UTF-8
  assert.throws(
    () => decodeCameraProps(new Uint8Array([0x00, 0x00, 0x00, 0x00, 1, 0, 0])),
    (err) => err instanceof CameraCodecError && err.code === "INVALID_MAGIC",
  );
  assert.throws(
    () => decodeCameraProps(new Uint8Array([0x4e, 0x56, 0x4e, 0x50, 99, 0, 0])),
    (err) => err instanceof CameraCodecError && err.code === "UNSUPPORTED_VERSION",
  );

  const validProps = encodeCameraProps({
    ...defaultCameraPreviewProps(),
    deviceId: "cam-back",
    stabilization: StabilizationMode.Cinematic,
  });
  const withTrailing = new Uint8Array(validProps.length + 1);
  withTrailing.set(validProps);
  withTrailing[validProps.length] = 0xaa;
  assert.throws(
    () => decodeCameraProps(withTrailing),
    (err) => err instanceof CameraCodecError && err.code === "TRAILING_BYTES",
  );

  // Corrupt UTF-8 in StartRecording outputPath
  const badUtf8Cmd = new Uint8Array([
    0x4e, 0x56, 0x4e, 0x43, 1, 6, 0, 1,
    7, 2, 0, 0, 0, 0xff, 0xfe,
  ]);
  assert.throws(
    () => decodeCameraCommand(badUtf8Cmd),
    (err) => err instanceof CameraCodecError && err.code === "INVALID_UTF8",
  );

  // Truncated and malformed NVCF
  const validNvcf = encodeCameraFrame({
    width: 1280,
    height: 720,
    strideBytes: 1280 * 4,
    pixelFormat: CameraPixelFormat.Rgba8888,
    rotationDeg: 90,
    timestampNs: 100n,
    nativeHandleId: 200n,
  });
  for (let cut = 0; cut < CAMERA_FRAME_DESCRIPTOR_SIZE; cut++) {
    assert.throws(
      () => decodeCameraFrame(validNvcf.subarray(0, cut)),
      (err) => err instanceof CameraCodecError && err.code === "TRUNCATED",
    );
  }
  const badRot = validNvcf.slice();
  badRot[6] = 45;
  assert.throws(
    () => decodeCameraFrame(badRot),
    (err) => err instanceof CameraCodecError && err.code === "INVALID_ENUM",
  );
  const badReserved = validNvcf.slice();
  badReserved[20] = 1;
  assert.throws(
    () => decodeCameraFrame(badReserved),
    (err) => err instanceof CameraCodecError && err.code === "INVALID_ENUM",
  );

  // Deterministic 300-case fuzz loop must only throw CameraCodecError, never crash
  let state = 0xcafebabe12345678n;
  const nextU8 = () => {
    state = (state * 6364136223846793005n + 1442695040888963407n) & 0xffffffffffffffffn;
    return Number((state >> 33n) & 0xffn);
  };
  for (let i = 0; i < 300; i++) {
    const len = (i * 13) % 64;
    const fuzz = new Uint8Array(len);
    for (let j = 0; j < len; j++) fuzz[j] = nextU8();
    if (len >= 8) {
      const magic = i % 3 === 0 ? [0x4e, 0x56, 0x4e, 0x50] : i % 3 === 1 ? [0x4e, 0x56, 0x4e, 0x43] : [0x4e, 0x56, 0x43, 0x46];
      fuzz.set(magic, 0);
      fuzz[4] = 1;
    }
    for (const fn of [decodeCameraProps, decodeCameraCommand, decodeCameraFrame]) {
      try {
        fn(fuzz);
      } catch (err) {
        assert.ok(err instanceof CameraCodecError);
      }
    }
  }
});

test("NveWebCameraSession: listDevices, NVNP applyProps, diff-aware controls, facing switch, photo, recording, and dispose cleanup", async () => {
  const mock = createMockWebEnv();
  const session = new NveWebCameraSession(mock.env);

  // 1. listDevices filters audioinput and infers facing
  const devices = await session.listDevices();
  assert.equal(devices.length, 3);
  assert.equal(devices[0].facing, "back");
  assert.equal(devices[1].facing, "front");
  assert.equal(devices[2].facing, "external");

  // 2. Apply binary NVNP props (Front facing, zoom 10 -> clamped to 4, torch On -> clamped Off on front)
  const nvnp = encodeCameraProps({
    deviceId: null,
    facing: CameraFacing.Front,
    torch: TorchMode.On,
    zoom: 10.0,
    fps: 60,
    hdr: false,
    stabilization: StabilizationMode.Off,
    enableFrameProcessor: true,
    audioEnabled: true,
    active: true,
  });
  const applied = await session.applyProps(nvnp);
  assert.equal(session.state, "running");
  assert.equal(applied.facing, CameraFacing.Front);
  assert.equal(applied.zoom, 4.0);
  assert.equal(applied.torch, TorchMode.Off);
  assert.equal(mock.getUserMediaCalls.length, 1);
  assert.equal(mock.videoEl.autoplay, true);
  assert.equal(mock.videoEl.playsInline, true);
  assert.equal(mock.videoEl.muted, true);
  assert.equal(mock.videoEl.playCalls, 1);
  assert.ok(mock.videoEl.srcObject !== null);

  // 3. Diff-aware zoom update via JS options must NOT reopen getUserMedia
  const zoomed = await session.applyProps({ zoom: 2.5 });
  assert.equal(zoomed.zoom, 2.5);
  assert.equal(mock.getUserMediaCalls.length, 1);
  assert.ok(mock.appliedConstraints.some((c) => c.constraints.advanced?.[0]?.zoom === 2.5));

  // 4. Switch facing via binary NVNC command -> stops old tracks, opens back camera with torch
  const switchCmd = encodeCameraCommand({ kind: "switch_facing", facing: CameraFacing.Back });
  await session.dispatchCommand(switchCmd);
  assert.equal(mock.getUserMediaCalls.length, 2);
  assert.equal(mock.createdTracks[0].stopped, true); // old front video track stopped
  assert.equal(mock.createdTracks[1].stopped, true); // old front audio track stopped
  assert.equal(session.activeProps.facing, CameraFacing.Back);

  // Torch On succeeds on Back camera
  await session.dispatchCommand(encodeCameraCommand({ kind: "set_torch", torch: TorchMode.On }));
  assert.equal(session.activeProps.torch, TorchMode.On);
  assert.ok(mock.appliedConstraints.some((c) => c.constraints.advanced?.[0]?.torch === true));

  // Focus point via NVNC
  await session.dispatchCommand(encodeCameraCommand({ kind: "focus_point", x: 0.25, y: 0.75 }));
  assert.deepEqual(session.lastFocusPoint, { x: 0.25, y: 0.75 });

  // 5. Frame processor emits 40-byte NVCF
  const receivedFrames = [];
  session.setFrameProcessor((desc, nvcf) => {
    receivedFrames.push({ desc, parsed: decodeCameraFrame(nvcf) });
  });
  const emitted = session.emitFrame(0xfeedface01n);
  assert.equal(emitted.byteLength, 40);
  assert.equal(session.frameCount, 1);
  assert.equal(receivedFrames.length, 1);
  assert.equal(receivedFrames[0].parsed.nativeHandleId, 0xfeedface01n);
  assert.deepEqual(session.lastFrame(), emitted);

  // 6. Photo capture via takePhoto & NVNC CapturePhoto
  const photo = await session.takePhoto({ quality: 92, flash: "on" });
  assert.equal(photo.width, 1920);
  assert.equal(photo.height, 1080);
  assert.equal(photo.quality, 92);
  assert.equal(photo.flashMode, 1);
  assert.equal(photo.bytes[0], 0xff);
  assert.equal(photo.bytes[1], 0xd8);
  assert.equal(mock.canvasDrawCalls.length, 1);

  const cmdPhoto = await session.dispatchCommand(
    encodeCameraCommand({ kind: "capture_photo", quality: 85, flash: 2 }),
  );
  assert.equal(cmdPhoto.quality, 85);
  assert.equal(cmdPhoto.flashMode, 2);

  await assert.rejects(() => session.takePhoto({ quality: 101 }), CameraCodecError);

  // 7. Recording lifecycle (startRecording -> advance clock -> stopRecording)
  await assert.rejects(() => session.startRecording(""), /EMPTY_OUTPUT_PATH/);
  await assert.rejects(() => session.stopRecording(), /NOT_RECORDING/);

  await session.dispatchCommand(
    encodeCameraCommand({ kind: "start_recording", outputPath: "clip-1.webm" }),
  );
  assert.equal(session.state, "recording");
  await assert.rejects(() => session.startRecording("clip-2.webm"), /ALREADY_RECORDING/);

  mock.advanceMs(1500);
  const video = await session.dispatchCommand(encodeCameraCommand({ kind: "stop_recording" }));
  assert.equal(session.state, "running");
  assert.equal(video.outputPath, "clip-1.webm");
  assert.equal(video.durationMs, 1500);
  assert.equal(video.hasAudio, true);
  assert.equal(video.byteLength, 8);

  // 8. Dispose stops every remaining MediaStreamTrack, clears video.srcObject, and is idempotent
  await session.startRecording("clip-active-on-dispose.webm");
  assert.equal(session.state, "recording");
  await session.dispose();
  assert.equal(session.isDisposed, true);
  assert.equal(session.state, "released");
  assert.equal(mock.videoEl.srcObject, null);
  assert.ok(mock.createdTracks.every((t) => t.stopped));
  assert.equal(mock.recorders.at(-1).state, "inactive");

  // Idempotent double dispose
  await session.dispose();
  assert.equal(session.state, "released");

  // Subsequent calls reject after dispose
  await assert.rejects(() => session.listDevices(), /SESSION_DISPOSED/);
  await assert.rejects(() => session.applyProps({ zoom: 2 }), /SESSION_DISPOSED/);
  await assert.rejects(() => session.takePhoto(), /SESSION_DISPOSED/);
});
