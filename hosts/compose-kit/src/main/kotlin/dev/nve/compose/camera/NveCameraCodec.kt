package dev.nve.compose.camera

import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction
import java.nio.charset.StandardCharsets

/** Physical lens facing direction matching `CameraPreviewFacing` / `nve-sdk::CameraFacing`. */
enum class NveCameraFacing(val wireCode: Int) {
    BACK(1), FRONT(2), EXTERNAL(3);

    companion object {
        fun fromWireCode(code: Int): NveCameraFacing =
            entries.firstOrNull { it.wireCode == code }
                ?: throw NveCameraCodecException.InvalidEnum("CameraPreviewFacing", code)
    }
}

/** Torch / flash mode matching `TorchMode` / `nve-sdk::FlashMode`. */
enum class NveTorchMode(val wireCode: Int) {
    OFF(0), ON(1), AUTO(2);

    companion object {
        fun fromWireCode(code: Int): NveTorchMode =
            entries.firstOrNull { it.wireCode == code }
                ?: throw NveCameraCodecException.InvalidEnum("TorchMode", code)
    }
}

/** Hardware video stabilization mode matching `StabilizationMode` in `crates/nve-interop`. */
enum class NveStabilizationMode(val wireCode: Int) {
    OFF(0), STANDARD(1), CINEMATIC(2);

    companion object {
        fun fromWireCode(code: Int): NveStabilizationMode =
            entries.firstOrNull { it.wireCode == code }
                ?: throw NveCameraCodecException.InvalidEnum("StabilizationMode", code)
    }
}

/** Pixel layout matching `CameraPixelFormat` in `crates/nve-interop`. */
enum class NveCameraPixelFormat(val wireCode: Int, val bytesPerPrimarySample: Long) {
    RGBA8888(0, 4L), BGRA8888(1, 4L), NV12(2, 1L), YUV420_888(3, 1L);

    companion object {
        fun fromWireCode(code: Int): NveCameraPixelFormat =
            entries.firstOrNull { it.wireCode == code }
                ?: throw NveCameraCodecException.InvalidEnum("CameraPixelFormat", code)
    }
}

/** Declarative `CameraPreview` props (`NVNP`) matching `CameraPreviewProps` in Rust. */
data class NveCameraPreviewProps(
    val deviceId: String? = null,
    val facing: NveCameraFacing = NveCameraFacing.BACK,
    val torch: NveTorchMode = NveTorchMode.OFF,
    val zoom: Float = 1.0f,
    val fps: Int = 30,
    val hdr: Boolean = false,
    val stabilization: NveStabilizationMode = NveStabilizationMode.OFF,
    val enableFrameProcessor: Boolean = false,
    val audioEnabled: Boolean = false,
    val active: Boolean = true,
) {
    fun validate() {
        NveCameraCodec.validateZoom(zoom)
        NveCameraCodec.validateFps(fps)
    }
}

/** Typed imperative `CameraPreview` commands (`NVNC`) matching `CameraPreviewCommand` in Rust. */
sealed interface NveCameraCommand {
    val opcode: Int
    val commandName: String

    data class SwitchFacing(val facing: NveCameraFacing) : NveCameraCommand {
        override val opcode = OP_SWITCH_FACING; override val commandName = "switch_facing"
    }
    data class SetTorch(val mode: NveTorchMode) : NveCameraCommand {
        override val opcode = OP_SET_TORCH; override val commandName = "set_torch"
    }
    data class SetZoom(val zoom: Float) : NveCameraCommand {
        override val opcode = OP_SET_ZOOM; override val commandName = "set_zoom"
    }
    data class FocusPoint(val x: Float, val y: Float) : NveCameraCommand {
        override val opcode = OP_FOCUS_POINT; override val commandName = "focus_point"
    }
    data class CapturePhoto(val quality: Int, val flash: Int) : NveCameraCommand {
        override val opcode = OP_CAPTURE_PHOTO; override val commandName = "capture_photo"
    }
    data class StartRecording(val outputPath: String) : NveCameraCommand {
        override val opcode = OP_START_RECORDING; override val commandName = "start_recording"
    }
    data object StopRecording : NveCameraCommand {
        override val opcode = OP_STOP_RECORDING; override val commandName = "stop_recording"
    }

