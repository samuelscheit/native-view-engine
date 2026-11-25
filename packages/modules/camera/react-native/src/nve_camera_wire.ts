/**
 * Binary wire codecs (`NVNP`, `NVNC`, 40-byte `NVCF`) byte-for-byte aligned with
 * `packages/modules/camera/src/{spec,frame}.rs` and `packages/modules/camera/cxx/include/nve_camera_codec.hpp`.
 */

import {
  CAMERA_FRAME_DESCRIPTOR_SIZE,
  CAMERA_FRAME_MAGIC,
  CAMERA_FRAME_VERSION,
  CameraCodecError,
  CameraCommandOp,
  CameraPropField,
  MAX_NATIVE_CMD_ARGS,
  MAX_NATIVE_PROP_FIELDS,
  NATIVE_CMD_HEADER_SIZE,
  NATIVE_CMD_MAGIC,
  NATIVE_CMD_VERSION,
  NATIVE_PROPS_HEADER_SIZE,
  NATIVE_PROPS_MAGIC,
  NATIVE_PROPS_VERSION,
  NativeValueTag,
  parseFacing,
  parsePixelFormat,
  parseStabilizationMode,
  parseTorchMode,
  toUint8Array,
  validateCameraFrame,
  validateCameraProps,
  validateFlash,
  validateFps,
  validateQuality,
  validateUnitCoord,
  validateZoom,
  type CameraFrameDescriptor,
  type CameraPreviewCommand,
  type CameraPreviewProps,
} from "./nve_camera_types.ts";

export * from "./nve_camera_types.ts";

const textEncoder = new TextEncoder();
const textDecoder = new TextDecoder("utf-8", { fatal: true });

type DecodedNativeValue =
  | { tag: typeof NativeValueTag.Null }
  | { tag: typeof NativeValueTag.Bool; value: boolean }
  | { tag: typeof NativeValueTag.U32; value: number }
  | { tag: typeof NativeValueTag.I64; value: bigint }
  | { tag: typeof NativeValueTag.U64; value: bigint }
  | { tag: typeof NativeValueTag.F32; value: number }
  | { tag: typeof NativeValueTag.F64; value: number }
  | { tag: typeof NativeValueTag.Str; value: string }
  | { tag: typeof NativeValueTag.Bytes; value: Uint8Array };

class ByteWriter {
  private buf: Uint8Array;
  private view: DataView;
  private len = 0;

  constructor(initialCap = 96) {
    this.buf = new Uint8Array(initialCap);
    this.view = new DataView(this.buf.buffer);
  }

  private ensure(extra: number): void {
    const needed = this.len + extra;
    if (needed <= this.buf.length) return;
    let nextCap = this.buf.length;
    while (nextCap < needed) nextCap *= 2;
    const next = new Uint8Array(nextCap);
    next.set(this.buf.subarray(0, this.len), 0);
    this.buf = next;
    this.view = new DataView(this.buf.buffer);
  }

  u8(v: number): void { this.ensure(1); this.buf[this.len++] = v & 0xff; }
  u16(v: number): void { this.ensure(2); this.view.setUint16(this.len, v, true); this.len += 2; }
  u32(v: number): void { this.ensure(4); this.view.setUint32(this.len, v, true); this.len += 4; }
  f32(v: number): void { this.ensure(4); this.view.setFloat32(this.len, v, true); this.len += 4; }
  bytes(src: readonly number[] | Uint8Array): void { this.ensure(src.length); this.buf.set(src, this.len); this.len += src.length; }

  str(s: string): void {
    const utf8 = textEncoder.encode(s);
    if (utf8.length > 0xffffffff) throw new CameraCodecError("PAYLOAD_TOO_LARGE", "String exceeds u32 length");
    this.u32(utf8.length);
    this.bytes(utf8);
  }

  finish(): Uint8Array { return this.buf.slice(0, this.len); }
}

