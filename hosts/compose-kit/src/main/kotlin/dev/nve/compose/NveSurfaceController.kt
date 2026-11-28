package dev.nve.compose

import android.os.Handler
import android.os.Looper
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import dev.nve.host.BinCodecException
import dev.nve.host.BinReader
import dev.nve.host.BinWriter
import dev.nve.host.EventRingReader
import dev.nve.host.FrameHost
import dev.nve.host.NativeRuntime
import dev.nve.host.RuntimeSurface
import dev.nve.host.SceneReader
import dev.nve.host.generated.NveAbi
import dev.nve.host.generated.NveRuntimeAbi
import java.util.concurrent.Executor

/** Thin raw callback bridge for `NVE_IFACE_HOST = 5` (passthrough to [NativeRuntime.HostCallbackBridge]). */
typealias NveHostBridge = NativeRuntime.HostCallbackBridge

/** One outbound event drained from an attached app via `NveComponentApi::drain_emitted_events`. */
class NveEmittedEvent(
    val tableBits: Long,
    val eventId: Int,
    val nodeId: Int,
    val timestampUs: Long,
    val flags: Int,
    val name: String,
    val payloadBytes: ByteArray,
) {
    /** `true` when this event is a named `nve.custom` event (`TABLE_CUSTOM` + `EVENT_FLAG_CUSTOM_NAMED`). */
    val isCustom: Boolean
        get() = tableBits == NveAbi.TABLE_CUSTOM.value() && (flags and NveAbi.EVENT_FLAG_CUSTOM_NAMED) != 0

    /** Opens a bounds-checked [BinReader] over [payloadBytes]. */
    fun payloadReader(): BinReader = BinReader(payloadBytes)
}

/** What a surface shows: a local app `.so` (hot-reloadable) or a paired `nve pair` server. */
sealed interface NveSource {
    /** [carryState]: a hot reload restoring the running app's state into the new module. */
    data class App(val soPath: String, val carryState: Boolean = false) : NveSource

    data class Paired(val uri: String, val token: String = "") : NveSource
}

/** The runtime behind a [NveSurfaceController]: [NativeRuntimeHost] in apps, fakes in JVM tests. */
interface NveRuntimeHost : AutoCloseable {
    /** Receives every new frame while it is acquired (resources readable only during the call). */
    fun setFrameListener(listener: (SceneReader, NveResourceLookup) -> Unit)

    /** Receives raw `NVEV` v2 batches drained after renders, event dispatches, prop updates, and hot patches. */
    fun setEmittedEventListener(listener: ((ByteArray) -> Unit)?)

    fun start(source: NveSource): Int

    fun setViewport(widthDp: Float, heightDp: Float, density: Float): Int

    fun setTextMeasurer(measurer: NativeRuntime.TextMeasurer?): Int

    /** Ticks and delivers a changed frame; `true` when one was delivered. */
    fun render(): Boolean

    /**
     * Writes events with [write] and dispatches them (rendering a changed frame); inside a frame
     * callback the batch is deferred until the frame is released.
     */
    fun send(write: FrameHost.EventWriter): Int

    fun scrollBy(nodeId: Int, dxDp: Float, dyDp: Float): Int

    /**
     * Forwards embedding-host props encoded as a single `BinWriter` `NveBinValue` map. When
     * [outChanged] is non-null (length >= 1), `outChanged[0]` receives whether the app changed.
     */
    fun setProps(binMap: ByteArray, outChanged: BooleanArray? = null): Int

    /** Manually drains outbound app events as an `NVEV` v2 byte array (`null` when empty/unsupported). */
    fun drainEmittedEvents(): ByteArray?

    /** Snapshots the attached app's state (`null` on error). */
    fun snapshotState(): ByteArray?

    /** Restores a state snapshot into the attached app. */
    fun restoreState(state: ByteArray): Int

    fun saveStateFile(path: String, schemaVersion: Int): Int

    fun restoreStateFile(path: String, schemaVersion: Int): Int

    fun clearStateFile(path: String): Int

    fun saveFrame0File(path: String, appBuildId: Long = 0L, envHash: Long = 0L): Int

    fun bootFrame0File(path: String, appBuildId: Long = 0L, envHash: Long = 0L): Int

    fun bootFrame0Bytes(envelope: ByteArray, appBuildId: Long = 0L, envHash: Long = 0L): Int

