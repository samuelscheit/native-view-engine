package dev.nve.compose

import org.junit.jupiter.api.Assertions.assertArrayEquals
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Assertions.assertSame
import org.junit.jupiter.api.Assertions.assertThrows
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * Verifies Kolibri-inspired zero-copy `DirectByteBuffer` event & prop channel optimizations:
 * - Byte-for-byte parity between `NveEventRingCodec.encodeInto(ByteBuffer)` and `encode(List<NveEvent>)`
 * - Zero-allocation direct buffer reuse across 1,000 flushes
 * - Automatic `ByteArray` fallback when a batch or prop update exceeds direct buffer capacity
 * - Strict guards for empty, malformed, read-only, and disposed states
 */
class NveDirectBufferChannelTest {

    private class DirectChannelBackend(
        private val eventScratchCapacity: Int = 64 * 1024,
        private val propScratchCapacity: Int = 64 * 1024,
    ) : NveBridgeBackend {
        val delegate = FakeNveBridgeBackend()
        val eventScratchBuffers = LinkedHashMap<Long, ByteBuffer>()
        val propScratchBuffers = LinkedHashMap<Long, ByteBuffer>()

        var eventScratchRequests: Int = 0
        var propScratchRequests: Int = 0
        var directEventSubmissions: Int = 0
        var arrayEventFallbacks: Int = 0
        var directPropSubmissions: Int = 0
        var arrayPropFallbacks: Int = 0
        val receivedEventPayloads = ArrayList<ByteArray>()
        val receivedPropPayloads = ArrayList<ByteArray>()

        override val isAvailable: Boolean
            get() = delegate.isAvailable

        override fun createSurface(
            surfaceId: Int,
            componentName: String,
            vtableSymbol: String,
            backendCode: Int,
        ): Long = delegate.createSurface(surfaceId, componentName, vtableSymbol, backendCode)

        override fun destroySurface(handlePtr: Long) {
            eventScratchBuffers.remove(handlePtr)
            propScratchBuffers.remove(handlePtr)
            delegate.destroySurface(handlePtr)
        }

        override fun resizeSurface(handlePtr: Long, width: Float, height: Float, scaleFactor: Float): Boolean =
            delegate.resizeSurface(handlePtr, width, height, scaleFactor)

        override fun getEventScratchBuffer(handlePtr: Long): ByteBuffer {
            eventScratchRequests += 1
            return eventScratchBuffers.getOrPut(handlePtr) {
                ByteBuffer.allocateDirect(eventScratchCapacity).order(ByteOrder.LITTLE_ENDIAN)
            }
        }

        override fun submitEventsDirect(handlePtr: Long, byteLength: Int): Boolean {
            val buf = eventScratchBuffers[handlePtr] ?: return false
            if (byteLength < NveEventRingCodec.WIRE_HEADER_BYTES || byteLength > buf.capacity()) return false
            directEventSubmissions += 1
            val snapshot = ByteArray(byteLength)
            val dup = buf.duplicate()
            dup.position(0)
            dup.limit(byteLength)
            dup.get(snapshot)
            receivedEventPayloads.add(snapshot)
            return true
        }

        override fun submitEvents(handlePtr: Long, eventRingBytes: ByteArray): Boolean {
            arrayEventFallbacks += 1
            receivedEventPayloads.add(eventRingBytes.copyOf())
            return delegate.submitEvents(handlePtr, eventRingBytes)
        }

        override fun getPropScratchBuffer(handlePtr: Long): ByteBuffer {
            propScratchRequests += 1
            return propScratchBuffers.getOrPut(handlePtr) {
                ByteBuffer.allocateDirect(propScratchCapacity).order(ByteOrder.LITTLE_ENDIAN)
            }
        }

        override fun setPropsDirect(handlePtr: Long, byteLength: Int): Boolean {
            val buf = propScratchBuffers[handlePtr] ?: return false
            if (byteLength <= 0 || byteLength > buf.capacity()) return false
            directPropSubmissions += 1
            val snapshot = ByteArray(byteLength)
            val dup = buf.duplicate()
            dup.position(0)
            dup.limit(byteLength)
            dup.get(snapshot)
            receivedPropPayloads.add(snapshot)
            return true
        }

        override fun setPropsBinary(handlePtr: Long, propsBin: ByteArray): Boolean {
            arrayPropFallbacks += 1
            receivedPropPayloads.add(propsBin.copyOf())
            return delegate.setPropsBinary(handlePtr, propsBin)
        }

        override fun dispatchCommandBinary(handlePtr: Long, commandName: String, argsBin: ByteArray): Boolean =
            delegate.dispatchCommandBinary(handlePtr, commandName, argsBin)

        override fun renderFrame(handlePtr: Long, frameId: Long, outMeta: LongArray): Int =
            delegate.renderFrame(handlePtr, frameId, outMeta)

        override fun getSceneSlabs(handlePtr: Long): NveSceneSlabs? = delegate.getSceneSlabs(handlePtr)

        override fun drainEmittedEvents(handlePtr: Long): ByteArray? = delegate.drainEmittedEvents(handlePtr)

        override fun hotReloadApp(handlePtr: Long, soPath: String): Boolean =
            delegate.hotReloadApp(handlePtr, soPath)

        override fun measureSurface(
            handlePtr: Long,
            minWidth: Float,
            minHeight: Float,
            maxWidth: Float,
            maxHeight: Float,
        ): FloatArray? = delegate.measureSurface(handlePtr, minWidth, minHeight, maxWidth, maxHeight)

        override fun hitTestPointer(handlePtr: Long, pointerId: Int, phase: Int, x: Float, y: Float): Int =
            delegate.hitTestPointer(handlePtr, pointerId, phase, x, y)
    }

