package dev.nve.compose

import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction
import java.nio.charset.StandardCharsets

/**
 * Native event kind bridged over the NVE `NVEV` v2 wire format (`packages/core/abi/src/event/`).
 */
enum class NveEventKind(val code: Int) {
    PRESS(1),
    LONG_PRESS(2),
    SCROLL(3),
    TEXT_CHANGE(4),
    FOCUS(5),
    BLUR(6),
    LAYOUT(7),
    CUSTOM(8);

    companion object {
        fun fromCode(code: Int): NveEventKind =
            entries.firstOrNull { it.code == code } ?: CUSTOM

        fun fromCodeOrNull(code: Int): NveEventKind? =
            entries.firstOrNull { it.code == code }
    }
}

/**
 * Gesture arbitration outcome mirroring `GestureDisposition` in `packages/core/interop`.
 */
enum class NveGestureDisposition(val code: Int) {
    CLAIM(1),
    PASS_THROUGH(2),
    DEFER_TO_PARENT(3);

    companion object {
        fun fromCode(code: Int): NveGestureDisposition =
            entries.firstOrNull { it.code == code } ?: PASS_THROUGH
    }
}

/**
 * Strictly typed binary payload carried by an [NveEvent] in `NVEV` v2 wire format
 * (`packages/core/abi/src/event/`).
 */
sealed interface NveEventPayload {
    val tag: Int

    data object Empty : NveEventPayload {
        override val tag: Int = 0
    }

    data class Point(val x: Float, val y: Float) : NveEventPayload {
        override val tag: Int = 1
    }

    data class Scroll(
        val x: Float,
        val y: Float,
        val contentWidth: Float,
        val contentHeight: Float,
    ) : NveEventPayload {
        override val tag: Int = 2
    }

    data class Text(val text: String) : NveEventPayload {
        override val tag: Int = 3
    }

    data class Layout(
        val x: Float,
        val y: Float,
        val width: Float,
        val height: Float,
    ) : NveEventPayload {
        override val tag: Int = 4
    }

    data class Custom(
        val name: String,
        val data: ByteArray = ByteArray(0),
    ) : NveEventPayload {
        override val tag: Int = 5

        /**
         * Parses [data] as a zero-copy binary [NveBinValueView] when the emitter encoded
         * structured custom event data with `NveBinValue`.
         */
        fun parseBinValue(): NveBinValueView = NveBinValueView.parse(data)

        override fun equals(other: Any?): Boolean {
            if (this === other) return true
            if (other !is Custom) return false
            return name == other.name && data.contentEquals(other.data)
        }

        override fun hashCode(): Int {
            var result = name.hashCode()
            result = 31 * result + data.contentHashCode()
            return result
        }
    }
}

/**
 * Strict validation error thrown when decoding malformed or truncated `NVEV` v2 buffers.
 */
sealed class NveEventRingException(message: String) : IllegalArgumentException(message) {
    class Truncated(detail: String) : NveEventRingException("Truncated NVEV v2 buffer: $detail")
    class InvalidHeader(detail: String) : NveEventRingException("Invalid NVEV v2 header: $detail")
    class InvalidTable(val table: Long, val eventId: Int) :
        NveEventRingException("Invalid NVEV v2 table=0x${table.toULong().toString(16)} event=$eventId")
    class InvalidBounds(detail: String) : NveEventRingException("Invalid NVEV v2 arena bounds: $detail")
    class InvalidUtf8(field: String) : NveEventRingException("Invalid UTF-8 in NVEV v2 $field")
    class TrailingBytes(val remaining: Int) : NveEventRingException("NVEV v2 buffer has $remaining trailing bytes")
}

/**
 * Binary event crossing the host <-> Rust `NVEV` v2 boundary (`packages/core/abi/src/event/`).
 */
data class NveEvent(
    val surfaceId: Int,
    val nodeId: Int,
    val timestampUs: Long,
    val kind: NveEventKind,
    val payload: NveEventPayload = NveEventPayload.Empty,
) {
    val name: String?
        get() = (payload as? NveEventPayload.Custom)?.name

    val payloadBytes: ByteArray
        get() = when (payload) {
            is NveEventPayload.Custom -> payload.data
            is NveEventPayload.Text -> payload.text.toByteArray(StandardCharsets.UTF_8)
            else -> ByteArray(0)
        }
}