function decodeNativeValue(bytes: Uint8Array, view: DataView, cursor: { pos: number }): DecodedNativeValue {
  if (cursor.pos >= bytes.length) throw new CameraCodecError("TRUNCATED", "Unexpected EOF reading tag");
  const tag = bytes[cursor.pos++];
  switch (tag) {
    case NativeValueTag.Null:
      return { tag: NativeValueTag.Null };
    case NativeValueTag.Bool: {
      if (bytes.length - cursor.pos < 1) throw new CameraCodecError("TRUNCATED", "Truncated bool");
      const b = bytes[cursor.pos++];
      if (b !== 0 && b !== 1) throw new CameraCodecError("INVALID_BOOL", `Invalid bool byte: ${b}`);
      return { tag: NativeValueTag.Bool, value: b === 1 };
    }
    case NativeValueTag.U32: {
      if (bytes.length - cursor.pos < 4) throw new CameraCodecError("TRUNCATED", "Truncated u32");
      const value = view.getUint32(cursor.pos, true);
      cursor.pos += 4;
      return { tag: NativeValueTag.U32, value };
    }
    case NativeValueTag.I64: {
      if (bytes.length - cursor.pos < 8) throw new CameraCodecError("TRUNCATED", "Truncated i64");
      const value = view.getBigInt64(cursor.pos, true);
      cursor.pos += 8;
      return { tag: NativeValueTag.I64, value };
    }
    case NativeValueTag.U64: {
      if (bytes.length - cursor.pos < 8) throw new CameraCodecError("TRUNCATED", "Truncated u64");
      const value = view.getBigUint64(cursor.pos, true);
      cursor.pos += 8;
      return { tag: NativeValueTag.U64, value };
    }
    case NativeValueTag.F32: {
      if (bytes.length - cursor.pos < 4) throw new CameraCodecError("TRUNCATED", "Truncated f32");
      const value = view.getFloat32(cursor.pos, true);
      cursor.pos += 4;
      return { tag: NativeValueTag.F32, value };
    }
    case NativeValueTag.F64: {
      if (bytes.length - cursor.pos < 8) throw new CameraCodecError("TRUNCATED", "Truncated f64");
      const value = view.getFloat64(cursor.pos, true);
      cursor.pos += 8;
      return { tag: NativeValueTag.F64, value };
    }
    case NativeValueTag.Str: {
      if (bytes.length - cursor.pos < 4) throw new CameraCodecError("TRUNCATED", "Truncated str len");
      const len = view.getUint32(cursor.pos, true);
      cursor.pos += 4;
      if (bytes.length - cursor.pos < len) throw new CameraCodecError("TRUNCATED", "Truncated str bytes");
      const slice = bytes.subarray(cursor.pos, cursor.pos + len);
      cursor.pos += len;
      try {
        return { tag: NativeValueTag.Str, value: textDecoder.decode(slice) };
      } catch {
        throw new CameraCodecError("INVALID_UTF8", "Invalid UTF-8 in NVNP/NVNC string");
      }
    }
    case NativeValueTag.Bytes: {
      if (bytes.length - cursor.pos < 4) throw new CameraCodecError("TRUNCATED", "Truncated bytes len");
      const len = view.getUint32(cursor.pos, true);
      cursor.pos += 4;
      if (bytes.length - cursor.pos < len) throw new CameraCodecError("TRUNCATED", "Truncated bytes payload");
      const slice = bytes.slice(cursor.pos, cursor.pos + len);
      cursor.pos += len;
      return { tag: NativeValueTag.Bytes, value: slice };
    }
    default:
      throw new CameraCodecError("INVALID_TAG", `Unknown NativeValueTag: ${tag}`);
  }
}

export function encodeCameraProps(props: CameraPreviewProps): Uint8Array {
  validateCameraProps(props);
  const w = new ByteWriter(96);
  w.bytes(NATIVE_PROPS_MAGIC);
  w.u8(NATIVE_PROPS_VERSION);
  w.u16(10);
  w.u16(CameraPropField.DeviceId);
  if (props.deviceId !== null) { w.u8(NativeValueTag.Str); w.str(props.deviceId); } else { w.u8(NativeValueTag.Null); }
  w.u16(CameraPropField.Facing); w.u8(NativeValueTag.U32); w.u32(props.facing);
  w.u16(CameraPropField.Torch); w.u8(NativeValueTag.U32); w.u32(props.torch);
  w.u16(CameraPropField.Zoom); w.u8(NativeValueTag.F32); w.f32(props.zoom);
  w.u16(CameraPropField.Fps); w.u8(NativeValueTag.U32); w.u32(props.fps);
  w.u16(CameraPropField.Hdr); w.u8(NativeValueTag.Bool); w.u8(props.hdr ? 1 : 0);
  w.u16(CameraPropField.Stabilization); w.u8(NativeValueTag.U32); w.u32(props.stabilization);
  w.u16(CameraPropField.EnableFrameProcessor); w.u8(NativeValueTag.Bool); w.u8(props.enableFrameProcessor ? 1 : 0);
  w.u16(CameraPropField.AudioEnabled); w.u8(NativeValueTag.Bool); w.u8(props.audioEnabled ? 1 : 0);
  w.u16(CameraPropField.Active); w.u8(NativeValueTag.Bool); w.u8(props.active ? 1 : 0);
  return w.finish();
}

