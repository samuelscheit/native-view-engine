package dev.nve.compose

import androidx.compose.foundation.layout.size
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.unit.dp
import dev.nve.android.generated.NveCoreAbi
import dev.nve.android.generated.NveTables
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
        s.set(interaction, 5, NveTables.Interaction.EVENT_MASK, 1 shl (NveCoreAbi.EVENT_PRESS - 1))
        val a11y = s.table(NveTables.Accessibility.KEY, NveTables.Accessibility.WORDS)
        s.set(a11y, 5, NveTables.Accessibility.ROLE, NveTables.Accessibility.ROLE_BUTTON).set(a11y, 5, NveTables.Accessibility.LABEL, "Submit form")
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

        compose.onNodeWithTag("submit-btn").performClick()
        val press = host.rings.last()
        assertEquals(NveCoreAbi.EVENT_PRESS, press.get(20).toInt())
        assertEquals(0x1000005, press.getInt(8))

        compose.onNodeWithText("draft").performTextInput("!")
        val text = host.rings.last { it.get(20).toInt() == NveCoreAbi.EVENT_TEXT_CHANGE }
        assertEquals(0x1000004, text.getInt(8))

        val measurer = host.measurer
        assertTrue("Compose text measurer installed", measurer is NveComposeTextMeasurer)
        val out = FloatArray(2)
        assertTrue(measurer!!.measure("Hello NVE", 16f, 400, Float.POSITIVE_INFINITY, out))
        assertTrue("measured ${out.toList()}", out[0] > 0f && out[1] > 0f)
        val narrow = FloatArray(2)
        measurer.measure("Hello NVE", 16f, 400, 0f, narrow)
        assertTrue("min-content wraps: ${narrow.toList()} vs ${out.toList()}", narrow[0] < out[0] && narrow[1] > out[1])
    }
}
