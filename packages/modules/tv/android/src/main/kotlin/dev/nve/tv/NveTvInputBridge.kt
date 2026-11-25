package dev.nve.tv

import android.content.Context
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction
import java.nio.charset.StandardCharsets

/**
 * Decoded zero-JSON `NK1` binary keydown payload.
 */
data class NveNk1KeyDown(
    val key: String,
    val meta: Boolean = false,
    val ctrl: Boolean = false,
    val shift: Boolean = false,
    val alt: Boolean = false,
)

/**
 * Zero-JSON `NK1` binary keydown codec matching `packages/core/codegen/src/wire_codecs/spec.rs` and
 * `packages/core/abi/src/events.rs` byte-for-byte:
 *
 * Layout: `[0x4E ('N'), 0x4B ('K'), 0x31 ('1'), flags_u8, utf8_key...]`
 * - `flags_u8` bit 0 (`0x01`): `meta`
 * - `flags_u8` bit 1 (`0x02`): `ctrl`
 * - `flags_u8` bit 2 (`0x04`): `shift`
 * - `flags_u8` bit 3 (`0x08`): `alt`
 */
object NveNk1Codec {
    const val MAGIC_0: Byte = 0x4E // 'N'
    const val MAGIC_1: Byte = 0x4B // 'K'
    const val MAGIC_2: Byte = 0x31 // '1'
    const val HEADER_SIZE: Int = 4
    const val MIN_LEN: Int = 5

    const val FLAG_META: Int = 1 shl 0
    const val FLAG_CTRL: Int = 1 shl 1
    const val FLAG_SHIFT: Int = 1 shl 2
    const val FLAG_ALT: Int = 1 shl 3

    const val KEYDOWN_EVENT_NAME: String = "keydown"

    fun encode(
        key: String,
        meta: Boolean = false,
        ctrl: Boolean = false,
        shift: Boolean = false,
        alt: Boolean = false,
    ): ByteArray {
        val keyBytes = key.toByteArray(StandardCharsets.UTF_8)
        require(keyBytes.isNotEmpty()) { "NK1 key name must not be empty" }
        var flags = 0
        if (meta) flags = flags or FLAG_META
        if (ctrl) flags = flags or FLAG_CTRL
        if (shift) flags = flags or FLAG_SHIFT
        if (alt) flags = flags or FLAG_ALT

        val out = ByteArray(HEADER_SIZE + keyBytes.size)
        out[0] = MAGIC_0
        out[1] = MAGIC_1
        out[2] = MAGIC_2
        out[3] = (flags and 0xFF).toByte()
        keyBytes.copyInto(out, destinationOffset = HEADER_SIZE)
        return out
    }

    fun decode(bytes: ByteArray): NveNk1KeyDown? {
        if (bytes.size < MIN_LEN) return null
        if (bytes[0] != MAGIC_0 || bytes[1] != MAGIC_1 || bytes[2] != MAGIC_2) return null
        val flags = bytes[3].toInt() and 0xFF
        val decoder = StandardCharsets.UTF_8
            .newDecoder()
            .onMalformedInput(CodingErrorAction.REPORT)
            .onUnmappableCharacter(CodingErrorAction.REPORT)
        val key = try {
            decoder.decode(ByteBuffer.wrap(bytes, HEADER_SIZE, bytes.size - HEADER_SIZE)).toString()
        } catch (_: CharacterCodingException) {
            return null
        }
        if (key.isEmpty()) return null
        return NveNk1KeyDown(
            key = key,
            meta = (flags and FLAG_META) != 0,
            ctrl = (flags and FLAG_CTRL) != 0,
            shift = (flags and FLAG_SHIFT) != 0,
            alt = (flags and FLAG_ALT) != 0,
        )
    }
}

/**
 * Standalone TV keydown event envelope serialized directly into the NVE `EventRing` binary wire
 * format (`NativeEventKind::Custom = 8`, `PayloadTag::Custom = 5`, name `"keydown"`).
 */