    @Test
    fun encodeIntoMatchesEncodeByteForByteAcrossAllEventKindsAndUtf8() {
        val customData = NveBinValue.encodeMap(mapOf("delta" to 7, "tag" to "🚀"))
        val events = listOf(
            NveEventRingCodec.press(1, 10, 42.5f, -0.0f, 1_000L),
            NveEventRingCodec.longPress(1, 11, Float.POSITIVE_INFINITY, Float.NaN, 2_000L),
            NveEventRingCodec.scroll(2, 12, 12.25f, 480.75f, 390f, 2400f, 3_000L),
            NveEventRingCodec.textChange(2, 13, "ASCII café 日本語 € 🚀🔥", 4_000L),
            NveEventRingCodec.textChange(2, 14, "", 4_500L),
            NveEvent(3, 15, 5_000L, NveEventKind.FOCUS),
            NveEvent(3, 15, 6_000L, NveEventKind.BLUR),
            NveEventRingCodec.layout(3, 0, 0f, 0f, 412f, 915f, 7_000L),
            NveEventRingCodec.custom(4, 16, "onSubmit🚀", customData, 8_000L),
            NveEventRingCodec.custom(4, 17, "emptyCustom", ByteArray(0), 9_000L),
        )

        val expected = NveEventRingCodec.encode(events)
        val direct = ByteBuffer.allocateDirect(4096)
        val written = NveEventRingCodec.encodeInto(events, direct)

        assertEquals(expected.size, written)
        assertEquals(0, direct.position())
        assertEquals(written, direct.limit())

        val actual = ByteArray(written)
        direct.duplicate().get(actual)
        assertArrayEquals(expected, actual)

        // Direct buffer decodes identically back to the original event list.
        val decoded = NveEventRingCodec.decode(direct)
        assertEquals(events.size, decoded.size)
        for (i in events.indices) {
            if (i == 1) {
                // NaN != NaN under Float equality; check raw bits directly.
                val p = decoded[i].payload as NveEventPayload.Point
                assertEquals(Float.POSITIVE_INFINITY, p.x)
                assertTrue(p.y.isNaN())
            } else {
                assertEquals(events[i], decoded[i])
            }
        }

        // Single-event fast helper matches encodeSingle byte-for-byte.
        val singleEvent = events[3]
        val expectedSingle = NveEventRingCodec.encodeSingle(singleEvent)
        val writtenSingle = NveEventRingCodec.encodeSingleInto(singleEvent, direct)
        assertEquals(expectedSingle.size, writtenSingle)
        val actualSingle = ByteArray(writtenSingle)
        direct.duplicate().get(actualSingle)
        assertArrayEquals(expectedSingle, actualSingle)
    }