    /**
     * Applies a Tier-1 binary hot patch. When [outChanged] is non-null (length >= 1),
     * `outChanged[0]` receives whether the app changed.
     */
    fun applyHotPatch(patch: ByteArray, outChanged: BooleanArray? = null): Int

    fun measureSurface(
        minWidthDp: Float,
        minHeightDp: Float,
        maxWidthDp: Float,
        maxHeightDp: Float,
        flags: Int = NveRuntimeAbi.MEASURE_FLAG_DRY_RUN,
    ): RuntimeSurface.SurfaceMeasure?

    fun setIntrinsicSizeInvalidatedListener(listener: Runnable?): Int

    fun setHostCallbackBridge(bridge: NveHostBridge?): Int

    fun completeHostRequest(requestId: Long, hostStatus: Int, payload: ByteArray? = null): Int

    fun pushHostEvent(serviceId: Int, eventId: Int, payload: ByteArray? = null): Int

    fun publishHostChannel(channelId: Int, flags: Int = 0, payload: ByteArray? = null): Int

    /** `true` when dispatched events render synchronously (local app); `false` while paired. */
    val dispatchIsSynchronous: Boolean

    val lastError: String
}

/** [NveRuntimeHost] over `libnve_android_shell` + `libnve_runtime` ([RuntimeSurface]). */
class NativeRuntimeHost(surfaceId: Int, widthDp: Float, heightDp: Float, density: Float, ownerThread: Executor) :
    NveRuntimeHost {
    private val surface = RuntimeSurface(surfaceId, widthDp, heightDp, density, ownerThread)

    override fun setFrameListener(listener: (SceneReader, NveResourceLookup) -> Unit) {
        surface.setFrameConsumer { scene, sf ->
            listener(scene) { id ->
                sf.resource(id)?.let { NveResource(sf.resourceKind(), it) }
            }
        }
    }

    override fun setEmittedEventListener(listener: ((ByteArray) -> Unit)?) {
        surface.setEmittedEventConsumer(listener?.let { cb -> RuntimeSurface.EmittedEventConsumer { cb(it) } })
    }

    override fun start(source: NveSource): Int = when (source) {
        is NveSource.App -> surface.attachApp(source.soPath, source.carryState)
        is NveSource.Paired -> surface.pair(source.uri, source.token)
    }

    override fun setViewport(widthDp: Float, heightDp: Float, density: Float): Int =
        surface.setViewport(widthDp, heightDp, density)

    override fun setTextMeasurer(measurer: NativeRuntime.TextMeasurer?): Int = surface.setTextMeasurer(measurer)

    override fun render(): Boolean = surface.render()

    override fun send(write: FrameHost.EventWriter): Int = surface.send(write)

    override fun scrollBy(nodeId: Int, dxDp: Float, dyDp: Float): Int = surface.scrollBy(nodeId, dxDp, dyDp)

    override fun setProps(binMap: ByteArray, outChanged: BooleanArray?): Int = surface.setProps(binMap, outChanged)

    override fun drainEmittedEvents(): ByteArray? = surface.drainEmittedEvents()

    override fun snapshotState(): ByteArray? = surface.snapshotState()

    override fun restoreState(state: ByteArray): Int = surface.restoreState(state)

    override fun saveStateFile(path: String, schemaVersion: Int): Int = surface.saveStateFile(path, schemaVersion)

    override fun restoreStateFile(path: String, schemaVersion: Int): Int =
        surface.restoreStateFile(path, schemaVersion)

    override fun clearStateFile(path: String): Int = surface.clearStateFile(path)

    override fun saveFrame0File(path: String, appBuildId: Long, envHash: Long): Int =
        surface.saveFrame0File(path, appBuildId, envHash)

    override fun bootFrame0File(path: String, appBuildId: Long, envHash: Long): Int =
        surface.bootFrame0File(path, appBuildId, envHash)

    override fun bootFrame0Bytes(envelope: ByteArray, appBuildId: Long, envHash: Long): Int =
        surface.bootFrame0Bytes(envelope, appBuildId, envHash)

    override fun applyHotPatch(patch: ByteArray, outChanged: BooleanArray?): Int = surface.applyHotPatch(patch, outChanged)

    override fun measureSurface(
        minWidthDp: Float,
        minHeightDp: Float,
        maxWidthDp: Float,
        maxHeightDp: Float,
        flags: Int,
    ): RuntimeSurface.SurfaceMeasure? = surface.measureSurface(minWidthDp, minHeightDp, maxWidthDp, maxHeightDp, flags)

    override fun setIntrinsicSizeInvalidatedListener(listener: Runnable?): Int =
        surface.setIntrinsicSizeInvalidatedListener(listener)

    override fun setHostCallbackBridge(bridge: NveHostBridge?): Int = surface.setHostCallbackBridge(bridge)

    override fun completeHostRequest(requestId: Long, hostStatus: Int, payload: ByteArray?): Int =
        surface.completeHostRequest(requestId, hostStatus, payload)

    override fun pushHostEvent(serviceId: Int, eventId: Int, payload: ByteArray?): Int =
        surface.pushHostEvent(serviceId, eventId, payload)

    override fun publishHostChannel(channelId: Int, flags: Int, payload: ByteArray?): Int =
        surface.publishHostChannel(channelId, flags, payload)

    override val dispatchIsSynchronous: Boolean get() = surface.dispatchesSynchronously()

    override val lastError: String get() = surface.lastError()

    override fun close() = surface.close()

    companion object {
        init {
            System.loadLibrary("nve_android_shell")
        }
    }
}