export function decodeCameraProps(input: ArrayBuffer | ArrayBufferView): CameraPreviewProps {
  const bytes = toUint8Array(input);
  if (bytes.length < NATIVE_PROPS_HEADER_SIZE) throw new CameraCodecError("TRUNCATED", "NVNP header truncated");
  for (let i = 0; i < 4; i++) {
    if (bytes[i] !== NATIVE_PROPS_MAGIC[i]) throw new CameraCodecError("INVALID_MAGIC", "Expected NVNP magic");
  }
  if (bytes[4] !== NATIVE_PROPS_VERSION) throw new CameraCodecError("UNSUPPORTED_VERSION", `NVNP v${bytes[4]}`);
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const count = view.getUint16(5, true);
  if (count > MAX_NATIVE_PROP_FIELDS) throw new CameraCodecError("TOO_MANY_FIELDS", `NVNP fields ${count}`);

  const cursor = { pos: NATIVE_PROPS_HEADER_SIZE };
  const fields = new Map<number, DecodedNativeValue>();
  for (let i = 0; i < count; i++) {
    if (bytes.length - cursor.pos < 2) throw new CameraCodecError("TRUNCATED", "Truncated NVNP field id");
    const id = view.getUint16(cursor.pos, true);
    cursor.pos += 2;
    fields.set(id, decodeNativeValue(bytes, view, cursor));
  }
  if (cursor.pos !== bytes.length) {
    throw new CameraCodecError("TRAILING_BYTES", `${bytes.length - cursor.pos} trailing NVNP bytes`);
  }

  const req = (id: number): DecodedNativeValue => {
    const v = fields.get(id);
    if (!v) throw new CameraCodecError("MISSING_FIELD", `Missing NVNP field ${id}`);
    return v;
  };
  const reqU32 = (id: number): number => {
    const v = req(id);
    if (v.tag !== NativeValueTag.U32) throw new CameraCodecError("TYPE_MISMATCH", `Field ${id} expected u32`);
    return v.value;
  };
  const reqF32 = (id: number): number => {
    const v = req(id);
    if (v.tag !== NativeValueTag.F32) throw new CameraCodecError("TYPE_MISMATCH", `Field ${id} expected f32`);
    return v.value;
  };
  const reqBool = (id: number): boolean => {
    const v = req(id);
    if (v.tag !== NativeValueTag.Bool) throw new CameraCodecError("TYPE_MISMATCH", `Field ${id} expected bool`);
    return v.value;
  };

  const rawDev = req(CameraPropField.DeviceId);
  const deviceId =
    rawDev.tag === NativeValueTag.Null
      ? null
      : rawDev.tag === NativeValueTag.Str
        ? rawDev.value
        : (() => { throw new CameraCodecError("TYPE_MISMATCH", "Field 1 expected null|str"); })();

  const facing = parseFacing(reqU32(CameraPropField.Facing));
  const torch = parseTorchMode(reqU32(CameraPropField.Torch));
  const zoom = reqF32(CameraPropField.Zoom);
  validateZoom(zoom);
  const fps = validateFps(reqU32(CameraPropField.Fps));
  const hdr = reqBool(CameraPropField.Hdr);
  const stabilization = parseStabilizationMode(reqU32(CameraPropField.Stabilization));
  const enableFrameProcessor = reqBool(CameraPropField.EnableFrameProcessor);
  const audioEnabled = reqBool(CameraPropField.AudioEnabled);
  const active = reqBool(CameraPropField.Active);
  return { deviceId, facing, torch, zoom, fps, hdr, stabilization, enableFrameProcessor, audioEnabled, active };
}

