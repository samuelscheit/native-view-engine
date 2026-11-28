package dev.nve.tv

import android.content.Context
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction
import java.nio.charset.StandardCharsets

/**
 * Typed TV keydown event matching `InteractionEvent::KeyDown` (`NVE_TABLE_INTERACTION`, event 4).
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
 * Minimal standalone `NVEV` v2 typed `KeyDown` wire encoder/decoder matching
 * `packages/core/abi/src/event/wire.rs` byte-for-byte without depending on `hosts/compose-kit`.
 */
object NveTvEventRingWire {
    val TABLE_INTERACTION: Long = 0x7F37FFD2CBF7C243uL.toLong()
    const val EVENT_ID_KEY_DOWN: Int = 4
    const val INLINE_KEY_CAP: Int = 16
    const val KEY_MOD_META: Int = 1 shl 0
    const val KEY_MOD_CTRL: Int = 1 shl 1
    const val KEY_MOD_SHIFT: Int = 1 shl 2
    const val KEY_MOD_ALT: Int = 1 shl 3
    const val KEY_MOD_REPEAT: Int = 1 shl 4
    const val FLAG_UTF8_BYTES: Int = 0x10000
    const val FLAG_INLINE_KEY_UTF8: Int = 0x20000
    const val FLAG_KEY_SPLIT_BYTES: Int = 0x80000
    const val WIRE_HEADER_BYTES: Int = 14
    const val EVENT_RECORD_BYTES: Int = 64
    const val WIRE_VERSION: Int = 2

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
        val keyList = ArrayList<ByteArray>(events.size)
        val codeList = ArrayList<ByteArray>(events.size)
        var arenaLen = 0
        for (event in events) {
            val keyBytes = event.key.toByteArray(StandardCharsets.UTF_8)
            val codeBytes = event.code.toByteArray(StandardCharsets.UTF_8)
            val added = if (keyBytes.size <= INLINE_KEY_CAP) codeBytes.size else (keyBytes.size + codeBytes.size)
            require(added >= 0 && arenaLen <= Int.MAX_VALUE - added) { "TV event arena exceeds Int.MAX_VALUE" }
            keyList.add(keyBytes)
            codeList.add(codeBytes)
            arenaLen += added
        }
        val out = ByteArray(WIRE_HEADER_BYTES + events.size * EVENT_RECORD_BYTES + arenaLen)
        val buf = ByteBuffer.wrap(out).order(ByteOrder.LITTLE_ENDIAN)
        buf.put(0x4E).put(0x56).put(0x45).put(0x56)
        buf.putShort(WIRE_VERSION.toShort())
        buf.putInt(events.size)
        buf.putInt(arenaLen)

