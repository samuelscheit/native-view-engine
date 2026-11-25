package dev.nve.compose

import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Assertions.assertNotNull
import org.junit.jupiter.api.Assertions.assertNull
import org.junit.jupiter.api.Assertions.assertThrows
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test

class NveSurfaceStateAndControllerTest {

    @Test
    fun encodesPressTextScrollLayoutAndCustomEventsIntoBinaryEventRings() {
        val bridge = FakeNveBridgeBackend()
        val controller = NveSurfaceController(
            componentName = "NveCounterCard",
            surfaceId = 42,
            bridge = bridge,
        )
        assertTrue(controller.attach())

        assertTrue(controller.dispatchPress(nodeId = 10, x = 15.5f, y = 25.5f, timestampUs = 100L))
        assertTrue(controller.dispatchTextChange(nodeId = 11, text = "hello", timestampUs = 200L))
        assertTrue(
            controller.dispatchScroll(
                nodeId = 12,
                x = 0f,
                y = 120f,
                contentWidth = 360f,
                contentHeight = 2000f,
                timestampUs = 300L,
            ),
        )
        assertTrue(
            controller.dispatchLayout(
                nodeId = 0,
                x = 0f,
                y = 0f,
                width = 400f,
                height = 800f,
                timestampUs = 400L,
            ),
        )
        assertTrue(
            controller.dispatchCustomEvent(
                nodeId = 13,
                name = "onSubmit",
                payload = NveBinValue.Map(listOf("ok" to NveBinValue.Bool(true))),
                timestampUs = 500L,
            ),
        )

        assertEquals(5, bridge.submittedEventRings.size)

        val press = NveEventRingCodec.decode(bridge.submittedEventRings[0]).single()
        assertEquals(NveEventKind.PRESS, press.kind)
        assertEquals(42, press.surfaceId)
        assertEquals(10, press.nodeId)
        assertEquals(NveEventPayload.Point(15.5f, 25.5f), press.payload)

        val text = NveEventRingCodec.decode(bridge.submittedEventRings[1]).single()
        assertEquals(NveEventKind.TEXT_CHANGE, text.kind)
        assertEquals(NveEventPayload.Text("hello"), text.payload)

        val scroll = NveEventRingCodec.decode(bridge.submittedEventRings[2]).single()
        assertEquals(NveEventKind.SCROLL, scroll.kind)
        assertEquals(NveEventPayload.Scroll(0f, 120f, 360f, 2000f), scroll.payload)

        val layout = NveEventRingCodec.decode(bridge.submittedEventRings[3]).single()
        assertEquals(NveEventKind.LAYOUT, layout.kind)
        assertEquals(NveEventPayload.Layout(0f, 0f, 400f, 800f), layout.payload)

        val custom = NveEventRingCodec.decode(bridge.submittedEventRings[4]).single()
        assertEquals(NveEventKind.CUSTOM, custom.kind)
        assertEquals("onSubmit", custom.name)
        assertEquals(true, (custom.payload as NveEventPayload.Custom).parseBinValue().mapValue("ok")!!.boolValue)
    }

    @Test
    fun batchesQueuedEventsAndForwardsOutboundEvents() {
        val bridge = FakeNveBridgeBackend()
        val received = mutableListOf<NveEvent>()
        val controller = NveSurfaceController(componentName = "NveEditor", surfaceId = 9, bridge = bridge)
        controller.onEventCallback = { received.add(it) }
        assertTrue(controller.attach())

        controller.enqueueEvent(NveEventRingCodec.press(9, 1, 5f, 6f, 10L))
        controller.enqueueEvent(NveEventRingCodec.textChange(9, 2, "abc", 20L))
        assertEquals(2, controller.pendingEventCount)

        bridge.enqueueOutboundEvents(
            NveEventRingCodec.custom(9, 2, "onAck", NveBinValue.I64(77).encode(), 30L),
        )
        assertTrue(controller.flushEvents())
        assertEquals(0, controller.pendingEventCount)

        val batch = NveEventRingCodec.decode(bridge.submittedEventRings.single())
        assertEquals(2, batch.size)
        assertEquals(1, received.size)
        assertEquals("onAck", received.single().name)
        assertEquals(77L, (received.single().payload as NveEventPayload.Custom).parseBinValue().int64Value)
    }

    @Test
    fun resizeUpdatesViewportEmitsLayoutEventAndDeduplicates() {
        val bridge = FakeNveBridgeBackend()
        val controller = NveSurfaceController(componentName = "NvePanel", surfaceId = 5, bridge = bridge)
        assertTrue(controller.attach())

        assertTrue(controller.resize(width = 390f, height = 844f, scaleFactor = 3f, timestampUs = 999L))
        assertEquals(NveViewport(390f, 844f, 3f), controller.state.viewport)
        assertEquals(1, bridge.resizeCalls.size)
        assertEquals(1, bridge.submittedEventRings.size)

        val layoutEvt = NveEventRingCodec.decode(bridge.submittedEventRings.single()).single()
        assertEquals(NveEventKind.LAYOUT, layoutEvt.kind)
        assertEquals(5, layoutEvt.surfaceId)
        assertEquals(NveEventPayload.Layout(0f, 0f, 390f, 844f), layoutEvt.payload)

        // Identical resize is deduplicated
        assertTrue(controller.resize(width = 390f, height = 844f, scaleFactor = 3f))
        assertEquals(1, bridge.resizeCalls.size)

        // Invalid viewport dimensions rejected
        assertThrows(IllegalArgumentException::class.java) {
            controller.resize(width = -1f, height = 100f, scaleFactor = 1f)
        }
        assertThrows(IllegalArgumentException::class.java) {
            controller.resize(width = 100f, height = 100f, scaleFactor = 0f)
        }
    }

