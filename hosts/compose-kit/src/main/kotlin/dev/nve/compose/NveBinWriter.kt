package dev.nve.compose

/**
 * Reusable flat-buffer binary writer for `NveBinValue` payloads.
 *
 * Encodes nested `Array` and `Map` containers directly into a single growable [ByteArray] by
 * reserving the 4-byte little-endian `payloadByteLen` header and back-patching it in place,
 * avoiding per-container `ByteArrayOutputStream` and intermediate `toByteArray()` allocations.
 * Also encodes Kotlin [Map]/[Any] graphs directly without constructing an intermediate
 * [NveBinValue] AST.
 */
class NveBinWriter(initialCapacity: Int = 128) {
    private var buf: ByteArray = ByteArray(maxOf(16, initialCapacity))
    var size: Int = 0
        private set

    fun reset() {
        size = 0
    }

    fun writeValue(value: NveBinValue) {
        writeValueDepth(value, 0)
    }

    fun writeMap(map: Map<String, Any?>) {
        writeRawMap(map, 0)
    }

    fun writeAny(value: Any?) {
        writeAnyDepth(value, 0)
    }

    fun toByteArray(): ByteArray = buf.copyOf(size)

    fun contentEquals(other: ByteArray, otherLen: Int): Boolean {
        if (size != otherLen || otherLen > other.size) return false
        for (i in 0 until size) {
            if (buf[i] != other[i]) return false
        }
        return true
    }

    /**
     * Copies `[0, size)` into [dest], reusing [dest] whenever its capacity suffices.
     */
    fun copyInto(dest: ByteArray): ByteArray {
        val target = if (dest.size >= size) dest else ByteArray(maxOf(size, dest.size shl 1))
        if (size > 0) {
            System.arraycopy(buf, 0, target, 0, size)
        }
        return target
    }

    private fun writeValueDepth(value: NveBinValue, depth: Int) {
        when (value) {
            NveBinValue.Null -> writeByte(NveBinValueTag.NULL.wireCode)
            is NveBinValue.Bool -> {
                ensureCapacity(2)
                buf[size++] = NveBinValueTag.BOOL.wireCode
                buf[size++] = if (value.value) 1 else 0
            }
            is NveBinValue.I64 -> {
                ensureCapacity(9)
                buf[size++] = NveBinValueTag.I64.wireCode
                putI64LeAt(size, value.value)
                size += 8
            }
            is NveBinValue.F64 -> {
                ensureCapacity(9)
                buf[size++] = NveBinValueTag.F64.wireCode
                putI64LeAt(size, value.value.toRawBits())
                size += 8
            }
            is NveBinValue.Str -> {
                writeByte(NveBinValueTag.STR.wireCode)
                writeLenPrefixedUtf8(value.value)
            }
            is NveBinValue.Bytes -> {
                val bytes = value.value
                ensureCapacity(5 + bytes.size)
                buf[size++] = NveBinValueTag.BYTES.wireCode
                putI32LeAt(size, bytes.size)
                size += 4
                if (bytes.isNotEmpty()) {
                    System.arraycopy(bytes, 0, buf, size, bytes.size)
                    size += bytes.size
                }
            }
            is NveBinValue.Array -> {
                if (depth >= NVE_BIN_VALUE_MAX_DEPTH) throw NveBinValueException.MaxDepthExceeded()
                ensureCapacity(9)
                buf[size++] = NveBinValueTag.ARRAY.wireCode
                val lenOffset = size
                size += 4
                val payloadStart = size
                putI32LeAt(size, value.elements.size)
                size += 4
                for (elem in value.elements) {
                    writeValueDepth(elem, depth + 1)
                }
                putI32LeAt(lenOffset, size - payloadStart)
            }
            is NveBinValue.Map -> {
                if (depth >= NVE_BIN_VALUE_MAX_DEPTH) throw NveBinValueException.MaxDepthExceeded()
                ensureCapacity(9)
                buf[size++] = NveBinValueTag.MAP.wireCode
                val lenOffset = size
                size += 4
                val payloadStart = size
                putI32LeAt(size, value.entries.size)
                size += 4
                for ((k, v) in value.entries) {
                    writeLenPrefixedUtf8(k)
                    writeValueDepth(v, depth + 1)
                }
                putI32LeAt(lenOffset, size - payloadStart)
            }
        }
    }