    companion object {
        const val OP_SWITCH_FACING: Int = 1
        const val OP_SET_TORCH: Int = 2
        const val OP_SET_ZOOM: Int = 3
        const val OP_FOCUS_POINT: Int = 4
        const val OP_CAPTURE_PHOTO: Int = 5
        const val OP_START_RECORDING: Int = 6
        const val OP_STOP_RECORDING: Int = 7
    }
}

/** Zero-copy 40-byte little-endian frame descriptor (`NVCF`) matching Rust `CameraFrameDescriptor`. */
data class NveCameraFrameDescriptor(
    val width: Int,
    val height: Int,
    val strideBytes: Int,
    val pixelFormat: NveCameraPixelFormat,
    val rotationDeg: Int,
    val timestampNs: Long,
    val nativeHandleId: Long,
) {
    fun validate() {
        val w = width.toLong() and 0xFFFF_FFFFL
        val h = height.toLong() and 0xFFFF_FFFFL
        val stride = strideBytes.toLong() and 0xFFFF_FFFFL
        if (w == 0L) throw NveCameraCodecException.InvalidValue("frame width must be non-zero")
        if (h == 0L) throw NveCameraCodecException.InvalidValue("frame height must be non-zero")
        if (rotationDeg !in intArrayOf(0, 90, 180, 270)) {
            throw NveCameraCodecException.InvalidEnum("CameraRotationDeg", rotationDeg)
        }
        val minStride = w * pixelFormat.bytesPerPrimarySample
        if (minStride > 0xFFFF_FFFFL) throw NveCameraCodecException.InvalidValue("stride overflows u32")
        if (stride < minStride) {
            throw NveCameraCodecException.InvalidValue("strideBytes=$stride < minStride=$minStride")
        }
    }
}

/** Strict validation errors thrown on malformed, truncated, or out-of-range camera wire buffers. */
sealed class NveCameraCodecException(message: String) : IllegalArgumentException(message) {
    class Truncated(detail: String) : NveCameraCodecException("Truncated camera buffer: $detail")
    class InvalidMagic(val magic: String) : NveCameraCodecException("Invalid magic: $magic")
    class UnsupportedVersion(val version: Int) : NveCameraCodecException("Unsupported version: $version")
    class InvalidTag(val tag: Int) : NveCameraCodecException("Invalid tag: $tag")
    class InvalidBool(val raw: Int) : NveCameraCodecException("Invalid bool byte: $raw")
    class InvalidEnum(val typeName: String, val raw: Int) : NveCameraCodecException("Invalid $typeName: $raw")
    class InvalidUtf8(field: String) : NveCameraCodecException("Invalid UTF-8 in $field")
    class MissingField(val fieldId: Int) : NveCameraCodecException("Missing field $fieldId")
    class UnexpectedArgCount(val expected: Int, val actual: Int) :
        NveCameraCodecException("Expected $expected args, got $actual")
    class TypeMismatch(val expected: String) : NveCameraCodecException("Type mismatch: expected $expected")
    class TrailingBytes(val count: Int) : NveCameraCodecException("Unexpected $count trailing bytes")
    class UnknownCommandOpcode(val opcode: Int) : NveCameraCodecException("Unknown opcode: $opcode")
    class TooManyFields(val count: Int) : NveCameraCodecException("Too many fields/args: $count")
    class InvalidValue(detail: String) : NveCameraCodecException("Invalid camera value: $detail")
}

/**
 * Pure-Kotlin zero-JSON binary `NVNP` / `NVNC` / `NVCF` codec matching
 * `crates/nve-interop/src/native_view/{wire,codec,camera}.rs` byte-for-byte.
 */
object NveCameraCodec {
    const val KIND: String = "nve.camera.preview"
    const val NVNP_VERSION: Int = 1
    const val NVNP_HEADER_SIZE: Int = 7
    const val MAX_PROP_FIELDS: Int = 1024
    const val NVNC_VERSION: Int = 1
    const val NVNC_HEADER_SIZE: Int = 8
    const val MAX_CMD_ARGS: Int = 64
    const val NVCF_VERSION: Int = 1
    const val NVCF_SIZE: Int = 40
    const val MAX_FPS: Int = 240

    const val FIELD_DEVICE_ID: Int = 1
    const val FIELD_FACING: Int = 2
    const val FIELD_TORCH: Int = 3
    const val FIELD_ZOOM: Int = 4
    const val FIELD_FPS: Int = 5
    const val FIELD_HDR: Int = 6
    const val FIELD_STABILIZATION: Int = 7
    const val FIELD_ENABLE_FRAME_PROCESSOR: Int = 8
    const val FIELD_AUDIO_ENABLED: Int = 9
    const val FIELD_ACTIVE: Int = 10

