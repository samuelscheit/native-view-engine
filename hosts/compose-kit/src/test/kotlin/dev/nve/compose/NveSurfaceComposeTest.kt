package dev.nve.compose

import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTextReplacement
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipeUp
import androidx.compose.ui.unit.dp
import dev.nve.host.generated.NveAbi
import dev.nve.host.generated.NveTables
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** `NveSurface` composition under Robolectric: rendered nodes, semantics, input → runtime events. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
@GraphicsMode(GraphicsMode.Mode.NATIVE)  // real text measurement (legacy mode fakes glyph widths)
class NveSurfaceComposeTest {
    @get:Rule
    val compose = createComposeRule()

    private fun scene(): TestScene {
        val s = TestScene()
            .node(2, 1, 0f, 0f, 300f, 400f)
            .node(3, 2, 10f, 10f, 200f, 30f)
            .node(4, 2, 10f, 50f, 200f, 40f)
            .node(5, 2, 10f, 100f, 120f, 44f)
            .node(6, 2, 10f, 150f, 120f, 30f)
        val text = s.table(NveTables.Text.KEY, NveTables.Text.WORDS)
        s.set(text, 3, NveTables.Text.TEXT, "Hello NVE").set(text, 5, NveTables.Text.TEXT, "Submit").set(text, 6, NveTables.Text.TEXT, "secret")
        val input = s.table(NveTables.TextInput.KEY, NveTables.TextInput.WORDS)
        s.set(input, 4, NveTables.TextInput.VALUE, "draft")
        val interaction = s.table(NveTables.Interaction.KEY, NveTables.Interaction.WORDS)
        s.setEvents(interaction, 5, NveTables.Interaction.EVENTS_WORD, NveTables.Values.InteractionEventSet.PRESS)
        val a11y = s.table(NveTables.Accessibility.KEY, NveTables.Accessibility.WORDS)
        s.set(a11y, 5, NveTables.Accessibility.ROLE, NveTables.Values.AriaRole.BUTTON).set(a11y, 5, NveTables.Accessibility.LABEL, "Submit form")
            .set(a11y, 6, NveTables.Accessibility.HIDDEN, 1)
        val debug = s.table(NveTables.DebugId.KEY, NveTables.DebugId.WORDS)
        s.set(debug, 5, NveTables.DebugId.TEST_ID, "submit-btn")
        return s
    }

    @Test
    fun rendersTheSceneAndSendsInputBack() {
        val host = FakeRuntimeHost(scene())
        val controller = NveSurfaceController(host)
        controller.start(NveSource.App("/libnve_app.so"))
        compose.setContent { NveSurface(controller, Modifier.size(300.dp, 400.dp)) }

        compose.onNodeWithText("Hello NVE").assertIsDisplayed()
        compose.onNodeWithContentDescription("Submit form").assertExists()
        assertEquals("hidden nodes leave the semantics tree", 0, compose.onAllNodesWithText("secret").fetchSemanticsNodes().size)
        assertEquals(listOf(300f, 400f), host.viewports.last().take(2))

        assertTrue(
            "mounting a text field sends no focus / blur: ${host.rings.map { it.table(0) to it.event(0) }}",
            host.rings.none {
                it.isEvent(NveAbi.TABLE_INTERACTION, NveAbi.INTERACTION_EVENT_BLUR) ||
                    it.isEvent(NveAbi.TABLE_INTERACTION, NveAbi.INTERACTION_EVENT_FOCUS)
            },
        )
        compose.onNodeWithTag("submit-btn").performClick()
        val press = host.rings.last()
        assertTrue(press.isEvent(NveAbi.TABLE_INTERACTION, NveAbi.INTERACTION_EVENT_PRESS))
        assertEquals(0x1000005, press.node(0))

        compose.onNodeWithText("draft").performTextInput("!")
        assertTrue(
            "focusing by input is a real focus event",
            host.rings.any { it.isEvent(NveAbi.TABLE_INTERACTION, NveAbi.INTERACTION_EVENT_FOCUS) },
        )
        val text = host.rings.last { it.isEvent(NveAbi.TABLE_TEXT_INPUT, NveAbi.TEXT_INPUT_EVENT_CHANGE) }
        assertEquals(0x1000004, text.node(0))

        val measurer = host.measurer
        assertTrue("Compose text measurer installed", measurer is NveComposeTextMeasurer)
        val out = FloatArray(2)
        measurer!!.measure("Hello NVE", 16f, 400, Float.POSITIVE_INFINITY, out)
        assertTrue("measured ${out.toList()}", out[0] > 0f && out[1] > 0f)
        val huge = FloatArray(2)
        measurer.measure("Hello NVE", 16f, 400, 1e9f, huge)
        assertEquals("widths past Constraints are unconstrained", out.toList(), huge.toList())
        val nan = FloatArray(2)
        measurer.measure("Hello NVE", Float.NaN, 400, Float.NaN, nan)
        assertEquals("NaN size / width fall back to defaults", out.toList(), nan.toList())
        val empty = FloatArray(2)
        measurer.measure("", 16f, 400, Float.POSITIVE_INFINITY, empty)
        assertTrue("empty text keeps a line height: ${empty.toList()}", empty[0] == 0f && empty[1] > 0f)
        val spaces = FloatArray(2)
        measurer.measure("   ", 16f, 400, 0f, spaces)
        assertTrue("whitespace-only min-content: ${spaces.toList()}", spaces[1] > 0f)
        val narrow = FloatArray(2)
        measurer.measure("Hello NVE", 16f, 400, 0f, narrow)
        assertTrue("min-content wraps: ${narrow.toList()} vs ${out.toList()}", narrow[0] < out[0] && narrow[1] > out[1])
    }