    private fun writeAnyDepth(value: Any?, depth: Int) {
        when (value) {
            null -> writeByte(NveBinValueTag.NULL.wireCode)
            is NveBinValue -> writeValueDepth(value, depth)
            is Boolean -> {
                ensureCapacity(2)
                buf[size++] = NveBinValueTag.BOOL.wireCode
                buf[size++] = if (value) 1 else 0
            }
            is Byte -> writeI64(value.toLong())
            is Short -> writeI64(value.toLong())
            is Int -> writeI64(value.toLong())
            is Long -> writeI64(value)
            is Float -> writeF64(value.toDouble())
            is Double -> writeF64(value)
            is String -> {
                writeByte(NveBinValueTag.STR.wireCode)
                writeLenPrefixedUtf8(value)
            }
            is ByteArray -> {
                ensureCapacity(5 + value.size)
                buf[size++] = NveBinValueTag.BYTES.wireCode
                putI32LeAt(size, value.size)
                size += 4
                if (value.isNotEmpty()) {
                    System.arraycopy(value, 0, buf, size, value.size)
                    size += value.size
                }
            }
            is Map<*, *> -> writeRawMap(value, depth)
            is List<*> -> {
                if (depth >= NVE_BIN_VALUE_MAX_DEPTH) throw NveBinValueException.MaxDepthExceeded()
                ensureCapacity(9)
                buf[size++] = NveBinValueTag.ARRAY.wireCode
                val lenOffset = size
                size += 4
                val payloadStart = size
                putI32LeAt(size, value.size)
                size += 4
                for (i in value.indices) {
                    writeAnyDepth(value[i], depth + 1)
                }
                putI32LeAt(lenOffset, size - payloadStart)
            }
            is Iterable<*> -> {
                if (depth >= NVE_BIN_VALUE_MAX_DEPTH) throw NveBinValueException.MaxDepthExceeded()
                ensureCapacity(9)
                buf[size++] = NveBinValueTag.ARRAY.wireCode
                val lenOffset = size
                size += 4
                val payloadStart = size
                val countOffset = size
                size += 4
                var count = 0
                for (elem in value) {
                    writeAnyDepth(elem, depth + 1)
                    count++
                }
                putI32LeAt(countOffset, count)
                putI32LeAt(lenOffset, size - payloadStart)
            }
            is Array<*> -> {
                if (depth >= NVE_BIN_VALUE_MAX_DEPTH) throw NveBinValueException.MaxDepthExceeded()
                ensureCapacity(9)
                buf[size++] = NveBinValueTag.ARRAY.wireCode
                val lenOffset = size
                size += 4
                val payloadStart = size
                putI32LeAt(size, value.size)
                size += 4
                for (elem in value) {
                    writeAnyDepth(elem, depth + 1)
                }
                putI32LeAt(lenOffset, size - payloadStart)
            }
            else -> {
                writeByte(NveBinValueTag.STR.wireCode)
                writeLenPrefixedUtf8(value.toString())
            }
        }
    }

    private fun writeRawMap(map: Map<*, *>, depth: Int) {
        if (depth >= NVE_BIN_VALUE_MAX_DEPTH) throw NveBinValueException.MaxDepthExceeded()
        ensureCapacity(9)
        buf[size++] = NveBinValueTag.MAP.wireCode
        val lenOffset = size
        size += 4
        val payloadStart = size
        val countOffset = size
        size += 4
        var count = 0
        for ((k, v) in map) {
            if (k == null) continue
            writeLenPrefixedUtf8(if (k is String) k else k.toString())
            writeAnyDepth(v, depth + 1)
            count++
        }
        putI32LeAt(countOffset, count)
        putI32LeAt(lenOffset, size - payloadStart)
    }