/**
 * Binary `NVEV` v2 event-batch encoder & decoder matching `packages/core/abi/src/event/wire.rs`
 * byte-for-byte.
 *
 * Wire layout (all integers little-endian):
 * - 14-byte header: `magic[4] = "NVEV"`, `u16 version = 2`, `u32 event_count`, `u32 bytes_len`
 * - `event_count * 64` bytes: uniform `NveEvent` records
 * - `bytes_len` bytes: contiguous payload byte arena
 */
object NveEventRingCodec {
    const val WIRE_HEADER_BYTES: Int = 14
    const val EVENT_RECORD_BYTES: Int = 64
    const val WIRE_VERSION: Int = 2

    val TABLE_INTERACTION: Long = 0x7F37FFD2CBF7C243uL.toLong()
    val TABLE_TEXT_INPUT: Long = 0xED1E95C480AF810DuL.toLong()
    val TABLE_SCROLL: Long = 0xBEF99001B482274AuL.toLong()
    val TABLE_LAYOUT: Long = 0xDB14E892BE101C33uL.toLong()
    val TABLE_CUSTOM: Long = 0x67CD76B58B37B058uL.toLong()

    const val INTERACTION_PRESS: Int = 0
    const val INTERACTION_LONG_PRESS: Int = 1
    const val INTERACTION_FOCUS: Int = 2
    const val INTERACTION_BLUR: Int = 3
    const val TEXT_INPUT_CHANGE: Int = 0
    const val SCROLL_SCROLL: Int = 0
    const val LAYOUT_LAYOUT: Int = 0

    const val FLAG_UTF8_BYTES: Int = 0x10000
    const val FLAG_INLINE_KEY_UTF8: Int = 0x20000
    const val FLAG_CUSTOM_NAMED: Int = 0x40000
    const val FLAG_KEY_SPLIT_BYTES: Int = 0x80000

    fun press(
        surfaceId: Int,
        nodeId: Int,
        x: Float,
        y: Float,
        timestampUs: Long = 0L,
    ): NveEvent = NveEvent(
        surfaceId = surfaceId,
        nodeId = nodeId,
        timestampUs = timestampUs,
        kind = NveEventKind.PRESS,
        payload = NveEventPayload.Point(x, y),
    )

    fun longPress(
        surfaceId: Int,
        nodeId: Int,
        x: Float,
        y: Float,
        timestampUs: Long = 0L,
    ): NveEvent = NveEvent(
        surfaceId = surfaceId,
        nodeId = nodeId,
        timestampUs = timestampUs,
        kind = NveEventKind.LONG_PRESS,
        payload = NveEventPayload.Point(x, y),
    )

    fun scroll(
        surfaceId: Int,
        nodeId: Int,
        x: Float,
        y: Float,
        contentWidth: Float,
        contentHeight: Float,
        timestampUs: Long = 0L,
    ): NveEvent = NveEvent(
        surfaceId = surfaceId,
        nodeId = nodeId,
        timestampUs = timestampUs,
        kind = NveEventKind.SCROLL,
        payload = NveEventPayload.Scroll(x, y, contentWidth, contentHeight),
    )

    fun textChange(
        surfaceId: Int,
        nodeId: Int,
        text: String,
        timestampUs: Long = 0L,
    ): NveEvent = NveEvent(
        surfaceId = surfaceId,
        nodeId = nodeId,
        timestampUs = timestampUs,
        kind = NveEventKind.TEXT_CHANGE,
        payload = NveEventPayload.Text(text),
    )

    fun layout(
        surfaceId: Int,
        nodeId: Int,
        x: Float,
        y: Float,
        width: Float,
        height: Float,
        timestampUs: Long = 0L,
    ): NveEvent = NveEvent(
        surfaceId = surfaceId,
        nodeId = nodeId,
        timestampUs = timestampUs,
        kind = NveEventKind.LAYOUT,
        payload = NveEventPayload.Layout(x, y, width, height),
    )

    fun custom(
        surfaceId: Int,
        nodeId: Int,
        name: String,
        data: ByteArray = ByteArray(0),
        timestampUs: Long = 0L,
    ): NveEvent = NveEvent(
        surfaceId = surfaceId,
        nodeId = nodeId,
        timestampUs = timestampUs,
        kind = NveEventKind.CUSTOM,
        payload = NveEventPayload.Custom(name, data),
    )