    /** Column(2) → [scroll(3) → [rows 4, 5], image(6), canvas(7), disabled input(8), hidden button(9)]. */
    private fun richScene(epoch: Int = 0, value: String = "fixed"): TestScene {
        val s = TestScene(epoch = epoch)
            .node(2, 1, 0f, 0f, 300f, 600f)
            .node(3, 2, 0f, 0f, 300f, 100f)
            .node(4, 3, 0f, 0f, 300f, 80f)
            .node(5, 3, 0f, 80f, 300f, 80f)
            .node(6, 2, 0f, 110f, 32f, 32f)
            .node(7, 2, 0f, 150f, 100f, 40f)
            .node(8, 2, 0f, 200f, 200f, 40f)
            .node(9, 2, 0f, 250f, 120f, 44f)
        val text = s.table(NveTables.Text.KEY, NveTables.Text.WORDS)
        s.set(text, 4, NveTables.Text.TEXT, "Row A").set(text, 5, NveTables.Text.TEXT, "Row B").set(text, 9, NveTables.Text.TEXT, "Ghost")
        val scroll = s.table(NveTables.Scroll.KEY, NveTables.Scroll.WORDS)
        s.set(scroll, 3, NveTables.Scroll.AXES, NveTables.Values.ScrollAxes.VERTICAL)
        val image = s.table(NveTables.Image.KEY, NveTables.Image.WORDS)
        s.set(image, 6, NveTables.Image.SRC, 1)
        val canvas = s.table(dev.nve.host.generated.NveModuleTables.Canvas.KEY, dev.nve.host.generated.NveModuleTables.Canvas.WORDS)
        s.set(canvas, 7, dev.nve.host.generated.NveModuleTables.Canvas.DRAW_LIST, 2)
        val input = s.table(NveTables.TextInput.KEY, NveTables.TextInput.WORDS)
        s.set(input, 8, NveTables.TextInput.VALUE, value)
        val interaction = s.table(NveTables.Interaction.KEY, NveTables.Interaction.WORDS)
        s.set(interaction, 8, NveTables.Interaction.DISABLED, 1)
            .setEvents(interaction, 9, NveTables.Interaction.EVENTS_WORD, NveTables.Values.InteractionEventSet.PRESS)
        val visual = s.table(NveTables.Visual.KEY, NveTables.Visual.WORDS)
        s.set(visual, 9, NveTables.Visual.VISIBILITY, NveTables.Values.Visibility.HIDDEN).set(visual, 9, NveTables.Visual.OPACITY, 1f)
        val debug = s.table(NveTables.DebugId.KEY, NveTables.DebugId.WORDS)
        s.set(debug, 8, NveTables.DebugId.TEST_ID, "field").set(debug, 9, NveTables.DebugId.TEST_ID, "ghost")
        return s
    }

