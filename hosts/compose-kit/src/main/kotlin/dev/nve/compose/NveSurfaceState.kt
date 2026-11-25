package dev.nve.compose

import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLong

/**
 * Rendering backend used for an embedded NVE surface inside Jetpack Compose.
 */
enum class NveRenderBackend {
    FABRIC,
    SKIA,
}

/**
 * Lifecycle state of an [NveSurfaceController] and its [NveSurfaceState].
 */
enum class NveSurfaceLifecycleState {
    DETACHED,
    ATTACHED,
    DISPOSED,
}

/**
 * Logical viewport metrics (dp + density scale factor) of a hosted NVE surface.
 */
data class NveViewport(
    val width: Float = 0f,
    val height: Float = 0f,
    val scaleFactor: Float = 1f,
) {
    init {
        require(width.isFinite() && width >= 0f) { "Viewport width must be finite and >= 0: $width" }
        require(height.isFinite() && height >= 0f) { "Viewport height must be finite and >= 0: $height" }
        require(scaleFactor.isFinite() && scaleFactor > 0f) {
            "Viewport scaleFactor must be finite and > 0: $scaleFactor"
        }
    }
}

/**
 * Zero-JSON reactive host state for an NVE surface.
 *
 * Holds the reusable zero-copy [sceneBuffer], incrementally updated [mountedNodes] keyed by
 * `nodeId`, current [viewport], and lifecycle metadata.
 */
class NveSurfaceState(val surfaceId: Int) {
    val sceneBuffer: NveSceneBuffer = NveSceneBuffer()

    var lifecycleState: NveSurfaceLifecycleState = NveSurfaceLifecycleState.DETACHED
        internal set

    var viewport: NveViewport = NveViewport()
        internal set

    var lastFrameId: Long = 0L
        private set

    var lastDiff: NveSceneDiff? = null
        private set

    var reloadGeneration: Int = 0
        internal set

    private val _mountedNodes = LinkedHashMap<Int, NveRenderNodeSnapshot>()
    val mountedNodes: Map<Int, NveRenderNodeSnapshot>
        get() = _mountedNodes

    private val _nodeMetadata = HashMap<Int, NveNodeMetadata>()
    val nodeMetadata: Map<Int, NveNodeMetadata>
        get() = _nodeMetadata

    fun applySceneFrame(meta: LongArray, slabs: NveSceneSlabs): NveSceneDiff {
        sceneBuffer.attachFromMeta(meta, slabs)
        val diff = sceneBuffer.applyIncrementalDiff(
            mountedNodes = _mountedNodes,
            metadataById = _nodeMetadata,
            metadataSlab = slabs.metadataSlab,
        )
        lastFrameId = diff.frameId
        lastDiff = diff
        return diff
    }

    internal fun clearScene() {
        sceneBuffer.clear()
        _mountedNodes.clear()
        _nodeMetadata.clear()
        lastFrameId = 0L
        lastDiff = null
    }
}

/**
 * Shared-state & pub/sub client mirroring Rust's `NveChannelBus` for Jetpack Compose hosts.
 * Uses raw bytes and compact binary [NveBinValue] payloads with zero JSON serialization.
 */
class NveChannelClient {
    data class Message(
        val topic: String,
        val payload: ByteArray,
        val seq: Long,
    ) {
        fun parseBinValue(): NveBinValueView = NveBinValueView.parse(payload)

        override fun equals(other: Any?): Boolean {
            if (this === other) return true
            if (other !is Message) return false
            return topic == other.topic && payload.contentEquals(other.payload) && seq == other.seq
        }

        override fun hashCode(): Int {
            var result = topic.hashCode()
            result = 31 * result + payload.contentHashCode()
            result = 31 * result + seq.hashCode()
            return result
        }
    }

    private val seqGen = AtomicLong(0L)
    private val retained = ConcurrentHashMap<String, Message>()
    private val listeners = ConcurrentHashMap<String, CopyOnWriteArrayList<(Message) -> Unit>>()

    fun subscribe(
        topic: String,
        replayRetained: Boolean = true,
        listener: (Message) -> Unit,
    ): () -> Unit {
        val list = listeners.computeIfAbsent(topic) { CopyOnWriteArrayList() }
        list.add(listener)
        if (replayRetained) {
            retained[topic]?.let(listener)
        }
        return { list.remove(listener) }
    }

