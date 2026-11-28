package dev.nve.compose

import dev.nve.host.BinReader
import dev.nve.host.BinTag
import dev.nve.host.BinWriter
import dev.nve.host.EventRingReader
import dev.nve.host.EventRingWriter
import dev.nve.host.FrameHost
import dev.nve.host.HybridRuntime
import dev.nve.host.NativeRuntime
import dev.nve.host.RuntimeSurface
import dev.nve.host.SceneReader
import dev.nve.host.generated.NveAbi
import dev.nve.host.generated.NveRuntimeAbi
import dev.nve.host.generated.NveTables
import org.junit.jupiter.api.Assertions.assertArrayEquals
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Assertions.assertNotNull
import org.junit.jupiter.api.Assertions.assertNull
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import java.nio.ByteBuffer

/** JVM double of `NativeRuntimeHost`: serves [TestScene] frames and records ABI v6 calls and `NVEV` v2 batches. */
class FakeRuntimeHost(var scene: TestScene) : NveRuntimeHost {
    var startStatus = NveRuntimeAbi.STATUS_OK
    var setPropsStatus = NveRuntimeAbi.STATUS_OK
    var hotPatchStatus = NveRuntimeAbi.STATUS_OK
    val started = ArrayList<NveSource>()
    val viewports = ArrayList<List<Float>>()
    val rings = ArrayList<EventRingReader>()
    val scrolls = ArrayList<Triple<Int, Float, Float>>()
    val propsHistory = ArrayList<ByteArray>()
    val hotPatches = ArrayList<ByteArray>()
    val hostCompletions = ArrayList<Triple<Long, Int, ByteArray?>>()
    val hostPushEvents = ArrayList<Triple<Int, Int, ByteArray?>>()
    val hostChannels = ArrayList<Triple<Int, Int, ByteArray?>>()
    var stateBytes: ByteArray? = byteArrayOf(1, 2, 3)
    var savedStateFiles = HashMap<String, Pair<Int, ByteArray>>()
    var savedFrame0Files = HashMap<String, Pair<Long, Long>>()
    var bootedFrame0Bytes: ByteArray? = null
    var measuredResult: RuntimeSurface.SurfaceMeasure? = RuntimeSurface.SurfaceMeasure(120f, 44f, 32f, 32f)
    var lastMeasureConstraints: FloatArray? = null
    var lastMeasureFlags: Int = 0
    var intrinsicListener: Runnable? = null
    var hostBridge: NveHostBridge? = null
    var measurer: NativeRuntime.TextMeasurer? = null
    var closed = false
    var synchronous = true
    private var listener: ((SceneReader, NveResourceLookup) -> Unit)? = null
    private var emittedListener: ((ByteArray) -> Unit)? = null
    private val pendingEmitted = ArrayDeque<ByteArray>()

    fun enqueueEmitted(write: (EventRingWriter) -> Unit) {
        val ring = EventRingWriter(ByteBuffer.allocate(4096), 1)
        write(ring)
        pendingEmitted.addLast(ring.buffer().array().copyOf(ring.length()))
    }

    private fun flushEmitted() {
        val cb = emittedListener ?: return
        while (pendingEmitted.isNotEmpty()) {
            cb(pendingEmitted.removeFirst())
        }
    }

    override fun setFrameListener(listener: (SceneReader, NveResourceLookup) -> Unit) {
        this.listener = listener
    }

    override fun setEmittedEventListener(listener: ((ByteArray) -> Unit)?) {
        this.emittedListener = listener
    }

    override fun start(source: NveSource): Int {
        started.add(source)
        if (startStatus == NveRuntimeAbi.STATUS_OK) synchronous = source is NveSource.App
        return startStatus
    }

    override fun setViewport(widthDp: Float, heightDp: Float, density: Float): Int {
        viewports.add(listOf(widthDp, heightDp, density))
        return NveRuntimeAbi.STATUS_OK
    }

    override fun setTextMeasurer(measurer: NativeRuntime.TextMeasurer?): Int {
        this.measurer = measurer
        return NveRuntimeAbi.STATUS_OK
    }

    /** Resources served to the frame listener by id. */
    val resources = HashMap<Int, NveResource>()
    private var frames = 0L

    /** Delivers [scene] as the next frame (consecutive frame ids, so captures chain). */
    override fun render(): Boolean {
        scene.frameId = ++frames
        listener?.invoke(scene.reader()) { resources[it] }
        flushEmitted()
        return true
    }