    @Test
    fun scrollImagesCanvasesDisabledAndHiddenNodes() {
        val host = FakeRuntimeHost(richScene())
        // 1x1 PNG as a base64 data URL, decoded by `ImageSource.bytes` at capture.
        val png = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="
        host.resources[1] = NveResource(dev.nve.host.ImageSource.KIND, imageSourceBytes(0, png.toByteArray()))
        host.resources[2] = NveResource(dev.nve.host.DrawList.KIND, java.nio.ByteBuffer.allocate(4))
        val controller = NveSurfaceController(host)
        controller.start(NveSource.App("/libnve_app.so"))
        compose.setContent { NveSurface(controller, Modifier.size(300.dp, 600.dp)) }

        compose.onNodeWithText("Row A").assertIsDisplayed()
        compose.onNodeWithText("Row B").assertExists()
        val image = controller.scene.at(0x1000006).image
        assertTrue("data URL decoded to inline bytes: $image", image is NveImage.Encoded)
        compose.onNodeWithTag("field").assertIsNotEnabled()
        assertEquals("hidden nodes leave the semantics tree", 0, compose.onAllNodesWithText("Ghost").fetchSemanticsNodes().size)
        assertEquals("and the tagged tree", 0, compose.onAllNodesWithTag("ghost").fetchSemanticsNodes().size)
    }

    @Test
    fun rejectedKeystrokesSnapBackAndANewEpochResetsFieldState() {
        val scene = TestScene()
            .node(2, 1, 0f, 0f, 300f, 400f)
            .node(3, 2, 10f, 10f, 200f, 40f)
        val input = scene.table(NveTables.TextInput.KEY, NveTables.TextInput.WORDS)
        scene.set(input, 3, NveTables.TextInput.VALUE, "fixed")
        val debug = scene.table(NveTables.DebugId.KEY, NveTables.DebugId.WORDS)
        scene.set(debug, 3, NveTables.DebugId.TEST_ID, "field")
        val host = FakeRuntimeHost(scene)
        val controller = NveSurfaceController(host)
        controller.start(NveSource.App("/libnve_app.so"))
        compose.setContent { NveSurface(controller, Modifier.size(300.dp, 400.dp)) }

        // The fake app rejects every keystroke (its value stays "fixed"): the field snaps back.
        compose.onNodeWithTag("field").performTextInput("!")
        compose.waitForIdle()
        compose.onNodeWithTag("field").assertTextEquals("fixed")
        assertTrue(host.rings.any { it.isEvent(NveAbi.TABLE_TEXT_INPUT, NveAbi.TEXT_INPUT_EVENT_CHANGE) })

        // A new epoch with the same node ids: fresh state, the new value shows.
        val next = TestScene(epoch = 1).node(2, 1, 0f, 0f, 300f, 400f).node(3, 2, 10f, 10f, 200f, 40f)
        next.set(next.table(NveTables.TextInput.KEY, NveTables.TextInput.WORDS), 3, NveTables.TextInput.VALUE, "other")
        next.set(next.table(NveTables.DebugId.KEY, NveTables.DebugId.WORDS), 3, NveTables.DebugId.TEST_ID, "field")
        host.scene = next
        compose.runOnIdle { controller.render() }
        compose.waitForIdle()
        compose.onNodeWithTag("field").assertTextEquals("other")
    }

    /** One text field (3) under the column (2); the app keeps `value`. */
    private fun fieldScene(value: String, frameDirty: IntArray? = null): TestScene {
        val s = TestScene().node(2, 1, 0f, 0f, 300f, 400f).node(3, 2, 10f, 10f, 200f, 40f)
        s.set(s.table(NveTables.TextInput.KEY, NveTables.TextInput.WORDS), 3, NveTables.TextInput.VALUE, value)
        s.set(s.table(NveTables.DebugId.KEY, NveTables.DebugId.WORDS), 3, NveTables.DebugId.TEST_ID, "field")
        s.dirty = frameDirty
        return s
    }

