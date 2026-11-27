package dev.nve.compose

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.nve.android.generated.NveCoreAbi
import dev.nve.host.EventRingWriter
import dev.nve.host.NativeRuntime
import dev.nve.host.RuntimeSceneReader
import dev.nve.host.RuntimeSurface
import java.util.concurrent.Executor

/** What a surface shows: a local app `.so` (hot-reloadable) or a paired `nve pair` server. */
sealed interface NveSource {
    data class App(val soPath: String) : NveSource

    data class Paired(val uri: String, val token: String = "") : NveSource
}

/** The runtime behind a [NveSurfaceController]: [NativeRuntimeHost] in apps, fakes in JVM tests. */
interface NveRuntimeHost : AutoCloseable {
    /** Receives every new frame while it is acquired (resources readable only during the call). */
    fun setFrameListener(listener: (RuntimeSceneReader, NveResourceLookup) -> Unit)

    fun start(source: NveSource): Int

    fun setViewport(widthDp: Float, heightDp: Float, density: Float): Int

    fun setTextMeasurer(measurer: NativeRuntime.TextMeasurer?): Int

    /** Ticks and delivers a changed frame; `true` when one was delivered. */
    fun render(): Boolean

    /** Writes events with [write], dispatches them, and renders a changed frame. */
    fun dispatch(write: (EventRingWriter) -> Unit): Int

    fun scrollBy(nodeId: Int, dxDp: Float, dyDp: Float): Int

    val lastError: String
}

/** [NveRuntimeHost] over `libnve_android_shell` + `libnve_runtime` ([RuntimeSurface]). */
class NativeRuntimeHost(surfaceId: Int, widthDp: Float, heightDp: Float, density: Float, ownerThread: Executor) :
    NveRuntimeHost {
    private val surface = RuntimeSurface(surfaceId, widthDp, heightDp, density, ownerThread)

    override fun setFrameListener(listener: (RuntimeSceneReader, NveResourceLookup) -> Unit) {
        surface.setFrameConsumer { scene, sf ->
            listener(scene) { id ->
                sf.resource(id)?.let { NveResource(sf.resourceKind(), sf.resourceFlags(), it) }
            }
        }
    }

    override fun start(source: NveSource): Int = when (source) {
        is NveSource.App -> surface.attachApp(source.soPath)
        is NveSource.Paired -> surface.pair(source.uri, source.token)
    }

    override fun setViewport(widthDp: Float, heightDp: Float, density: Float): Int =
        surface.setViewport(widthDp, heightDp, density)

    override fun setTextMeasurer(measurer: NativeRuntime.TextMeasurer?): Int = surface.setTextMeasurer(measurer)

    override fun render(): Boolean = surface.render()

    override fun dispatch(write: (EventRingWriter) -> Unit): Int {
        write(surface.events())
        return surface.dispatch()
    }

    override fun scrollBy(nodeId: Int, dxDp: Float, dyDp: Float): Int = surface.scrollBy(nodeId, dxDp, dyDp, null)

    override val lastError: String get() = surface.lastError()

    override fun close() = surface.close()

    companion object {
        init {
            System.loadLibrary("nve_android_shell")
        }
    }
}

/**
 * Compose-facing state of one runtime surface: the latest [NveSceneSnapshot] (observable) plus
 * input entry points that become `EventRing` events. Use on the main thread.
 */
class NveSurfaceController(private val host: NveRuntimeHost) : AutoCloseable {
    var scene: NveSceneSnapshot by mutableStateOf(NveSceneSnapshot.EMPTY)
        private set

    /** Last start / runtime failure, `null` while healthy. */
    var error: String? by mutableStateOf(null)
        private set

    private var lastViewport = FloatArray(3)

    init {
        host.setFrameListener { reader, resources -> scene = NveSceneCapture.capture(reader, resources, scene) }
    }

    /** Attaches the app or pairing session and renders the first frame; `false` (with [error]) on failure. */
    fun start(source: NveSource): Boolean {
        val st = host.start(source)
        if (st != NativeRuntime.STATUS_OK) {
            error = "start failed ($st): ${host.lastError}"
            return false
        }
        error = null
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
        host.dispatch { it.point(NveCoreAbi.EVENT_PRESS, nodeId, now(), xDp, yDp) }
    }

    fun textChanged(nodeId: Int, text: String) {
        host.dispatch { it.text(NveCoreAbi.EVENT_TEXT_CHANGE, nodeId, now(), text) }
    }

    fun submit(nodeId: Int, text: String) {
        host.dispatch { it.text(NveCoreAbi.EVENT_SUBMIT, nodeId, now(), text) }
    }

    fun focusChanged(nodeId: Int, focused: Boolean) {
        host.dispatch { it.empty(if (focused) NveCoreAbi.EVENT_FOCUS else NveCoreAbi.EVENT_BLUR, nodeId, now()) }
    }

    /** Native scroll moved by `(dx, dy)` to `(x, y)`: syncs runtime hit testing and tells the app. */
    fun scrolled(nodeId: Int, dxDp: Float, dyDp: Float, xDp: Float, yDp: Float, contentWDp: Float, contentHDp: Float) {
        host.scrollBy(nodeId, dxDp, dyDp)
        host.dispatch { it.scroll(NveCoreAbi.EVENT_SCROLL, nodeId, now(), xDp, yDp, contentWDp, contentHDp) }
    }

    /** Wake-driven or explicit re-render (e.g. after resuming). */
    fun render(): Boolean = host.render()

    override fun close() = host.close()

    private fun now(): Long = System.nanoTime() / 1000L
}