    private fun writeI64(v: Long) {
        ensureCapacity(9)
        buf[size++] = NveBinValueTag.I64.wireCode
        putI64LeAt(size, v)
        size += 8
    }

    private fun writeF64(v: Double) {
        ensureCapacity(9)
        buf[size++] = NveBinValueTag.F64.wireCode
        putI64LeAt(size, v.toRawBits())
        size += 8
    }

    private fun writeByte(b: Byte) {
        ensureCapacity(1)
        buf[size++] = b
    }

    /**
     * Encodes [s] directly as UTF-8 into [buf] prefixed by its 4-byte little-endian byte length,
     * with zero temporary `ByteArray` allocations.
     */
    private fun writeLenPrefixedUtf8(s: String) {
        val strLen = s.length
        // Worst-case UTF-8 expansion is 3 bytes per UTF-16 code unit (surrogate pairs use 4 bytes for 2 units).
        ensureCapacity(4 + strLen * 3)
        val lenOffset = size
        size += 4
        val start = size
        var i = 0
        while (i < strLen) {
            val c = s[i].code
            if (c < 0x80) {
                buf[size++] = c.toByte()
                i++
            } else if (c < 0x800) {
                buf[size++] = (0xC0 or (c ushr 6)).toByte()
                buf[size++] = (0x80 or (c and 0x3F)).toByte()
                i++
            } else if (c in 0xD800..0xDBFF && i + 1 < strLen && s[i + 1].code in 0xDC00..0xDFFF) {
                val cp = 0x10000 + (((c - 0xD800) shl 10) or (s[i + 1].code - 0xDC00))
                buf[size++] = (0xF0 or (cp ushr 18)).toByte()
                buf[size++] = (0x80 or ((cp ushr 12) and 0x3F)).toByte()
                buf[size++] = (0x80 or ((cp ushr 6) and 0x3F)).toByte()
                buf[size++] = (0x80 or (cp and 0x3F)).toByte()
                i += 2
            } else if (c in 0xD800..0xDFFF) {
                // Unpaired surrogate -> U+FFFD replacement character (matches Java/Kotlin UTF-8 encoder).
                buf[size++] = 0xEF.toByte()
                buf[size++] = 0xBF.toByte()
                buf[size++] = 0xBD.toByte()
                i++
            } else {
                buf[size++] = (0xE0 or (c ushr 12)).toByte()
                buf[size++] = (0x80 or ((c ushr 6) and 0x3F)).toByte()
                buf[size++] = (0x80 or (c and 0x3F)).toByte()
                i++
            }
        }
        putI32LeAt(lenOffset, size - start)
    }

    private fun ensureCapacity(extra: Int) {
        val required = size + extra
        if (required <= buf.size) return
        var next = buf.size
        while (next < required) {
            next = maxOf(required, next shl 1)
        }
        buf = buf.copyOf(next)
    }

    private fun putI32LeAt(offset: Int, v: Int) {
        buf[offset] = (v and 0xFF).toByte()
        buf[offset + 1] = ((v ushr 8) and 0xFF).toByte()
        buf[offset + 2] = ((v ushr 16) and 0xFF).toByte()
        buf[offset + 3] = ((v ushr 24) and 0xFF).toByte()
    }

    private fun putI64LeAt(offset: Int, v: Long) {
        buf[offset] = (v and 0xFF).toByte()
        buf[offset + 1] = ((v ushr 8) and 0xFF).toByte()
        buf[offset + 2] = ((v ushr 16) and 0xFF).toByte()
        buf[offset + 3] = ((v ushr 24) and 0xFF).toByte()
        buf[offset + 4] = ((v ushr 32) and 0xFF).toByte()
        buf[offset + 5] = ((v ushr 40) and 0xFF).toByte()
        buf[offset + 6] = ((v ushr 48) and 0xFF).toByte()
        buf[offset + 7] = ((v ushr 56) and 0xFF).toByte()
    }
}
