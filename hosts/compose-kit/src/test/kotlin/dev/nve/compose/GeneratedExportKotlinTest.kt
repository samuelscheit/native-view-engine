package dev.nve.compose

import android.widget.FrameLayout
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import com.facebook.react.bridge.SimpleReadableArray
import com.facebook.react.bridge.SimpleReadableMap
import com.facebook.react.uimanager.ThemedReactContext
import dev.nve.components.AdversarialShadow
import dev.nve.components.AdversarialShadowController
import dev.nve.components.AdversarialShadowHostView
import dev.nve.components.AdversarialShadowMark
import dev.nve.components.AdversarialShadowMatrixEvent
import dev.nve.components.AdversarialShadowReader
import dev.nve.components.AdversarialShadowViewManager
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
import dev.nve.components.rememberNveCounterCardController
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
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import org.robolectric.shadows.ShadowLooper
import java.nio.ByteBuffer

/**
 * Real Kotlin + Compose compilation and behavioral verification of generated `export_rn` Android
 * `ViewManager.generated.kt` and `export_compose` `Composable.generated.kt` artifacts against real
 * `dev.nve.host.*` + `dev.nve.compose.*` classes and `com.facebook.react.*` bridge stubs.
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

    private fun decodeBinValue(r: BinReader): Any? = when (r.peekTag()) {
        BinTag.NULL -> { r.readNull(); null }
        BinTag.BOOL -> r.readBool()
        BinTag.I64 -> r.readI64()
        BinTag.F64 -> r.readF64()
        BinTag.STR -> r.readStr()
        BinTag.BYTES -> r.readBytes()
        BinTag.ARRAY -> {
            val m = r.beginArray()
            val list = ArrayList<Any?>(m.count)
            for (i in 0 until m.count) list.add(decodeBinValue(r))
            r.end(m)
            list
        }
        BinTag.MAP -> {
            val m = r.beginMap()
            val map = LinkedHashMap<String, Any?>(m.count)
            for (i in 0 until m.count) {
                val k = r.readMapKey()
                map[k] = decodeBinValue(r)
            }
            r.end(m)
            map
        }
    }

    @Suppress("UNCHECKED_CAST")
    private fun decodeBinMap(bytes: ByteArray): Map<String, Any?> {
        val r = BinReader(bytes)
        assertEquals(BinTag.MAP, r.peekTag())
        val map = decodeBinValue(r) as Map<String, Any?>
        r.expectEnd()
        return map
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
        val props = decodeBinMap(host.propsHistory.last())
        assertEquals(5, props.size)
        assertEquals(5L, props["initialCount"])
        assertEquals("#2563EB", props["accentColor"])
        assertEquals("outline", props["variant"])
        assertEquals(1.5, props["stepScale"] as Double, 1e-6)
        assertEquals(true, props["disabled"])

        // Imperative command encodes BinWriter map as root nve.custom event
        controller.setCount(42)
        val cmdRing = host.rings.last { it.table(0) == NveAbi.TABLE_CUSTOM }
        assertEquals("setCount", cmdRing.customName(0))
        assertEquals(mapOf("value" to 42L), decodeBinMap(cmdRing.customData(0)))

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
    fun rememberCounterCardControllerSwitchesSourceAndReappliesProps() {
        val host = FakeRuntimeHost(simpleScene())
        var activeSource: NveSource by mutableStateOf(NveSource.App("/app/v1.so"))

        compose.setContent {
            val ctrl = rememberNveCounterCardController(
                source = activeSource,
                hostFactory = { _, _, _, _, _ -> host },
            )
            NveCounterCard(
                initialCount = 7,
                controller = ctrl,
                source = activeSource,
            )
        }
        compose.waitForIdle()
        assertEquals(listOf<NveSource>(NveSource.App("/app/v1.so")), host.started)
        val propsAfterV1 = host.propsHistory.size
        assertTrue(propsAfterV1 >= 1)

        // Switching `source` restarts the controller AND re-applies props via LaunchedEffect(controller, source, ...)
        activeSource = NveSource.App("/app/v2.so", carryState = true)
        compose.waitForIdle()
        assertEquals(
            listOf<NveSource>(NveSource.App("/app/v1.so"), NveSource.App("/app/v2.so", carryState = true)),
            host.started,
        )
        assertTrue("props re-applied after source switch", host.propsHistory.size > propsAfterV1)
        assertEquals(7L, decodeBinMap(host.propsHistory.last())["initialCount"])
    }

    @Test
    fun nveSurfaceDisposalAfterClosedControllerNeverThrows() {
        val host = FakeRuntimeHost(simpleScene()).apply { throwOnClosedStructural = true }
        val controller = NveSurfaceController(host)
        var show by mutableStateOf(true)
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

        compose.setContent {
            if (show) {
                NveSurface(
                    controller = controller,
                    onEvent = {},
                    hostBridge = bridge,
                )
            }
        }
        compose.waitForIdle()
        assertEquals(bridge, host.hostBridge)

        // Close controller BEFORE unmounting NveSurface (mirrors NveHostActivity.onDestroy)
        compose.runOnIdle { controller.close() }
        show = false
        compose.waitForIdle()
        assertTrue(host.closed)
    }

    @Test
    fun generatedRnViewManagerBatchesPropsDispatchesCommandsEventsWindowAttachAndDrop() {
        val activity = Robolectric.buildActivity(ComponentActivity::class.java).setup().get()
        val reactContext = ThemedReactContext(activity)
        val host = FakeRuntimeHost(simpleScene()).apply { throwOnClosedStructural = true }
        host.measuredResult = RuntimeSurface.SurfaceMeasure(180f, 64f, Float.NaN, Float.NaN)
        val controller = NveSurfaceController(host)
        val manager = NveCounterCardViewManager()
        val view = NveCounterCardHostView(reactContext).apply { id = 77 }
        // Attach to a real Activity window so ComposeView actually composes NveSurface!
        val root = FrameLayout(activity)
        activity.setContentView(root)
        root.addView(view)
        manager.bindController(reactContext, view, controller)
        ShadowLooper.idleMainLooper()
        assertNotNull("ComposeView composed NveSurface and installed text measurer", host.measurer)

        assertEquals("NveCounterCard", manager.getName())
        assertTrue(manager.getExportedCustomDirectEventTypeConstants().containsKey("topCountChange"))
        assertTrue(manager.getExportedCustomBubblingEventTypeConstants().containsKey("topReset"))

        // Props batch until onAfterUpdateTransaction; ARGB Color encodes positive unsigned u32 in i64
        manager.setInitialCount(view, 9)
        manager.setAccentColor(view, 0xFF112233.toInt())
        manager.setVariant(view, "ghost")
        manager.setStepScale(view, 2.5f)
        manager.setDisabled(view, false)
        assertTrue("props are batched before onAfterUpdateTransaction", host.propsHistory.isEmpty())

        manager.onAfterUpdateTransaction(view)
        assertEquals(1, host.propsHistory.size)
        val decoded = decodeBinMap(host.propsHistory[0])
        assertEquals(9L, decoded["initialCount"])
        assertEquals(0xFF112233L, decoded["accentColor"])
        assertEquals("ghost", decoded["variant"])
        assertEquals(2.5, decoded["stepScale"] as Double, 1e-6)
        assertEquals(false, decoded["disabled"])

        // Null prop clearing encodes BinTag.NULL
        manager.setAccentColor(view, null)
        manager.onAfterUpdateTransaction(view)
        assertEquals(2, host.propsHistory.size)
        val cleared = decodeBinMap(host.propsHistory[1])
        assertTrue(cleared.containsKey("accentColor"))
        assertNull(cleared["accentColor"])

        // Second onAfterUpdateTransaction without new props is a no-op
        manager.onAfterUpdateTransaction(view)
        assertEquals(2, host.propsHistory.size)

        // Imperative RN command (`receiveCommand`)
        manager.receiveCommand(view, "setCount", SimpleReadableArray(listOf(123)))
        val cmdRing = host.rings.last { it.table(0) == NveAbi.TABLE_CUSTOM }
        assertEquals("setCount", cmdRing.customName(0))
        assertEquals(mapOf("value" to 123L), decodeBinMap(cmdRing.customData(0)))

        // Missing/null required arg safely ignored
        val ringCountBefore = host.rings.size
        manager.receiveCommand(view, "setCount", SimpleReadableArray(listOf(null)))
        manager.receiveCommand(view, "setCount", null)
        assertEquals(ringCountBefore, host.rings.size)

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

        // Corrupt binary event payload aborts cleanly and NEVER emits a partial WritableMap!
        reactContext.recordedEvents.clear()
        val truncatedPayload = payload.copyOf(payload.size - 2)
        manager.emitCountChange(reactContext, view, truncatedPayload)
        manager.emitCountChange(reactContext, view, byteArrayOf(0x02, 1, 0, 0, 0, 0, 0, 0, 0)) // non-map
        manager.emitCountChange(reactContext, view, payload + byteArrayOf(0)) // trailing byte
        assertTrue("corrupt event payloads never reach RCTEventEmitter", reactContext.recordedEvents.isEmpty())

        // Direct NVEV v2 batch dispatch via EventRingReader
        val ring = EventRingWriter(ByteBuffer.allocate(1024), 1)
        ring.custom(0, 12L, "countChange", payload)
        manager.dispatchEmittedBatch(reactContext, view, ring.buffer().array().copyOf(ring.length()))
        assertEquals(1, reactContext.recordedEvents.size)
        assertEquals("topCountChange", reactContext.recordedEvents[0].eventName)

        // Surface measurement + clean window-attached teardown (composeView disposes BEFORE controller.close!)
        val measured = manager.measureSurface(view, 0f, 0f, 300f, Float.POSITIVE_INFINITY, NveRuntimeAbi.MEASURE_FLAG_DRY_RUN)
        assertNotNull(measured)
        assertEquals(180f, measured!!.width, 1e-3f)
        assertEquals(64f, measured.height, 1e-3f)
        assertFalse(host.closed)
        manager.onDropViewInstance(view)
        assertTrue(host.closed)
        assertNull(view.controller)
        assertNull("NveSurface disposed its text measurer cleanly before close", host.measurer)
        // Even if Android runs another measure/traversal frame while the view is still in `root`,
        // cleared `composeView.setContent {}` must never re-compose the closed controller.
        view.requestLayout()
        ShadowLooper.idleMainLooper()
    }

    @Test
    fun dataGridObjectPropsMultiTransactionMutationCommandsEventsAndEmptyCanvas() {
        val reactContext = ThemedReactContext(RuntimeEnvironment.getApplication())
        val gridHost = FakeRuntimeHost(simpleScene())
        val gridController = NveSurfaceController(gridHost)
        val gridManager = DataGridViewManager()
        val gridView = DataGridHostView(reactContext).apply { id = 88 }
        gridManager.bindController(reactContext, gridView, gridController)

        // Transaction 1: eager encoding captures ReadableArray & ReadableMap BEFORE mutation!
        val colsArr = SimpleReadableArray(listOf("Name", null, "Score"))
        val metaMap = SimpleReadableMap(
            mapOf(
                "page" to 2,
                "nested" to SimpleReadableMap(mapOf("flag" to true, "tag" to "ok")),
                "items" to SimpleReadableArray(listOf(10, "x", null)),
                "empty" to null,
            ),
        )
        gridManager.setDensity(gridView, 2)
        gridManager.setRowHeight(gridView, 52.5)
        gridManager.setColumns(gridView, colsArr)
        gridManager.setMeta(gridView, metaMap)
        // Mutate both RN bridge objects BEFORE onAfterUpdateTransaction: eager encoding MUST survive!
        colsArr.mutateClear()
        metaMap.mutateClear()

        gridManager.onAfterUpdateTransaction(gridView)
        assertEquals(1, gridHost.propsHistory.size)
        val tx1 = decodeBinMap(gridHost.propsHistory[0])
        assertEquals(listOf("Name", null, "Score"), tx1["columns"])
        @Suppress("UNCHECKED_CAST")
        val decodedMeta = tx1["meta"] as Map<String, Any?>
        assertEquals(2.0, decodedMeta["page"] as Double, 1e-6)
        assertEquals(mapOf("flag" to true, "tag" to "ok"), decodedMeta["nested"])
        assertEquals(listOf(10.0, "x", null), decodedMeta["items"])
        assertTrue(decodedMeta.containsKey("empty") && decodedMeta["empty"] == null)

        // Transaction 2: update array + clear meta to null
        val colsArr2 = SimpleReadableArray(listOf("OnlyOne"))
        gridManager.setColumns(gridView, colsArr2)
        gridManager.setMeta(gridView, null)
        colsArr2.mutateSet(0, "MUTATED")
        gridManager.onAfterUpdateTransaction(gridView)
        assertEquals(2, gridHost.propsHistory.size)
        val tx2 = decodeBinMap(gridHost.propsHistory[1])
        assertEquals(listOf("OnlyOne"), tx2["columns"])
        assertNull(tx2["meta"])

        // receiveCommand with required + optional param (present and omitted)
        gridManager.receiveCommand(gridView, "scrollToRow", SimpleReadableArray(listOf(5, true)))
        assertEquals(
            mapOf("index" to 5L, "animated" to true),
            decodeBinMap(gridHost.rings.last().customData(0)),
        )
        gridManager.receiveCommand(gridView, "scrollToRow", SimpleReadableArray(listOf(9)))
        assertEquals(
            mapOf("index" to 9L),
            decodeBinMap(gridHost.rings.last().customData(0)),
        )

        // DataGrid onRowSelect event with nested Object `extra` AND `null` `extra` in RN & Compose
        val rowSelectMapPayload = BinWriter().apply {
            val pm = beginMap(3)
            writeMapKey("rowIndex"); writeI64(4)
            writeMapKey("note"); writeStr("picked")
            writeMapKey("extra")
            val em = beginMap(2)
            writeMapKey("source"); writeStr("tap")
            writeMapKey("ids")
            val am = beginArray(2); writeI64(1); writeI64(2); end(am)
            end(em)
            end(pm)
        }.toByteArray()
        val rowSelectNullPayload = BinWriter().apply {
            val pm = beginMap(3)
            writeMapKey("rowIndex"); writeI64(7)
            writeMapKey("note"); writeNull()
            writeMapKey("extra"); writeNull()
            end(pm)
        }.toByteArray()

        gridManager.emitRowSelect(reactContext, gridView, rowSelectMapPayload)
        gridManager.emitRowSelect(reactContext, gridView, rowSelectNullPayload)
        assertEquals(2, reactContext.recordedEvents.size)
        assertEquals(
            mapOf(
                "rowIndex" to 4,
                "note" to "picked",
                "extra" to mapOf("source" to "tap", "ids" to listOf(1, 2)),
            ),
            reactContext.recordedEvents[0].payload,
        )
        assertEquals(
            mapOf("rowIndex" to 7, "note" to null, "extra" to null),
            reactContext.recordedEvents[1].payload,
        )

        // Compose DataGridRowSelectEvent decodes `BinTag.NULL` for nullable `extra: DynamicValue?` as Kotlin `null`!
        val evWithExtra = DataGridRowSelectEvent.fromBinBytes(rowSelectMapPayload)
        assertNotNull(evWithExtra)
        assertEquals(4, evWithExtra!!.rowIndex)
        assertEquals("picked", evWithExtra.note)
        assertNotNull(evWithExtra.extra)
        assertEquals(BinTag.MAP, evWithExtra.extra!!.tag())

        val evWithNullExtra = DataGridRowSelectEvent.fromBinBytes(rowSelectNullPayload)
        assertNotNull(evWithNullExtra)
        assertEquals(7, evWithNullExtra!!.rowIndex)
        assertNull(evWithNullExtra.note)
        assertNull("nullable Object? field decodes BinTag.NULL as Kotlin null", evWithNullExtra.extra)

        // EmptyCanvas skips setProps completely!
        val emptyComposeHost = FakeRuntimeHost(simpleScene())
        val gridComposeHost = FakeRuntimeHost(simpleScene())
        val gridComposeCtrl = DataGridController(NveSurfaceController(gridComposeHost))
        val dynamicMeta = HybridRuntime.DynamicValue.of(BinWriter().apply { writeStr("v1") }.toByteArray())
        compose.setContent {
            DataGrid(
                density = DataGridDensity.V2,
                rowHeight = 48.0,
                columns = listOf("A", "B"),
                meta = dynamicMeta,
                controller = gridComposeCtrl,
            )
            EmptyCanvas(controller = EmptyCanvasController(NveSurfaceController(emptyComposeHost)))
        }
        compose.waitForIdle()
        assertTrue(gridComposeHost.propsHistory.isNotEmpty())
        assertTrue("EmptyCanvas must not call setProps", emptyComposeHost.propsHistory.isEmpty())

        val emptyManager = EmptyCanvasViewManager()
        val emptyView = EmptyCanvasHostView(reactContext)
        emptyManager.bindController(reactContext, emptyView, NveSurfaceController(FakeRuntimeHost(simpleScene())))
        emptyManager.onDropViewInstance(emptyView)
    }

    @Test
    fun adversarialShadowingNestedArraysColorsAndEscapingCompileAndExecuteCleanly() {
        val reactContext = ThemedReactContext(RuntimeEnvironment.getApplication())
        val rnHost = FakeRuntimeHost(simpleScene())
        val rnCtrl = NveSurfaceController(rnHost)
        val mgr = AdversarialShadowViewManager()
        val view = AdversarialShadowHostView(reactContext).apply { id = 99 }
        mgr.bindController(reactContext, view, rnCtrl)

        // Colliding prop names (count, w, mark, reader, am, list, view, arr, m, i, j) + nested arrays + Color + $/quotes
        mgr.setCount(view, 11)
        mgr.setW(view, "\$price \"ok\"")
        mgr.setMark(view, 2)
        mgr.setReader(view, "b\"c")
        mgr.setAm(view, true)
        mgr.setList(
            view,
            SimpleReadableArray(
                listOf(
                    SimpleReadableArray(listOf(1, 2)),
                    SimpleReadableArray(listOf(3, null, 4)),
                ),
            ),
        )
        mgr.setView(view, 0xCCAA5500.toInt())
        mgr.setArr(view, SimpleReadableArray(listOf(0xFF010203.toInt(), null)))
        mgr.setM(view, SimpleReadableArray(listOf(SimpleReadableMap(mapOf("k" to "v")), null)))
        mgr.setI(view, 3.5f)
        mgr.setJ(view, 6.25)
        mgr.onAfterUpdateTransaction(view)

        assertEquals(1, rnHost.propsHistory.size)
        val rnProps = decodeBinMap(rnHost.propsHistory[0])
        assertEquals(11L, rnProps["count"])
        assertEquals("\$price \"ok\"", rnProps["w"])
        assertEquals(2L, rnProps["mark"])
        assertEquals("b\"c", rnProps["reader"])
        assertEquals(true, rnProps["am"])
        assertEquals(listOf(listOf(1L, 2L), listOf(3L, null, 4L)), rnProps["list"])
        assertEquals(0xCCAA5500L, rnProps["view"])
        assertEquals(listOf(0xFF010203L, null), rnProps["arr"])
        assertEquals(listOf(mapOf("k" to "v"), null), rnProps["m"])

        // receiveCommand with nested array + color + object
        mgr.receiveCommand(
            view,
            "applyAll",
            SimpleReadableArray(
                listOf(
                    5,
                    "\$cmd",
                    true,
                    0xFFABCDEF.toInt(),
                    SimpleReadableArray(listOf(SimpleReadableArray(listOf(7, 8)))),
                    SimpleReadableMap(mapOf("x" to 1)),
                ),
            ),
        )
        val cmdMap = decodeBinMap(rnHost.rings.last().customData(0))
        assertEquals(5L, cmdMap["count"])
        assertEquals("\$cmd", cmdMap["w"])
        assertEquals(true, cmdMap["mark"])
        assertEquals(0xFFABCDEFL, cmdMap["view"])
        assertEquals(listOf(listOf(7L, 8L)), cmdMap["list"])
        assertEquals(mapOf("x" to 1.0), cmdMap["obj"])

        // Compose wrapper: nullable-with-default `count: Int? = 3` accepts null AND default, nested Array<Array<Int>>, $/quotes
        val composeHost = FakeRuntimeHost(simpleScene())
        val composeCtrl = AdversarialShadowController(NveSurfaceController(composeHost))
        val events = ArrayList<AdversarialShadowMatrixEvent>()
        val dynItem = HybridRuntime.DynamicValue.of(BinWriter().apply { writeI64(99) }.toByteArray())
        compose.setContent {
            AdversarialShadow(
                count = null, // nullable: true preserved despite default_value = 3!
                list = listOf(listOf(10, 20), listOf(30)),
                m = listOf(dynItem),
                mark = AdversarialShadowMark.V2,
                reader = AdversarialShadowReader.A,
                onMatrix = { events.add(it) },
                controller = composeCtrl,
            )
        }
        compose.waitForIdle()
        val cProps = decodeBinMap(composeHost.propsHistory.last())
        assertFalse("null count omitted from Compose DirectPropsWriter", cProps.containsKey("count"))
        assertEquals("\$price \"quoted\"", cProps["w"])
        assertEquals("\$a", cProps["reader"])
        assertEquals(listOf(listOf(10L, 20L), listOf(30L)), cProps["list"])
        assertEquals(listOf(99L), cProps["m"])

        // Compose event decoding: nested Array<Array<Int>>, colliding fields, Color I64 -> #AARRGGBB, nullable Object?
        val matrixBin = BinWriter().apply {
            val pm = beginMap(11)
            writeMapKey("count"); writeI64(4)
            writeMapKey("w"); writeStr("hello")
            writeMapKey("mark"); writeI64(9)
            writeMapKey("reader"); writeStr("r")
            writeMapKey("am"); writeBool(true)
            writeMapKey("list")
            val outer = beginArray(2)
            val row0 = beginArray(2); writeI64(1); writeI64(2); end(row0)
            val row1 = beginArray(1); writeI64(3); end(row1)
            end(outer)
            writeMapKey("view"); writeI64(0xFF112233L)
            writeMapKey("arr")
            val sa = beginArray(1); writeStr("s0"); end(sa)
            writeMapKey("m"); writeNull()
            writeMapKey("i"); writeF64(1.25)
            writeMapKey("j"); writeF64(2.5)
            end(pm)
        }.toByteArray()
        val decodedEv = AdversarialShadowMatrixEvent.fromBinBytes(matrixBin)
        assertNotNull(decodedEv)
        assertEquals(4, decodedEv!!.count)
        assertEquals("hello", decodedEv.w)
        assertEquals(9, decodedEv.mark)
        assertEquals("r", decodedEv.reader)
        assertEquals(true, decodedEv.am)
        assertEquals(listOf(listOf(1, 2), listOf(3)), decodedEv.list)
        assertEquals("#FF112233", decodedEv.view)
        assertEquals(listOf("s0"), decodedEv.arr)
        assertNull(decodedEv.m)
        assertEquals(1.25f, decodedEv.i, 1e-6f)
        assertEquals(2.5, decodedEv.j, 1e-6)

        // RN event decoding with nested Array<Array<Int>> + colliding names
        mgr.emitMatrix(reactContext, view, matrixBin)
        assertEquals(1, reactContext.recordedEvents.size)
        val rnEv = reactContext.recordedEvents[0].payload
        assertEquals(listOf(listOf(1, 2), listOf(3)), rnEv["list"])
        assertEquals(0xFF112233.toInt(), rnEv["view"])
        assertNull(rnEv["m"])

        mgr.onDropViewInstance(view)
    }
}
