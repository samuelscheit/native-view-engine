package dev.nve.compose

import androidx.compose.foundation.layout.size
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import com.facebook.react.bridge.SimpleReadableArray
import com.facebook.react.uimanager.ThemedReactContext
import dev.nve.components.DataGrid
import dev.nve.components.DataGridController
import dev.nve.components.DataGridDensity
import dev.nve.components.DataGridHostView
import dev.nve.components.DataGridRowSelectEvent
import dev.nve.components.DataGridViewManager
import dev.nve.components.EmptyCanvas
import dev.nve.components.EmptyCanvasController
import dev.nve.components.EmptyCanvasHostView
import dev.nve.components.EmptyCanvasViewManager
import dev.nve.components.NveCounterCard
import dev.nve.components.NveCounterCardController
import dev.nve.components.NveCounterCardCountChangeEvent
import dev.nve.components.NveCounterCardHostView
import dev.nve.components.NveCounterCardVariant
import dev.nve.components.NveCounterCardViewManager
import dev.nve.host.BinReader
import dev.nve.host.BinTag
import dev.nve.host.BinWriter
import dev.nve.host.EventRingWriter
import dev.nve.host.HybridRuntime
import dev.nve.host.RuntimeSurface
import dev.nve.host.generated.NveAbi
import dev.nve.host.generated.NveRuntimeAbi
import dev.nve.host.generated.NveTables
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import java.nio.ByteBuffer

