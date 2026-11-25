package dev.nve.compose

import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction
import java.nio.charset.StandardCharsets

/**
 * Native event discriminator matching `NativeEventKind` in `packages/core/abi/src/events.rs`.
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
 * Strictly typed binary payload carried by an [NveEvent] in `EventRing` wire format
 * (`packages/core/abi/src/events.rs`), with zero JSON conversion.
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
 * Strict validation error thrown when decoding malformed or truncated `EventRing` buffers.
 */
sealed class NveEventRingException(message: String) : IllegalArgumentException(message) {
    class Truncated(detail: String) : NveEventRingException("Truncated EventRing: $detail")
    class InvalidKind(val code: Int) : NveEventRingException("Invalid EventRing event kind: $code")
    class InvalidPayloadTag(val tag: Int) : NveEventRingException("Invalid EventRing payload tag: $tag")
    class InvalidUtf8(field: String) : NveEventRingException("Invalid UTF-8 in EventRing $field")
    class TrailingBytes(val remaining: Int) : NveEventRingException("EventRing has $remaining trailing bytes")
}

/**
 * Binary event crossing the host <-> Rust `EventRing` boundary (`packages/core/abi/src/events.rs`).
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
 * Binary `EventRing` encoder & decoder matching `packages/core/abi/src/events.rs` byte-for-byte.
 *
 * Wire layout (all integers little-endian):
 * - Header: `u32 event_count`
 * - Per event (18-byte fixed header + variable payload):
 *   - `u32 surface_id`
 *   - `u32 node_id`
 *   - `u64 timestamp_us`
 *   - `u8 kind` (1..8)
 *   - `u8 payload_tag` (0=Empty, 1=Point, 2=Scroll, 3=Text, 4=Layout, 5=Custom)
 */
object NveEventRingCodec {
    const val EVENT_HEADER_BYTES: Int = 18

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
     * Zero-allocation `EventRing` encoder writing little-endian bytes directly into [out]
     * (typically a native-owned `DirectByteBuffer` from [NveBridgeBackend.getEventScratchBuffer]).
     *
     * On success, flips [out] (`position = 0`, `limit = written`) and returns the exact byte count
     * written (byte-for-byte identical to [encode]).
     * Returns `-1` (leaving [out] cleared) if [out] is read-only or lacks capacity for the batch,
     * allowing the caller to fall back cleanly to [encode] (`ByteArray`).
     */
    fun encodeInto(events: List<NveEvent>, out: ByteBuffer): Int {
        if (out.isReadOnly || out.capacity() < 4) return -1
        out.clear()
        out.order(ByteOrder.LITTLE_ENDIAN)
        out.putInt(events.size)
        for (i in events.indices) {
            if (!writeEventInto(events[i], out)) {
                out.clear()
                return -1
            }
        }
        val written = out.position()
        out.flip()
        return written
    }

    /**
     * Zero-allocation single-event encoder writing directly into [out] without wrapping in a [List].
     * Returns bytes written (`>= 22`) or `-1` if [out] lacks capacity.
     */
    fun encodeSingleInto(event: NveEvent, out: ByteBuffer): Int {
        if (out.isReadOnly || out.capacity() < 4 + EVENT_HEADER_BYTES) return -1
        out.clear()
        out.order(ByteOrder.LITTLE_ENDIAN)
        out.putInt(1)
        if (!writeEventInto(event, out)) {
            out.clear()
            return -1
        }
        val written = out.position()
        out.flip()
        return written
    }