    @Test
    fun encodeIntoHandlesEmptyBatchExactBoundaryCapacityOverflowAndReadOnlyGuards() {
        val direct = ByteBuffer.allocateDirect(64)

        // Empty batch encodes 14-byte NVEV v2 header matching encode(emptyList()).
        val emptyWritten = NveEventRingCodec.encodeInto(emptyList(), direct)
        assertEquals(14, emptyWritten)
        val emptyActual = ByteArray(14)
        direct.duplicate().get(emptyActual)
        assertArrayEquals(NveEventRingCodec.encode(emptyList()), emptyActual)
        assertTrue(NveEventRingCodec.decode(direct).isEmpty())

        // Capacity < 14 rejected cleanly.
        for (cap in 0..13) {
            assertEquals(-1, NveEventRingCodec.encodeInto(emptyList(), ByteBuffer.allocateDirect(cap)))
        }

        // Exact-fit succeeds; exact-fit - 1 fails with -1 and resets buffer state.
        val sample = listOf(
            NveEventRingCodec.press(1, 2, 1f, 2f, 100L),
            NveEventRingCodec.textChange(1, 3, "hello café", 200L),
            NveEventRingCodec.custom(1, 4, "evt", byteArrayOf(9, 8, 7), 300L),
        )
        val exactBytes = NveEventRingCodec.encode(sample)
        val exactBuf = ByteBuffer.allocateDirect(exactBytes.size)
        assertEquals(exactBytes.size, NveEventRingCodec.encodeInto(sample, exactBuf))

        val tooSmallBuf = ByteBuffer.allocateDirect(exactBytes.size - 1)
        assertEquals(-1, NveEventRingCodec.encodeInto(sample, tooSmallBuf))
        assertEquals(0, tooSmallBuf.position())
        assertEquals(tooSmallBuf.capacity(), tooSmallBuf.limit())

        // Read-only buffer rejected cleanly.
        val readOnly = ByteBuffer.allocateDirect(256).asReadOnlyBuffer()
        assertEquals(-1, NveEventRingCodec.encodeInto(sample, readOnly))
        assertEquals(-1, NveEventRingCodec.encodeSingleInto(sample[0], readOnly))

        // NVEV v2 frames names with u32 lengths: a > uint16 custom name encodes and round-trips.
        val hugeName = "a".repeat(70_000)
        val hugeNameEvent = NveEventRingCodec.custom(1, 5, hugeName, byteArrayOf(1, 2))
        val hugeBuf = ByteBuffer.allocateDirect(128 * 1024)
        assertEquals(
            NveEventRingCodec.encode(listOf(hugeNameEvent)).size,
            NveEventRingCodec.encodeInto(listOf(hugeNameEvent), hugeBuf),
        )
        val hugeDecoded = NveEventRingCodec.decode(hugeBuf).single().payload as NveEventPayload.Custom
        assertEquals(hugeName, hugeDecoded.name)
        assertArrayEquals(byteArrayOf(1, 2), hugeDecoded.data)
    }

    @Test
    fun surfaceControllerReusesSingleDirectBufferAcrossOneThousandFlushesWithZeroArrayFallbacks() {
        val backend = DirectChannelBackend(eventScratchCapacity = 64 * 1024)
        val controller = NveSurfaceController(
            componentName = "NveScrollFeed",
            surfaceId = 11,
            bridge = backend,
        )
        assertTrue(controller.attach())
        assertEquals(1, backend.eventScratchRequests)
        val pinnedBuffer = backend.eventScratchBuffers.values.single()

        for (i in 0 until 1_000) {
            controller.enqueueEvent(
                NveEventRingCodec.scroll(
                    surfaceId = 11,
                    nodeId = 6,
                    x = 0f,
                    y = i.toFloat() * 4.5f,
                    contentWidth = 390f,
                    contentHeight = 5000f,
                    timestampUs = i.toLong() * 8_333L,
                ),
            )
            if (i % 5 == 0) {
                controller.enqueueEvent(NveEventRingCodec.press(11, 3, 12f, 24f, i.toLong()))
            }
            assertTrue(controller.flushEvents())
        }

        assertEquals(1, backend.eventScratchRequests)
        assertSame(pinnedBuffer, backend.eventScratchBuffers.values.single())
        assertEquals(1_000, backend.directEventSubmissions)
        assertEquals(0, backend.arrayEventFallbacks)
        assertEquals(1_000, backend.receivedEventPayloads.size)

        // Spot-check first, middle, and final decoded batches for exact fidelity.
        val firstBatch = NveEventRingCodec.decode(backend.receivedEventPayloads[0])
        assertEquals(2, firstBatch.size)
        assertEquals(NveEventKind.SCROLL, firstBatch[0].kind)
        assertEquals(NveEventKind.PRESS, firstBatch[1].kind)

        val lastBatch = NveEventRingCodec.decode(backend.receivedEventPayloads[999])
        assertEquals(1, lastBatch.size)
        assertEquals(
            NveEventPayload.Scroll(0f, 999f * 4.5f, 390f, 5000f),
            lastBatch[0].payload,
        )

        // Empty flush returns false without touching backend.
        assertFalse(controller.flushEvents())
        assertEquals(1_000, backend.directEventSubmissions)
    }