/**
 * Remembers and manages a [NveSurfaceController] bound to [source], closing its native runtime
 * surface automatically when leaving composition.
 */
@Composable
fun rememberNveSurfaceController(
    source: NveSource,
    surfaceId: Int = 1,
    hostFactory: ((surfaceId: Int, widthDp: Float, heightDp: Float, density: Float, ownerThread: Executor) -> NveRuntimeHost)? = null,
): NveSurfaceController {
    val context = LocalContext.current
    val density = LocalDensity.current.density.let { if (it > 0f) it else 1f }
    val controller = remember(surfaceId, hostFactory) {
        val metrics = context.resources.displayMetrics
        val wDp = if (metrics.widthPixels > 0) metrics.widthPixels / density else 412f
        val hDp = if (metrics.heightPixels > 0) metrics.heightPixels / density else 915f
        val main = Handler(Looper.getMainLooper())
        val executor = Executor { main.post(it) }
        val host = hostFactory?.invoke(surfaceId, wDp, hDp, density, executor)
            ?: NativeRuntimeHost(surfaceId, wDp, hDp, density, executor)
        NveSurfaceController(host)
    }
    DisposableEffect(controller) {
        onDispose { controller.close() }
    }
    LaunchedEffect(controller, source) {
        controller.start(source)
    }
    return controller
}

/**
 * Compose-facing state of one runtime surface: the [NveSceneState] (one per-node observable slot
 * each, updated in O(changed)) plus input, Runtime ABI v6 props/events/state/Frame-0/hot-patch,
 * measurement, and thin host-bridge entry points. Use on the main thread (except cross-thread host
 * completion/push/channel helpers).
 */
class NveSurfaceController(private val host: NveRuntimeHost) : AutoCloseable {
    val scene = NveSceneState()

    /**
     * `true` while a local app renders: a dispatched event's frame is applied before
     * [textChanged] returns, so a controlled field can show the app's verdict at once. `false`
     * while paired: the echo arrives later (or never), so fields keep the user's typing.
     * Delegated to [NveRuntimeHost.dispatchIsSynchronous] (`RuntimeSurface.dispatchesSynchronously`).
     */
    val dispatchIsSynchronous: Boolean get() = host.dispatchIsSynchronous

    /** Last start / runtime failure, `null` while healthy. */
    var error: String? by mutableStateOf(null)
        private set

    private val emittedBatchCallback: (ByteArray) -> Unit = { batch -> dispatchEmittedBatch(batch) }
    private val eventRingReader = EventRingReader()
    private val changedOut = BooleanArray(1)
    private var dispatchDepth = 0
    private var closed = false

    /**
     * Listener receiving each outbound event drained from the attached app after renders and
     * dispatches. Wired to [NveRuntimeHost.setEmittedEventListener] only while non-null so manual
     * [drainEmittedEvents] receives queued events when [onEvent] is `null`.
     */
    var onEvent: ((NveEmittedEvent) -> Unit)? = null
        set(value) {
            val hadListener = field != null
            field = value
            val hasListener = value != null
            if (hadListener != hasListener && !closed) {
                host.setEmittedEventListener(if (hasListener) emittedBatchCallback else null)
            }
        }

    private var lastViewport = FloatArray(3)
    private val scratchWriter = BinWriter()