    private const val TAG_NULL = 0
    private const val TAG_BOOL = 1
    private const val TAG_U32 = 2
    private const val TAG_I64 = 3
    private const val TAG_U64 = 4
    private const val TAG_F32 = 5
    private const val TAG_F64 = 6
    private const val TAG_STR = 7
    private const val TAG_BYTES = 8

    private val MAGIC_NVNP = "NVNP".toByteArray(StandardCharsets.US_ASCII)
    private val MAGIC_NVNC = "NVNC".toByteArray(StandardCharsets.US_ASCII)
    private val MAGIC_NVCF = "NVCF".toByteArray(StandardCharsets.US_ASCII)

    private sealed interface WireValue {
        data object Null : WireValue
        data class Bool(val v: Boolean) : WireValue
        data class U32(val v: Int) : WireValue
        data class I64(val v: Long) : WireValue
        data class U64(val v: Long) : WireValue
        data class F32(val v: Float) : WireValue
        data class F64(val v: Double) : WireValue
        data class Str(val v: String) : WireValue
        data class Bytes(val v: ByteArray) : WireValue
    }

    fun encodeProps(props: NveCameraPreviewProps): ByteArray {
        props.validate()
        val out = ByteArrayOutputStream(96)
        out.write(MAGIC_NVNP); out.write(NVNP_VERSION); writeU16Le(out, 10)
        if (props.deviceId == null) {
            writeFieldHdr(out, FIELD_DEVICE_ID, TAG_NULL)
        } else {
            writeFieldHdr(out, FIELD_DEVICE_ID, TAG_STR); writeUtf8(out, props.deviceId)
        }
        writeU32Field(out, FIELD_FACING, props.facing.wireCode)
        writeU32Field(out, FIELD_TORCH, props.torch.wireCode)
        writeF32Field(out, FIELD_ZOOM, props.zoom)
        writeU32Field(out, FIELD_FPS, props.fps)
        writeBoolField(out, FIELD_HDR, props.hdr)
        writeU32Field(out, FIELD_STABILIZATION, props.stabilization.wireCode)
        writeBoolField(out, FIELD_ENABLE_FRAME_PROCESSOR, props.enableFrameProcessor)
        writeBoolField(out, FIELD_AUDIO_ENABLED, props.audioEnabled)
        writeBoolField(out, FIELD_ACTIVE, props.active)
        return out.toByteArray()
    }

    fun decodeProps(bytes: ByteArray): NveCameraPreviewProps {
        if (bytes.size < NVNP_HEADER_SIZE) throw NveCameraCodecException.Truncated("NVNP header")
        checkMagic(bytes, MAGIC_NVNP)
        val version = bytes[4].toInt() and 0xFF
        if (version != NVNP_VERSION) throw NveCameraCodecException.UnsupportedVersion(version)

        val buf = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN).apply { position(5) }
        val count = buf.short.toInt() and 0xFFFF
        if (count > MAX_PROP_FIELDS) throw NveCameraCodecException.TooManyFields(count)
        if (count > buf.remaining() / 3) throw NveCameraCodecException.Truncated("field count")

        val fields = LinkedHashMap<Int, WireValue>(count)
        repeat(count) {
            requireRemaining(buf, 2, "field_id")
            val id = buf.short.toInt() and 0xFFFF
            fields.putIfAbsent(id, readValue(buf))
        }
        if (buf.hasRemaining()) throw NveCameraCodecException.TrailingBytes(buf.remaining())

