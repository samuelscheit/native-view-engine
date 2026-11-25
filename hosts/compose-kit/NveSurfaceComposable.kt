package dev.nve.compose

import android.content.Context
import android.view.MotionEvent
import android.view.View
import android.widget.FrameLayout
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.viewinterop.AndroidView
import org.json.JSONArray
import org.json.JSONObject
import java.nio.ByteBuffer
import java.nio.ByteOrder
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
 * Native event discriminator matching `NativeEventKind` in `crates/nve-abi/src/events.rs`.
 */
enum class NveEventKind(val code: Int) {
    PRESS(1),
    LONG_PRESS(2),
    SCROLL(3),
    TEXT_CHANGE(4),
    FOCUS(5),
    BLUR(6),
    LAYOUT(7),
    CUSTOM(8);

    companion object {
        fun fromCode(code: Int): NveEventKind =
            entries.firstOrNull { it.code == code } ?: CUSTOM
    }
}

/**
 * Gesture arbitration outcome mirroring `GestureDisposition` in `crates/nve-interop`.
 */
enum class NveGestureDisposition(val code: Int) {
    CLAIM(1),
    PASS_THROUGH(2),
    DEFER_TO_PARENT(3);

    companion object {
        fun fromCode(code: Int): NveGestureDisposition =
            entries.firstOrNull { it.code == code } ?: PASS_THROUGH
    }
}

/**
 * Decoded outbound event emitted by an embedded NVE Rust component.
 */
data class NveEvent(
    val surfaceId: Int,
    val nodeId: Int,
    val timestampUs: Long,
    val kind: NveEventKind,
    val name: String? = null,
    val payloadBytes: ByteArray = ByteArray(0),
    val payloadJson: String? = null,
) {
    override fun equals(other: Any?): Boolean {
        if (this === other) return true
        if (other !is NveEvent) return false
        return surfaceId == other.surfaceId &&
            nodeId == other.nodeId &&
            timestampUs == other.timestampUs &&
            kind == other.kind &&
            name == other.name &&
            payloadBytes.contentEquals(other.payloadBytes) &&
            payloadJson == other.payloadJson
    }

    override fun hashCode(): Int {
        var result = surfaceId
        result = 31 * result + nodeId
        result = 31 * result + timestampUs.hashCode()
        result = 31 * result + kind.hashCode()
        result = 31 * result + (name?.hashCode() ?: 0)
        result = 31 * result + payloadBytes.contentHashCode()
        result = 31 * result + (payloadJson?.hashCode() ?: 0)
        return result
    }
}

/**
 * Binary `EventRing` encoder/decoder matching `crates/nve-abi/src/events.rs`.
 */
object NveEventRingCodec {
    fun decode(bytes: ByteArray): List<NveEvent> {
        if (bytes.size < 4) return emptyList()
        val buf = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
        val count = buf.int
        val out = ArrayList<NveEvent>(count.coerceAtMost(4096))

        repeat(count) {
            if (buf.remaining() < 18) return out
            val surfaceId = buf.int
            val nodeId = buf.int
            val timestampUs = buf.long
            val kind = NveEventKind.fromCode(buf.get().toInt() and 0xFF)
            val payloadTag = buf.get().toInt() and 0xFF

            var name: String? = null
            var payloadBytes = ByteArray(0)
            var payloadJson: String? = null

            when (payloadTag) {
                0 -> Unit
                1 -> {
                    val x = buf.float
                    val y = buf.float
                    payloadJson = JSONObject().put("x", x).put("y", y).toString()
                }
                2 -> {
                    val x = buf.float
                    val y = buf.float
                    val cw = buf.float
                    val ch = buf.float
                    payloadJson = JSONObject()
                        .put("x", x)
                        .put("y", y)
                        .put("contentWidth", cw)
                        .put("contentHeight", ch)
                        .toString()
                }
                3 -> {
                    val len = buf.int
                    payloadBytes = ByteArray(len)
                    buf.get(payloadBytes)
                    payloadJson = JSONObject().put("text", String(payloadBytes, Charsets.UTF_8)).toString()
                }
                4 -> {
                    val x = buf.float
                    val y = buf.float
                    val w = buf.float
                    val h = buf.float
                    payloadJson = JSONObject()
                        .put("x", x)
                        .put("y", y)
                        .put("width", w)
                        .put("height", h)
                        .toString()
                }
                5 -> {
                    val nameLen = buf.short.toInt() and 0xFFFF
                    val nameBuf = ByteArray(nameLen)
                    buf.get(nameBuf)
                    name = String(nameBuf, Charsets.UTF_8)
                    val dataLen = buf.int
                    payloadBytes = ByteArray(dataLen)
                    buf.get(payloadBytes)
                    val text = String(payloadBytes, Charsets.UTF_8)
                    if (text.trimStart().startsWith("{")) {
                        payloadJson = text
                    }
                }
            }

            out += NveEvent(
                surfaceId = surfaceId,
                nodeId = nodeId,
                timestampUs = timestampUs,
                kind = kind,
                name = name,
                payloadBytes = payloadBytes,
                payloadJson = payloadJson,
            )
        }
        return out
    }
}