    init {
        host.setFrameListener { reader, resources -> NveSceneCapture.capture(reader, resources, scene) }
    }

    private fun dispatchEmittedBatch(batchBytes: ByteArray) {
        val listener = onEvent ?: return
        if (batchBytes.isEmpty()) return
        val reader = if (dispatchDepth == 0) eventRingReader else EventRingReader()
        dispatchDepth++
        try {
            try {
                reader.attach(batchBytes)
            } catch (_: BinCodecException) {
                return
            }
            for (i in 0 until reader.count()) {
                val tableBits = reader.tableBits(i)
                val flags = reader.flags(i)
                val custom = tableBits == NveAbi.TABLE_CUSTOM.value() && (flags and NveAbi.EVENT_FLAG_CUSTOM_NAMED) != 0
                val ev = NveEmittedEvent(
                    tableBits = tableBits,
                    eventId = reader.event(i),
                    nodeId = reader.node(i),
                    timestampUs = reader.timestamp(i),
                    flags = flags,
                    name = if (custom) reader.customName(i) else "",
                    payloadBytes = if (custom) reader.customData(i) else reader.bytes(i),
                )
                listener(ev)
            }
        } finally {
            dispatchDepth--
        }
    }

    /** Attaches the app or pairing session and renders the first frame; `false` (with [error]) on failure. */
    fun start(source: NveSource): Boolean {
        val st = host.start(source)
        if (st != NveRuntimeAbi.STATUS_OK) {
            error = "start failed ($st): ${host.lastError}"
            return false
        }
        error = null
        host.render()
        return true
    }

    fun setTextMeasurer(measurer: NativeRuntime.TextMeasurer?) {
        if (measurer == null && closed) return
        host.setTextMeasurer(measurer)
        host.render()
    }

    fun setViewport(widthDp: Float, heightDp: Float, density: Float) {
        if (!(widthDp > 0f && heightDp > 0f)) return
        if (lastViewport[0] == widthDp && lastViewport[1] == heightDp && lastViewport[2] == density) return
        lastViewport = floatArrayOf(widthDp, heightDp, density)
        host.setViewport(widthDp, heightDp, density)
        host.render()
    }

    /** Forwards embedding-host props encoded as a single `BinWriter` `NveBinValue` map and renders when changed. */
    fun setProps(binMap: ByteArray): Boolean {
        changedOut[0] = false
        val st = host.setProps(binMap, changedOut)
        if (st != NveRuntimeAbi.STATUS_OK) {
            error = "setProps failed ($st): ${host.lastError}"
            return false
        }
        error = null
        if (changedOut[0]) host.render()
        return true
    }

    /** Encodes a `BinWriter` `NveBinValue` map and forwards it via [setProps]. */
    fun setProps(encode: (BinWriter) -> Unit): Boolean {
        scratchWriter.clear()
        encode(scratchWriter)
        return setProps(scratchWriter.toByteArray())
    }

    fun press(nodeId: Int, xDp: Float, yDp: Float) {
        host.send { it.press(nodeId, now(), xDp, yDp) }
    }

    fun textChanged(nodeId: Int, text: String) {
        host.send { it.textChange(nodeId, now(), text) }
    }

    fun submit(nodeId: Int, text: String) {
        host.send { it.submit(nodeId, now(), text) }
    }

    fun focusChanged(nodeId: Int, focused: Boolean) {
        host.send { if (focused) it.focus(nodeId, now()) else it.blur(nodeId, now()) }
    }

    /** Sends a named `nve.custom` event (e.g. imperative exported commands) to the runtime. */
    fun dispatchCustom(name: String, payload: ByteArray? = null, nodeId: Int = 0): Int =
        host.send { it.custom(nodeId, now(), name, payload) }

    /** Encodes a `BinWriter` payload and sends a named `nve.custom` event to the runtime. */
    fun dispatchCustom(name: String, nodeId: Int = 0, encode: (BinWriter) -> Unit): Int {
        scratchWriter.clear()
        encode(scratchWriter)
        return dispatchCustom(name, scratchWriter.toByteArray(), nodeId)
    }

    /** Native scroll moved by `(dx, dy)` to `(x, y)`: syncs runtime hit testing and tells the app. */
    fun scrolled(nodeId: Int, dxDp: Float, dyDp: Float, xDp: Float, yDp: Float, contentWDp: Float, contentHDp: Float) {
        host.scrollBy(nodeId, dxDp, dyDp)
        host.send { it.scroll(nodeId, now(), xDp, yDp, contentWDp, contentHDp) }
    }