    @Test
    fun surfaceControllerFallsBackToByteArrayOnCapacityOverflowAndResumesDirectPath() {
        val backend = DirectChannelBackend(eventScratchCapacity = 128, propScratchCapacity = 64)
        val controller = NveSurfaceController(
            componentName = "NveEditor",
            surfaceId = 22,
            bridge = backend,
        )
        assertTrue(controller.attach())

        // 1. Small event batch fits in 128-byte scratch -> direct fast path.
        assertTrue(controller.dispatchPress(nodeId = 1, x = 5f, y = 6f, timestampUs = 10L))
        assertEquals(1, backend.directEventSubmissions)
        assertEquals(0, backend.arrayEventFallbacks)

        // 2. Giant text event exceeds 128-byte scratch -> transparent ByteArray fallback!
        val bigText = "X".repeat(300)
        assertTrue(controller.dispatchTextChange(nodeId = 2, text = bigText, timestampUs = 20L))
        assertEquals(1, backend.directEventSubmissions)
        assertEquals(1, backend.arrayEventFallbacks)
        val decodedBig = NveEventRingCodec.decode(backend.receivedEventPayloads.last()).single()
        assertEquals(NveEventPayload.Text(bigText), decodedBig.payload)

        // 3. Subsequent small batch resumes direct zero-copy path cleanly.
        assertTrue(controller.resize(width = 390f, height = 844f, scaleFactor = 3f, timestampUs = 30L))
        assertEquals(2, backend.directEventSubmissions)
        assertEquals(1, backend.arrayEventFallbacks)

        // 4. Small props fit in 64-byte prop scratch -> direct fast path, deduplicated on repeat.
        controller.updateProps(mapOf("k" to 42))
        controller.updateProps(mapOf("k" to 42))
        assertEquals(1, backend.directPropSubmissions)
        assertEquals(0, backend.arrayPropFallbacks)
        assertEquals(42L, NveBinValueView.parse(backend.receivedPropPayloads[0]).mapValue("k")!!.int64Value)

        // 5. Large props (> 64 bytes) fall back to setPropsBinary(ByteArray).
        controller.updateProps(mapOf("blob" to "Y".repeat(120)))
        assertEquals(1, backend.directPropSubmissions)
        assertEquals(1, backend.arrayPropFallbacks)
        assertEquals("Y".repeat(120), NveBinValueView.parse(backend.receivedPropPayloads[1]).mapValue("blob")!!.stringValue)

        // 6. Dispose clears cached buffers and rejects subsequent flushes.
        controller.dispose()
        controller.enqueueEvent(NveEventRingCodec.press(22, 1, 0f, 0f, 40L))
        assertFalse(controller.flushEvents())
        assertEquals(2, backend.directEventSubmissions)
        assertEquals(1, backend.arrayEventFallbacks)
    }

    @Test
    fun defaultBridgeBackendUsesByteArrayWithoutOverridingDirectMethods() {
        val backend = FakeNveBridgeBackend()
        assertEquals(null, backend.getEventScratchBuffer(100L))
        assertFalse(backend.submitEventsDirect(100L, 30))
        assertEquals(null, backend.getPropScratchBuffer(100L))
        assertFalse(backend.setPropsDirect(100L, 16))

        val controller = NveSurfaceController(componentName = "NveMock", surfaceId = 33, bridge = backend)
        assertTrue(controller.attach())
        assertTrue(controller.dispatchPress(nodeId = 4, x = 1f, y = 2f, timestampUs = 50L))
        assertEquals(1, backend.submittedEventRings.size)
        assertEquals(NveEventKind.PRESS, NveEventRingCodec.decode(backend.submittedEventRings.single()).single().kind)
    }
}
