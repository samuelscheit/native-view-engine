package dev.nve.compose

import androidx.compose.foundation.layout.size
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
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
        s.set(interaction, 5, NveTables.Interaction.EVENT_MASK, NveTables.Values.EventMask.PRESS)
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
            "mounting a text field sends no focus / blur: ${host.rings.map { it.get(20).toInt() }}",
            host.rings.none { it.get(20).toInt() == NveAbi.EVENT_BLUR || it.get(20).toInt() == NveAbi.EVENT_FOCUS },
        )
        compose.onNodeWithTag("submit-btn").performClick()
        val press = host.rings.last()
        assertEquals(NveAbi.EVENT_PRESS, press.get(20).toInt())
        assertEquals(0x1000005, press.getInt(8))

        compose.onNodeWithText("draft").performTextInput("!")
        assertTrue("focusing by input is a real focus event", host.rings.any { it.get(20).toInt() == NveAbi.EVENT_FOCUS })
        val text = host.rings.last { it.get(20).toInt() == NveAbi.EVENT_TEXT_CHANGE }
        assertEquals(0x1000004, text.getInt(8))

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
            .set(interaction, 9, NveTables.Interaction.EVENT_MASK, NveTables.Values.EventMask.PRESS)
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
        host.resources[2] = NveResource(dev.nve.host.Key64.of("nve.res.draw_list"), java.nio.ByteBuffer.allocate(0))
        val controller = NveSurfaceController(host)
        controller.start(NveSource.App("/libnve_app.so"))
        compose.setContent { NveSurface(controller, Modifier.size(300.dp, 600.dp)) }

        compose.onNodeWithText("Row A").assertIsDisplayed()
        compose.onNodeWithText("Row B").assertExists()
        val image = controller.scene.nodes.getValue(0x1000006).image
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
        assertTrue(host.rings.any { it.get(20).toInt() == NveAbi.EVENT_TEXT_CHANGE })

        // A new epoch with the same node ids: fresh state, the new value shows.
        val next = TestScene(epoch = 1).node(2, 1, 0f, 0f, 300f, 400f).node(3, 2, 10f, 10f, 200f, 40f)
        next.set(next.table(NveTables.TextInput.KEY, NveTables.TextInput.WORDS), 3, NveTables.TextInput.VALUE, "other")
        next.set(next.table(NveTables.DebugId.KEY, NveTables.DebugId.WORDS), 3, NveTables.DebugId.TEST_ID, "field")
        host.scene = next
        compose.runOnIdle { controller.render() }
        compose.waitForIdle()
        compose.onNodeWithTag("field").assertTextEquals("other")
    }
}