    @Test
    fun propsAndCommandsUseBinaryNveBinValueAndDeduplicateProps() {
        val bridge = FakeNveBridgeBackend()
        val controller = NveSurfaceController(componentName = "NveCard", surfaceId = 3, bridge = bridge)
        assertTrue(controller.attach())

        controller.updateProps(mapOf("title" to "Hello", "count" to 1))
        controller.updateProps(mapOf("title" to "Hello", "count" to 1)) // duplicate -> skipped
        controller.updateProps(mapOf("title" to "Hello", "count" to 2))

        assertEquals(2, bridge.propsBinCalls.size)
        val decodedFirst = NveBinValueView.parse(bridge.propsBinCalls[0])
        assertEquals("Hello", decodedFirst.mapValue("title")!!.stringValue)
        assertEquals(1L, decodedFirst.mapValue("count")!!.int64Value)

        val decodedSecond = NveBinValueView.parse(bridge.propsBinCalls[1])
        assertEquals(2L, decodedSecond.mapValue("count")!!.int64Value)

        controller.dispatchCommand("reset", mapOf("to" to 0))
        assertEquals(1, bridge.commandCalls.size)
        assertEquals("reset", bridge.commandCalls[0].name)
        assertEquals(0L, NveBinValueView.parse(bridge.commandCalls[0].argsBin).mapValue("to")!!.int64Value)
    }

    @Test
    fun detachReattachAndDisposeLifecyclePreserveInvariantsAndCleanUp() {
        val bridge = FakeNveBridgeBackend()
        val controller = NveSurfaceController(componentName = "NveApp", surfaceId = 77, bridge = bridge)

        assertEquals(NveSurfaceLifecycleState.DETACHED, controller.state.lifecycleState)
        assertTrue(controller.attach())
        assertEquals(NveSurfaceLifecycleState.ATTACHED, controller.state.lifecycleState)
        assertTrue(controller.isMounted)
        assertEquals(1, bridge.activeHandleCount())

        controller.resize(360f, 720f, 2f)
        controller.updateProps(mapOf("theme" to "dark"))
        val diff = controller.requestRender()
        assertNotNull(diff)
        assertEquals(1, controller.state.mountedNodes.size)

        // Detach destroys native handle and clears scene while keeping surfaceId & viewport
        controller.detach()
        assertFalse(controller.isMounted)
        assertEquals(NveSurfaceLifecycleState.DETACHED, controller.state.lifecycleState)
        assertEquals(0, bridge.activeHandleCount())
        assertTrue(controller.state.mountedNodes.isEmpty())

        // Re-attach restores handle under the same surfaceId and re-syncs viewport & binary props
        assertTrue(controller.attach())
        assertEquals(2, bridge.createCalls.size)
        assertEquals(77, bridge.createCalls[0].surfaceId)
        assertEquals(77, bridge.createCalls[1].surfaceId)
        assertEquals(2, bridge.resizeCalls.size)
        assertEquals(2, bridge.propsBinCalls.size)

        // Reload increments generation and re-renders
        assertTrue(controller.reload("/data/local/tmp/libapp_v2.so"))
        assertEquals(1, controller.state.reloadGeneration)
        bridge.reloadShouldSucceed = false
        assertFalse(controller.reload("/data/local/tmp/libapp_broken.so"))
        assertEquals(1, controller.state.reloadGeneration)
        assertFalse(controller.reload("   "))

        // Permanent dispose cleans up and is idempotent
        controller.dispose()
        controller.dispose()
        assertTrue(controller.isDisposed)
        assertEquals(NveSurfaceLifecycleState.DISPOSED, controller.state.lifecycleState)
        assertEquals(0, bridge.activeHandleCount())
        assertEquals(2, bridge.destroyCalls.size) // 1 from detach + 1 from dispose (not 3)
        assertFalse(controller.attach())
        assertFalse(controller.resize(100f, 100f, 1f))
        assertNull(controller.requestRender())
    }

    @Test
    fun channelClientPubSubAndRetainedBinaryMessagesWorkZeroJson() {
        val client = NveChannelClient()
        val seen = mutableListOf<Long>()

        client.publishMap("counter/state", mapOf("value" to 41), retain = true)
        val unsub = client.subscribe("counter/state", replayRetained = true) { msg ->
            seen.add(msg.parseBinValue().mapValue("value")!!.int64Value!!)
        }
        client.publishMap("counter/state", mapOf("value" to 42), retain = true)
        unsub()
        client.publishMap("counter/state", mapOf("value" to 43), retain = true)

        assertEquals(listOf(41L, 42L), seen)
    }
}