data class NveTvCustomEvent(
    val surfaceId: Int,
    val nodeId: Int,
    val timestampUs: Long,
    val name: String,
    val data: ByteArray,
) {
    override fun equals(other: Any?): Boolean {
        if (this === other) return true
        if (other !is NveTvCustomEvent) return false
        return surfaceId == other.surfaceId &&
            nodeId == other.nodeId &&
            timestampUs == other.timestampUs &&
            name == other.name &&
            data.contentEquals(other.data)
    }

    override fun hashCode(): Int {
        var result = surfaceId
        result = 31 * result + nodeId
        result = 31 * result + timestampUs.hashCode()
        result = 31 * result + name.hashCode()
        result = 31 * result + data.contentHashCode()
        return result
    }
}

/**
 * Minimal standalone `EventRing` custom-event wire encoder/decoder matching
 * `packages/core/abi/src/events.rs` byte-for-byte without depending on `hosts/compose-kit`.
 */
object NveTvEventRingWire {
    const val EVENT_KIND_CUSTOM: Int = 8
    const val PAYLOAD_TAG_CUSTOM: Int = 5
    private const val EVENT_HEADER_BYTES: Int = 18

    fun encodeSingle(event: NveTvCustomEvent): ByteArray = encode(listOf(event))

    fun encode(events: List<NveTvCustomEvent>): ByteArray {
        val out = ByteArrayOutputStream(4 + events.size * 36)
        writeI32Le(out, events.size)
        for (event in events) {
            writeI32Le(out, event.surfaceId)
            writeI32Le(out, event.nodeId)
            writeI64Le(out, event.timestampUs)
            out.write(EVENT_KIND_CUSTOM)
            out.write(PAYLOAD_TAG_CUSTOM)
            val nameBytes = event.name.toByteArray(StandardCharsets.UTF_8)
            require(nameBytes.size <= 0xFFFF) { "Custom event name exceeds uint16 limit" }
            writeU16Le(out, nameBytes.size)
            out.write(nameBytes)
            writeI32Le(out, event.data.size)
            out.write(event.data)
        }
        return out.toByteArray()
    }

    fun decode(bytes: ByteArray): List<NveTvCustomEvent> {
        if (bytes.isEmpty()) return emptyList()
        val buf = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
        require(buf.remaining() >= 4) { "Truncated EventRing header" }
        val count = buf.int
        require(count >= 0 && count.toLong() * EVENT_HEADER_BYTES <= buf.remaining().toLong()) {
            "Truncated EventRing body for $count events"
        }
        val out = ArrayList<NveTvCustomEvent>(count)
        repeat(count) {
            require(buf.remaining() >= EVENT_HEADER_BYTES) { "Truncated EventRing event header" }
            val surfaceId = buf.int
            val nodeId = buf.int
            val timestampUs = buf.long
            val kind = buf.get().toInt() and 0xFF
            val tag = buf.get().toInt() and 0xFF
            require(kind == EVENT_KIND_CUSTOM && tag == PAYLOAD_TAG_CUSTOM) {
                "Unexpected EventRing kind=$kind tag=$tag in TV custom ring"
            }
            require(buf.remaining() >= 2) { "Truncated custom name length" }
            val nameLen = buf.short.toInt() and 0xFFFF
            require(buf.remaining() >= nameLen) { "Truncated custom name bytes" }
            val nameBytes = ByteArray(nameLen)
            buf.get(nameBytes)
            val name = String(nameBytes, StandardCharsets.UTF_8)
            require(buf.remaining() >= 4) { "Truncated custom data length" }
            val dataLen = buf.int
            require(dataLen >= 0 && buf.remaining() >= dataLen) { "Truncated custom data bytes" }
            val data = ByteArray(dataLen)
            buf.get(data)
            out.add(NveTvCustomEvent(surfaceId, nodeId, timestampUs, name, data))
        }
        require(!buf.hasRemaining()) { "Trailing bytes in EventRing: ${buf.remaining()}" }
        return out
    }

    private fun writeU16Le(out: ByteArrayOutputStream, v: Int) {
        out.write(v and 0xFF)
        out.write((v ushr 8) and 0xFF)
    }

    private fun writeI32Le(out: ByteArrayOutputStream, v: Int) {
        out.write(v and 0xFF)
        out.write((v ushr 8) and 0xFF)
        out.write((v ushr 16) and 0xFF)
        out.write((v ushr 24) and 0xFF)
    }