export function encodeCameraCommand(cmd: CameraPreviewCommand): Uint8Array {
  const w = new ByteWriter(32);
  w.bytes(NATIVE_CMD_MAGIC);
  w.u8(NATIVE_CMD_VERSION);
  switch (cmd.kind) {
    case "switch_facing":
      w.u16(CameraCommandOp.SwitchFacing); w.u8(1); w.u8(NativeValueTag.U32); w.u32(parseFacing(cmd.facing));
      return w.finish();
    case "set_torch":
      w.u16(CameraCommandOp.SetTorch); w.u8(1); w.u8(NativeValueTag.U32); w.u32(parseTorchMode(cmd.torch));
      return w.finish();
    case "set_zoom":
      validateZoom(cmd.zoom);
      w.u16(CameraCommandOp.SetZoom); w.u8(1); w.u8(NativeValueTag.F32); w.f32(cmd.zoom);
      return w.finish();
    case "focus_point":
      validateUnitCoord(cmd.x, "focus_point.x");
      validateUnitCoord(cmd.y, "focus_point.y");
      w.u16(CameraCommandOp.FocusPoint); w.u8(2);
      w.u8(NativeValueTag.F32); w.f32(cmd.x);
      w.u8(NativeValueTag.F32); w.f32(cmd.y);
      return w.finish();
    case "capture_photo":
      validateQuality(cmd.quality);
      validateFlash(cmd.flash);
      w.u16(CameraCommandOp.CapturePhoto); w.u8(2);
      w.u8(NativeValueTag.U32); w.u32(cmd.quality);
      w.u8(NativeValueTag.U32); w.u32(cmd.flash);
      return w.finish();
    case "start_recording":
      w.u16(CameraCommandOp.StartRecording); w.u8(1); w.u8(NativeValueTag.Str); w.str(cmd.outputPath);
      return w.finish();
    case "stop_recording":
      w.u16(CameraCommandOp.StopRecording); w.u8(0);
      return w.finish();
  }
}

export function decodeCameraCommand(input: ArrayBuffer | ArrayBufferView): CameraPreviewCommand {
  const bytes = toUint8Array(input);
  if (bytes.length < NATIVE_CMD_HEADER_SIZE) throw new CameraCodecError("TRUNCATED", "NVNC header truncated");
  for (let i = 0; i < 4; i++) {
    if (bytes[i] !== NATIVE_CMD_MAGIC[i]) throw new CameraCodecError("INVALID_MAGIC", "Expected NVNC magic");
  }
  if (bytes[4] !== NATIVE_CMD_VERSION) throw new CameraCodecError("UNSUPPORTED_VERSION", `NVNC v${bytes[4]}`);
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const opcode = view.getUint16(5, true);
  const argc = bytes[7];
  if (argc > MAX_NATIVE_CMD_ARGS) throw new CameraCodecError("TOO_MANY_FIELDS", `NVNC argc ${argc}`);

  const cursor = { pos: NATIVE_CMD_HEADER_SIZE };
  const args: DecodedNativeValue[] = [];
  for (let i = 0; i < argc; i++) args.push(decodeNativeValue(bytes, view, cursor));
  if (cursor.pos !== bytes.length) {
    throw new CameraCodecError("TRAILING_BYTES", `${bytes.length - cursor.pos} trailing NVNC bytes`);
  }

  const expectArgc = (n: number): void => {
    if (args.length !== n) throw new CameraCodecError("UNEXPECTED_ARG_COUNT", `Opcode ${opcode} expects ${n}`);
  };
  const argU32 = (i: number): number => {
    const v = args[i];
    if (!v || v.tag !== NativeValueTag.U32) throw new CameraCodecError("TYPE_MISMATCH", `Arg ${i} expected u32`);
    return v.value;
  };
  const argF32 = (i: number): number => {
    const v = args[i];
    if (!v || v.tag !== NativeValueTag.F32) throw new CameraCodecError("TYPE_MISMATCH", `Arg ${i} expected f32`);
    return v.value;
  };
  const argStr = (i: number): string => {
    const v = args[i];
    if (!v || v.tag !== NativeValueTag.Str) throw new CameraCodecError("TYPE_MISMATCH", `Arg ${i} expected str`);
    return v.value;
  };

  switch (opcode) {
    case CameraCommandOp.SwitchFacing:
      expectArgc(1);
      return { kind: "switch_facing", facing: parseFacing(argU32(0)) };
    case CameraCommandOp.SetTorch:
      expectArgc(1);
      return { kind: "set_torch", torch: parseTorchMode(argU32(0)) };
    case CameraCommandOp.SetZoom: {
      expectArgc(1);
      const zoom = argF32(0);
      validateZoom(zoom);
      return { kind: "set_zoom", zoom };
    }
    case CameraCommandOp.FocusPoint: {
      expectArgc(2);
      const x = argF32(0), y = argF32(1);
      validateUnitCoord(x, "focus_point.x");
      validateUnitCoord(y, "focus_point.y");
      return { kind: "focus_point", x, y };
    }
    case CameraCommandOp.CapturePhoto:
      expectArgc(2);
      return { kind: "capture_photo", quality: validateQuality(argU32(0)), flash: validateFlash(argU32(1)) };
    case CameraCommandOp.StartRecording:
      expectArgc(1);
      return { kind: "start_recording", outputPath: argStr(0) };
    case CameraCommandOp.StopRecording:
      expectArgc(0);
      return { kind: "stop_recording" };
    default:
      throw new CameraCodecError("UNKNOWN_COMMAND_OPCODE", `Unknown NVNC opcode ${opcode}`);
  }
}