    @Test
    fun pairedFieldsKeepTypingAndIgnoreLateEchoesButAdoptAppRewrites() {
        val host = FakeRuntimeHost(fieldScene("a"))
        val controller = NveSurfaceController(host)
        controller.start(NveSource.Paired("ws://pair.example.test:9000/nve-wire"))
        compose.setContent { NveSurface(controller, Modifier.size(300.dp, 400.dp)) }

        // The remote echo is asynchronous: the fake re-delivers the old value, the field keeps "ab".
        compose.onNodeWithTag("field").performTextReplacement("ab")
        compose.waitForIdle()
        compose.onNodeWithTag("field").assertTextEquals("ab")
        compose.onNodeWithTag("field").performTextReplacement("abc")
        compose.waitForIdle()
        compose.onNodeWithTag("field").assertTextEquals("abc")
        // A late echo of "ab" (sent earlier) must not clobber the newer "abc".
        host.scene = fieldScene("ab", intArrayOf(3))
        compose.runOnIdle { controller.render() }
        compose.waitForIdle()
        compose.onNodeWithTag("field").assertTextEquals("abc")
        // A value the field never sent (the app rewrote it) replaces the text.
        host.scene = fieldScene("ABC", intArrayOf(3))
        compose.runOnIdle { controller.render() }
        compose.waitForIdle()
        compose.onNodeWithTag("field").assertTextEquals("ABC")
    }

    @Test
    fun visibleChildrenOfAHiddenContainerTakeNoInput() {
        // Hidden column(2) → visible pressable button(3) and a visible field(4).
        val s = TestScene().node(2, 1, 0f, 0f, 300f, 400f).node(3, 2, 0f, 0f, 120f, 44f).node(4, 2, 0f, 60f, 200f, 40f)
        val visual = s.table(NveTables.Visual.KEY, NveTables.Visual.WORDS)
        s.set(visual, 2, NveTables.Visual.VISIBILITY, NveTables.Values.Visibility.HIDDEN).set(visual, 2, NveTables.Visual.OPACITY, 1f)
        s.set(s.table(NveTables.Text.KEY, NveTables.Text.WORDS), 3, NveTables.Text.TEXT, "Go")
        s.setEvents(
            s.table(NveTables.Interaction.KEY, NveTables.Interaction.WORDS),
            3,
            NveTables.Interaction.EVENTS_WORD,
            NveTables.Values.InteractionEventSet.PRESS,
        )
        s.set(s.table(NveTables.TextInput.KEY, NveTables.TextInput.WORDS), 4, NveTables.TextInput.VALUE, "x")
        val debug = s.table(NveTables.DebugId.KEY, NveTables.DebugId.WORDS)
        s.set(debug, 3, NveTables.DebugId.TEST_ID, "go").set(debug, 4, NveTables.DebugId.TEST_ID, "field")
        val host = FakeRuntimeHost(s)
        val controller = NveSurfaceController(host)
        controller.start(NveSource.App("/libnve_app.so"))
        compose.setContent { NveSurface(controller, Modifier.size(300.dp, 400.dp)) }
        compose.waitForIdle()
        assertEquals("no semantics under a hidden ancestor", 0, compose.onAllNodesWithTag("go", useUnmergedTree = true).fetchSemanticsNodes().size)
        assertEquals(0, compose.onAllNodesWithTag("field", useUnmergedTree = true).fetchSemanticsNodes().size)
        assertTrue("no press / focus / text event", host.rings.none {
            it.isEvent(NveAbi.TABLE_INTERACTION, NveAbi.INTERACTION_EVENT_PRESS) ||
                it.isEvent(NveAbi.TABLE_INTERACTION, NveAbi.INTERACTION_EVENT_FOCUS) ||
                it.isEvent(NveAbi.TABLE_TEXT_INPUT, NveAbi.TEXT_INPUT_EVENT_CHANGE)
        })
    }

