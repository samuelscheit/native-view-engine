package dev.nve.tv

import android.content.Context
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.StandardCharsets

/**
 * Typed TV keydown event matching `NativeEventKind::KeyDown = 12` / `PayloadTag::Key = 6`.
 */
data class NveTvKeyEvent(
    val surfaceId: Int,
    val nodeId: Int,
    val timestampUs: Long,
    val key: String,
    val code: String = "",
    val keyCode: Int = 0,
    val modifiers: Int = 0,
) {
    val meta: Boolean get() = (modifiers and NveTvEventRingWire.KEY_MOD_META) != 0
    val ctrl: Boolean get() = (modifiers and NveTvEventRingWire.KEY_MOD_CTRL) != 0
    val shift: Boolean get() = (modifiers and NveTvEventRingWire.KEY_MOD_SHIFT) != 0
    val alt: Boolean get() = (modifiers and NveTvEventRingWire.KEY_MOD_ALT) != 0
    val repeat: Boolean get() = (modifiers and NveTvEventRingWire.KEY_MOD_REPEAT) != 0
}

/**
 * Minimal standalone `EventRing` typed `KeyDown` wire encoder/decoder matching
 * `packages/core/abi/src/events.rs` byte-for-byte without depending on `hosts/compose-kit`.
 */
object NveTvEventRingWire {
    const val EVENT_KIND_KEY_DOWN: Int = 12
    const val PAYLOAD_TAG_KEY: Int = 6
    const val KEY_MOD_META: Int = 1 shl 0
    const val KEY_MOD_CTRL: Int = 1 shl 1
    const val KEY_MOD_SHIFT: Int = 1 shl 2
    const val KEY_MOD_ALT: Int = 1 shl 3
    const val KEY_MOD_REPEAT: Int = 1 shl 4
    private const val EVENT_HEADER_BYTES: Int = 18

    fun packModifiers(
        meta: Boolean = false,
        ctrl: Boolean = false,
        shift: Boolean = false,
        alt: Boolean = false,
        repeat: Boolean = false,
    ): Int {
        var bits = 0
        if (meta) bits = bits or KEY_MOD_META
        if (ctrl) bits = bits or KEY_MOD_CTRL
        if (shift) bits = bits or KEY_MOD_SHIFT
        if (alt) bits = bits or KEY_MOD_ALT
        if (repeat) bits = bits or KEY_MOD_REPEAT
        return bits
    }

    fun encodeSingle(event: NveTvKeyEvent): ByteArray = encode(listOf(event))

    fun encode(events: List<NveTvKeyEvent>): ByteArray {
        val out = ByteArrayOutputStream(4 + events.size * 36)
        writeI32Le(out, events.size)
        for (event in events) {
            val keyBytes = event.key.toByteArray(StandardCharsets.UTF_8)
            val codeBytes = event.code.toByteArray(StandardCharsets.UTF_8)
            require(keyBytes.size <= 0xFFFF) { "Key name exceeds uint16 limit" }
            require(codeBytes.size <= 0xFFFF) { "Key code exceeds uint16 limit" }
            writeI32Le(out, event.surfaceId)
            writeI32Le(out, event.nodeId)
            writeI64Le(out, event.timestampUs)
            out.write(EVENT_KIND_KEY_DOWN)
            out.write(PAYLOAD_TAG_KEY)
            out.write(event.modifiers and 0xFF)
            writeI32Le(out, event.keyCode)
            writeU16Le(out, keyBytes.size)
            out.write(keyBytes)
            writeU16Le(out, codeBytes.size)
            out.write(codeBytes)
        }
        return out.toByteArray()
    }

    fun decode(bytes: ByteArray): List<NveTvKeyEvent> {
        if (bytes.isEmpty()) return emptyList()
        val buf = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
        require(buf.remaining() >= 4) { "Truncated EventRing header" }
        val count = buf.int
        require(count >= 0 && count.toLong() * EVENT_HEADER_BYTES <= buf.remaining().toLong()) {
            "Truncated EventRing body for $count events"
        }
        val out = ArrayList<NveTvKeyEvent>(count)
        repeat(count) {
            require(buf.remaining() >= EVENT_HEADER_BYTES) { "Truncated EventRing event header" }
            val surfaceId = buf.int
            val nodeId = buf.int
            val timestampUs = buf.long
            val kind = buf.get().toInt() and 0xFF
            val tag = buf.get().toInt() and 0xFF
            require(kind == EVENT_KIND_KEY_DOWN && tag == PAYLOAD_TAG_KEY) {
                "Unexpected EventRing kind=$kind tag=$tag in TV keydown ring"
            }
            require(buf.remaining() >= 7) { "Truncated key payload header" }
            val modifiers = buf.get().toInt() and 0xFF
            val keyCode = buf.int
            val keyLen = buf.short.toInt() and 0xFFFF
            require(buf.remaining() >= keyLen + 2) { "Truncated key string bytes" }
            val keyBytes = ByteArray(keyLen)
            buf.get(keyBytes)
            val key = String(keyBytes, StandardCharsets.UTF_8)
            val codeLen = buf.short.toInt() and 0xFFFF
            require(buf.remaining() >= codeLen) { "Truncated key code bytes" }
            val codeBytes = ByteArray(codeLen)
            buf.get(codeBytes)
            val code = String(codeBytes, StandardCharsets.UTF_8)
            out.add(NveTvKeyEvent(surfaceId, nodeId, timestampUs, key, code, keyCode, modifiers))
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
 * typed `KeyDown` (`NativeEventKind::KeyDown = 12`, `PayloadTag::Key = 6`) events.
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
    ): NveTvKeyEvent? {
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
            keyCode = keyCode,
            timestampUs = timestampUs,
            meta = meta,
            ctrl = ctrl,
            shift = shift,
            alt = alt,
            repeat = repeatCount > 0,
        )
    }

    fun encodeAction(
        surfaceId: Int,
        nodeId: Int,
        action: NveTvAction,
        keyCode: Int = 0,
        timestampUs: Long = 0L,
        meta: Boolean = false,
        ctrl: Boolean = false,
        shift: Boolean = false,
        alt: Boolean = false,
        repeat: Boolean = false,
    ): NveTvKeyEvent = NveTvKeyEvent(
        surfaceId = surfaceId,
        nodeId = nodeId,
        timestampUs = timestampUs,
        key = action.keyName,
        code = action.keyName,
        keyCode = keyCode,
        modifiers = NveTvEventRingWire.packModifiers(meta, ctrl, shift, alt, repeat),
    )

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

    fun encodeBatchToRing(events: List<NveTvKeyEvent>): ByteArray =
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
