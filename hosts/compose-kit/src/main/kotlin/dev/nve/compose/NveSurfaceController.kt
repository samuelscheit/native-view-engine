package dev.nve.compose

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.nve.host.NativeRuntime
import dev.nve.host.SceneReader
import dev.nve.host.FrameHost
import dev.nve.host.RuntimeSurface
import dev.nve.host.generated.NveRuntimeAbi
import java.util.concurrent.Executor

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

    override val lastError: String get() = surface.lastError()

    override fun close() = surface.close()

    companion object {
        init {
            System.loadLibrary("nve_android_shell")
        }
    }
}

/**
 * Compose-facing state of one runtime surface: the [NveSceneState] (one per-node observable slot
 * each, updated in O(changed)) plus input entry points that become `EventRing` events. Use on
 * the main thread.
 */
class NveSurfaceController(private val host: NveRuntimeHost) : AutoCloseable {
    val scene = NveSceneState()

    /**
     * `true` while a local app renders: a dispatched event's frame is applied before
     * [textChanged] returns, so a controlled field can show the app's verdict at once. `false`
     * while paired: the echo arrives later (or never), so fields keep the user's typing.
     */
    var dispatchIsSynchronous: Boolean = true
        private set

    /** Last start / runtime failure, `null` while healthy. */
    var error: String? by mutableStateOf(null)
        private set

    private var lastViewport = FloatArray(3)

    init {
        host.setFrameListener { reader, resources -> NveSceneCapture.capture(reader, resources, scene) }
    }

    /** Attaches the app or pairing session and renders the first frame; `false` (with [error]) on failure. */
    fun start(source: NveSource): Boolean {
        val st = host.start(source)
        if (st != NveRuntimeAbi.STATUS_OK) {
            error = "start failed ($st): ${host.lastError}"
            return false
        }
        error = null
        dispatchIsSynchronous = source is NveSource.App
        host.render()
        return true
    }

    fun setTextMeasurer(measurer: NativeRuntime.TextMeasurer?) {
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

    /** Native scroll moved by `(dx, dy)` to `(x, y)`: syncs runtime hit testing and tells the app. */
    fun scrolled(nodeId: Int, dxDp: Float, dyDp: Float, xDp: Float, yDp: Float, contentWDp: Float, contentHDp: Float) {
        host.scrollBy(nodeId, dxDp, dyDp)
        host.send { it.scroll(nodeId, now(), xDp, yDp, contentWDp, contentHDp) }
    }

    /** Wake-driven or explicit re-render (e.g. after resuming). */
    fun render(): Boolean = host.render()

    override fun close() = host.close()

    private fun now(): Long = System.nanoTime() / 1000L
}