    @Test
    fun aBothAxesScrollReportsBothAxesTogether() {
        // Scroll container(3) scrolling BOTH at runtime offset (30, 45) over a 400x400 row(4).
        val s = TestScene().node(2, 1, 0f, 0f, 300f, 400f).node(3, 2, 0f, 0f, 200f, 200f).node(4, 3, 0f, 0f, 400f, 400f)
            .scroll(3, 30f, 45f)
        s.set(s.table(NveTables.Scroll.KEY, NveTables.Scroll.WORDS), 3, NveTables.Scroll.AXES, NveTables.Values.ScrollAxes.BOTH)
        s.set(s.table(NveTables.DebugId.KEY, NveTables.DebugId.WORDS), 4, NveTables.DebugId.TEST_ID, "content")
        val host = FakeRuntimeHost(s)
        val controller = NveSurfaceController(host)
        controller.start(NveSource.App("/libnve_app.so"))
        compose.setContent { NveSurface(controller, Modifier.size(300.dp, 400.dp)) }
        compose.waitForIdle()
        assertTrue("restoring the runtime offset sends nothing", host.scrolls.isEmpty())

        compose.onNodeWithTag("content").performTouchInput { swipeUp() }
        compose.waitForIdle()
        assertTrue("the vertical gesture reached the runtime", host.scrolls.isNotEmpty())
        assertTrue("no horizontal delta: ${host.scrolls}", host.scrolls.all { it.second == 0f })
        val scroll = host.rings.last { it.isEvent(NveAbi.TABLE_SCROLL, NveAbi.SCROLL_EVENT_SCROLL) }
        assertEquals("the untouched axis keeps its position (not 0)", 30f, scroll.payloadF32(0, 0), 0.5f)
        assertTrue("vertical position advanced: ${scroll.payloadF32(0, 1)}", scroll.payloadF32(0, 1) > 45f)
        val dySum = host.scrolls.sumOf { it.third.toDouble() }.toFloat()
        assertEquals("deltas add up to the absolute position", scroll.payloadF32(0, 1) - 45f, dySum, 0.5f)
    }

    @Test
    fun aClippingContainerStillFollowsItsRuntimeScrollOffset() {
        // ScrollAxes.NONE container(3) at runtime offset (10, 30) over a 400x400 row(4).
        val s = TestScene().node(2, 1, 0f, 0f, 300f, 400f).node(3, 2, 0f, 0f, 200f, 200f).node(4, 3, 0f, 0f, 400f, 400f)
            .scroll(3, 10f, 30f)
        s.set(s.table(NveTables.Scroll.KEY, NveTables.Scroll.WORDS), 3, NveTables.Scroll.AXES, NveTables.Values.ScrollAxes.NONE)
        s.set(s.table(NveTables.DebugId.KEY, NveTables.DebugId.WORDS), 4, NveTables.DebugId.TEST_ID, "content")
        val controller = NveSurfaceController(FakeRuntimeHost(s))
        controller.start(NveSource.App("/libnve_app.so"))
        compose.setContent { NveSurface(controller, Modifier.size(300.dp, 400.dp)) }
        compose.waitForIdle()
        val bounds = compose.onNodeWithTag("content").getUnclippedBoundsInRoot()
        assertEquals(-10f, bounds.left.value, 0.5f)
        assertEquals(-30f, bounds.top.value, 0.5f)
    }

    @Test
    fun aFrameRecomposesOnlyTheNodesItChanged() {
        val s = TestScene().node(2, 1, 0f, 0f, 300f, 400f).node(3, 2, 0f, 0f, 200f, 30f).node(4, 2, 0f, 40f, 200f, 30f)
            .node(5, 2, 0f, 80f, 200f, 30f)
        val text = s.table(NveTables.Text.KEY, NveTables.Text.WORDS)
        s.set(text, 3, NveTables.Text.TEXT, "one").set(text, 4, NveTables.Text.TEXT, "two").set(text, 5, NveTables.Text.TEXT, "three")
        val host = FakeRuntimeHost(s)
        val controller = NveSurfaceController(host)
        controller.start(NveSource.App("/libnve_app.so"))
        val counts = HashMap<Int, Int>()
        compose.setContent {
            CompositionLocalProvider(LocalNodeCompositions provides { id: Int -> counts[id] = (counts[id] ?: 0) + 1 }) {
                NveSurface(controller, Modifier.size(300.dp, 400.dp))
            }
        }
        compose.waitForIdle()
        val before = HashMap(counts)
        assertEquals(setOf(0x1000002, 0x1000003, 0x1000004, 0x1000005), before.keys)

        // Next frame: only node 4's text changed.
        s.set(text, 4, NveTables.Text.TEXT, "TWO")
        s.dirty = intArrayOf(4)
        compose.runOnIdle { controller.render() }
        compose.waitForIdle()
        compose.onNodeWithText("TWO").assertIsDisplayed()
        val delta = counts.mapValues { (id, n) -> n - (before[id] ?: 0) }.filterValues { it != 0 }
        assertEquals("only the changed node recomposed: $delta", mapOf(0x1000004 to 1), delta)

        // A frame that changes nothing recomposes nothing.
        val settled = HashMap(counts)
        s.dirty = intArrayOf()
        compose.runOnIdle { controller.render() }
        compose.waitForIdle()
        assertEquals(settled, HashMap(counts))
    }