        val deviceId = when (val v = reqField(fields, FIELD_DEVICE_ID)) {
            WireValue.Null -> null
            is WireValue.Str -> v.v
            else -> throw NveCameraCodecException.TypeMismatch("Option<str>")
        }
        val facing = NveCameraFacing.fromWireCode(asU32(reqField(fields, FIELD_FACING)))
        val torch = NveTorchMode.fromWireCode(asU32(reqField(fields, FIELD_TORCH)))
        val zoom = asF32(reqField(fields, FIELD_ZOOM)).also(::validateZoom)
        val fps = asU32(reqField(fields, FIELD_FPS)).also(::validateFps)
        val hdr = asBool(reqField(fields, FIELD_HDR))
        val stab = NveStabilizationMode.fromWireCode(asU32(reqField(fields, FIELD_STABILIZATION)))
        return NveCameraPreviewProps(
            deviceId = deviceId,
            facing = facing,
            torch = torch,
            zoom = zoom,
            fps = fps,
            hdr = hdr,
            stabilization = stab,
            enableFrameProcessor = asBool(reqField(fields, FIELD_ENABLE_FRAME_PROCESSOR)),
            audioEnabled = asBool(reqField(fields, FIELD_AUDIO_ENABLED)),
            active = asBool(reqField(fields, FIELD_ACTIVE)),
        )
    }

    fun encodeCommand(command: NveCameraCommand): ByteArray {
        val out = ByteArrayOutputStream(32)
        out.write(MAGIC_NVNC); out.write(NVNC_VERSION); writeU16Le(out, command.opcode)
        when (command) {
            is NveCameraCommand.SwitchFacing -> {
                out.write(1); out.write(TAG_U32); writeI32Le(out, command.facing.wireCode)
            }
            is NveCameraCommand.SetTorch -> {
                out.write(1); out.write(TAG_U32); writeI32Le(out, command.mode.wireCode)
            }
            is NveCameraCommand.SetZoom -> {
                validateZoom(command.zoom)
                out.write(1); out.write(TAG_F32); writeI32Le(out, command.zoom.toRawBits())
            }
            is NveCameraCommand.FocusPoint -> {
                validateUnitCoord(command.x, "focus_point.x"); validateUnitCoord(command.y, "focus_point.y")
                out.write(2)
                out.write(TAG_F32); writeI32Le(out, command.x.toRawBits())
                out.write(TAG_F32); writeI32Le(out, command.y.toRawBits())
            }
            is NveCameraCommand.CapturePhoto -> {
                validateQuality(command.quality); validateFlash(command.flash)
                out.write(2)
                out.write(TAG_U32); writeI32Le(out, command.quality)
                out.write(TAG_U32); writeI32Le(out, command.flash)
            }
            is NveCameraCommand.StartRecording -> {
                out.write(1); out.write(TAG_STR); writeUtf8(out, command.outputPath)
            }
            NveCameraCommand.StopRecording -> out.write(0)
        }
        return out.toByteArray()
    }

    fun decodeCommand(bytes: ByteArray): NveCameraCommand {
        if (bytes.size < NVNC_HEADER_SIZE) throw NveCameraCodecException.Truncated("NVNC header")
        checkMagic(bytes, MAGIC_NVNC)
        val version = bytes[4].toInt() and 0xFF
        if (version != NVNC_VERSION) throw NveCameraCodecException.UnsupportedVersion(version)

        val buf = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN).apply { position(5) }
        val opcode = buf.short.toInt() and 0xFFFF
        val argCount = buf.get().toInt() and 0xFF
        if (argCount > MAX_CMD_ARGS) throw NveCameraCodecException.TooManyFields(argCount)
        if (argCount > buf.remaining()) throw NveCameraCodecException.Truncated("arg count")

        val args = List(argCount) { readValue(buf) }
        if (buf.hasRemaining()) throw NveCameraCodecException.TrailingBytes(buf.remaining())

        return when (opcode) {
            NveCameraCommand.OP_SWITCH_FACING -> {
                expectArgs(args, 1); NveCameraCommand.SwitchFacing(NveCameraFacing.fromWireCode(asU32(args[0])))
            }
            NveCameraCommand.OP_SET_TORCH -> {
                expectArgs(args, 1); NveCameraCommand.SetTorch(NveTorchMode.fromWireCode(asU32(args[0])))
            }
            NveCameraCommand.OP_SET_ZOOM -> {
                expectArgs(args, 1); NveCameraCommand.SetZoom(asF32(args[0]).also(::validateZoom))
            }
            NveCameraCommand.OP_FOCUS_POINT -> {
                expectArgs(args, 2)
                val x = asF32(args[0]).also { validateUnitCoord(it, "focus_point.x") }
                val y = asF32(args[1]).also { validateUnitCoord(it, "focus_point.y") }
                NveCameraCommand.FocusPoint(x, y)
            }
            NveCameraCommand.OP_CAPTURE_PHOTO -> {
                expectArgs(args, 2)
                val q = asU32(args[0]).also(::validateQuality)
                val f = asU32(args[1]).also(::validateFlash)
                NveCameraCommand.CapturePhoto(quality = q, flash = f)
            }
            NveCameraCommand.OP_START_RECORDING -> {
                expectArgs(args, 1); NveCameraCommand.StartRecording(asStr(args[0]))
            }
            NveCameraCommand.OP_STOP_RECORDING -> {
                expectArgs(args, 0); NveCameraCommand.StopRecording
            }
            else -> throw NveCameraCodecException.UnknownCommandOpcode(opcode)
        }
    }

    fun encodeFrameDescriptor(descriptor: NveCameraFrameDescriptor): ByteArray =
        ByteArray(NVCF_SIZE).also { encodeFrameDescriptorInto(descriptor, it, 0) }

    fun encodeFrameDescriptorInto(descriptor: NveCameraFrameDescriptor, dest: ByteArray, offset: Int = 0) {
        descriptor.validate()
        if (offset < 0 || offset + NVCF_SIZE > dest.size) {
            throw NveCameraCodecException.Truncated("destination requires $NVCF_SIZE bytes")
        }
        ByteBuffer.wrap(dest, offset, NVCF_SIZE).order(ByteOrder.LITTLE_ENDIAN).apply {
            put(MAGIC_NVCF)
            put(NVCF_VERSION.toByte())
            put(descriptor.pixelFormat.wireCode.toByte())
            putShort(descriptor.rotationDeg.toShort())
            putInt(descriptor.width)
            putInt(descriptor.height)
            putInt(descriptor.strideBytes)
            putInt(0) // reserved = 0 in v1
            putLong(descriptor.timestampNs)
            putLong(descriptor.nativeHandleId)
        }
    }

    fun decodeFrameDescriptor(bytes: ByteArray): NveCameraFrameDescriptor =
        decodeFrameDescriptor(ByteBuffer.wrap(bytes))

    fun decodeFrameDescriptor(source: ByteBuffer): NveCameraFrameDescriptor {
        val buf = source.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        val rem = buf.remaining()
        if (rem < NVCF_SIZE) throw NveCameraCodecException.Truncated("NVCF requires $NVCF_SIZE bytes, got $rem")
        if (rem > NVCF_SIZE) throw NveCameraCodecException.TrailingBytes(rem - NVCF_SIZE)
        val magic = ByteArray(4).also { buf.get(it) }
        checkMagic(magic, MAGIC_NVCF)
        val ver = buf.get().toInt() and 0xFF
        if (ver != NVCF_VERSION) throw NveCameraCodecException.UnsupportedVersion(ver)
        val fmt = NveCameraPixelFormat.fromWireCode(buf.get().toInt() and 0xFF)
        val rot = buf.short.toInt() and 0xFFFF
        val w = buf.int; val h = buf.int; val stride = buf.int; val reserved = buf.int
        if (reserved != 0) throw NveCameraCodecException.InvalidEnum("CameraFrameReserved", reserved)
        return NveCameraFrameDescriptor(w, h, stride, fmt, rot, buf.long, buf.long).also { it.validate() }
    }

    internal fun validateZoom(zoom: Float) {
        if (!zoom.isFinite() || zoom <= 0.0f) {
            throw NveCameraCodecException.InvalidValue("zoom must be finite and > 0.0, got $zoom")
        }
    }

    internal fun validateFps(fps: Int) {
        if (fps !in 1..MAX_FPS) throw NveCameraCodecException.InvalidEnum("CameraFps", fps)
    }

    internal fun validateUnitCoord(value: Float, name: String) {
        if (!value.isFinite() || value < 0.0f || value > 1.0f) {
            throw NveCameraCodecException.InvalidValue("$name must be finite in 0.0..=1.0, got $value")
        }
    }

    internal fun validateQuality(quality: Int) {
        if (quality !in 0..100) throw NveCameraCodecException.InvalidEnum("PhotoQuality", quality)
    }

    internal fun validateFlash(flash: Int) {
        if (flash !in 0..2) throw NveCameraCodecException.InvalidEnum("FlashMode", flash)
    }

    private fun checkMagic(bytes: ByteArray, expected: ByteArray) {
        for (i in 0 until 4) {
            if (bytes[i] != expected[i]) {
                throw NveCameraCodecException.InvalidMagic(String(bytes, 0, 4, StandardCharsets.ISO_8859_1))
            }
        }
    }

    private fun readValue(buf: ByteBuffer): WireValue {
        requireRemaining(buf, 1, "tag")
        return when (val tag = buf.get().toInt() and 0xFF) {
            TAG_NULL -> WireValue.Null
            TAG_BOOL -> {
                requireRemaining(buf, 1, "bool")
                when (val b = buf.get().toInt() and 0xFF) {
                    0 -> WireValue.Bool(false); 1 -> WireValue.Bool(true)
                    else -> throw NveCameraCodecException.InvalidBool(b)
                }
            }
            TAG_U32 -> { requireRemaining(buf, 4, "u32"); WireValue.U32(buf.int) }
            TAG_I64 -> { requireRemaining(buf, 8, "i64"); WireValue.I64(buf.long) }
            TAG_U64 -> { requireRemaining(buf, 8, "u64"); WireValue.U64(buf.long) }
            TAG_F32 -> { requireRemaining(buf, 4, "f32"); WireValue.F32(buf.float) }
            TAG_F64 -> { requireRemaining(buf, 8, "f64"); WireValue.F64(buf.double) }
            TAG_STR -> WireValue.Str(decodeUtf8Strict(readLenPrefixed(buf, "string"), "string"))
            TAG_BYTES -> WireValue.Bytes(readLenPrefixed(buf, "bytes"))
            else -> throw NveCameraCodecException.InvalidTag(tag)
        }
    }

    private fun readLenPrefixed(buf: ByteBuffer, label: String): ByteArray {
        requireRemaining(buf, 4, "$label len")
        val len = buf.int.toLong() and 0xFFFF_FFFFL
        if (len > buf.remaining().toLong()) throw NveCameraCodecException.Truncated("$label len=$len")
        return ByteArray(len.toInt()).also { buf.get(it) }
    }

    private fun decodeUtf8Strict(bytes: ByteArray, field: String): String {
        if (bytes.isEmpty()) return ""
        return try {
            StandardCharsets.UTF_8.newDecoder()
                .onMalformedInput(CodingErrorAction.REPORT)
                .onUnmappableCharacter(CodingErrorAction.REPORT)
                .decode(ByteBuffer.wrap(bytes))
                .toString()
        } catch (_: CharacterCodingException) {
            throw NveCameraCodecException.InvalidUtf8(field)
        }
    }

    private fun requireRemaining(buf: ByteBuffer, n: Int, detail: String) {
        if (buf.remaining() < n) throw NveCameraCodecException.Truncated("$detail needs $n bytes")
    }

    private fun reqField(fields: Map<Int, WireValue>, id: Int): WireValue =
        fields[id] ?: throw NveCameraCodecException.MissingField(id)

    private fun asBool(v: WireValue) = (v as? WireValue.Bool)?.v ?: throw NveCameraCodecException.TypeMismatch("bool")
    private fun asU32(v: WireValue) = (v as? WireValue.U32)?.v ?: throw NveCameraCodecException.TypeMismatch("u32")
    private fun asF32(v: WireValue) = (v as? WireValue.F32)?.v ?: throw NveCameraCodecException.TypeMismatch("f32")
    private fun asStr(v: WireValue) = (v as? WireValue.Str)?.v ?: throw NveCameraCodecException.TypeMismatch("str")

    private fun expectArgs(args: List<WireValue>, expected: Int) {
        if (args.size != expected) throw NveCameraCodecException.UnexpectedArgCount(expected, args.size)
    }

    private fun writeFieldHdr(out: ByteArrayOutputStream, id: Int, tag: Int) { writeU16Le(out, id); out.write(tag) }
    private fun writeBoolField(out: ByteArrayOutputStream, id: Int, v: Boolean) { writeFieldHdr(out, id, TAG_BOOL); out.write(if (v) 1 else 0) }
    private fun writeU32Field(out: ByteArrayOutputStream, id: Int, v: Int) { writeFieldHdr(out, id, TAG_U32); writeI32Le(out, v) }
    private fun writeF32Field(out: ByteArrayOutputStream, id: Int, v: Float) { writeFieldHdr(out, id, TAG_F32); writeI32Le(out, v.toRawBits()) }
    private fun writeUtf8(out: ByteArrayOutputStream, s: String) { val b = s.toByteArray(StandardCharsets.UTF_8); writeI32Le(out, b.size); out.write(b) }
    private fun writeU16Le(out: ByteArrayOutputStream, v: Int) { out.write(v and 0xFF); out.write((v ushr 8) and 0xFF) }
    private fun writeI32Le(out: ByteArrayOutputStream, v: Int) {
        out.write(v and 0xFF); out.write((v ushr 8) and 0xFF)
        out.write((v ushr 16) and 0xFF); out.write((v ushr 24) and 0xFF)
    }
}
