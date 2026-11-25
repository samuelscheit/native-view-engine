package dev.nve.nitro

import org.junit.jupiter.api.Assertions.assertArrayEquals
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Assertions.assertNotNull
import org.junit.jupiter.api.Assertions.assertNull
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.StandardCharsets
import java.util.Collections
import java.util.IdentityHashMap

/**
 * JVM unit tests verifying [NveNitroRuntime], [NveHybridObject], and [NveDirectScratchBuffer]
 * direct-buffer binary roundtrips, raw-pointer handle lifecycle, and error/buffer guards.
 */
class NveNitroRuntimeTest {

    private class FakeNitroBackend : NveNitroBackend {
        private class Entry(
            val name: String,
            var value: Long = 10L,
            var disposed: Boolean = false,
        )

        override var isAvailable: Boolean = true
        private var nextPtr: Long = 0x1000_0000L
        private val objects = LinkedHashMap<Long, Entry>()
        private val liveBuffers = Collections.newSetFromMap(IdentityHashMap<ByteBuffer, Boolean>())

        var disposeCalls: Int = 0
        var invokeCalls: Int = 0

        fun liveObjectCount(): Int = objects.size
        fun liveBufferCount(): Int = liveBuffers.size

        override fun createHybridObject(name: String): Long {
            if (name != "Counter") return 0L
            val ptr = nextPtr
            nextPtr += 0x100L
            objects[ptr] = Entry(name)
            return ptr
        }

        override fun disposeHybridObject(hybridPtr: Long) {
            disposeCalls += 1
            objects.remove(hybridPtr)?.disposed = true
        }

        override fun getMemorySize(hybridPtr: Long): Long {
            val entry = objects[hybridPtr] ?: return 0L
            return if (entry.disposed) 0L else 128L
        }

        override fun allocDirectBuffer(bytes: Int): ByteBuffer? {
            if (bytes <= 0 || bytes > 64 * 1024 * 1024) return null
            val buf = ByteBuffer.allocateDirect(bytes).order(ByteOrder.LITTLE_ENDIAN)
            liveBuffers.add(buf)
            return buf
        }

        override fun freeDirectBuffer(directBuffer: ByteBuffer?) {
            if (directBuffer != null) {
                liveBuffers.remove(directBuffer)
            }
        }

        override fun invokeBinary(
            hybridPtr: Long,
            methodId: Int,
            directIn: ByteBuffer?,
            inLen: Int,
            directOut: ByteBuffer?,
        ): Int {
            invokeCalls += 1
            val entry = objects[hybridPtr] ?: return NveNitroRuntime.ERR_INVALID_HANDLE
            if (entry.disposed) return NveNitroRuntime.ERR_DISPOSED
            return when (methodId) {
                NveNitroRuntime.METHOD_ECHO_BYTES -> {
                    if (inLen == 0) return 0
                    val src = directIn ?: return NveNitroRuntime.ERR_INVALID_BUFFER
                    val dst = directOut ?: return NveNitroRuntime.ERR_BUFFER_TOO_SMALL
                    if (dst.capacity() < inLen) return NveNitroRuntime.ERR_BUFFER_TOO_SMALL
                    val tmp = ByteArray(inLen)
                    val s = src.duplicate()
                    s.position(0)
                    s.limit(inLen)
                    s.get(tmp)
                    dst.clear()
                    dst.put(tmp)
                    inLen
                }
                NveNitroRuntime.METHOD_PING -> {
                    val reply = "pong".toByteArray(StandardCharsets.UTF_8)
                    val dst = directOut ?: return NveNitroRuntime.ERR_BUFFER_TOO_SMALL
                    if (dst.capacity() < reply.size) return NveNitroRuntime.ERR_BUFFER_TOO_SMALL
                    dst.clear()
                    dst.put(reply)
                    reply.size
                }
                NveNitroRuntime.METHOD_INCREMENT -> {
                    val delta = if (inLen == 1 && directIn != null) (directIn.get(0).toInt() and 0xFF).toLong() else 1L
                    entry.value += delta
                    val dst = directOut ?: return NveNitroRuntime.ERR_BUFFER_TOO_SMALL
                    if (dst.capacity() < 8) return NveNitroRuntime.ERR_BUFFER_TOO_SMALL
                    dst.clear()
                    dst.order(ByteOrder.LITTLE_ENDIAN)
                    dst.putLong(entry.value)
                    8
                }
                NveNitroRuntime.METHOD_FAIL -> NveNitroRuntime.ERR_INVOCATION_FAILED
                else -> NveNitroRuntime.ERR_NOT_FOUND
            }
        }
    }