    fun publish(topic: String, payload: ByteArray, retain: Boolean = false) {
        val msg = Message(topic = topic, payload = payload, seq = seqGen.incrementAndGet())
        if (retain) {
            retained[topic] = msg
        }
        listeners[topic]?.forEach { it(msg) }
    }

    fun publishBinValue(topic: String, value: NveBinValue, retain: Boolean = false) {
        publish(topic, value.encode(), retain = retain)
    }

    fun publishMap(topic: String, data: Map<String, Any?>, retain: Boolean = false) {
        publish(topic, NveBinValue.encodeMap(data), retain = retain)
    }
}

/**
 * Imperative controller for an embedded [NveSurface] composable or [NveHostActivity].
 *
 * Encodes props/commands with [NveBinValue], batches input/lifecycle events into binary
 * `EventRing` buffers via [NveEventRingCodec], and consumes frames directly through
 * [NveSceneBuffer] with zero JSON serialization.
 */
class NveSurfaceController(
    val componentName: String,
    val vtableSymbol: String = "nve_get_interface",
    val backend: NveRenderBackend = NveRenderBackend.FABRIC,
    val surfaceId: Int = nextSurfaceId.getAndIncrement(),
    private val bridge: NveBridgeBackend = NveNativeBridge,
) {
    companion object {
        private val nextSurfaceId = AtomicInteger(1)
    }

    val state: NveSurfaceState = NveSurfaceState(surfaceId)

    private var nativeHandle: Long = 0L
    private var frameCounter: Long = 0L
    private val propsWriter = NveBinWriter(128)
    private val commandWriter = NveBinWriter(128)
    private var lastPropsBuf: ByteArray = ByteArray(0)
    private var lastPropsLen: Int = -1
    private val pendingEvents = ArrayList<NveEvent>()
    private val frameMetaScratch = LongArray(NveSceneBuffer.META_FIELD_COUNT)
    internal var onEventCallback: ((NveEvent) -> Unit)? = null

    val isMounted: Boolean
        get() = nativeHandle != 0L && state.lifecycleState == NveSurfaceLifecycleState.ATTACHED

    val isDisposed: Boolean
        get() = state.lifecycleState == NveSurfaceLifecycleState.DISPOSED

    val pendingEventCount: Int
        get() = pendingEvents.size

    fun attach(): Boolean {
        if (isDisposed) return false
        if (nativeHandle != 0L) return true
        if (!bridge.isAvailable) return false

        val handle = bridge.createSurface(
            surfaceId = surfaceId,
            componentName = componentName,
            vtableSymbol = vtableSymbol,
            backendCode = backend.ordinal,
        )
        if (handle == 0L) return false

        nativeHandle = handle
        state.lifecycleState = NveSurfaceLifecycleState.ATTACHED

        // Re-sync remembered viewport and props when re-attaching after a detach.
        val vp = state.viewport
        if (vp.width > 0f || vp.height > 0f) {
            bridge.resizeSurface(handle, vp.width, vp.height, vp.scaleFactor)
        }
        if (lastPropsLen >= 0) {
            bridge.setPropsBinary(handle, lastPropsBuf.copyOf(lastPropsLen))
        }
        return true
    }

    internal fun mountIfNeeded() {
        attach()
    }

    /**
     * Updates logical viewport dimensions, notifies the native host, and submits a binary
     * `LAYOUT` event on the root node (`nodeId = 0`) when the surface is attached.
     */
    fun resize(
        width: Float,
        height: Float,
        scaleFactor: Float = state.viewport.scaleFactor,
        timestampUs: Long = 0L,
    ): Boolean {
        if (isDisposed) return false
        val next = NveViewport(width, height, scaleFactor)
        if (state.viewport == next && nativeHandle != 0L) {
            return true
        }
        state.viewport = next
        if (nativeHandle == 0L || !bridge.isAvailable) return false

        val resized = bridge.resizeSurface(nativeHandle, width, height, scaleFactor)
        if (resized) {
            val layoutBytes = NveEventRingCodec.encodeSingle(
                NveEventRingCodec.layout(
                    surfaceId = surfaceId,
                    nodeId = 0,
                    x = 0f,
                    y = 0f,
                    width = width,
                    height = height,
                    timestampUs = timestampUs,
                ),
            )
            bridge.submitEvents(nativeHandle, layoutBytes)
        }
        return resized
    }

    fun updateProps(props: Map<String, Any?>) {
        if (isDisposed) return
        propsWriter.reset()
        propsWriter.writeMap(props)
        if (lastPropsLen == propsWriter.size && propsWriter.contentEquals(lastPropsBuf, lastPropsLen)) {
            return
        }
        lastPropsBuf = propsWriter.copyInto(lastPropsBuf)
        lastPropsLen = propsWriter.size
        if (nativeHandle == 0L || !bridge.isAvailable) return
        if (bridge.setPropsBinary(nativeHandle, propsWriter.toByteArray())) {
            requestRender()
        }
        drainAndForwardEvents()
    }

    fun updatePropsBinary(propsBin: ByteArray) {
        if (isDisposed) return
        if (lastPropsLen == propsBin.size && regionEquals(lastPropsBuf, propsBin, propsBin.size)) {
            return
        }
        if (lastPropsBuf.size < propsBin.size) {
            lastPropsBuf = ByteArray(maxOf(propsBin.size, lastPropsBuf.size shl 1))
        }
        if (propsBin.isNotEmpty()) {
            System.arraycopy(propsBin, 0, lastPropsBuf, 0, propsBin.size)
        }
        lastPropsLen = propsBin.size
        if (nativeHandle == 0L || !bridge.isAvailable) return
        if (bridge.setPropsBinary(nativeHandle, propsBin)) {
            requestRender()
        }
        drainAndForwardEvents()
    }

    fun dispatchCommand(name: String, args: Map<String, Any?> = emptyMap()) {
        if (isDisposed || nativeHandle == 0L || !bridge.isAvailable) return
        commandWriter.reset()
        commandWriter.writeMap(args)
        dispatchCommandBinary(name, commandWriter.toByteArray())
    }

    fun dispatchCommandBinary(name: String, argsBin: ByteArray) {
        if (isDisposed || nativeHandle == 0L || !bridge.isAvailable) return
        if (bridge.dispatchCommandBinary(nativeHandle, name, argsBin)) {
            requestRender()
        }
        drainAndForwardEvents()
    }

    fun enqueueEvent(event: NveEvent) {
        if (isDisposed) return
        pendingEvents.add(event)
    }

    fun flushEvents(): Boolean {
        if (isDisposed || nativeHandle == 0L || !bridge.isAvailable || pendingEvents.isEmpty()) {
            return false
        }
        val encoded = NveEventRingCodec.encode(pendingEvents)
        pendingEvents.clear()
        val ok = bridge.submitEvents(nativeHandle, encoded)
        if (ok) {
            drainAndForwardEvents()
        }
        return ok
    }

    fun dispatchPress(
        nodeId: Int,
        x: Float,
        y: Float,
        timestampUs: Long = 0L,
    ): Boolean {
        enqueueEvent(NveEventRingCodec.press(surfaceId, nodeId, x, y, timestampUs))
        return flushEvents()
    }

    fun dispatchTextChange(
        nodeId: Int,
        text: String,
        timestampUs: Long = 0L,
    ): Boolean {
        enqueueEvent(NveEventRingCodec.textChange(surfaceId, nodeId, text, timestampUs))
        return flushEvents()
    }

    fun dispatchScroll(
        nodeId: Int,
        x: Float,
        y: Float,
        contentWidth: Float,
        contentHeight: Float,
        timestampUs: Long = 0L,
    ): Boolean {
        enqueueEvent(
            NveEventRingCodec.scroll(surfaceId, nodeId, x, y, contentWidth, contentHeight, timestampUs),
        )
        return flushEvents()
    }

    fun dispatchLayout(
        nodeId: Int,
        x: Float,
        y: Float,
        width: Float,
        height: Float,
        timestampUs: Long = 0L,
    ): Boolean {
        enqueueEvent(NveEventRingCodec.layout(surfaceId, nodeId, x, y, width, height, timestampUs))
        return flushEvents()
    }

    fun dispatchCustomEvent(
        nodeId: Int,
        name: String,
        payload: NveBinValue = NveBinValue.Null,
        timestampUs: Long = 0L,
    ): Boolean {
        commandWriter.reset()
        commandWriter.writeValue(payload)
        enqueueEvent(NveEventRingCodec.custom(surfaceId, nodeId, name, commandWriter.toByteArray(), timestampUs))
        return flushEvents()
    }

    fun requestRender(): NveSceneDiff? {
        if (isDisposed || nativeHandle == 0L || !bridge.isAvailable) return null
        frameCounter += 1L
        frameMetaScratch.fill(0L)
        val renderedNodes = bridge.renderFrame(nativeHandle, frameCounter, frameMetaScratch)
        if (renderedNodes < 0) return null
        val slabs = bridge.getSceneSlabs(nativeHandle) ?: return null
        return state.applySceneFrame(frameMetaScratch, slabs)
    }

    fun drainEvents(): List<NveEvent> {
        if (isDisposed || nativeHandle == 0L || !bridge.isAvailable) return emptyList()
        val bytes = bridge.drainEmittedEvents(nativeHandle) ?: return emptyList()
        return NveEventRingCodec.decode(bytes)
    }

    internal fun drainAndForwardEvents() {
        val cb = onEventCallback ?: return
        for (event in drainEvents()) {
            cb(event)
        }
    }

    /**
     * Hot-reloads the Rust component shared library while preserving [surfaceId] and [state.viewport].
     */
    fun reload(soPath: String): Boolean {
        if (isDisposed || nativeHandle == 0L || !bridge.isAvailable || soPath.isBlank()) {
            return false
        }
        val ok = bridge.hotReloadApp(nativeHandle, soPath)
        if (ok) {
            state.reloadGeneration += 1
            val vp = state.viewport
            if (vp.width > 0f || vp.height > 0f) {
                bridge.resizeSurface(nativeHandle, vp.width, vp.height, vp.scaleFactor)
            }
            requestRender()
            drainAndForwardEvents()
        }
        return ok
    }

    internal fun measureIntrinsic(
        minW: Float,
        minH: Float,
        maxW: Float,
        maxH: Float,
    ): Pair<Float, Float>? {
        if (isDisposed || nativeHandle == 0L || !bridge.isAvailable) return null
        val result = bridge.measureSurface(nativeHandle, minW, minH, maxW, maxH)
        if (result == null || result.size < 2) return null
        return result[0] to result[1]
    }

    internal fun arbitrateTouch(
        pointerId: Int,
        phase: Int,
        x: Float,
        y: Float,
    ): NveGestureDisposition {
        if (isDisposed || nativeHandle == 0L || !bridge.isAvailable) {
            return NveGestureDisposition.CLAIM
        }
        val code = bridge.hitTestPointer(nativeHandle, pointerId, phase, x, y)
        return NveGestureDisposition.fromCode(code)
    }

    /**
     * Releases the active native surface handle while keeping configuration & viewport so a later
     * [attach] can restore the surface under the same [surfaceId].
     */
    fun detach() {
        if (isDisposed) return
        if (nativeHandle != 0L && bridge.isAvailable) {
            flushEvents()
            bridge.destroySurface(nativeHandle)
        }
        nativeHandle = 0L
        pendingEvents.clear()
        state.clearScene()
        state.lifecycleState = NveSurfaceLifecycleState.DETACHED
    }

    /**
     * Permanently disposes this controller, releases native resources, and clears callbacks.
     * Safe to invoke multiple times.
     */
    fun dispose() {
        if (isDisposed) return
        if (nativeHandle != 0L && bridge.isAvailable) {
            bridge.destroySurface(nativeHandle)
        }
        nativeHandle = 0L
        pendingEvents.clear()
        lastPropsLen = -1
        lastPropsBuf = ByteArray(0)
        onEventCallback = null
        state.clearScene()
        state.lifecycleState = NveSurfaceLifecycleState.DISPOSED
    }

    private fun regionEquals(a: ByteArray, b: ByteArray, len: Int): Boolean {
        if (a.size < len || b.size < len) return false
        for (i in 0 until len) {
            if (a[i] != b[i]) return false
        }
        return true
    }
}
