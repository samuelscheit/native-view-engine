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

/**
 * Custom Android host container performing native `onMeasure` intrinsic layout negotiation,
 * viewport resize forwarding, and `requestDisallowInterceptTouchEvent` nested-scroll arbitration
 * with parent Compose containers.
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

    override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) {
        super.onSizeChanged(w, h, oldw, oldh)
        if (w > 0 && h > 0) {
            val density = resources.displayMetrics.density.coerceAtLeast(1f)
            controller.resize(
                width = w / density,
                height = h / density,
                scaleFactor = density,
            )
            controller.requestRender()
        }
    }

    override fun onTouchEvent(event: MotionEvent): Boolean {
        val phase = when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> 1
            MotionEvent.ACTION_MOVE -> 2
            MotionEvent.ACTION_UP -> 3
            else -> 4
        }
        val pointerId = event.getPointerId(event.actionIndex)
        val disposition = controller.arbitrateTouch(
            pointerId = pointerId,
            phase = phase,
            x = event.x,
            y = event.y,
        )
        return when (disposition) {
            NveGestureDisposition.CLAIM -> {
                parent?.requestDisallowInterceptTouchEvent(true)
                if (event.actionMasked == MotionEvent.ACTION_UP) {
                    val density = resources.displayMetrics.density.coerceAtLeast(1f)
                    controller.dispatchPress(
                        nodeId = 0,
                        x = event.x / density,
                        y = event.y / density,
                        timestampUs = event.eventTime * 1_000L,
                    )
                } else {
                    controller.drainAndForwardEvents()
                }
                true
            }
            NveGestureDisposition.DEFER_TO_PARENT -> {
                parent?.requestDisallowInterceptTouchEvent(false)
                false
            }
            NveGestureDisposition.PASS_THROUGH -> false
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
    bridge: NveBridgeBackend = NveNativeBridge,
): NveSurfaceController {
    return remember(componentName, vtableSymbol, backend, bridge) {
        NveSurfaceController(
            componentName = componentName,
            vtableSymbol = vtableSymbol,
            backend = backend,
            bridge = bridge,
        )
    }
}

/**
 * Embeds a Native View Engine (NVE) Rust component natively inside Jetpack Compose.
 *
 * Bridges Compose state updates (`props`) via compact binary [NveBinValue], participates in
 * Compose sizing via `Modifier` + intrinsic `onMeasure`, arbitrates nested scroll gestures with
 * parent Compose scrollables, and delivers outbound binary `EventRing` events via [onEvent].
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