    @Test
    fun hybridObjectLifecycleAndThousandDirectBufferRoundtrips() {
        val backend = FakeNitroBackend()
        assertNull(NveNitroRuntime.createHybridObject("", backend))
        assertNull(NveNitroRuntime.createHybridObject("Unknown", backend))

        val obj = NveNitroRuntime.createHybridObject("Counter", backend)
        assertNotNull(obj)
        obj!!
        assertEquals("Counter", obj.name)
        assertFalse(obj.isDisposed)
        assertTrue(obj.rawPointer != 0L)
        assertEquals(128L, obj.memorySize())
        assertEquals(1, backend.liveObjectCount())

        val inScratch = NveNitroRuntime.allocDirectBuffer(256, backend)!!
        val outScratch = NveNitroRuntime.allocDirectBuffer(256, backend)!!
        assertTrue(inScratch.buffer.isDirect)
        assertTrue(outScratch.buffer.isDirect)
        assertEquals(2, backend.liveBufferCount())

        // 1,000 reusable DirectByteBuffer echo roundtrips.
        for (i in 0 until 1_000) {
            inScratch.buffer.clear()
            inScratch.buffer.putInt(i)
            inScratch.buffer.putInt(i * 31)
            val written = obj.invokeBinary(NveNitroRuntime.METHOD_ECHO_BYTES, inScratch.buffer, 8, outScratch.buffer)
            assertEquals(8, written)
            assertEquals(0, outScratch.buffer.position())
            assertEquals(8, outScratch.buffer.limit())
            assertEquals(i, outScratch.buffer.getInt(0))
            assertEquals(i * 31, outScratch.buffer.getInt(4))
        }
        assertEquals(1_000, backend.invokeCalls)

        // Increment by 7 -> 17 little-endian i64.
        inScratch.buffer.put(0, 7)
        assertEquals(8, obj.invokeBinary(NveNitroRuntime.METHOD_INCREMENT, inScratch.buffer, 1, outScratch.buffer))
        assertEquals(17L, outScratch.buffer.getLong(0))

        // Zero-input ping -> "pong".
        assertEquals(4, obj.invokeBinary(NveNitroRuntime.METHOD_PING, null, 0, outScratch.buffer))
        val pong = ByteArray(4)
        outScratch.buffer.get(pong)
        assertArrayEquals("pong".toByteArray(StandardCharsets.UTF_8), pong)

        // Idempotent close/dispose prevents double-free and rejects post-dispose calls.
        obj.close()
        obj.dispose()
        assertTrue(obj.isDisposed)
        assertEquals(0L, obj.rawPointer)
        assertEquals(0L, obj.memorySize())
        assertEquals(1, backend.disposeCalls)
        assertEquals(0, backend.liveObjectCount())
        assertEquals(
            NveNitroRuntime.ERR_DISPOSED,
            obj.invokeBinary(NveNitroRuntime.METHOD_PING, null, 0, outScratch.buffer),
        )

        // Direct buffer wrapper close is idempotent and releases all tracked native buffers.
        inScratch.close()
        inScratch.close()
        outScratch.close()
        assertTrue(inScratch.isClosed)
        assertEquals(0, backend.liveBufferCount())
    }

    @Test
    fun guardsNonDirectBuffersOverflowAndErrorCodes() {
        val backend = FakeNitroBackend()
        assertNull(NveNitroRuntime.allocDirectBuffer(0, backend))
        assertNull(NveNitroRuntime.allocDirectBuffer(-8, backend))

        NveNitroRuntime.createHybridObject("Counter", backend)!!.use { obj ->
            val heapBuf = ByteBuffer.allocate(64)
            val directBuf = ByteBuffer.allocateDirect(64)
            val tinyDirect = ByteBuffer.allocateDirect(2)

            // Heap (non-direct) ByteBuffers rejected before crossing JNI.
            assertEquals(
                NveNitroRuntime.ERR_INVALID_BUFFER,
                obj.invokeBinary(NveNitroRuntime.METHOD_ECHO_BYTES, heapBuf, 4, directBuf),
            )
            assertEquals(
                NveNitroRuntime.ERR_INVALID_BUFFER,
                obj.invokeBinary(NveNitroRuntime.METHOD_PING, null, 0, heapBuf),
            )
            assertEquals(
                NveNitroRuntime.ERR_INVALID_BUFFER,
                obj.invokeBinary(NveNitroRuntime.METHOD_ECHO_BYTES, directBuf, -1, directBuf),
            )
            assertEquals(
                NveNitroRuntime.ERR_INVALID_BUFFER,
                obj.invokeBinary(NveNitroRuntime.METHOD_ECHO_BYTES, directBuf, 65, directBuf),
            )
            assertEquals(
                NveNitroRuntime.ERR_INVALID_BUFFER,
                obj.invokeBinary(NveNitroRuntime.METHOD_ECHO_BYTES, null, 4, directBuf),
            )
            assertEquals(0, backend.invokeCalls)

            // Native error codes propagate cleanly.
            assertEquals(
                NveNitroRuntime.ERR_BUFFER_TOO_SMALL,
                obj.invokeBinary(NveNitroRuntime.METHOD_PING, null, 0, tinyDirect),
            )
            assertEquals(
                NveNitroRuntime.ERR_NOT_FOUND,
                obj.invokeBinary(999, null, 0, directBuf),
            )
            assertEquals(
                NveNitroRuntime.ERR_INVOCATION_FAILED,
                obj.invokeBinary(NveNitroRuntime.METHOD_FAIL, null, 0, directBuf),
            )
        }
        assertEquals(0, backend.liveObjectCount())
    }
}