    fun encodeSingle(event: NveEvent): ByteArray = encode(listOf(event))

    /**
     * Zero-allocation `NVEV` v2 encoder writing little-endian bytes directly into [out]
     * (typically a native-owned `DirectByteBuffer` from [NveBridgeBackend.getEventScratchBuffer]).
     *
     * On success, flips [out] (`position = 0`, `limit = written`) and returns the exact byte count
     * written (byte-for-byte identical to [encode]).
     * Returns `-1` (leaving [out] cleared) if [out] is read-only or lacks capacity for the batch.
     */
    fun encodeInto(events: List<NveEvent>, out: ByteBuffer): Int {
        var arenaLen = 0L
        for (i in events.indices) {
            arenaLen += measureEventArenaBytes(events[i])
        }
        val totalLen = WIRE_HEADER_BYTES.toLong() + events.size.toLong() * EVENT_RECORD_BYTES + arenaLen
        if (out.isReadOnly || totalLen > out.capacity().toLong()) {
            if (!out.isReadOnly) out.clear()
            return -1
        }
        out.clear()
        out.order(ByteOrder.LITTLE_ENDIAN)
        writeHeader(out, events.size, arenaLen.toInt())
        var arenaOff = 0
        for (i in events.indices) {
            arenaOff += writeRecordInto(events[i], arenaOff, out)
        }
        for (i in events.indices) {
            writeArenaInto(events[i], out)
        }
        val written = out.position()
        out.flip()
        return written
    }

    /**
     * Zero-allocation single-event encoder writing directly into [out] without wrapping in a [List].
     * Returns bytes written (`>= 78`) or `-1` if [out] lacks capacity.
     */
    fun encodeSingleInto(event: NveEvent, out: ByteBuffer): Int {
        val arenaLen = measureEventArenaBytes(event)
        val totalLen = WIRE_HEADER_BYTES.toLong() + EVENT_RECORD_BYTES + arenaLen
        if (out.isReadOnly || totalLen > out.capacity().toLong()) {
            if (!out.isReadOnly) out.clear()
            return -1
        }
        out.clear()
        out.order(ByteOrder.LITTLE_ENDIAN)
        writeHeader(out, 1, arenaLen.toInt())
        writeRecordInto(event, 0, out)
        writeArenaInto(event, out)
        val written = out.position()
        out.flip()
        return written
    }

    private fun measureEventArenaBytes(event: NveEvent): Long = when (val payload = event.payload) {
        is NveEventPayload.Text -> utf8ByteLength(payload.text).toLong()
        is NveEventPayload.Custom -> {
            val nameLen = utf8ByteLength(payload.name)
            nameLen.toLong() + payload.data.size.toLong()
        }
        else -> 0L
    }

    private fun writeHeader(out: ByteBuffer, eventCount: Int, bytesLen: Int) {
        out.put(0x4E)
        out.put(0x56)
        out.put(0x45)
        out.put(0x56)
        out.putShort(WIRE_VERSION.toShort())
        out.putInt(eventCount)
        out.putInt(bytesLen)
    }