        var arenaOff = 0
        for (i in events.indices) {
            val event = events[i]
            val keyBytes = keyList[i]
            val codeBytes = codeList[i]
            var flags = event.modifiers and 0x1F
            var p2 = 0
            var p3 = 0
            var p4 = 0
            var p5 = 0
            val bytesOff: Int
            val bytesLen: Int
            if (keyBytes.size <= INLINE_KEY_CAP) {
                val kb = ByteArray(INLINE_KEY_CAP)
                System.arraycopy(keyBytes, 0, kb, 0, keyBytes.size)
                val kbb = ByteBuffer.wrap(kb).order(ByteOrder.LITTLE_ENDIAN)
                p2 = kbb.int
                p3 = kbb.int
                p4 = kbb.int
                p5 = kbb.int
                flags = flags or FLAG_INLINE_KEY_UTF8
                if (codeBytes.isNotEmpty()) flags = flags or FLAG_UTF8_BYTES
                bytesLen = codeBytes.size
                bytesOff = if (bytesLen > 0) arenaOff else 0
            } else {
                flags = flags or FLAG_KEY_SPLIT_BYTES or FLAG_UTF8_BYTES
                bytesLen = keyBytes.size + codeBytes.size
                bytesOff = arenaOff
            }
            arenaOff += bytesLen
            buf.putLong(event.timestampUs)
            buf.putLong(TABLE_INTERACTION)
            buf.putInt(event.surfaceId)
            buf.putInt(event.nodeId)
            buf.putInt(EVENT_ID_KEY_DOWN)
            buf.putInt(flags)
            buf.putInt(event.keyCode)
            buf.putInt(keyBytes.size)
            buf.putInt(p2)
            buf.putInt(p3)
            buf.putInt(p4)
            buf.putInt(p5)
            buf.putInt(bytesOff)
            buf.putInt(bytesLen)
        }
        for (i in events.indices) {
            val keyBytes = keyList[i]
            val codeBytes = codeList[i]
            if (keyBytes.size > INLINE_KEY_CAP && keyBytes.isNotEmpty()) {
                buf.put(keyBytes)
            }
            if (codeBytes.isNotEmpty()) {
                buf.put(codeBytes)
            }
        }
        return out
    }

    fun decode(bytes: ByteArray): List<NveTvKeyEvent> {
        if (bytes.isEmpty()) return emptyList()
        val buf = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
        require(buf.remaining() >= WIRE_HEADER_BYTES) { "Truncated NVEV v2 header" }
        require(
            buf.get().toInt() == 0x4E &&
                buf.get().toInt() == 0x56 &&
                buf.get().toInt() == 0x45 &&
                buf.get().toInt() == 0x56,
        ) { "Invalid NVEV v2 magic" }
        val version = buf.short.toInt() and 0xFFFF
        require(version == WIRE_VERSION) { "Unsupported NVEV version $version" }
        val count = buf.int
        val arenaLen = buf.int
        require(count >= 0 && arenaLen >= 0) { "Negative header counts" }
        val recordsBytes = count.toLong() * EVENT_RECORD_BYTES
        require(recordsBytes + arenaLen.toLong() == buf.remaining().toLong()) {
            "NVEV v2 size mismatch for $count events and $arenaLen arena bytes"
        }
        val arenaStart = WIRE_HEADER_BYTES + recordsBytes.toInt()
        val out = ArrayList<NveTvKeyEvent>(count)
        for (i in 0 until count) {
            buf.position(WIRE_HEADER_BYTES + i * EVENT_RECORD_BYTES)
            val timestampUs = buf.long
            val table = buf.long
            val surfaceId = buf.int
            val nodeId = buf.int
            val eventId = buf.int
            val flags = buf.int
            require(table == TABLE_INTERACTION && eventId == EVENT_ID_KEY_DOWN) {
                "Unexpected NVEV v2 table=$table event=$eventId in TV keydown batch"
            }
            val keyCode = buf.int
            val keyLen = buf.int
            val p2 = buf.int
            val p3 = buf.int
            val p4 = buf.int
            val p5 = buf.int
            val bytesOff = buf.int
            val bytesLen = buf.int
            require(bytesOff >= 0 && bytesLen >= 0 && bytesOff <= arenaLen && bytesLen <= arenaLen - bytesOff) {
                "Out-of-bounds arena slice"
            }
            val hasInlineKey = (flags and FLAG_INLINE_KEY_UTF8) != 0
            val hasSplitKey = (flags and FLAG_KEY_SPLIT_BYTES) != 0
            require(!(hasInlineKey && hasSplitKey)) { "Conflicting inline + split key flags" }
            val key: String
            val code: String
            if (hasInlineKey) {
                require(keyLen in 0..INLINE_KEY_CAP) { "Invalid inline key length $keyLen" }
                val kb = ByteBuffer.allocate(INLINE_KEY_CAP).order(ByteOrder.LITTLE_ENDIAN)
                    .putInt(p2).putInt(p3).putInt(p4).putInt(p5).array()
                key = decodeUtf8Strict(kb, 0, keyLen, "inline key")
                code = decodeUtf8Strict(bytes, arenaStart + bytesOff, bytesLen, "key code")
            } else if (hasSplitKey) {
                require(keyLen in 0..bytesLen) { "Invalid split key length $keyLen" }
                key = decodeUtf8Strict(bytes, arenaStart + bytesOff, keyLen, "split key")
                code = decodeUtf8Strict(bytes, arenaStart + bytesOff + keyLen, bytesLen - keyLen, "split code")
            } else {
                key = decodeUtf8Strict(bytes, arenaStart + bytesOff, bytesLen, "key")
                code = ""
            }
            out.add(NveTvKeyEvent(surfaceId, nodeId, timestampUs, key, code, keyCode, flags and 0x1F))
        }
        return out
    }

    private fun decodeUtf8Strict(bytes: ByteArray, offset: Int, len: Int, field: String): String {
        if (len == 0) return ""
        val decoder = StandardCharsets.UTF_8.newDecoder()
            .onMalformedInput(CodingErrorAction.REPORT)
            .onUnmappableCharacter(CodingErrorAction.REPORT)
        return try {
            decoder.decode(ByteBuffer.wrap(bytes, offset, len)).toString()
        } catch (e: CharacterCodingException) {
            throw IllegalArgumentException("Invalid UTF-8 in TV $field", e)
        }
    }
}

/**
 * Android TV / Fire TV D-Pad & Leanback input bridge converting remote key events directly into
 * typed `InteractionEvent::KeyDown` events.
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
