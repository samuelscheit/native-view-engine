import test from "node:test";
import assert from "node:assert/strict";

import {
  Camera,
  CameraFacing,
  CameraPixelFormat,
  CameraSession,
  TorchMode,
  decodeCameraCommand,
  decodeCameraProps,
  encodeCameraFrame,
  getAvailableCameraDevices,
  useCameraDevice,
} from "../src/index.ts";

test("@nve/camera routes to global.__nveCameraInstall native JSI host with binary NVNP/NVNC/NVCF buffers", async () => {
  const receivedNvnp = [];
  const receivedNvnc = [];
  let disposed = false;

  const sampleNvcf = encodeCameraFrame({
    width: 1920,
    height: 1080,
    strideBytes: 1920,
    pixelFormat: CameraPixelFormat.Nv12,
    rotationDeg: 90,
    timestampNs: 4242n,
    nativeHandleId: 9999n,
  });

  const mockDevices = [
    {
      id: "ios-back-0",
      name: "Wide Back",
      facing: "back",
      hasTorch: true,
      minZoom: 0.5,
      maxZoom: 8.0,
      maxFps: 60,
      supportsHdr: true,
      supportedStabilization: ["off", "standard", "cinematic"],
    },
    {
      id: "ios-front-1",
      name: "TrueDepth Front",
      facing: "front",
      hasTorch: false,
      minZoom: 1.0,
      maxZoom: 4.0,
      maxFps: 60,
      supportsHdr: true,
      supportedStabilization: ["off", "standard"],
    },
  ];

  const prevInstall = globalThis.__nveCameraInstall;
  const prevMod = globalThis.__nveCamera;
  try {
    globalThis.__nveCamera = undefined;
    globalThis.__nveCameraInstall = () => ({
      isAvailable: true,
      listDevices: () => mockDevices,
      createSession: () => ({
        state: 2,
        frameCount: 1,
        get isDisposed() {
          return disposed;
        },
        listDevices: () => mockDevices,
        applyProps: () => true,
        applyEncodedProps(ab) {
          assert.ok(ab instanceof ArrayBuffer);
          receivedNvnp.push(decodeCameraProps(ab));
          return true;
        },
        dispatchCommand(ab) {
          assert.ok(ab instanceof ArrayBuffer);
          receivedNvnc.push(decodeCameraCommand(ab));
          return true;
        },
        dispatchEncodedCommand(ab) {
          assert.ok(ab instanceof ArrayBuffer);
          receivedNvnc.push(decodeCameraCommand(ab));
          return true;
        },
        start: () => true,
        stop: () => {},
        takePhoto: (opts) => ({
          filePath: "/tmp/native.jpg",
          width: 4032,
          height: 3024,
          quality: opts?.quality ?? 90,
          flashMode: 1,
          timestampNs: 123456,
        }),
        startRecording: () => true,
        stopRecording: () => ({
          outputPath: "/tmp/native.mp4",
          durationMs: 2000,
          width: 1920,
          height: 1080,
          hasAudio: true,
        }),
        lastFrame: () => sampleNvcf.buffer.slice(sampleNvcf.byteOffset, sampleNvcf.byteOffset + 40),
        dispose: () => {
          disposed = true;
        },
      }),
    });

    const devices = await getAvailableCameraDevices();
    assert.equal(devices.length, 2);
    assert.equal(useCameraDevice("front", devices)?.id, "ios-front-1");
    assert.equal(useCameraDevice(CameraFacing.Back, devices)?.id, "ios-back-0");

    const session = new CameraSession();
    assert.equal(session.backend, "native");

    await session.applyProps({
      device: devices[0],
      torch: "on",
      zoom: 2.5,
      fps: 60,
      audio: true,
      isActive: true,
    });
    assert.equal(receivedNvnp.length, 1);
    assert.equal(receivedNvnp[0].deviceId, "ios-back-0");
    assert.equal(receivedNvnp[0].torch, TorchMode.On);
    assert.equal(receivedNvnp[0].zoom, 2.5);
    assert.equal(receivedNvnp[0].audioEnabled, true);

    await session.dispatchCommand({ kind: "focus_point", x: 0.5, y: 0.5 });
    assert.equal(receivedNvnc.length, 1);
    assert.deepEqual(receivedNvnc[0], { kind: "focus_point", x: 0.5, y: 0.5 });

    const photo = await session.takePhoto({ quality: 95, flash: "on" });
    assert.equal(photo.filePath, "/tmp/native.jpg");
    assert.equal(photo.quality, 95);

    const frame = session.lastFrame();
    assert.equal(frame?.width, 1920);
    assert.equal(frame?.nativeHandleId, 9999n);

    const el = Camera({ device: devices[0], zoom: 1.5, isActive: true });
    assert.equal(el.type, "NveCameraPreview");
    assert.equal(decodeCameraProps(el.props.nvnpProps).zoom, 1.5);

    await session.dispose();
    assert.equal(session.isDisposed, true);
  } finally {
    globalThis.__nveCameraInstall = prevInstall;
    globalThis.__nveCamera = prevMod;
  }
});

test("@nve/camera throws explicitly when no VisionCamera native bridge is installed", () => {
  const prevInstall = globalThis.__nveCameraInstall;
  const prevMod = globalThis.__nveCamera;
  try {
    globalThis.__nveCamera = undefined;
    globalThis.__nveCameraInstall = undefined;
    assert.throws(() => new CameraSession(), /NVE_CAMERA_NATIVE_UNAVAILABLE/);
  } finally {
    globalThis.__nveCameraInstall = prevInstall;
    globalThis.__nveCamera = prevMod;
  }
});