/**
 * JNI bridge connecting Jetpack Compose to NVE's modular `nve_get_interface` C ABI and `nve-interop`.
 *
 * Gracefully degrades to a no-op stub when running in Android Studio previews or JVM unit tests
 * where `libnve_android_shell.so` is not present.
 */
object NveNativeBridge {
    @Volatile
    var isNativeLoaded: Boolean = false
        private set

    init {
        isNativeLoaded = try {
            System.loadLibrary("nve_android_shell")
            true
        } catch (_: UnsatisfiedLinkError) {
            false
        }
    }

    external fun nativeCreateSurface(
        surfaceId: Int,
        componentName: String,
        vtableSymbol: String,
        backendCode: Int,
    ): Long

    external fun nativeDestroySurface(handlePtr: Long)

    external fun nativeSetPropsJson(handlePtr: Long, propsJson: String): Boolean

    external fun nativeRenderFrame(handlePtr: Long, frameId: Long): ByteArray?

    external fun nativeDispatchCommand(handlePtr: Long, commandName: String, argsJson: String): Boolean

    external fun nativeDrainEmittedEvents(handlePtr: Long): ByteArray?

    /**
     * Intrinsic measurement hook returning packed `(width << 32) | height` in float bits
     * for `View.onMeasure` / Compose `MeasurePolicy` integration.
     */
    external fun nativeMeasureSurface(
        handlePtr: Long,
        minWidth: Float,
        minHeight: Float,
        maxWidth: Float,
        maxHeight: Float,
    ): FloatArray?

    /**
     * Cross-framework pointer arbitration hook returning a [NveGestureDisposition] code.
     */
    external fun nativeHitTestPointer(
        handlePtr: Long,
        pointerId: Int,
        phase: Int,
        x: Float,
        y: Float,
    ): Int
}

/**
 * Shared-state & pub/sub client mirroring Rust's `NveChannelBus` for Jetpack Compose hosts.
 */
class NveChannelClient {
    data class Message(
        val topic: String,
        val payload: ByteArray,
        val seq: Long,
    )

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

    fun publishJson(topic: String, data: Map<String, Any?>, retain: Boolean = false) {
        val json = mapToJsonObject(data).toString()
        publish(topic, json.toByteArray(Charsets.UTF_8), retain = retain)
    }
}

/**
 * Imperative controller for an embedded [NveSurface] composable.
 */