    private fun writeEventInto(event: NveEvent, out: ByteBuffer): Boolean {
        if (out.remaining() < EVENT_HEADER_BYTES) return false
        out.putInt(event.surfaceId)
        out.putInt(event.nodeId)
        out.putLong(event.timestampUs)
        out.put((event.kind.code and 0xFF).toByte())
        val payload = event.payload
        out.put((payload.tag and 0xFF).toByte())
        return when (payload) {
            NveEventPayload.Empty -> true
            is NveEventPayload.Point -> {
                if (out.remaining() < 8) return false
                out.putInt(payload.x.toRawBits())
                out.putInt(payload.y.toRawBits())
                true
            }
            is NveEventPayload.Scroll -> {
                if (out.remaining() < 16) return false
                out.putInt(payload.x.toRawBits())
                out.putInt(payload.y.toRawBits())
                out.putInt(payload.contentWidth.toRawBits())
                out.putInt(payload.contentHeight.toRawBits())
                true
            }
            is NveEventPayload.Text -> {
                val utf8Len = utf8ByteLength(payload.text)
                if (out.remaining() < 4 + utf8Len) return false
                out.putInt(utf8Len)
                writeUtf8Into(payload.text, out)
                true
            }
            is NveEventPayload.Layout -> {
                if (out.remaining() < 16) return false
                out.putInt(payload.x.toRawBits())
                out.putInt(payload.y.toRawBits())
                out.putInt(payload.width.toRawBits())
                out.putInt(payload.height.toRawBits())
                true
            }
            is NveEventPayload.Custom -> {
                val nameLen = utf8ByteLength(payload.name)
                require(nameLen <= 0xFFFF) {
                    "Custom event name exceeds uint16 length limit: $nameLen"
                }
                val dataLen = payload.data.size
                val needed = 2L + nameLen.toLong() + 4L + dataLen.toLong()
                if (out.remaining().toLong() < needed) return false
                out.putShort((nameLen and 0xFFFF).toShort())
                writeUtf8Into(payload.name, out)
                out.putInt(dataLen)
                if (dataLen > 0) {
                    out.put(payload.data)
                }
                true
            }
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
        val out = ByteArrayOutputStream(4 + events.size * 32)
        writeI32Le(out, events.size)
        for (event in events) {
            writeI32Le(out, event.surfaceId)
            writeI32Le(out, event.nodeId)
            writeI64Le(out, event.timestampUs)
            out.write(event.kind.code and 0xFF)
            val payload = event.payload
            out.write(payload.tag and 0xFF)
            when (payload) {
                NveEventPayload.Empty -> Unit
                is NveEventPayload.Point -> {
                    writeF32Le(out, payload.x)
                    writeF32Le(out, payload.y)
                }
                is NveEventPayload.Scroll -> {
                    writeF32Le(out, payload.x)
                    writeF32Le(out, payload.y)
                    writeF32Le(out, payload.contentWidth)
                    writeF32Le(out, payload.contentHeight)
                }
                is NveEventPayload.Text -> {
                    val bytes = payload.text.toByteArray(StandardCharsets.UTF_8)
                    writeI32Le(out, bytes.size)
                    out.write(bytes)
                }
                is NveEventPayload.Layout -> {
                    writeF32Le(out, payload.x)
                    writeF32Le(out, payload.y)
                    writeF32Le(out, payload.width)
                    writeF32Le(out, payload.height)
                }
                is NveEventPayload.Custom -> {
                    val nameBytes = payload.name.toByteArray(StandardCharsets.UTF_8)
                    require(nameBytes.size <= 0xFFFF) {
                        "Custom event name exceeds uint16 length limit: ${nameBytes.size}"
                    }
                    writeU16Le(out, nameBytes.size)
                    out.write(nameBytes)
                    writeI32Le(out, payload.data.size)
                    out.write(payload.data)
                }
            }
        }
        return out.toByteArray()
    }

    fun decode(bytes: ByteArray): List<NveEvent> {
        if (bytes.isEmpty()) return emptyList()
        return decode(ByteBuffer.wrap(bytes))
    }

    fun decode(source: ByteBuffer): List<NveEvent> {
        val buf = source.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        if (!buf.hasRemaining()) return emptyList()
        if (buf.remaining() < 4) {
            throw NveEventRingException.Truncated("missing 4-byte event count")
        }
        val rawCount = buf.int.toLong() and 0xFFFF_FFFFL
        if (rawCount * EVENT_HEADER_BYTES > buf.remaining().toLong()) {
            throw NveEventRingException.Truncated(
                "declared $rawCount events require at least ${rawCount * EVENT_HEADER_BYTES} bytes, " +
                    "only ${buf.remaining()} available",
            )
        }
        val count = rawCount.toInt()
        val out = ArrayList<NveEvent>(count)
        repeat(count) {
            if (buf.remaining() < EVENT_HEADER_BYTES) {
                throw NveEventRingException.Truncated("missing 18-byte event header")
            }
            val surfaceId = buf.int
            val nodeId = buf.int
            val timestampUs = buf.long
            val kindCode = buf.get().toInt() and 0xFF
            val kind = NveEventKind.fromCodeOrNull(kindCode)
                ?: throw NveEventRingException.InvalidKind(kindCode)
            val payloadTag = buf.get().toInt() and 0xFF

            val payload: NveEventPayload = when (payloadTag) {
                0 -> NveEventPayload.Empty
                1 -> {
                    requireRemaining(buf, 8, "Point payload")
                    NveEventPayload.Point(buf.float, buf.float)
                }
                2 -> {
                    requireRemaining(buf, 16, "Scroll payload")
                    NveEventPayload.Scroll(buf.float, buf.float, buf.float, buf.float)
                }
                3 -> {
                    requireRemaining(buf, 4, "Text length")
                    val len = readU32Length(buf, "Text payload")
                    val textBytes = ByteArray(len)
                    buf.get(textBytes)
                    NveEventPayload.Text(decodeUtf8Strict(textBytes, "text"))
                }
                4 -> {
                    requireRemaining(buf, 16, "Layout payload")
                    NveEventPayload.Layout(buf.float, buf.float, buf.float, buf.float)
                }
                5 -> {
                    requireRemaining(buf, 2, "Custom name length")
                    val nameLen = buf.short.toInt() and 0xFFFF
                    requireRemaining(buf, nameLen, "Custom name bytes")
                    val nameBytes = ByteArray(nameLen)
                    buf.get(nameBytes)
                    val name = decodeUtf8Strict(nameBytes, "custom name")
                    requireRemaining(buf, 4, "Custom data length")
                    val dataLen = readU32Length(buf, "Custom data payload")
                    val data = ByteArray(dataLen)
                    buf.get(data)
                    NveEventPayload.Custom(name, data)
                }
                else -> throw NveEventRingException.InvalidPayloadTag(payloadTag)
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
        if (buf.hasRemaining()) {
            throw NveEventRingException.TrailingBytes(buf.remaining())
        }
        return out
    }

    private fun requireRemaining(buf: ByteBuffer, needed: Int, detail: String) {
        if (buf.remaining() < needed) {
            throw NveEventRingException.Truncated("$detail requires $needed bytes, remaining=${buf.remaining()}")
        }
    }

    private fun readU32Length(buf: ByteBuffer, detail: String): Int {
        val len = buf.int.toLong() and 0xFFFF_FFFFL
        if (len > buf.remaining().toLong()) {
            throw NveEventRingException.Truncated("$detail declares $len bytes, remaining=${buf.remaining()}")
        }
        return len.toInt()
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

    private fun writeF32Le(out: ByteArrayOutputStream, v: Float) {
        writeI32Le(out, v.toRawBits())
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