    @Test
    fun scrollContainerDoesNotRecomposeWhenChildChangesTextOnly() {
        val s = TestScene()
            .node(2, 1, 0f, 0f, 300f, 400f)
            .node(3, 2, 0f, 0f, 300f, 100f)
            .node(4, 3, 0f, 0f, 300f, 180f)
        val scroll = s.table(NveTables.Scroll.KEY, NveTables.Scroll.WORDS)
        s.set(scroll, 3, NveTables.Scroll.AXES, NveTables.Values.ScrollAxes.VERTICAL)
        val text = s.table(NveTables.Text.KEY, NveTables.Text.WORDS)
        s.set(text, 4, NveTables.Text.TEXT, "initial")
        val host = FakeRuntimeHost(s)
        val controller = NveSurfaceController(host)
        controller.start(NveSource.App("/libnve_app.so"))
        val counts = HashMap<Int, Int>()
        compose.setContent {
            CompositionLocalProvider(LocalNodeCompositions provides { id: Int -> counts[id] = (counts[id] ?: 0) + 1 }) {
                NveSurface(controller, Modifier.size(300.dp, 400.dp))
            }
        }
        compose.waitForIdle()
        val before = HashMap(counts)

        // Child 4 updates text without changing geometry: scroll container 3 must NOT recompose.
        s.set(text, 4, NveTables.Text.TEXT, "updated")
        s.dirty = intArrayOf(4)
        compose.runOnIdle { controller.render() }
        compose.waitForIdle()
        compose.onNodeWithText("updated").assertIsDisplayed()
        val delta = counts.mapValues { (id, n) -> n - (before[id] ?: 0) }.filterValues { it != 0 }
        assertEquals("only the child recomposed, not its scroll parent: $delta", mapOf(0x1000004 to 1), delta)
    }

    @Test
    fun pairedFieldCapsInFlightEchoQueueAt32Entries() {
        val host = FakeRuntimeHost(fieldScene("v0"))
        val controller = NveSurfaceController(host)
        controller.start(NveSource.Paired("ws://pair.example.test:9000/nve-wire"))
        compose.setContent { NveSurface(controller, Modifier.size(300.dp, 400.dp)) }

        // Cycle 1: type 35 distinct values without echoes (v1..v3 evicted; v4..v35 stay in-flight).
        for (k in 1..35) {
            compose.onNodeWithTag("field").performTextReplacement("v$k")
        }
        compose.waitForIdle()
        compose.onNodeWithTag("field").assertTextEquals("v35")

        // Deliver v3 FIRST: because v3 was evicted at the 32-entry cap, it is treated as an app
        // rewrite and resets the field to "v3" (an unbounded queue would ignore v3 and keep "v35").
        host.scene = fieldScene("v3", intArrayOf(3))
        compose.runOnIdle { controller.render() }
        compose.waitForIdle()
        compose.onNodeWithTag("field").assertTextEquals("v3")

        // Cycle 2: in a fresh queue (cleared by the app rewrite above), type v1..v35 again and
        // deliver v4 FIRST — the 32nd entry from the end stays retained, so the field keeps "v35".
        for (k in 1..35) {
            compose.onNodeWithTag("field").performTextReplacement("v$k")
        }
        compose.waitForIdle()
        compose.onNodeWithTag("field").assertTextEquals("v35")

        host.scene = fieldScene("v4", intArrayOf(3))
        compose.runOnIdle { controller.render() }
        compose.waitForIdle()
        compose.onNodeWithTag("field").assertTextEquals("v35")
    }
}