    private fun writeI64Le(out: ByteArrayOutputStream, v: Long) {
        out.write((v and 0xFF).toInt())
        out.write(((v ushr 8) and 0xFF).toInt())
        out.write(((v ushr 16) and 0xFF).toInt())
        out.write(((v ushr 24) and 0xFF).toInt())
        out.write(((v ushr 32) and 0xFF).toInt())
        out.write(((v ushr 40) and 0xFF).toInt())
        out.write(((v ushr 48) and 0xFF).toInt())
        out.write(((v ushr 56) and 0xFF).toInt())
    }
}

/**
 * Android TV / Fire TV D-Pad & Leanback input bridge converting remote key events directly into
 * zero-JSON `NK1` binary payloads wrapped inside `EventRing` `CUSTOM` (`"keydown"`) events.
 */
class NveTvInputBridge(
    private val repeatThrottleUs: Long = DEFAULT_REPEAT_THROTTLE_US,
) {
    private val lastDispatchByAction = HashMap<NveTvAction, Long>()

    fun onKeyDown(
        surfaceId: Int,
        nodeId: Int,
        keyCode: Int,
        repeatCount: Int = 0,
        timestampUs: Long = 0L,
        meta: Boolean = false,
        ctrl: Boolean = false,
        shift: Boolean = false,
        alt: Boolean = false,
    ): NveTvCustomEvent? {
        val action = NveTvKeyMapper.mapKeyCode(keyCode) ?: return null
        if (repeatCount > 0 && repeatThrottleUs > 0L) {
            val prevUs = lastDispatchByAction[action]
            if (prevUs != null && timestampUs >= prevUs && (timestampUs - prevUs) < repeatThrottleUs) {
                return null
            }
        }
        lastDispatchByAction[action] = timestampUs
        return encodeAction(
            surfaceId = surfaceId,
            nodeId = nodeId,
            action = action,
            timestampUs = timestampUs,
            meta = meta,
            ctrl = ctrl,
            shift = shift,
            alt = alt,
        )
    }

    fun encodeAction(
        surfaceId: Int,
        nodeId: Int,
        action: NveTvAction,
        timestampUs: Long = 0L,
        meta: Boolean = false,
        ctrl: Boolean = false,
        shift: Boolean = false,
        alt: Boolean = false,
    ): NveTvCustomEvent {
        val nk1 = NveNk1Codec.encode(
            key = action.keyName,
            meta = meta,
            ctrl = ctrl,
            shift = shift,
            alt = alt,
        )
        return NveTvCustomEvent(
            surfaceId = surfaceId,
            nodeId = nodeId,
            timestampUs = timestampUs,
            name = NveNk1Codec.KEYDOWN_EVENT_NAME,
            data = nk1,
        )
    }

    fun encodeKeyDownToRing(
        surfaceId: Int,
        nodeId: Int,
        keyCode: Int,
        repeatCount: Int = 0,
        timestampUs: Long = 0L,
        meta: Boolean = false,
        ctrl: Boolean = false,
        shift: Boolean = false,
        alt: Boolean = false,
    ): ByteArray? {
        val event = onKeyDown(
            surfaceId = surfaceId,
            nodeId = nodeId,
            keyCode = keyCode,
            repeatCount = repeatCount,
            timestampUs = timestampUs,
            meta = meta,
            ctrl = ctrl,
            shift = shift,
            alt = alt,
        ) ?: return null
        return NveTvEventRingWire.encodeSingle(event)
    }

    fun encodeBatchToRing(events: List<NveTvCustomEvent>): ByteArray =
        NveTvEventRingWire.encode(events)

    fun resetThrottle() {
        lastDispatchByAction.clear()
    }

    companion object {
        const val DEFAULT_REPEAT_THROTTLE_US: Long = 100_000L
    }
}

/**
 * TV title-safe overscan insets (Android TV Leanback standard: 5% per edge => 48dp horizontal,
 * 27dp vertical on a 1080p / 960x540dp viewport).
 */