/**
 * Real Kotlin + Compose compilation and behavioral verification of generated `export_rn` Android
 * `ViewManager.generated.kt` and `export_compose` `Composable.generated.kt` artifacts against real
 * `dev.nve.host.*` + `dev.nve.compose.*` classes and stub `com.facebook.react.*` types.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class GeneratedExportKotlinTest {
    @get:Rule
    val compose = createComposeRule()

    private fun simpleScene(): TestScene = TestScene().node(2, 1, 0f, 0f, 200f, 80f).also {
        it.set(it.table(NveTables.Text.KEY, NveTables.Text.WORDS), 2, NveTables.Text.TEXT, "Counter")
    }

    @Test
    fun generatedComposeWrapperEncodesPropsDispatchesCommandsAndHandlesEvents() {
        val host = FakeRuntimeHost(simpleScene())
        val inner = NveSurfaceController(host)
        val controller = NveCounterCardController(inner)
        val received = ArrayList<NveCounterCardCountChangeEvent>()
        var resets = 0

        compose.setContent {
            NveCounterCard(
                initialCount = 5,
                accentColor = "#2563EB",
                variant = NveCounterCardVariant.Outline,
                stepScale = 1.5f,
                disabled = true,
                onCountChange = { received.add(it) },
                onReset = { resets++ },
                modifier = Modifier.size(200.dp, 80.dp),
                controller = controller,
            )
        }
        compose.waitForIdle()

        assertTrue("props encoded and pushed via setProps", host.propsHistory.isNotEmpty())
        val r = BinReader(host.propsHistory.last())
        assertEquals(BinTag.MAP, r.peekTag())
        val m = r.beginMap()
        assertEquals(5, m.count)
        val props = LinkedHashMap<String, Any?>()
        for (i in 0 until m.count) {
            val k = r.readMapKey()
            props[k] = when (r.peekTag()) {
                BinTag.I64 -> r.readI64()
                BinTag.F64 -> r.readF64()
                BinTag.BOOL -> r.readBool()
                BinTag.STR -> r.readStr()
                else -> { r.skipValue(); null }
            }
        }
        r.end(m)
        r.expectEnd()
        assertEquals(5L, props["initialCount"])
        assertEquals("#2563EB", props["accentColor"])
        assertEquals("outline", props["variant"])
        assertEquals(1.5, props["stepScale"] as Double, 1e-6)
        assertEquals(true, props["disabled"])

        // Imperative command encodes BinWriter map as root nve.custom event
        controller.setCount(42)
        val cmdRing = host.rings.last { it.table(0) == NveAbi.TABLE_CUSTOM }
        assertEquals("setCount", cmdRing.customName(0))
        val cr = BinReader(cmdRing.customData(0))
        val cm = cr.beginMap()
        assertEquals(1, cm.count)
        assertEquals("value", cr.readMapKey())
        assertEquals(42L, cr.readI64())
        cr.end(cm)
        cr.expectEnd()

        // Outbound events decode via BinReader -> typed Compose callbacks
        val payload = BinWriter().apply {
            val pm = beginMap(2)
            writeMapKey("count"); writeI64(12)
            writeMapKey("delta"); writeI64(3)
            end(pm)
        }.toByteArray()
        host.enqueueEmitted {
            it.custom(0, 1L, "onCountChange", payload)
            it.custom(0, 2L, "reset", null)
        }
        compose.runOnIdle { inner.render() }
        compose.waitForIdle()
        assertEquals(listOf(NveCounterCardCountChangeEvent(count = 12, delta = 3)), received)
        assertEquals(1, resets)

        // Malformed / empty payloads return null safely
        assertNull(NveCounterCardCountChangeEvent.fromBinBytes(ByteArray(0)))
        assertNull(NveCounterCardCountChangeEvent.fromBinBytes(byteArrayOf(0x7F)))

        // Measure passthrough
        val measured = controller.measureSurface(maxWidthDp = 240f)
        assertNotNull(measured)
        assertEquals(120f, measured!!.width, 1e-3f)
        assertEquals(44f, measured.height, 1e-3f)
    }

    @Test
    fun generatedRnViewManagerBatchesPropsDispatchesEventsAndMeasuresSurface() {
        val reactContext = ThemedReactContext(RuntimeEnvironment.getApplication())
        val host = FakeRuntimeHost(simpleScene())
        host.measuredResult = RuntimeSurface.SurfaceMeasure(180f, 64f, Float.NaN, Float.NaN)
        val controller = NveSurfaceController(host)
        val manager = NveCounterCardViewManager()
        val view = NveCounterCardHostView(reactContext).apply { id = 77 }
        manager.bindController(reactContext, view, controller)

        assertEquals("NveCounterCard", manager.getName())
        assertTrue(manager.getExportedCustomDirectEventTypeConstants().containsKey("topCountChange"))
        assertTrue(manager.getExportedCustomBubblingEventTypeConstants().containsKey("topReset"))

        // Props batch until onAfterUpdateTransaction
        manager.setInitialCount(view, 9)
        manager.setAccentColor(view, 0xFF112233.toInt())
        manager.setVariant(view, "ghost")
        manager.setStepScale(view, 2.5f)
        manager.setDisabled(view, false)
        assertTrue("props are batched before onAfterUpdateTransaction", host.propsHistory.isEmpty())

        manager.onAfterUpdateTransaction(view)
        assertEquals(1, host.propsHistory.size)
        val r = BinReader(host.propsHistory[0])
        val m = r.beginMap()
        assertEquals(5, m.count)
        val decoded = LinkedHashMap<String, Any?>()
        for (i in 0 until m.count) {
            val k = r.readMapKey()
            decoded[k] = when (r.peekTag()) {
                BinTag.I64 -> r.readI64()
                BinTag.F64 -> r.readF64()
                BinTag.BOOL -> r.readBool()
                BinTag.STR -> r.readStr()
                else -> { r.skipValue(); null }
            }
        }
        r.end(m)
        assertEquals(9L, decoded["initialCount"])
        assertEquals(0xFF112233.toInt().toLong(), decoded["accentColor"])
        assertEquals("ghost", decoded["variant"])
        assertEquals(2.5, decoded["stepScale"] as Double, 1e-6)
        assertEquals(false, decoded["disabled"])

        // Second onAfterUpdateTransaction without new props is a no-op
        manager.onAfterUpdateTransaction(view)
        assertEquals(1, host.propsHistory.size)

        // Outbound emitted events route through controller.onEvent -> RCTEventEmitter
        val payload = BinWriter().apply {
            val pm = beginMap(2)
            writeMapKey("count"); writeI64(21)
            writeMapKey("delta"); writeI64(-4)
            end(pm)
        }.toByteArray()
        host.enqueueEmitted {
            it.custom(0, 10L, "onCountChange", payload)
            it.custom(0, 11L, "onReset", null)
        }
        controller.render()
        assertEquals(2, reactContext.recordedEvents.size)
        assertEquals(77, reactContext.recordedEvents[0].targetTag)
        assertEquals("topCountChange", reactContext.recordedEvents[0].eventName)
        assertEquals(mapOf("count" to 21, "delta" to -4), reactContext.recordedEvents[0].payload)
        assertEquals("topReset", reactContext.recordedEvents[1].eventName)

        // Direct NVEV v2 batch dispatch via EventRingReader
        reactContext.recordedEvents.clear()
        val ring = EventRingWriter(ByteBuffer.allocate(1024), 1)
        ring.custom(0, 12L, "countChange", payload)
        manager.dispatchEmittedBatch(reactContext, view, ring.buffer().array().copyOf(ring.length()))
        assertEquals(1, reactContext.recordedEvents.size)
        assertEquals("topCountChange", reactContext.recordedEvents[0].eventName)

        // Surface measurement + lifecycle cleanup
        val measured = manager.measureSurface(view, 0f, 0f, 300f, Float.POSITIVE_INFINITY, NveRuntimeAbi.MEASURE_FLAG_DRY_RUN)
        assertNotNull(measured)
        assertEquals(180f, measured!!.width, 1e-3f)
        assertEquals(64f, measured.height, 1e-3f)
        assertFalse(host.closed)
        manager.onDropViewInstance(view)
        assertTrue(host.closed)
    }

    @Test
    fun generatedComplexAndMinimalSpecsCompileAndExecuteCleanly() {
        val reactContext = ThemedReactContext(RuntimeEnvironment.getApplication())
        val gridHost = FakeRuntimeHost(simpleScene())
        val gridController = NveSurfaceController(gridHost)
        val gridManager = DataGridViewManager()
        val gridView = DataGridHostView(reactContext).apply { id = 88 }
        gridManager.bindController(reactContext, gridView, gridController)
        gridManager.setDensity(gridView, 2)
        gridManager.setRowHeight(gridView, 52.5)
        gridManager.setColumns(gridView, SimpleReadableArray(listOf("Name", "Score")))
        gridManager.onAfterUpdateTransaction(gridView)
        assertEquals(1, gridHost.propsHistory.size)

        val gridComposeHost = FakeRuntimeHost(simpleScene())
        val gridComposeCtrl = DataGridController(NveSurfaceController(gridComposeHost))
        val selected = ArrayList<DataGridRowSelectEvent>()
        val dynamicMeta = HybridRuntime.DynamicValue.of(BinWriter().apply { writeStr("v1") }.toByteArray())
        compose.setContent {
            DataGrid(
                density = DataGridDensity.V2,
                rowHeight = 48.0,
                columns = listOf("A", "B"),
                meta = dynamicMeta,
                onRowSelect = { selected.add(it) },
                controller = gridComposeCtrl,
            )
            EmptyCanvas(controller = EmptyCanvasController(NveSurfaceController(FakeRuntimeHost(simpleScene()))))
        }
        compose.waitForIdle()
        gridComposeCtrl.scrollToRow(index = 3, animated = true)
        assertTrue(gridComposeHost.propsHistory.isNotEmpty())
        assertTrue(gridComposeHost.rings.any { it.table(0) == NveAbi.TABLE_CUSTOM && it.customName(0) == "scrollToRow" })

        val emptyManager = EmptyCanvasViewManager()
        val emptyView = EmptyCanvasHostView(reactContext)
        emptyManager.bindController(reactContext, emptyView, NveSurfaceController(FakeRuntimeHost(simpleScene())))
        emptyManager.onDropViewInstance(emptyView)
    }
}