    /** Manually drains outbound app events as an `NVEV` v2 byte array (`null` when empty/unsupported). */
    fun drainEmittedEvents(): ByteArray? = host.drainEmittedEvents()

    /** Snapshots the attached app's state (`null` when unavailable/unsupported). */
    fun snapshotState(): ByteArray? = host.snapshotState()

    /** Restores a state snapshot and renders on success. */
    fun restoreState(state: ByteArray): Boolean {
        val st = host.restoreState(state)
        if (st != NveRuntimeAbi.STATUS_OK) {
            error = "restoreState failed ($st): ${host.lastError}"
            return false
        }
        error = null
        host.render()
        return true
    }

    fun saveStateFile(path: String, schemaVersion: Int): Boolean =
        host.saveStateFile(path, schemaVersion) == NveRuntimeAbi.STATUS_OK

    fun restoreStateFile(path: String, schemaVersion: Int): Boolean {
        val st = host.restoreStateFile(path, schemaVersion)
        if (st != NveRuntimeAbi.STATUS_OK) return false
        host.render()
        return true
    }

    fun clearStateFile(path: String): Boolean = host.clearStateFile(path) == NveRuntimeAbi.STATUS_OK

    fun saveFrame0File(path: String, appBuildId: Long = 0L, envHash: Long = 0L): Boolean =
        host.saveFrame0File(path, appBuildId, envHash) == NveRuntimeAbi.STATUS_OK

    fun bootFrame0File(path: String, appBuildId: Long = 0L, envHash: Long = 0L): Boolean {
        val st = host.bootFrame0File(path, appBuildId, envHash)
        if (st != NveRuntimeAbi.STATUS_OK) return false
        error = null
        host.render()
        return true
    }

    fun bootFrame0Bytes(envelope: ByteArray, appBuildId: Long = 0L, envHash: Long = 0L): Boolean {
        val st = host.bootFrame0Bytes(envelope, appBuildId, envHash)
        if (st != NveRuntimeAbi.STATUS_OK) return false
        error = null
        host.render()
        return true
    }

    /** Applies one Tier-1 binary hot patch and renders when changed. */
    fun applyHotPatch(patch: ByteArray): Boolean {
        changedOut[0] = false
        val st = host.applyHotPatch(patch, changedOut)
        if (st != NveRuntimeAbi.STATUS_OK) {
            error = "applyHotPatch failed ($st): ${host.lastError}"
            return false
        }
        error = null
        if (changedOut[0]) host.render()
        return true
    }

    /** Measures the surface under constraints (`MEASURE_FLAG_DRY_RUN` by default) without mutating committed rects. */
    fun measureSurface(
        minWidthDp: Float = 0f,
        minHeightDp: Float = 0f,
        maxWidthDp: Float = Float.POSITIVE_INFINITY,
        maxHeightDp: Float = Float.POSITIVE_INFINITY,
        flags: Int = NveRuntimeAbi.MEASURE_FLAG_DRY_RUN,
    ): RuntimeSurface.SurfaceMeasure? =
        host.measureSurface(minWidthDp, minHeightDp, maxWidthDp, maxHeightDp, flags)

    fun setIntrinsicSizeInvalidatedListener(listener: Runnable?): Int {
        if (listener == null && closed) return NveRuntimeAbi.STATUS_OK
        return host.setIntrinsicSizeInvalidatedListener(listener)
    }

    fun setHostBridge(bridge: NveHostBridge?): Int {
        if (bridge == null && closed) return NveRuntimeAbi.STATUS_OK
        return host.setHostCallbackBridge(bridge)
    }

    fun completeHostRequest(requestId: Long, hostStatus: Int, payload: ByteArray? = null): Int =
        host.completeHostRequest(requestId, hostStatus, payload)

    fun pushHostEvent(serviceId: Int, eventId: Int, payload: ByteArray? = null): Int =
        host.pushHostEvent(serviceId, eventId, payload)

    fun publishHostChannel(channelId: Int, flags: Int = 0, payload: ByteArray? = null): Int =
        host.publishHostChannel(channelId, flags, payload)

    /** Wake-driven or explicit re-render (e.g. after resuming). */
    fun render(): Boolean = host.render()

    override fun close() {
        if (closed) return
        onEvent = null
        closed = true
        host.close()
    }

    private fun now(): Long = System.nanoTime() / 1000L
}