data class NveTvOverscanInsets(
    val topDp: Float,
    val rightDp: Float,
    val bottomDp: Float,
    val leftDp: Float,
) {
    fun safeWidthDp(viewportWidthDp: Float): Float =
        if (viewportWidthDp.isFinite()) (viewportWidthDp - leftDp - rightDp).coerceAtLeast(0f) else 0f

    fun safeHeightDp(viewportHeightDp: Float): Float =
        if (viewportHeightDp.isFinite()) (viewportHeightDp - topDp - bottomDp).coerceAtLeast(0f) else 0f

    companion object {
        const val DEFAULT_OVERSCAN_RATIO: Float = 0.05f

        val STANDARD_1080P: NveTvOverscanInsets = NveTvOverscanInsets(
            topDp = 27f,
            rightDp = 48f,
            bottomDp = 27f,
            leftDp = 48f,
        )

        fun forViewportDp(
            widthDp: Float,
            heightDp: Float,
            ratio: Float = DEFAULT_OVERSCAN_RATIO,
        ): NveTvOverscanInsets {
            val safeRatio = if (ratio.isFinite()) ratio.coerceIn(0f, 0.45f) else DEFAULT_OVERSCAN_RATIO
            val safeW = if (widthDp.isFinite() && widthDp > 0f) widthDp else 0f
            val safeH = if (heightDp.isFinite() && heightDp > 0f) heightDp else 0f
            val horizontal = safeW * safeRatio
            val vertical = safeH * safeRatio
            return NveTvOverscanInsets(
                topDp = vertical,
                rightDp = horizontal,
                bottomDp = vertical,
                leftDp = horizontal,
            )
        }

        fun forViewportPx(
            widthPx: Int,
            heightPx: Int,
            density: Float = 2.0f,
            ratio: Float = DEFAULT_OVERSCAN_RATIO,
        ): NveTvOverscanInsets {
            val safeDensity = if (density.isFinite() && density > 0f) density else 1.0f
            return forViewportDp(
                widthDp = widthPx.coerceAtLeast(0) / safeDensity,
                heightDp = heightPx.coerceAtLeast(0) / safeDensity,
                ratio = ratio,
            )
        }
    }
}

/**
 * Injectable environment capability signals for deterministic TV / Leanback / Fire TV detection.
 */
data class NveTvEnvironmentSignals(
    val hasLeanbackFeature: Boolean = false,
    val hasLeanbackOnlyFeature: Boolean = false,
    val uiMode: Int = 0,
    val manufacturer: String = "",
    val model: String = "",
)

/**
 * Detects Android TV / Google TV / Amazon Fire TV environments cleanly via Leanback system
 * features, `Configuration.UI_MODE_TYPE_TELEVISION`, or Fire TV hardware identifiers.
 */
object NveTvEnvironmentDetector {
    const val FEATURE_LEANBACK: String = "android.software.leanback"
    const val FEATURE_LEANBACK_ONLY: String = "android.software.leanback_only"
    const val UI_MODE_TYPE_MASK: Int = 0x0F
    const val UI_MODE_TYPE_TELEVISION: Int = 0x04

    fun isTelevisionUiMode(uiMode: Int): Boolean =
        (uiMode and UI_MODE_TYPE_MASK) == UI_MODE_TYPE_TELEVISION

    fun isAmazonFireTv(manufacturer: String, model: String): Boolean =
        manufacturer.equals("Amazon", ignoreCase = true) &&
            model.startsWith("AFT", ignoreCase = true)

    fun isTvEnvironment(signals: NveTvEnvironmentSignals): Boolean =
        signals.hasLeanbackFeature ||
            signals.hasLeanbackOnlyFeature ||
            isTelevisionUiMode(signals.uiMode) ||
            isAmazonFireTv(signals.manufacturer, signals.model)

    fun isTvDevice(context: Context): Boolean {
        val pm = context.packageManager
        val uiMode = context.resources.configuration.uiMode
        return isTvEnvironment(
            NveTvEnvironmentSignals(
                hasLeanbackFeature = pm.hasSystemFeature(FEATURE_LEANBACK),
                hasLeanbackOnlyFeature = pm.hasSystemFeature(FEATURE_LEANBACK_ONLY),
                uiMode = uiMode,
                manufacturer = android.os.Build.MANUFACTURER ?: "",
                model = android.os.Build.MODEL ?: "",
            ),
        )
    }
}