    private fun writeRecordInto(event: NveEvent, arenaOff: Int, out: ByteBuffer): Int {
        var table = when (event.kind) {
            NveEventKind.PRESS, NveEventKind.LONG_PRESS, NveEventKind.FOCUS, NveEventKind.BLUR -> TABLE_INTERACTION
            NveEventKind.TEXT_CHANGE -> TABLE_TEXT_INPUT
            NveEventKind.SCROLL -> TABLE_SCROLL
            NveEventKind.LAYOUT -> TABLE_LAYOUT
            NveEventKind.CUSTOM -> TABLE_CUSTOM
        }
        var eventId = when (event.kind) {
            NveEventKind.PRESS -> INTERACTION_PRESS
            NveEventKind.LONG_PRESS -> INTERACTION_LONG_PRESS
            NveEventKind.FOCUS -> INTERACTION_FOCUS
            NveEventKind.BLUR -> INTERACTION_BLUR
            NveEventKind.TEXT_CHANGE -> TEXT_INPUT_CHANGE
            NveEventKind.SCROLL -> SCROLL_SCROLL
            NveEventKind.LAYOUT -> LAYOUT_LAYOUT
            NveEventKind.CUSTOM -> 0
        }
        var flags = 0
        var p0 = 0
        var p1 = 0
        var p2 = 0
        var p3 = 0
        var bytesOff = 0
        var bytesLen = 0

        when (val payload = event.payload) {
            NveEventPayload.Empty -> Unit
            is NveEventPayload.Point -> {
                p0 = payload.x.toRawBits()
                p1 = payload.y.toRawBits()
            }
            is NveEventPayload.Scroll -> {
                p0 = payload.x.toRawBits()
                p1 = payload.y.toRawBits()
                p2 = payload.contentWidth.toRawBits()
                p3 = payload.contentHeight.toRawBits()
            }
            is NveEventPayload.Text -> {
                flags = FLAG_UTF8_BYTES
                bytesLen = utf8ByteLength(payload.text)
                bytesOff = if (bytesLen > 0) arenaOff else 0
            }
            is NveEventPayload.Layout -> {
                p0 = payload.x.toRawBits()
                p1 = payload.y.toRawBits()
                p2 = payload.width.toRawBits()
                p3 = payload.height.toRawBits()
            }
            is NveEventPayload.Custom -> {
                table = TABLE_CUSTOM
                eventId = 0
                flags = FLAG_CUSTOM_NAMED
                val nameLen = utf8ByteLength(payload.name)
                p0 = nameLen
                bytesLen = nameLen + payload.data.size
                bytesOff = if (bytesLen > 0) arenaOff else 0
            }
        }

        out.putLong(event.timestampUs)
        out.putLong(table)
        out.putInt(event.surfaceId)
        out.putInt(event.nodeId)
        out.putInt(eventId)
        out.putInt(flags)
        out.putInt(p0)
        out.putInt(p1)
        out.putInt(p2)
        out.putInt(p3)
        out.putInt(0)
        out.putInt(0)
        out.putInt(bytesOff)
        out.putInt(bytesLen)
        return bytesLen
    }

    private fun writeArenaInto(event: NveEvent, out: ByteBuffer) {
        when (val payload = event.payload) {
            is NveEventPayload.Text -> writeUtf8Into(payload.text, out)
            is NveEventPayload.Custom -> {
                writeUtf8Into(payload.name, out)
                if (payload.data.isNotEmpty()) {
                    out.put(payload.data)
                }
            }
            else -> Unit
        }
    }

    private fun utf8ByteLength(s: String): Int {
        val len = s.length
        var bytes = 0
        var i = 0
        while (i < len) {
            val c = s[i].code
            if (c < 0x80) {
                bytes += 1
                i += 1
            } else if (c < 0x800) {
                bytes += 2
                i += 1
            } else if (c in 0xD800..0xDBFF && i + 1 < len && s[i + 1].code in 0xDC00..0xDFFF) {
                bytes += 4
                i += 2
            } else if (c in 0xD800..0xDFFF) {
                // Unpaired surrogate matches Java String.getBytes(UTF_8) -> '?' (0x3F).
                bytes += 1
                i += 1
            } else {
                bytes += 3
                i += 1
            }
        }
        return bytes
    }

    private fun writeUtf8Into(s: String, out: ByteBuffer) {
        val len = s.length
        var i = 0
        while (i < len) {
            val c = s[i].code
            if (c < 0x80) {
                out.put(c.toByte())
                i += 1
            } else if (c < 0x800) {
                out.put((0xC0 or (c ushr 6)).toByte())
                out.put((0x80 or (c and 0x3F)).toByte())
                i += 1
            } else if (c in 0xD800..0xDBFF && i + 1 < len && s[i + 1].code in 0xDC00..0xDFFF) {
                val cp = 0x10000 + (((c - 0xD800) shl 10) or (s[i + 1].code - 0xDC00))
                out.put((0xF0 or (cp ushr 18)).toByte())
                out.put((0x80 or ((cp ushr 12) and 0x3F)).toByte())
                out.put((0x80 or ((cp ushr 6) and 0x3F)).toByte())
                out.put((0x80 or (cp and 0x3F)).toByte())
                i += 2
            } else if (c in 0xD800..0xDFFF) {
                out.put(0x3F.toByte())
                i += 1
            } else {
                out.put((0xE0 or (c ushr 12)).toByte())
                out.put((0x80 or ((c ushr 6) and 0x3F)).toByte())
                out.put((0x80 or (c and 0x3F)).toByte())
                i += 1
            }
        }
    }

