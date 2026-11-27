package dev.nve.compose

import dev.nve.android.generated.NveCoreAbi
import dev.nve.android.generated.NveTables
import dev.nve.host.EventRingWriter
import dev.nve.host.NativeRuntime
import dev.nve.host.RuntimeSceneReader
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Assertions.assertNull
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** JVM double of `NativeRuntimeHost`: serves [TestScene] frames and records dispatched rings. */
class FakeRuntimeHost(var scene: TestScene) : NveRuntimeHost {
    var startStatus = NativeRuntime.STATUS_OK
    val started = ArrayList<NveSource>()
    val viewports = ArrayList<List<Float>>()
    val rings = ArrayList<ByteBuffer>()
    val scrolls = ArrayList<List<Float>>()
    var measurer: NativeRuntime.TextMeasurer? = null
    var closed = false
    private var listener: ((RuntimeSceneReader, NveResourceLookup) -> Unit)? = null

    override fun setFrameListener(listener: (RuntimeSceneReader, NveResourceLookup) -> Unit) {
        this.listener = listener
    }

    override fun start(source: NveSource): Int {
        started.add(source)
        return startStatus
    }

    override fun setViewport(widthDp: Float, heightDp: Float, density: Float): Int {
        viewports.add(listOf(widthDp, heightDp, density))
        return NativeRuntime.STATUS_OK
    }

    override fun setTextMeasurer(measurer: NativeRuntime.TextMeasurer?): Int {
        this.measurer = measurer
        return NativeRuntime.STATUS_OK
    }

    override fun render(): Boolean {
        listener?.invoke(scene.reader()) { null }
        return true
    }

    override fun dispatch(write: (EventRingWriter) -> Unit): Int {
        val ring = EventRingWriter(ByteBuffer.allocate(1024), 1)
        write(ring)
        rings.add(ByteBuffer.wrap(ring.buffer().array().copyOf(ring.length())).order(ByteOrder.LITTLE_ENDIAN))
        return 0
    }

    override fun scrollBy(nodeId: Int, dxDp: Float, dyDp: Float): Int {
        scrolls.add(listOf(nodeId.toFloat(), dxDp, dyDp))
        return NativeRuntime.STATUS_OK
    }

    override val lastError: String get() = "app library missing"

    override fun close() {
        closed = true
    }
}

class NveSurfaceControllerTest {
    private fun scene() = TestScene().node(2, 1, 0f, 0f, 100f, 40f).also {
        it.set(it.table(NveTables.Text.KEY, NveTables.Text.WORDS), 2, NveTables.Text.TEXT, "Hi")
    }

    /** `(count, node, kind, payload tag)` of the ring's first event. */
    private fun head(ring: ByteBuffer): List<Int> =
        listOf(ring.getInt(0), ring.getInt(8), ring.get(20).toInt(), ring.get(21).toInt())

    @Test
    fun startRendersTheFirstFrame() {
        val host = FakeRuntimeHost(scene())
        val c = NveSurfaceController(host)
        assertTrue(c.start(NveSource.App("/data/app/libnve_app.so")))
        assertEquals(listOf<NveSource>(NveSource.App("/data/app/libnve_app.so")), host.started)
        assertEquals("Hi", c.scene.nodes.getValue(0x1000002).text)
        assertNull(c.error)
    }

    @Test
    fun failedStartReportsTheRuntimeError() {
        val host = FakeRuntimeHost(scene()).apply { startStatus = NativeRuntime.STATUS_NOT_FOUND }
        val c = NveSurfaceController(host)
        assertFalse(c.start(NveSource.Paired("ws://pair.example.test:9000/nve-wire", "t")))
        assertEquals("start failed (6): app library missing", c.error)
        assertEquals(NveSceneSnapshot.EMPTY, c.scene)
    }

    @Test
    fun inputBecomesRuntimeEvents() {
        val host = FakeRuntimeHost(scene())
        val c = NveSurfaceController(host)
        c.press(0x1000002, 50f, 20f)
        c.textChanged(0x1000003, "h\u00e9")
        c.submit(0x1000003, "done")
        c.focusChanged(0x1000003, true)
        c.focusChanged(0x1000003, false)
        c.scrolled(0x1000004, 0f, 12f, 0f, 30f, 100f, 300f)
        assertEquals(listOf(1, 0x1000002, NveCoreAbi.EVENT_PRESS, 1), head(host.rings[0]))
        assertEquals(50f, host.rings[0].getFloat(22))
        assertEquals(listOf(1, 0x1000003, NveCoreAbi.EVENT_TEXT_CHANGE, 3), head(host.rings[1]))
        assertEquals(3, host.rings[1].getInt(22), "UTF-8 byte length")
        assertEquals(listOf(1, 0x1000003, NveCoreAbi.EVENT_SUBMIT, 3), head(host.rings[2]))
        assertEquals(NveCoreAbi.EVENT_FOCUS, head(host.rings[3])[2])
        assertEquals(NveCoreAbi.EVENT_BLUR, head(host.rings[4])[2])
        assertEquals(listOf(1, 0x1000004, NveCoreAbi.EVENT_SCROLL, 2), head(host.rings[5]))
        assertEquals(listOf(0x1000004.toFloat(), 0f, 12f), host.scrolls.single(), "native scroll syncs runtime offsets")
    }

    @Test
    fun viewportChangesAreDeduplicatedAndInvalidOnesIgnored() {
        val host = FakeRuntimeHost(scene())
        val c = NveSurfaceController(host)
        c.setViewport(360f, 800f, 2f)
        c.setViewport(360f, 800f, 2f)
        c.setViewport(0f, 800f, 2f)
        c.setViewport(Float.NaN, 800f, 2f)
        c.setViewport(400f, 800f, 2f)
        assertEquals(listOf(listOf(360f, 800f, 2f), listOf(400f, 800f, 2f)), host.viewports)
    }

    @Test
    fun measurerAndCloseReachTheHost() {
        val host = FakeRuntimeHost(scene())
        val c = NveSurfaceController(host)
        val m = NativeRuntime.TextMeasurer { _, _, _, _, out -> out[0] = 1f; out[1] = 1f; true }
        c.setTextMeasurer(m)
        assertEquals(m, host.measurer)
        c.setTextMeasurer(null)
        assertNull(host.measurer)
        c.close()
        assertTrue(host.closed)
    }

    @Test
    fun launchSourcesPreferPairingThenExplicitPathThenBundledApp() {
        assertEquals(NveSource.Paired("ws://h:1/x", "tok"), NveHostActivity.sourceOf("/a.so", " ws://h:1/x ", " tok ", "/b.so"))
        assertEquals(NveSource.App("/a.so"), NveHostActivity.sourceOf(" /a.so ", "  ", null, "/b.so"))
        assertEquals(NveSource.App("/b.so"), NveHostActivity.sourceOf(null, null, null, "/b.so"))
    }
}
