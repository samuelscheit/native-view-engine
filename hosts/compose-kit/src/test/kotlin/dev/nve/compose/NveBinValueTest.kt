package dev.nve.compose

import org.junit.jupiter.api.Assertions.assertArrayEquals
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertNull
import org.junit.jupiter.api.Assertions.assertThrows
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import java.io.ByteArrayOutputStream

class NveBinValueTest {

    @Test
    fun roundTripsAllEightWireTagsZeroCopy() {
        val payload = NveBinValue.Map(
            listOf(
                "null" to NveBinValue.Null,
                "flag" to NveBinValue.Bool(true),
                "neg" to NveBinValue.I64(-9_007_199_254_740_991L),
                "pi" to NveBinValue.F64(3.141592653589793),
                "greeting" to NveBinValue.Str("Hello NVE 🚀"),
                "blob" to NveBinValue.Bytes(byteArrayOf(0xDE.toByte(), 0xAD.toByte(), 0xBE.toByte(), 0xEF.toByte())),
                "nested" to NveBinValue.Array(
                    listOf(
                        NveBinValue.I64(10),
                        NveBinValue.Map(listOf("inner" to NveBinValue.Str("ok"))),
                        NveBinValue.Bool(false),
                    ),
                ),
            ),
        )

        val encoded = payload.encode()
        val root = NveBinValueView.parse(encoded)

        assertEquals(NveBinValueTag.MAP, root.tag)
        assertEquals(7, root.mapCount)
        assertTrue(root.mapValue("null")!!.isNull)
        assertEquals(true, root.mapValue("flag")!!.boolValue)
        assertEquals(-9_007_199_254_740_991L, root.mapValue("neg")!!.int64Value)
        assertEquals(3.141592653589793, root.mapValue("pi")!!.doubleValue)
        assertEquals("Hello NVE 🚀", root.mapValue("greeting")!!.stringValue)
        assertArrayEquals(
            byteArrayOf(0xDE.toByte(), 0xAD.toByte(), 0xBE.toByte(), 0xEF.toByte()),
            root.mapValue("blob")!!.bytesValue,
        )
        assertNull(root.mapValue("missing"))

        val nested = root.mapValue("nested")!!
        assertEquals(NveBinValueTag.ARRAY, nested.tag)
        assertEquals(3, nested.arrayCount)
        assertEquals(10L, nested.arrayElement(0)!!.int64Value)
        assertEquals("ok", nested.arrayElement(1)!!.mapValue("inner")!!.stringValue)
        assertEquals(false, nested.arrayElement(2)!!.boolValue)
        assertNull(nested.arrayElement(3))
        assertNull(nested.arrayElement(-1))

        assertEquals(payload, root.toOwned())
    }

    @Test
    fun encodesKotlinMapsWithoutJson() {
        val bytes = NveBinValue.encodeMap(
            mapOf(
                "title" to "Counter",
                "count" to 42,
                "enabled" to true,
                "ratio" to 0.75,
                "tags" to listOf("fast", "zero-copy"),
                "empty" to null,
            ),
        )

        val view = NveBinValueView.parse(bytes)
        assertEquals("Counter", view.mapValue("title")!!.stringValue)
        assertEquals(42L, view.mapValue("count")!!.int64Value)
        assertEquals(true, view.mapValue("enabled")!!.boolValue)
        assertEquals(0.75, view.mapValue("ratio")!!.doubleValue)
        assertEquals("zero-copy", view.mapValue("tags")!!.arrayElement(1)!!.stringValue)
        assertTrue(view.mapValue("empty")!!.isNull)
    }

    @Test
    fun enforcesDepthLimit64AndRejectsTruncationBadTagsBoolUtf8AndTrailingBytes() {
        // Depth 64 succeeds
        var depth64: NveBinValue = NveBinValue.Null
        repeat(NVE_BIN_VALUE_MAX_DEPTH) {
            depth64 = NveBinValue.Array(listOf(depth64))
        }
        val encoded64 = depth64.encode()
        assertEquals(NveBinValueTag.ARRAY, NveBinValueView.parse(encoded64).tag)

        // Depth 65 fails on encode
        val depth65 = NveBinValue.Array(listOf(depth64))
        assertThrows(NveBinValueException.MaxDepthExceeded::class.java) {
            depth65.encode()
        }

        // Hand-craft a 65-deep wire buffer to verify parser depth guard directly
        val deepWire = ByteArrayOutputStream()
        for (level in 0 until 65) {
            val remainingContainers = 64 - level
            val payloadLen = 4 + remainingContainers * 9 + 1
            deepWire.write(NveBinValueTag.ARRAY.wireCode.toInt())
            writeLe32(deepWire, payloadLen)
            writeLe32(deepWire, 1)
        }
        deepWire.write(NveBinValueTag.NULL.wireCode.toInt())
        assertThrows(NveBinValueException.MaxDepthExceeded::class.java) {
            NveBinValueView.parse(deepWire.toByteArray())
        }

        // Every strict prefix of a valid encoded map fails with Truncated
        val validMap = NveBinValue.Map(listOf("k" to NveBinValue.Str("value"))).encode()
        for (cut in 0 until validMap.size) {
            assertThrows(NveBinValueException.Truncated::class.java) {
                NveBinValueView.parse(validMap.copyOfRange(0, cut))
            }
        }

        // Trailing byte rejected
        val trailing = validMap + byteArrayOf(0)
        val trailErr = assertThrows(NveBinValueException.TrailingBytes::class.java) {
            NveBinValueView.parse(trailing)
        }
        assertEquals(1, trailErr.remaining)

        // Invalid tag
        val tagErr = assertThrows(NveBinValueException.InvalidTag::class.java) {
            NveBinValueView.parse(byteArrayOf(9))
        }
        assertEquals(9, tagErr.tag)

        // Invalid bool value (must be 0 or 1)
        val boolErr = assertThrows(NveBinValueException.InvalidBool::class.java) {
            NveBinValueView.parse(byteArrayOf(1, 2))
        }
        assertEquals(2, boolErr.value)

        // Invalid UTF-8 in string
        val badUtf8 = byteArrayOf(4, 2, 0, 0, 0, 0xFF.toByte(), 0xFE.toByte())
        assertThrows(NveBinValueException.InvalidUtf8::class.java) {
            NveBinValueView.parse(badUtf8)
        }
    }

    private fun writeLe32(out: ByteArrayOutputStream, v: Int) {
        out.write(v and 0xFF)
        out.write((v ushr 8) and 0xFF)
        out.write((v ushr 16) and 0xFF)
        out.write((v ushr 24) and 0xFF)
    }
}