    fun encode(events: List<NveEvent>): ByteArray {
        var arenaLen = 0L
        for (i in events.indices) {
            arenaLen += measureEventArenaBytes(events[i])
        }
        val totalLen = WIRE_HEADER_BYTES.toLong() + events.size.toLong() * EVENT_RECORD_BYTES + arenaLen
        require(totalLen <= Int.MAX_VALUE) { "Event batch exceeds ByteArray size limit: $totalLen" }
        val bytes = ByteArray(totalLen.toInt())
        val written = encodeInto(events, ByteBuffer.wrap(bytes))
        check(written == bytes.size) { "Expected $totalLen bytes written, got $written" }
        return bytes
    }

    fun decode(bytes: ByteArray): List<NveEvent> {
        if (bytes.isEmpty()) return emptyList()
        return decode(ByteBuffer.wrap(bytes))
    }

    fun decode(source: ByteBuffer): List<NveEvent> {
        val buf = source.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        if (!buf.hasRemaining()) return emptyList()
        if (buf.remaining() < WIRE_HEADER_BYTES) {
            throw NveEventRingException.Truncated("missing 14-byte NVEV v2 header")
        }
        val m0 = buf.get().toInt() and 0xFF
        val m1 = buf.get().toInt() and 0xFF
        val m2 = buf.get().toInt() and 0xFF
        val m3 = buf.get().toInt() and 0xFF
        if (m0 != 0x4E || m1 != 0x56 || m2 != 0x45 || m3 != 0x56) {
            throw NveEventRingException.InvalidHeader("invalid magic bytes")
        }
        val version = buf.short.toInt() and 0xFFFF
        if (version != WIRE_VERSION) {
            throw NveEventRingException.InvalidHeader("unsupported version $version")
        }
        val rawCount = buf.int.toLong() and 0xFFFF_FFFFL
        val rawArenaLen = buf.int.toLong() and 0xFFFF_FFFFL
        val bodyAvail = buf.remaining().toLong()
        if (rawCount > bodyAvail / EVENT_RECORD_BYTES) {
            throw NveEventRingException.Truncated(
                "declared $rawCount events require ${rawCount * EVENT_RECORD_BYTES} bytes, only $bodyAvail available",
            )
        }
        val recordsBytes = rawCount * EVENT_RECORD_BYTES
        val arenaAvail = bodyAvail - recordsBytes
        if (arenaAvail < rawArenaLen) {
            throw NveEventRingException.Truncated(
                "declared $rawArenaLen arena bytes, only $arenaAvail available",
            )
        }
        if (arenaAvail > rawArenaLen) {
            throw NveEventRingException.TrailingBytes((arenaAvail - rawArenaLen).toInt())
        }

        val count = rawCount.toInt()
        val arenaLen = rawArenaLen.toInt()
        val recordsStart = buf.position()
        val arenaStart = recordsStart + recordsBytes.toInt()
        val out = ArrayList<NveEvent>(count)

        fun readArenaSlice(off: Long, len: Long): ByteArray {
            if (off < 0L || len < 0L || off > arenaLen.toLong() || len > arenaLen.toLong() - off) {
                throw NveEventRingException.InvalidBounds("arena slice [$off..+${len}] exceeds $arenaLen")
            }
            val arr = ByteArray(len.toInt())
            if (arr.isNotEmpty()) {
                val slice = buf.duplicate()
                slice.position(arenaStart + off.toInt())
                slice.get(arr)
            }
            return arr
        }

        for (i in 0 until count) {
            buf.position(recordsStart + i * EVENT_RECORD_BYTES)
            val timestampUs = buf.long
            val table = buf.long
            val surfaceId = buf.int
            val nodeId = buf.int
            val eventId = buf.int
            val flags = buf.int
            val p0 = buf.int
            val p1 = buf.int
            val p2 = buf.int
            val p3 = buf.int
            buf.int // p4
            buf.int // p5
            val bytesOff = buf.int.toLong() and 0xFFFF_FFFFL
            val bytesLen = buf.int.toLong() and 0xFFFF_FFFFL
            val slice = readArenaSlice(bytesOff, bytesLen)

            val hasUtf8 = (flags and FLAG_UTF8_BYTES) != 0
            val hasInlineKey = (flags and FLAG_INLINE_KEY_UTF8) != 0
            val hasSplitKey = (flags and FLAG_KEY_SPLIT_BYTES) != 0
            val hasCustom = (flags and FLAG_CUSTOM_NAMED) != 0
            if ((hasInlineKey && hasSplitKey) || (hasCustom && (hasInlineKey || hasSplitKey))) {
                throw NveEventRingException.InvalidBounds("conflicting framing flags 0x${flags.toUInt().toString(16)}")
            }
            if (hasUtf8) {
                decodeUtf8Strict(slice, "utf8 payload")
            }

            val kind: NveEventKind
            val payload: NveEventPayload
            when (table) {
                TABLE_CUSTOM -> {
                    kind = NveEventKind.CUSTOM
                    val nameLen = p0.toLong() and 0xFFFF_FFFFL
                    if (nameLen > slice.size.toLong()) {
                        throw NveEventRingException.InvalidBounds("custom name len $nameLen > ${slice.size}")
                    }
                    val nlen = nameLen.toInt()
                    val name = decodeUtf8Strict(slice.copyOfRange(0, nlen), "custom name")
                    val data = if (nlen < slice.size) slice.copyOfRange(nlen, slice.size) else ByteArray(0)
                    payload = NveEventPayload.Custom(name, data)
                }
                TABLE_INTERACTION -> when (eventId) {
                    INTERACTION_PRESS -> {
                        kind = NveEventKind.PRESS
                        payload = NveEventPayload.Point(Float.fromBits(p0), Float.fromBits(p1))
                    }
                    INTERACTION_LONG_PRESS -> {
                        kind = NveEventKind.LONG_PRESS
                        payload = NveEventPayload.Point(Float.fromBits(p0), Float.fromBits(p1))
                    }
                    INTERACTION_FOCUS -> {
                        kind = NveEventKind.FOCUS
                        payload = NveEventPayload.Empty
                    }
                    INTERACTION_BLUR -> {
                        kind = NveEventKind.BLUR
                        payload = NveEventPayload.Empty
                    }
                    else -> throw NveEventRingException.InvalidTable(table, eventId)
                }
                TABLE_TEXT_INPUT -> {
                    if (eventId != TEXT_INPUT_CHANGE) {
                        throw NveEventRingException.InvalidTable(table, eventId)
                    }
                    kind = NveEventKind.TEXT_CHANGE
                    payload = NveEventPayload.Text(decodeUtf8Strict(slice, "text"))
                }
                TABLE_SCROLL -> {
                    if (eventId != SCROLL_SCROLL) {
                        throw NveEventRingException.InvalidTable(table, eventId)
                    }
                    kind = NveEventKind.SCROLL
                    payload = NveEventPayload.Scroll(
                        Float.fromBits(p0),
                        Float.fromBits(p1),
                        Float.fromBits(p2),
                        Float.fromBits(p3),
                    )
                }
                TABLE_LAYOUT -> {
                    if (eventId != LAYOUT_LAYOUT) {
                        throw NveEventRingException.InvalidTable(table, eventId)
                    }
                    kind = NveEventKind.LAYOUT
                    payload = NveEventPayload.Layout(
                        Float.fromBits(p0),
                        Float.fromBits(p1),
                        Float.fromBits(p2),
                        Float.fromBits(p3),
                    )
                }
                else -> throw NveEventRingException.InvalidTable(table, eventId)
            }

            out.add(
                NveEvent(
                    surfaceId = surfaceId,
                    nodeId = nodeId,
                    timestampUs = timestampUs,
                    kind = kind,
                    payload = payload,
                ),
            )
        }
        return out
    }

    private fun decodeUtf8Strict(bytes: ByteArray, field: String): String {
        if (bytes.isEmpty()) return ""
        val decoder = StandardCharsets.UTF_8.newDecoder()
            .onMalformedInput(CodingErrorAction.REPORT)
            .onUnmappableCharacter(CodingErrorAction.REPORT)
        return try {
            decoder.decode(ByteBuffer.wrap(bytes)).toString()
        } catch (_: CharacterCodingException) {
            throw NveEventRingException.InvalidUtf8(field)
        }
    }
}
