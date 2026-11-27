package dev.nve.compose

import org.junit.jupiter.api.Assertions.assertArrayEquals
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertThrows
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder

class NveEventRingCodecTest {

    @Test
    fun roundTripsAllEventKindsAndPayloadsByteForByte() {
        val customPayloadBytes = NveBinValue.encodeMap(mapOf("delta" to 3, "label" to "tap"))
        val events = listOf(
            NveEventRingCodec.press(surfaceId = 1, nodeId = 10, x = 42.5f, y = 88.25f, timestampUs = 1_000L),
            NveEventRingCodec.longPress(surfaceId = 1, nodeId = 11, x = 12f, y = 34f, timestampUs = 2_000L),
            NveEventRingCodec.scroll(
                surfaceId = 1,
                nodeId = 12,
                x = 0f,
                y = 240.5f,
                contentWidth = 390f,
                contentHeight = 1600f,
                timestampUs = 3_000L,
            ),
            NveEventRingCodec.textChange(
                surfaceId = 1,
                nodeId = 13,
                text = "Search 🚀 café",
                timestampUs = 4_000L,
            ),
            NveEvent(surfaceId = 1, nodeId = 14, timestampUs = 5_000L, kind = NveEventKind.FOCUS),
            NveEvent(surfaceId = 1, nodeId = 14, timestampUs = 6_000L, kind = NveEventKind.BLUR),
            NveEventRingCodec.layout(
                surfaceId = 1,
                nodeId = 0,
                x = 0f,
                y = 0f,
                width = 412f,
                height = 915f,
                timestampUs = 7_000L,
            ),
            NveEventRingCodec.custom(
                surfaceId = 1,
                nodeId = 15,
                name = "onIncrement",
                data = customPayloadBytes,
                timestampUs = 8_000L,
            ),
        )

        val encoded = NveEventRingCodec.encode(events)
        val decoded = NveEventRingCodec.decode(encoded)

        assertEquals(events, decoded)

        // Verify zero-copy binary payload inside Custom event
        val custom = decoded.last()
        assertEquals("onIncrement", custom.name)
        val payloadView = (custom.payload as NveEventPayload.Custom).parseBinValue()
        assertEquals(3L, payloadView.mapValue("delta")!!.int64Value)
        assertEquals("tap", payloadView.mapValue("label")!!.stringValue)
    }

    @Test
    fun matchesExactLittleEndianRustWireLayout() {
        val bytes = NveEventRingCodec.encodeSingle(
            NveEventRingCodec.press(surfaceId = 7, nodeId = 42, x = 10.0f, y = 20.0f, timestampUs = 0x1122334455667788L),
        )
        // 14 (NVEV v2 header) + 64 (uniform NveEvent record) = 78 bytes
        assertEquals(78, bytes.size)
        val buf = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
        assertEquals('N'.code.toByte(), buf.get())
        assertEquals('V'.code.toByte(), buf.get())
        assertEquals('E'.code.toByte(), buf.get())
        assertEquals('V'.code.toByte(), buf.get())
        assertEquals(2.toShort(), buf.short)
        assertEquals(1, buf.int) // event_count
        assertEquals(0, buf.int) // bytes_len
        assertEquals(0x1122334455667788L, buf.long) // timestamp
        assertEquals(NveEventRingCodec.TABLE_INTERACTION, buf.long) // table
        assertEquals(7, buf.int) // surface
        assertEquals(42, buf.int) // node
        assertEquals(NveEventRingCodec.INTERACTION_PRESS, buf.int) // event
        assertEquals(0, buf.int) // flags
        assertEquals(10.0f, buf.float) // payload[0]
        assertEquals(20.0f, buf.float) // payload[1]
    }

    @Test
    fun handlesEmptyBuffersAndRejectsMalformedTruncatedPayloads() {
        assertTrue(NveEventRingCodec.decode(ByteArray(0)).isEmpty())
        assertTrue(NveEventRingCodec.decode(NveEventRingCodec.encode(emptyList())).isEmpty())

        val validBatch = NveEventRingCodec.encode(
            listOf(
                NveEventRingCodec.press(1, 2, 1f, 2f, 100L),
                NveEventRingCodec.textChange(1, 3, "hello", 200L),
                NveEventRingCodec.custom(1, 4, "evt", byteArrayOf(1, 2, 3), 300L),
            ),
        )

        // Every strict prefix (1..len-1) must fail with Truncated
        for (cut in 1 until validBatch.size) {
            assertThrows(NveEventRingException.Truncated::class.java) {
                NveEventRingCodec.decode(validBatch.copyOfRange(0, cut))
            }
        }

        // Trailing bytes rejected
        assertThrows(NveEventRingException.TrailingBytes::class.java) {
            NveEventRingCodec.decode(validBatch + byteArrayOf(0x00))
        }

        // Invalid magic rejected
        val badMagic = NveEventRingCodec.encode(emptyList())
        badMagic[0] = 0
        assertThrows(NveEventRingException.InvalidHeader::class.java) {
            NveEventRingCodec.decode(badMagic)
        }

        // Invalid table key rejected
        val badTable = NveEventRingCodec.encodeSingle(NveEvent(1, 2, 3L, NveEventKind.FOCUS))
        badTable[14 + 8] = 0x55
        assertThrows(NveEventRingException.InvalidTable::class.java) {
            NveEventRingCodec.decode(badTable)
        }

        // Invalid UTF-8 in Text event
        val badText = NveEventRingCodec.encodeSingle(NveEventRingCodec.textChange(1, 2, "ab", 10L))
        badText[badText.size - 2] = 0xFF.toByte()
        badText[badText.size - 1] = 0xFE.toByte()
        assertThrows(NveEventRingException.InvalidUtf8::class.java) {
            NveEventRingCodec.decode(badText)
        }

        // Huge declared count (OOM guard)
        val hugeCount = ByteBuffer.allocate(14).order(ByteOrder.LITTLE_ENDIAN)
            .put('N'.code.toByte()).put('V'.code.toByte()).put('E'.code.toByte()).put('V'.code.toByte())
            .putShort(2)
            .putInt(0x7FFF_FFFF)
            .putInt(0)
            .array()
        assertThrows(NveEventRingException.Truncated::class.java) {
            NveEventRingCodec.decode(hugeCount)
        }
    }
}
