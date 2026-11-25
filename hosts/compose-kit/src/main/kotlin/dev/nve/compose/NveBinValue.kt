package dev.nve.compose

import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction
import java.nio.charset.StandardCharsets

/**
 * Maximum container nesting depth permitted by the binary `NveBinValue` wire format
 * (matches `NVE_BIN_VALUE_MAX_DEPTH` in `crates/nve-abi/src/value.rs`).
 */
const val NVE_BIN_VALUE_MAX_DEPTH: Int = 64

/**
 * Binary `NveBinValue` wire tags `0..7` matching `NveValueTag` in `crates/nve-abi/src/value.rs`.
 */
enum class NveBinValueTag(val wireCode: Byte) {
    NULL(0),
    BOOL(1),
    I64(2),
    F64(3),
    STR(4),
    BYTES(5),
    ARRAY(6),
    MAP(7);

    companion object {
        fun fromWireCode(code: Int): NveBinValueTag? = when (code) {
            0 -> NULL
            1 -> BOOL
            2 -> I64
            3 -> F64
            4 -> STR
            5 -> BYTES
            6 -> ARRAY
            7 -> MAP
            else -> null
        }
    }
}

/**
 * Strict validation errors thrown when encoding or parsing compact binary `NveBinValue` payloads.
 */
sealed class NveBinValueException(message: String) : IllegalArgumentException(message) {
    class Truncated : NveBinValueException("Truncated NveBinValue buffer")
    class InvalidTag(val tag: Int) : NveBinValueException("Invalid NveBinValue tag: $tag")
    class InvalidBool(val value: Int) : NveBinValueException("Invalid NveBinValue bool byte: $value")
    class InvalidUtf8 : NveBinValueException("Invalid UTF-8 bytes in NveBinValue")
    class MaxDepthExceeded : NveBinValueException("NveBinValue nesting exceeded max depth $NVE_BIN_VALUE_MAX_DEPTH")
    class TrailingBytes(val remaining: Int) : NveBinValueException("Trailing $remaining bytes after NveBinValue")
}

/**
 * Owned structured value matching Rust `NveValue` / Swift `NveBinValue` / Dart `NveBinValue`.
 * Encodes directly to the little-endian binary `NveBinValue` wire format with zero JSON.
 */
sealed interface NveBinValue {
    data object Null : NveBinValue
    data class Bool(val value: Boolean) : NveBinValue
    data class I64(val value: Long) : NveBinValue
    data class F64(val value: Double) : NveBinValue {
        override fun equals(other: Any?): Boolean =
            other is F64 && value.toRawBits() == other.value.toRawBits()

        override fun hashCode(): Int = value.toRawBits().hashCode()
    }
    data class Str(val value: String) : NveBinValue
    data class Bytes(val value: ByteArray) : NveBinValue {
        override fun equals(other: Any?): Boolean =
            other is Bytes && value.contentEquals(other.value)

        override fun hashCode(): Int = value.contentHashCode()
    }
    data class Array(val elements: List<NveBinValue>) : NveBinValue
    data class Map(val entries: List<Pair<String, NveBinValue>>) : NveBinValue

    fun encode(): ByteArray {
        val out = ByteArrayOutputStream(64)
        encodeDepth(this, out, 0)
        return out.toByteArray()
    }