    override fun send(write: FrameHost.EventWriter): Int {
        val ring = EventRingWriter(ByteBuffer.allocate(4096), 1)
        write.write(ring)
        rings.add(EventRingReader().attach(ring.buffer().array().copyOf(ring.length())))
        flushEmitted()
        return 0
    }

    override fun scrollBy(nodeId: Int, dxDp: Float, dyDp: Float): Int {
        scrolls.add(Triple(nodeId, dxDp, dyDp))
        return NveRuntimeAbi.STATUS_OK
    }

    override fun setProps(binMap: ByteArray): Int {
        if (setPropsStatus != NveRuntimeAbi.STATUS_OK) return setPropsStatus
        propsHistory.add(binMap.copyOf())
        flushEmitted()
        return NveRuntimeAbi.STATUS_OK
    }

    override fun drainEmittedEvents(): ByteArray? =
        if (pendingEmitted.isEmpty()) null else pendingEmitted.removeFirst()

    override fun snapshotState(): ByteArray? = stateBytes?.copyOf()

    override fun restoreState(state: ByteArray): Int {
        stateBytes = state.copyOf()
        return NveRuntimeAbi.STATUS_OK
    }

    override fun saveStateFile(path: String, schemaVersion: Int): Int {
        savedStateFiles[path] = schemaVersion to (stateBytes?.copyOf() ?: ByteArray(0))
        return NveRuntimeAbi.STATUS_OK
    }

    override fun restoreStateFile(path: String, schemaVersion: Int): Int {
        val saved = savedStateFiles[path] ?: return NveRuntimeAbi.STATUS_NOT_FOUND
        if (saved.first != schemaVersion) return NveRuntimeAbi.STATUS_NOT_FOUND
        stateBytes = saved.second.copyOf()
        return NveRuntimeAbi.STATUS_OK
    }

    override fun clearStateFile(path: String): Int {
        savedStateFiles.remove(path)
        return NveRuntimeAbi.STATUS_OK
    }

    override fun saveFrame0File(path: String, appBuildId: Long, envHash: Long): Int {
        savedFrame0Files[path] = appBuildId to envHash
        return NveRuntimeAbi.STATUS_OK
    }

    override fun bootFrame0File(path: String, appBuildId: Long, envHash: Long): Int {
        val saved = savedFrame0Files[path] ?: return NveRuntimeAbi.STATUS_NOT_FOUND
        return if (saved == (appBuildId to envHash)) NveRuntimeAbi.STATUS_OK else NveRuntimeAbi.STATUS_NOT_FOUND
    }

    override fun bootFrame0Bytes(envelope: ByteArray, appBuildId: Long, envHash: Long): Int {
        bootedFrame0Bytes = envelope.copyOf()
        return NveRuntimeAbi.STATUS_OK
    }

    override fun applyHotPatch(patch: ByteArray): Int {
        if (hotPatchStatus != NveRuntimeAbi.STATUS_OK) return hotPatchStatus
        hotPatches.add(patch.copyOf())
        flushEmitted()
        return NveRuntimeAbi.STATUS_OK
    }

    override fun measureSurface(
        minWidthDp: Float,
        minHeightDp: Float,
        maxWidthDp: Float,
        maxHeightDp: Float,
        flags: Int,
    ): RuntimeSurface.SurfaceMeasure? {
        lastMeasureConstraints = floatArrayOf(minWidthDp, minHeightDp, maxWidthDp, maxHeightDp)
        lastMeasureFlags = flags
        return measuredResult
    }

    override fun setIntrinsicSizeInvalidatedListener(listener: Runnable?): Int {
        intrinsicListener = listener
        return NveRuntimeAbi.STATUS_OK
    }

    override fun setHostCallbackBridge(bridge: NveHostBridge?): Int {
        hostBridge = bridge
        return NveRuntimeAbi.STATUS_OK
    }

    override fun completeHostRequest(requestId: Long, hostStatus: Int, payload: ByteArray?): Int {
        hostCompletions.add(Triple(requestId, hostStatus, payload?.copyOf()))
        return NveRuntimeAbi.STATUS_OK
    }

    override fun pushHostEvent(serviceId: Int, eventId: Int, payload: ByteArray?): Int {
        hostPushEvents.add(Triple(serviceId, eventId, payload?.copyOf()))
        return NveRuntimeAbi.STATUS_OK
    }

    override fun publishHostChannel(channelId: Int, flags: Int, payload: ByteArray?): Int {
        hostChannels.add(Triple(channelId, flags, payload?.copyOf()))
        return NveRuntimeAbi.STATUS_OK
    }

    override val dispatchIsSynchronous: Boolean get() = synchronous