export function encodeCameraFrame(desc: CameraFrameDescriptor): Uint8Array {
  validateCameraFrame(desc);
  const out = new Uint8Array(CAMERA_FRAME_DESCRIPTOR_SIZE);
  const view = new DataView(out.buffer);
  out.set(CAMERA_FRAME_MAGIC, 0);
  out[4] = CAMERA_FRAME_VERSION;
  out[5] = desc.pixelFormat;
  view.setUint16(6, desc.rotationDeg, true);
  view.setUint32(8, desc.width, true);
  view.setUint32(12, desc.height, true);
  view.setUint32(16, desc.strideBytes, true);
  view.setUint32(20, 0, true);
  view.setBigUint64(24, BigInt.asUintN(64, desc.timestampNs), true);
  view.setBigUint64(32, BigInt.asUintN(64, desc.nativeHandleId), true);
  return out;
}

export function decodeCameraFrame(input: ArrayBuffer | ArrayBufferView): CameraFrameDescriptor {
  const bytes = toUint8Array(input);
  if (bytes.length < CAMERA_FRAME_DESCRIPTOR_SIZE) throw new CameraCodecError("TRUNCATED", "NVCF < 40 bytes");
  if (bytes.length > CAMERA_FRAME_DESCRIPTOR_SIZE) throw new CameraCodecError("TRAILING_BYTES", "NVCF > 40 bytes");
  for (let i = 0; i < 4; i++) {
    if (bytes[i] !== CAMERA_FRAME_MAGIC[i]) throw new CameraCodecError("INVALID_MAGIC", "Expected NVCF magic");
  }
  if (bytes[4] !== CAMERA_FRAME_VERSION) throw new CameraCodecError("UNSUPPORTED_VERSION", `NVCF v${bytes[4]}`);
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const pixelFormat = parsePixelFormat(bytes[5]);
  const rotationDeg = view.getUint16(6, true);
  const width = view.getUint32(8, true);
  const height = view.getUint32(12, true);
  const strideBytes = view.getUint32(16, true);
  const reserved = view.getUint32(20, true);
  if (reserved !== 0) throw new CameraCodecError("INVALID_ENUM", `Non-zero NVCF reserved: ${reserved}`);
  const desc: CameraFrameDescriptor = {
    width,
    height,
    strideBytes,
    pixelFormat,
    rotationDeg: rotationDeg as 0 | 90 | 180 | 270,
    timestampNs: view.getBigUint64(24, true),
    nativeHandleId: view.getBigUint64(32, true),
  };
  validateCameraFrame(desc);
  return desc;
}