class NveSurfaceController(
    val componentName: String,
    val vtableSymbol: String = "nve_get_interface",
    val backend: NveRenderBackend = NveRenderBackend.FABRIC,
    val surfaceId: Int = nextSurfaceId.getAndIncrement(),
) {
    companion object {
        private val nextSurfaceId = AtomicInteger(1)
    }

    private var nativeHandle: Long = 0L
    private var frameCounter: Long = 0L
    private var lastPropsJson: String? = null
    internal var onEventCallback: ((NveEvent) -> Unit)? = null

    val isMounted: Boolean
        get() = nativeHandle != 0L

    internal fun mountIfNeeded() {
        if (nativeHandle != 0L || !NveNativeBridge.isNativeLoaded) return
        nativeHandle = NveNativeBridge.nativeCreateSurface(
            surfaceId = surfaceId,
            componentName = componentName,
            vtableSymbol = vtableSymbol,
            backendCode = backend.ordinal,
        )
    }

    fun updateProps(props: Map<String, Any?>) {
        updatePropsJson(mapToJsonObject(props).toString())
    }

    fun updatePropsJson(propsJson: String) {
        if (lastPropsJson == propsJson) return
        lastPropsJson = propsJson
        if (nativeHandle == 0L || !NveNativeBridge.isNativeLoaded) return
        if (NveNativeBridge.nativeSetPropsJson(nativeHandle, propsJson)) {
            requestRender()
        }
        drainAndForwardEvents()
    }

    fun dispatchCommand(name: String, args: Map<String, Any?>) {
        dispatchCommand(name, mapToJsonObject(args).toString())
    }

    fun dispatchCommand(name: String, argsJson: String = "{}") {
        if (nativeHandle == 0L || !NveNativeBridge.isNativeLoaded) return
        if (NveNativeBridge.nativeDispatchCommand(nativeHandle, name, argsJson)) {
            requestRender()
        }
        drainAndForwardEvents()
    }

    fun requestRender() {
        if (nativeHandle == 0L || !NveNativeBridge.isNativeLoaded) return
        frameCounter += 1
        NveNativeBridge.nativeRenderFrame(nativeHandle, frameCounter)
    }

    fun drainEvents(): List<NveEvent> {
        if (nativeHandle == 0L || !NveNativeBridge.isNativeLoaded) return emptyList()
        val bytes = NveNativeBridge.nativeDrainEmittedEvents(nativeHandle) ?: return emptyList()
        return NveEventRingCodec.decode(bytes)
    }

    internal fun drainAndForwardEvents() {
        val cb = onEventCallback ?: return
        for (event in drainEvents()) {
            cb(event)
        }
    }

    internal fun measureIntrinsic(
        minW: Float,
        minH: Float,
        maxW: Float,
        maxH: Float,
    ): Pair<Float, Float>? {
        if (nativeHandle == 0L || !NveNativeBridge.isNativeLoaded) return null
        val result = NveNativeBridge.nativeMeasureSurface(nativeHandle, minW, minH, maxW, maxH)
        if (result == null || result.size < 2) return null
        return result[0] to result[1]
    }

    internal fun arbitrateTouch(event: MotionEvent): NveGestureDisposition {
        if (nativeHandle == 0L || !NveNativeBridge.isNativeLoaded) {
            return NveGestureDisposition.CLAIM
        }
        val phase = when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> 1
            MotionEvent.ACTION_MOVE -> 2
            MotionEvent.ACTION_UP -> 3
            else -> 4
        }
        val pointerId = event.getPointerId(event.actionIndex)
        val code = NveNativeBridge.nativeHitTestPointer(
            nativeHandle,
            pointerId,
            phase,
            event.x,
            event.y,
        )
        return NveGestureDisposition.fromCode(code)
    }

    fun dispose() {
        if (nativeHandle != 0L && NveNativeBridge.isNativeLoaded) {
            NveNativeBridge.nativeDestroySurface(nativeHandle)
        }
        nativeHandle = 0L
        onEventCallback = null
    }
}

/**
 * Custom Android host container performing native `onMeasure` intrinsic layout negotiation
 * and `requestDisallowInterceptTouchEvent` nested-scroll arbitration with parent Compose containers.
 */