    override val lastError: String get() = "app library missing"

    override fun close() {
        closed = true
    }
}

class NveSurfaceControllerTest {
    private fun scene() = TestScene().node(2, 1, 0f, 0f, 100f, 40f).also {
        it.set(it.table(NveTables.Text.KEY, NveTables.Text.WORDS), 2, NveTables.Text.TEXT, "Hi")
    }

    @Test
    fun startRendersTheFirstFrame() {
        val host = FakeRuntimeHost(scene())
        val c = NveSurfaceController(host)
        assertTrue(c.start(NveSource.App("/data/app/libnve_app.so")))
        assertEquals(listOf<NveSource>(NveSource.App("/data/app/libnve_app.so")), host.started)
        assertEquals("Hi", c.scene.at(0x1000002).text)
        assertNull(c.error)
    }

    @Test
    fun failedStartReportsTheRuntimeError() {
        val host = FakeRuntimeHost(scene()).apply { startStatus = NveRuntimeAbi.STATUS_NOT_FOUND }
        val c = NveSurfaceController(host)
        assertFalse(c.start(NveSource.Paired("ws://pair.example.test:9000/nve-wire", "t")))
        assertEquals("start failed (6): app library missing", c.error)
        assertEquals(0L, c.scene.frameId)
        assertEquals(0, c.scene.size)
        assertTrue(c.dispatchIsSynchronous, "failed pair start keeps synchronous flag unchanged")
    }

