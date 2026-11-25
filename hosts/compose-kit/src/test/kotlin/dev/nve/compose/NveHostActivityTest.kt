package dev.nve.compose

import android.content.Intent
import android.content.res.Configuration
import android.os.SystemClock
import android.view.MotionEvent
import android.view.View
import android.widget.FrameLayout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

/**
 * Robolectric lifecycle & Android View tests for [NveHostActivity] and [NveSurfaceView].
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class NveHostActivityTest {

    companion object {
        lateinit var testBridge: FakeNveBridgeBackend
        val capturedEvents = mutableListOf<NveEvent>()
    }

    class TestNveHostActivity : NveHostActivity() {
        override fun provideBridgeBackend(): NveBridgeBackend = testBridge

        override fun initialProps(): Map<String, Any?> = mapOf("host" to "robolectric", "version" to 2)

        override fun onSurfaceEvent(event: NveEvent) {
            capturedEvents.add(event)
        }

        override fun installHostContent(controller: NveSurfaceController) {
            // Install NveSurfaceView directly so Robolectric tests remain fast and hermetic
            // without spinning up a Compose recomposer clock.
            setContentView(NveSurfaceView(this, controller))
        }
    }

    @Before
    fun setUp() {
        testBridge = FakeNveBridgeBackend()
        capturedEvents.clear()
    }

    @Test
    fun activityLifecycleWiresSurfaceIdAttachDetachReattachResizeReloadAndDispose() {
        val intent = Intent(RuntimeEnvironment.getApplication(), TestNveHostActivity::class.java).apply {
            putExtra(NveHostConfig.EXTRA_SURFACE_ID, 314)
            putExtra(NveHostConfig.EXTRA_COMPONENT_NAME, "NveCounterDemo")
            putExtra(NveHostConfig.EXTRA_VTABLE_SYMBOL, "nve_get_interface")
            putExtra(NveHostConfig.EXTRA_LIBRARY_PATH, "/data/local/tmp/libcounter_v1.so")
        }

        testBridge.enqueueOutboundEvents(
            NveEventRingCodec.custom(314, 1, "onReady", NveBinValue.Bool(true).encode(), 55L),
        )

        val controller = Robolectric.buildActivity(TestNveHostActivity::class.java, intent)
        val activity = controller.create().start().resume().get()

        // Surface attached with exact Intent surfaceId & initial library loaded
        assertEquals(314, activity.surfaceId)
        assertTrue(activity.isSurfaceAttached)
        assertTrue(activity.surfaceController.isMounted)
        assertEquals("/data/local/tmp/libcounter_v1.so", activity.lastLoadedLibraryPath)
        assertEquals(1, activity.surfaceState.reloadGeneration)
        assertEquals(1, testBridge.createCalls.size)
        assertEquals(314, testBridge.createCalls[0].surfaceId)
        assertEquals("NveCounterDemo", testBridge.createCalls[0].componentName)

        // Initial binary props were pushed with binary FFI
        assertTrue(testBridge.propsBinCalls.isNotEmpty())
        val propsView = NveBinValueView.parse(testBridge.propsBinCalls.first())
        assertEquals("robolectric", propsView.mapValue("host")!!.stringValue)
        assertEquals(2L, propsView.mapValue("version")!!.int64Value)

        // Outbound event drained on resume
        assertEquals(1, capturedEvents.size)
        assertEquals("onReady", capturedEvents.single().name)

        // Explicit pixel viewport resize converts to dp and submits LAYOUT event
        assertTrue(activity.onViewportChanged(widthPx = 1080, heightPx = 2400, density = 3.0f))
        assertEquals(NveViewport(360f, 800f, 3.0f), activity.surfaceState.viewport)

        // Configuration change resizes surface
        val landscape = Configuration(activity.resources.configuration).apply {
            screenWidthDp = 844
            screenHeightDp = 390
            densityDpi = 480 // 3.0x
        }
        activity.onConfigurationChanged(landscape)
        assertEquals(NveViewport(844f, 390f, 3.0f), activity.surfaceState.viewport)

        // Tier-2 hot-swap reload preserves surfaceId & updates library path
        assertTrue(activity.reloadModule("/data/local/tmp/libcounter_v2.so"))
        assertEquals("/data/local/tmp/libcounter_v2.so", activity.lastLoadedLibraryPath)
        assertEquals(2, activity.surfaceState.reloadGeneration)
        assertEquals(314, activity.surfaceId)

        // Failed reload preserves previous library path and generation
        testBridge.reloadShouldSucceed = false
        assertFalse(activity.reloadModule("/data/local/tmp/libcounter_corrupt.so"))
        assertEquals("/data/local/tmp/libcounter_v2.so", activity.lastLoadedLibraryPath)
        assertEquals(2, activity.surfaceState.reloadGeneration)
        testBridge.reloadShouldSucceed = true

        // Stop detaches surface; start re-attaches under the exact same surfaceId
        controller.pause().stop()
        assertFalse(activity.isSurfaceAttached)
        assertEquals(NveSurfaceLifecycleState.DETACHED, activity.surfaceState.lifecycleState)
        assertEquals(0, testBridge.activeHandleCount())

        // Reload while detached returns false safely
        assertFalse(activity.reloadModule("/data/local/tmp/libcounter_v3.so"))

        controller.start().resume()
        assertTrue(activity.isSurfaceAttached)
        assertEquals(314, activity.surfaceId)
        assertEquals(2, testBridge.createCalls.size)
        assertEquals(314, testBridge.createCalls[1].surfaceId)
        assertEquals(1, testBridge.activeHandleCount())

        // Destroy permanently disposes controller and cleans up handle
        controller.pause().stop().destroy()
        assertFalse(activity.isSurfaceAttached)
        assertTrue(activity.surfaceController.isDisposed)
        assertEquals(0, testBridge.activeHandleCount())
        assertFalse(activity.attachSurface())
    }

    @Test
    fun surfaceViewNegotiatesIntrinsicMeasureAndTouchArbitration() {
        val context = RuntimeEnvironment.getApplication()
        val bridge = FakeNveBridgeBackend().apply {
            measuredIntrinsic = floatArrayOf(180f, 96f)
        }
        val surfaceController = NveSurfaceController(
            componentName = "NveWidget",
            surfaceId = 19,
            bridge = bridge,
        )
        assertTrue(surfaceController.attach())

        class TrackingParent : FrameLayout(context) {
            var lastDisallowIntercept: Boolean? = null
            override fun requestDisallowInterceptTouchEvent(disallowIntercept: Boolean) {
                lastDisallowIntercept = disallowIntercept
                super.requestDisallowInterceptTouchEvent(disallowIntercept)
            }
        }

        val parent = TrackingParent()
        val view = NveSurfaceView(context, surfaceController)
        parent.addView(view)

        val density = context.resources.displayMetrics.density.coerceAtLeast(1f)
        view.measure(
            View.MeasureSpec.makeMeasureSpec(500, View.MeasureSpec.AT_MOST),
            View.MeasureSpec.makeMeasureSpec(500, View.MeasureSpec.AT_MOST),
        )
        assertEquals((180f * density).toInt(), view.measuredWidth)
        assertEquals((96f * density).toInt(), view.measuredHeight)

        // Layout triggers onSizeChanged -> resize + render
        view.layout(0, 0, (200f * density).toInt(), (100f * density).toInt())
        assertEquals(200f, surfaceController.state.viewport.width, 0.5f)
        assertEquals(100f, surfaceController.state.viewport.height, 0.5f)
        assertTrue(bridge.renderedFrames.isNotEmpty())

        // Touch arbitration: CLAIM disallows parent intercept & dispatches binary PRESS on ACTION_UP
        bridge.pointerDisposition = NveGestureDisposition.CLAIM
        val now = SystemClock.uptimeMillis()
        val down = MotionEvent.obtain(now, now, MotionEvent.ACTION_DOWN, 40f, 20f, 0)
        assertTrue(view.onTouchEvent(down))
        assertEquals(true, parent.lastDisallowIntercept)
        down.recycle()

        val up = MotionEvent.obtain(now, now + 15, MotionEvent.ACTION_UP, 40f, 20f, 0)
        assertTrue(view.onTouchEvent(up))
        up.recycle()

        val submittedPress = bridge.submittedEventRings
            .flatMap { NveEventRingCodec.decode(it) }
            .last { it.kind == NveEventKind.PRESS }
        assertEquals(19, submittedPress.surfaceId)
        assertEquals(NveEventPayload.Point(40f / density, 20f / density), submittedPress.payload)

        // DEFER_TO_PARENT releases parent intercept and returns false
        bridge.pointerDisposition = NveGestureDisposition.DEFER_TO_PARENT
        val move = MotionEvent.obtain(now, now + 20, MotionEvent.ACTION_MOVE, 45f, 30f, 0)
        assertFalse(view.onTouchEvent(move))
        assertEquals(false, parent.lastDisallowIntercept)
        move.recycle()

        // PASS_THROUGH leaves parent intercept unchanged and returns false
        parent.lastDisallowIntercept = null
        bridge.pointerDisposition = NveGestureDisposition.PASS_THROUGH
        val cancel = MotionEvent.obtain(now, now + 25, MotionEvent.ACTION_CANCEL, 0f, 0f, 0)
        assertFalse(view.onTouchEvent(cancel))
        assertNull(parent.lastDisallowIntercept)
        cancel.recycle()

        surfaceController.dispose()
    }
}