    companion object {
        fun fromAny(value: Any?): NveBinValue = when (value) {
            null -> Null
            is NveBinValue -> value
            is Boolean -> Bool(value)
            is Byte -> I64(value.toLong())
            is Short -> I64(value.toLong())
            is Int -> I64(value.toLong())
            is Long -> I64(value)
            is Float -> F64(value.toDouble())
            is Double -> F64(value)
            is String -> Str(value)
            is ByteArray -> Bytes(value)
            is kotlin.collections.Map<*, *> -> Map(
                value.entries.mapNotNull { (k, v) ->
                    k?.let { it.toString() to fromAny(v) }
                },
            )
            is Iterable<*> -> Array(value.map { fromAny(it) })
            is kotlin.Array<*> -> Array(value.map { fromAny(it) })
            else -> Str(value.toString())
        }

        fun fromMap(map: kotlin.collections.Map<String, Any?>): Map =
            Map(map.entries.map { (k, v) -> k to fromAny(v) })

        fun encodeMap(map: kotlin.collections.Map<String, Any?>): ByteArray =
            fromMap(map).encode()

        private fun encodeDepth(value: NveBinValue, out: ByteArrayOutputStream, depth: Int) {
            when (value) {
                Null -> out.write(NveBinValueTag.NULL.wireCode.toInt())
                is Bool -> {
                    out.write(NveBinValueTag.BOOL.wireCode.toInt())
                    out.write(if (value.value) 1 else 0)
                }
                is I64 -> {
                    out.write(NveBinValueTag.I64.wireCode.toInt())
                    writeI64Le(out, value.value)
                }
                is F64 -> {
                    out.write(NveBinValueTag.F64.wireCode.toInt())
                    writeI64Le(out, value.value.toRawBits())
                }
                is Str -> {
                    val utf8 = value.value.toByteArray(StandardCharsets.UTF_8)
                    out.write(NveBinValueTag.STR.wireCode.toInt())
                    writeI32Le(out, utf8.size)
                    out.write(utf8)
                }
                is Bytes -> {
                    out.write(NveBinValueTag.BYTES.wireCode.toInt())
                    writeI32Le(out, value.value.size)
                    out.write(value.value)
                }
                is Array -> {
                    if (depth >= NVE_BIN_VALUE_MAX_DEPTH) throw NveBinValueException.MaxDepthExceeded()
                    val body = ByteArrayOutputStream(32)
                    writeI32Le(body, value.elements.size)
                    for (elem in value.elements) {
                        encodeDepth(elem, body, depth + 1)
                    }
                    val bodyBytes = body.toByteArray()
                    out.write(NveBinValueTag.ARRAY.wireCode.toInt())
                    writeI32Le(out, bodyBytes.size)
                    out.write(bodyBytes)
                }
                is Map -> {
                    if (depth >= NVE_BIN_VALUE_MAX_DEPTH) throw NveBinValueException.MaxDepthExceeded()
                    val body = ByteArrayOutputStream(64)
                    writeI32Le(body, value.entries.size)
                    for ((k, v) in value.entries) {
                        val keyBytes = k.toByteArray(StandardCharsets.UTF_8)
                        writeI32Le(body, keyBytes.size)
                        body.write(keyBytes)
                        encodeDepth(v, body, depth + 1)
                    }
                    val bodyBytes = body.toByteArray()
                    out.write(NveBinValueTag.MAP.wireCode.toInt())
                    writeI32Le(out, bodyBytes.size)
                    out.write(bodyBytes)
                }
            }
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
}

/**
 * Zero-copy borrowed view over a validated compact binary `NveBinValue` slice inside a [ByteBuffer].
 *
 * Supports O(1) subtree skipping over length-prefixed `Array` (tag 6) and `Map` (tag 7)
 * containers without intermediate JSON parsing or heap allocation of skipped subtrees.
 */
class NveBinValueView private constructor(
    private val buffer: ByteBuffer,
    private val startOffset: Int,
    private val endOffset: Int,
    val tag: NveBinValueTag,
) {
    val byteLength: Int get() = endOffset - startOffset
    val isNull: Boolean get() = tag == NveBinValueTag.NULL

    val boolValue: Boolean?
        get() = if (tag == NveBinValueTag.BOOL) buffer.get(startOffset + 1).toInt() != 0 else null

    val int64Value: Long?
        get() = if (tag == NveBinValueTag.I64) buffer.getLong(startOffset + 1) else null

    val doubleValue: Double?
        get() = if (tag == NveBinValueTag.F64) {
            java.lang.Double.longBitsToDouble(buffer.getLong(startOffset + 1))
        } else {
            null
        }

    val numberValue: Double?
        get() = when (tag) {
            NveBinValueTag.I64 -> buffer.getLong(startOffset + 1).toDouble()
            NveBinValueTag.F64 -> java.lang.Double.longBitsToDouble(buffer.getLong(startOffset + 1))
            else -> null
        }

    val stringValue: String?
        get() {
            if (tag != NveBinValueTag.STR) return null
            val len = buffer.getInt(startOffset + 1)
            return readString(buffer, startOffset + 5, len)
        }

    val bytesValue: ByteArray?
        get() {
            if (tag != NveBinValueTag.BYTES) return null
            val len = buffer.getInt(startOffset + 1)
            val out = ByteArray(len)
            val dup = buffer.duplicate()
            dup.position(startOffset + 5)
            dup.get(out)
            return out
        }

    val arrayCount: Int?
        get() = if (tag == NveBinValueTag.ARRAY) buffer.getInt(startOffset + 5) else null

    val mapCount: Int?
        get() = if (tag == NveBinValueTag.MAP) buffer.getInt(startOffset + 5) else null

    fun arrayElement(index: Int): NveBinValueView? {
        if (tag != NveBinValueTag.ARRAY || index < 0) return null
        val count = buffer.getInt(startOffset + 5)
        if (index >= count) return null
        var cursor = startOffset + 9
        for (i in 0 until index) {
            cursor = skipValidatedValue(buffer, cursor)
        }
        val elemTag = NveBinValueTag.fromWireCode(buffer.get(cursor).toInt() and 0xFF)!!
        val elemEnd = skipValidatedValue(buffer, cursor)
        return NveBinValueView(buffer, cursor, elemEnd, elemTag)
    }

    fun mapValue(key: String): NveBinValueView? {
        if (tag != NveBinValueTag.MAP) return null
        val targetBytes = key.toByteArray(StandardCharsets.UTF_8)
        val count = buffer.getInt(startOffset + 5)
        var cursor = startOffset + 9
        repeat(count) {
            val kLen = buffer.getInt(cursor)
            cursor += 4
            val keyStart = cursor
            cursor += kLen
            val valStart = cursor
            val valEnd = skipValidatedValue(buffer, valStart)
            if (kLen == targetBytes.size && regionEquals(buffer, keyStart, targetBytes)) {
                val valTag = NveBinValueTag.fromWireCode(buffer.get(valStart).toInt() and 0xFF)!!
                return NveBinValueView(buffer, valStart, valEnd, valTag)
            }
            cursor = valEnd
        }
        return null
    }

    fun forEachMapEntry(action: (String, NveBinValueView) -> Unit) {
        if (tag != NveBinValueTag.MAP) return
        val count = buffer.getInt(startOffset + 5)
        var cursor = startOffset + 9
        repeat(count) {
            val kLen = buffer.getInt(cursor)
            cursor += 4
            val key = readString(buffer, cursor, kLen)
            cursor += kLen
            val valStart = cursor
            val valEnd = skipValidatedValue(buffer, valStart)
            val valTag = NveBinValueTag.fromWireCode(buffer.get(valStart).toInt() and 0xFF)!!
            action(key, NveBinValueView(buffer, valStart, valEnd, valTag))
            cursor = valEnd
        }
    }

    fun toOwned(): NveBinValue = when (tag) {
        NveBinValueTag.NULL -> NveBinValue.Null
        NveBinValueTag.BOOL -> NveBinValue.Bool(boolValue!!)
        NveBinValueTag.I64 -> NveBinValue.I64(int64Value!!)
        NveBinValueTag.F64 -> NveBinValue.F64(doubleValue!!)
        NveBinValueTag.STR -> NveBinValue.Str(stringValue!!)
        NveBinValueTag.BYTES -> NveBinValue.Bytes(bytesValue!!)
        NveBinValueTag.ARRAY -> {
            val count = arrayCount!!
            val items = ArrayList<NveBinValue>(count)
            var cursor = startOffset + 9
            repeat(count) {
                val elemEnd = skipValidatedValue(buffer, cursor)
                val elemTag = NveBinValueTag.fromWireCode(buffer.get(cursor).toInt() and 0xFF)!!
                items.add(NveBinValueView(buffer, cursor, elemEnd, elemTag).toOwned())
                cursor = elemEnd
            }
            NveBinValue.Array(items)
        }
        NveBinValueTag.MAP -> {
            val count = mapCount!!
            val entries = ArrayList<Pair<String, NveBinValue>>(count)
            forEachMapEntry { key, valueView ->
                entries.add(key to valueView.toOwned())
            }
            NveBinValue.Map(entries)
        }
    }

    companion object {
        fun parse(bytes: ByteArray): NveBinValueView =
            parse(ByteBuffer.wrap(bytes))

        fun parse(source: ByteBuffer): NveBinValueView {
            val buf = source.duplicate().order(ByteOrder.LITTLE_ENDIAN)
            val start = buf.position()
            val limit = buf.limit()
            val (tag, end) = validateValue(buf, start, limit, depth = 0)
            if (end != limit) {
                throw NveBinValueException.TrailingBytes(limit - end)
            }
            return NveBinValueView(buf, start, end, tag)
        }

        private fun validateValue(
            buf: ByteBuffer,
            start: Int,
            limit: Int,
            depth: Int,
        ): Pair<NveBinValueTag, Int> {
            if (start >= limit) throw NveBinValueException.Truncated()
            val rawTag = buf.get(start).toInt() and 0xFF
            val tag = NveBinValueTag.fromWireCode(rawTag)
                ?: throw NveBinValueException.InvalidTag(rawTag)

            return when (tag) {
                NveBinValueTag.NULL -> tag to (start + 1)
                NveBinValueTag.BOOL -> {
                    if (start + 2 > limit) throw NveBinValueException.Truncated()
                    val b = buf.get(start + 1).toInt() and 0xFF
                    if (b != 0 && b != 1) throw NveBinValueException.InvalidBool(b)
                    tag to (start + 2)
                }
                NveBinValueTag.I64, NveBinValueTag.F64 -> {
                    if (start + 9 > limit) throw NveBinValueException.Truncated()
                    tag to (start + 9)
                }
                NveBinValueTag.STR -> {
                    if (start + 5 > limit) throw NveBinValueException.Truncated()
                    val len = readU32Bounded(buf, start + 1, limit - (start + 5))
                    validateUtf8(buf, start + 5, len)
                    tag to (start + 5 + len)
                }
                NveBinValueTag.BYTES -> {
                    if (start + 5 > limit) throw NveBinValueException.Truncated()
                    val len = readU32Bounded(buf, start + 1, limit - (start + 5))
                    tag to (start + 5 + len)
                }
                NveBinValueTag.ARRAY -> {
                    if (depth >= NVE_BIN_VALUE_MAX_DEPTH) throw NveBinValueException.MaxDepthExceeded()
                    if (start + 5 > limit) throw NveBinValueException.Truncated()
                    val payloadLen = readU32Bounded(buf, start + 1, limit - (start + 5))
                    val payloadStart = start + 5
                    val payloadEnd = payloadStart + payloadLen
                    if (payloadLen < 4) throw NveBinValueException.Truncated()
                    val count = readU32Bounded(buf, payloadStart, payloadLen - 4)
                    var cursor = payloadStart + 4
                    repeat(count) {
                        val (_, next) = validateValue(buf, cursor, payloadEnd, depth + 1)
                        cursor = next
                    }
                    if (cursor != payloadEnd) throw NveBinValueException.TrailingBytes(payloadEnd - cursor)
                    tag to payloadEnd
                }
                NveBinValueTag.MAP -> {
                    if (depth >= NVE_BIN_VALUE_MAX_DEPTH) throw NveBinValueException.MaxDepthExceeded()
                    if (start + 5 > limit) throw NveBinValueException.Truncated()
                    val payloadLen = readU32Bounded(buf, start + 1, limit - (start + 5))
                    val payloadStart = start + 5
                    val payloadEnd = payloadStart + payloadLen
                    if (payloadLen < 4) throw NveBinValueException.Truncated()
                    val count = readU32Bounded(buf, payloadStart, payloadLen - 4)
                    var cursor = payloadStart + 4
                    repeat(count) {
                        if (cursor + 4 > payloadEnd) throw NveBinValueException.Truncated()
                        val kLen = readU32Bounded(buf, cursor, payloadEnd - (cursor + 4))
                        cursor += 4
                        validateUtf8(buf, cursor, kLen)
                        cursor += kLen
                        val (_, next) = validateValue(buf, cursor, payloadEnd, depth + 1)
                        cursor = next
                    }
                    if (cursor != payloadEnd) throw NveBinValueException.TrailingBytes(payloadEnd - cursor)
                    tag to payloadEnd
                }
            }
        }

        private fun readU32Bounded(buf: ByteBuffer, offset: Int, maxAllowed: Int): Int {
            val u = buf.getInt(offset).toLong() and 0xFFFF_FFFFL
            if (u > maxAllowed.toLong()) throw NveBinValueException.Truncated()
            return u.toInt()
        }

        private fun validateUtf8(buf: ByteBuffer, offset: Int, len: Int) {
            if (len == 0) return
            val slice = buf.duplicate()
            slice.position(offset)
            slice.limit(offset + len)
            val decoder = StandardCharsets.UTF_8.newDecoder()
                .onMalformedInput(CodingErrorAction.REPORT)
                .onUnmappableCharacter(CodingErrorAction.REPORT)
            try {
                decoder.decode(slice)
            } catch (_: CharacterCodingException) {
                throw NveBinValueException.InvalidUtf8()
            }
        }

        private fun skipValidatedValue(buf: ByteBuffer, offset: Int): Int {
            return when (NveBinValueTag.fromWireCode(buf.get(offset).toInt() and 0xFF)!!) {
                NveBinValueTag.NULL -> offset + 1
                NveBinValueTag.BOOL -> offset + 2
                NveBinValueTag.I64, NveBinValueTag.F64 -> offset + 9
                NveBinValueTag.STR, NveBinValueTag.BYTES -> offset + 5 + buf.getInt(offset + 1)
                // O(1) subtree skip using length-prefixed container payload_byte_len.
                NveBinValueTag.ARRAY, NveBinValueTag.MAP -> offset + 5 + buf.getInt(offset + 1)
            }
        }

        private fun readString(buf: ByteBuffer, offset: Int, len: Int): String {
            if (len == 0) return ""
            val bytes = ByteArray(len)
            val dup = buf.duplicate()
            dup.position(offset)
            dup.get(bytes)
            return String(bytes, StandardCharsets.UTF_8)
        }

        private fun regionEquals(buf: ByteBuffer, offset: Int, target: ByteArray): Boolean {
            for (i in target.indices) {
                if (buf.get(offset + i) != target[i]) return false
            }
            return true
        }
    }
}