    @Test
    fun dispatchIsSynchronousDelegatesToHost() {
        val host = FakeRuntimeHost(scene())
        val c = NveSurfaceController(host)
        assertTrue(c.dispatchIsSynchronous)
        assertTrue(c.start(NveSource.Paired("ws://pair.example.test:9000/nve-wire")))
        assertFalse(c.dispatchIsSynchronous)
        host.synchronous = true
        assertTrue(c.dispatchIsSynchronous, "delegates live to host rather than caching locally")
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
        c.scrolled(0xFF000005.toInt(), 0f, 1f, 0f, 1f, 100f, 300f)
        assertEquals(1, host.rings[0].count())
        assertTrue(host.rings[0].isEvent(NveAbi.TABLE_INTERACTION, NveAbi.INTERACTION_EVENT_PRESS))
        assertEquals(0x1000002, host.rings[0].node(0))
        assertEquals(50f to 20f, host.rings[0].payloadF32(0, 0) to host.rings[0].payloadF32(0, 1))
        assertEquals(1, host.rings[1].count())
        assertTrue(host.rings[1].isEvent(NveAbi.TABLE_TEXT_INPUT, NveAbi.TEXT_INPUT_EVENT_CHANGE))
        assertEquals(0x1000003, host.rings[1].node(0))
        assertEquals(3, host.rings[1].bytesLen(0), "UTF-8 byte length")
        assertEquals("h\u00e9", host.rings[1].text(0))
        assertTrue(host.rings[2].isEvent(NveAbi.TABLE_TEXT_INPUT, NveAbi.TEXT_INPUT_EVENT_SUBMIT))
        assertEquals(0x1000003, host.rings[2].node(0))
        assertEquals("done", host.rings[2].text(0))
        assertTrue(host.rings[3].isEvent(NveAbi.TABLE_INTERACTION, NveAbi.INTERACTION_EVENT_FOCUS))
        assertEquals(0x1000003, host.rings[3].node(0))
        assertTrue(host.rings[4].isEvent(NveAbi.TABLE_INTERACTION, NveAbi.INTERACTION_EVENT_BLUR))
        assertEquals(0x1000003, host.rings[4].node(0))
        assertTrue(host.rings[5].isEvent(NveAbi.TABLE_SCROLL, NveAbi.SCROLL_EVENT_SCROLL))
        assertEquals(0x1000004, host.rings[5].node(0))
        assertEquals(
            listOf(0f, 30f, 100f, 300f),
            List(4) { host.rings[5].payloadF32(0, it) },
        )
        assertEquals(
            listOf(Triple(0x1000004, 0f, 12f), Triple(0xFF000005.toInt(), 0f, 1f)), host.scrolls,
            "native scroll syncs runtime offsets with exact raw u32 ids (no float truncation)",
        )
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
        val m = NativeRuntime.TextMeasurer { _, _, _, _, out -> out[0] = 1f; out[1] = 1f }
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

    @Test
    fun abiV6PropsEventsStateFrame0HotPatchMeasureAndHostBridge() {
        val host = FakeRuntimeHost(scene())
        val c = NveSurfaceController(host)
        val seenEvents = ArrayList<NveEmittedEvent>()
        c.onEvent = { seenEvents.add(it) }

        val replyPayload = BinWriter().apply {
            val m = beginMap(1)
            writeMapKey("count")
            writeI64(7)
            end(m)
        }.toByteArray()
        host.enqueueEmitted { it.custom(0, 123L, "onCountChange", replyPayload) }

        assertTrue(c.setProps { w ->
            val m = w.beginMap(1)
            w.writeMapKey("initialCount")
            w.writeI64(7)
            w.end(m)
        })
        assertEquals(1, host.propsHistory.size)
        val r = BinReader(host.propsHistory[0])
        assertEquals(BinTag.MAP, r.peekTag())
        val m = r.beginMap()
        assertEquals(1, m.count)
        assertEquals("initialCount", r.readMapKey())
        assertEquals(7L, r.readI64())
        r.end(m)

        assertEquals(1, seenEvents.size)
        assertTrue(seenEvents[0].isCustom)
        assertEquals("onCountChange", seenEvents[0].name)
        val evReader = seenEvents[0].payloadReader()
        val evMark = evReader.beginMap()
        assertEquals("count", evReader.readMapKey())
        assertEquals(7L, evReader.readI64())
        evReader.end(evMark)

        assertArrayEquals(byteArrayOf(1, 2, 3), c.snapshotState())
        assertTrue(c.restoreState(byteArrayOf(9, 8)))
        assertArrayEquals(byteArrayOf(9, 8), c.snapshotState())
        assertTrue(c.saveStateFile("/tmp/state.nve", 1))
        assertTrue(c.restoreState(byteArrayOf(0)))
        assertTrue(c.restoreStateFile("/tmp/state.nve", 1))
        assertArrayEquals(byteArrayOf(9, 8), c.snapshotState())
        assertTrue(c.clearStateFile("/tmp/state.nve"))
        assertFalse(c.restoreStateFile("/tmp/state.nve", 1))

        assertTrue(c.saveFrame0File("/tmp/f0.nve2", 42L, 99L))
        assertTrue(c.bootFrame0File("/tmp/f0.nve2", 42L, 99L))
        assertFalse(c.bootFrame0File("/tmp/f0.nve2", 43L, 99L))
        assertTrue(c.bootFrame0Bytes(byteArrayOf(4, 5, 6), 42L, 99L))
        assertArrayEquals(byteArrayOf(4, 5, 6), host.bootedFrame0Bytes)

        assertTrue(c.applyHotPatch(byteArrayOf(0x4E, 0x56, 0x50, 0x31)))
        assertEquals(1, host.hotPatches.size)

        val measured = c.measureSurface(0f, 0f, 320f, Float.POSITIVE_INFINITY, NveRuntimeAbi.MEASURE_FLAG_DRY_RUN)
        assertNotNull(measured)
        assertEquals(120f, measured!!.width)
        assertEquals(44f, measured.height)
        assertTrue(measured.hasFirstBaseline())
        assertEquals(32f, measured.firstBaseline)

        var invalidated = 0
        val listener = Runnable { invalidated++ }
        assertEquals(NveRuntimeAbi.STATUS_OK, c.setIntrinsicSizeInvalidatedListener(listener))
        host.intrinsicListener?.run()
        assertEquals(1, invalidated)

        val bridge = object : NveHostBridge {
            override fun invoke(
                serviceId: Int,
                methodId: Int,
                requestId: Long,
                request: ByteArray,
                outPayload: Array<ByteArray?>,
            ): Int = HybridRuntime.HostStatus.OK.code

            override fun publishChannel(channelId: Int, flags: Int, payload: ByteArray): Int =
                HybridRuntime.HostStatus.OK.code
        }
        assertEquals(NveRuntimeAbi.STATUS_OK, c.setHostBridge(bridge))
        assertEquals(bridge, host.hostBridge)
        assertEquals(NveRuntimeAbi.STATUS_OK, c.completeHostRequest(10L, HybridRuntime.HostStatus.OK.code, byteArrayOf(7)))
        assertEquals(NveRuntimeAbi.STATUS_OK, c.pushHostEvent(0x0010, 1, byteArrayOf(8)))
        assertEquals(NveRuntimeAbi.STATUS_OK, c.publishHostChannel(42, 1, byteArrayOf(9)))
        assertEquals(1, host.hostCompletions.size)
        assertEquals(1, host.hostPushEvents.size)
        assertEquals(1, host.hostChannels.size)
    }
}