class NveSurfaceView(
    context: Context,
    private val controller: NveSurfaceController,
) : FrameLayout(context) {

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        val density = resources.displayMetrics.density.coerceAtLeast(1f)
        val widthMode = View.MeasureSpec.getMode(widthMeasureSpec)
        val heightMode = View.MeasureSpec.getMode(heightMeasureSpec)
        val widthPx = View.MeasureSpec.getSize(widthMeasureSpec)
        val heightPx = View.MeasureSpec.getSize(heightMeasureSpec)

        val minW = if (widthMode == View.MeasureSpec.EXACTLY) widthPx / density else 0f
        val maxW = if (widthMode == View.MeasureSpec.UNSPECIFIED) Float.POSITIVE_INFINITY else widthPx / density
        val minH = if (heightMode == View.MeasureSpec.EXACTLY) heightPx / density else 0f
        val maxH = if (heightMode == View.MeasureSpec.UNSPECIFIED) Float.POSITIVE_INFINITY else heightPx / density

        val measured = controller.measureIntrinsic(minW, minH, maxW, maxH)
        if (measured != null) {
            val resolvedW = View.resolveSize((measured.first * density).toInt(), widthMeasureSpec)
            val resolvedH = View.resolveSize((measured.second * density).toInt(), heightMeasureSpec)
            setMeasuredDimension(resolvedW, resolvedH)
        } else {
            super.onMeasure(widthMeasureSpec, heightMeasureSpec)
        }
    }

    override fun onTouchEvent(event: MotionEvent): Boolean {
        val disposition = controller.arbitrateTouch(event)
        when (disposition) {
            NveGestureDisposition.CLAIM -> {
                parent?.requestDisallowInterceptTouchEvent(true)
                controller.drainAndForwardEvents()
                return true
            }
            NveGestureDisposition.DEFER_TO_PARENT -> {
                parent?.requestDisallowInterceptTouchEvent(false)
                return false
            }
            NveGestureDisposition.PASS_THROUGH -> {
                return false
            }
        }
    }
}

/**
 * Creates and remembers an [NveSurfaceController] tied to the composition lifecycle.
 */
@Composable
fun rememberNveSurfaceController(
    componentName: String,
    vtableSymbol: String = "nve_get_interface",
    backend: NveRenderBackend = NveRenderBackend.FABRIC,
): NveSurfaceController {
    return remember(componentName, vtableSymbol, backend) {
        NveSurfaceController(
            componentName = componentName,
            vtableSymbol = vtableSymbol,
            backend = backend,
        )
    }
}

/**
 * Embeds a Native View Engine (NVE) Rust component natively inside Jetpack Compose.
 *
 * Bridges Compose state updates (`props`) into `NveComponentApi.set_props`, participates in
 * Compose sizing via `Modifier` + intrinsic `onMeasure`, arbitrates nested scroll gestures with
 * parent Compose scrollables, and delivers outbound NVE events via [onEvent].
 */
@Composable
fun NveSurface(
    componentName: String,
    modifier: Modifier = Modifier,
    vtableSymbol: String = "nve_get_interface",
    backend: NveRenderBackend = NveRenderBackend.FABRIC,
    props: Map<String, Any?> = emptyMap(),
    controller: NveSurfaceController? = null,
    onEvent: ((NveEvent) -> Unit)? = null,
) {
    val ownedController = controller == null
    val activeController = controller ?: rememberNveSurfaceController(
        componentName = componentName,
        vtableSymbol = vtableSymbol,
        backend = backend,
    )

    activeController.onEventCallback = onEvent

    DisposableEffect(activeController) {
        activeController.mountIfNeeded()
        activeController.updateProps(props)
        onDispose {
            if (ownedController) {
                activeController.dispose()
            } else {
                activeController.onEventCallback = null
            }
        }
    }

    LaunchedEffect(activeController, props) {
        activeController.updateProps(props)
    }

    AndroidView(
        modifier = modifier,
        factory = { context ->
            activeController.mountIfNeeded()
            NveSurfaceView(context, activeController)
        },
        update = { view ->
            activeController.onEventCallback = onEvent
            activeController.updateProps(props)
            view.requestLayout()
        },
    )
}

private fun mapToJsonObject(map: Map<String, Any?>): JSONObject {
    val obj = JSONObject()
    for ((key, value) in map) {
        obj.put(key, toJsonValue(value))
    }
    return obj
}

private fun toJsonValue(value: Any?): Any = when (value) {
    null -> JSONObject.NULL
    is Map<*, *> -> {
        val nested = JSONObject()
        for ((k, v) in value) {
            if (k != null) nested.put(k.toString(), toJsonValue(v))
        }
        nested
    }
    is Iterable<*> -> {
        val arr = JSONArray()
        for (item in value) arr.put(toJsonValue(item))
        arr
    }
    is Array<*> -> {
        val arr = JSONArray()
        for (item in value) arr.put(toJsonValue(item))
        arr
    }
    else -> value
}
