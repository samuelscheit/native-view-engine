package dev.nve.compose

import android.graphics.Bitmap
import android.graphics.Canvas as AndroidCanvas
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Canvas as ComposeCanvas
import androidx.compose.ui.graphics.drawscope.CanvasDrawScope
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.node.ModifierNodeElement
import androidx.compose.ui.node.SemanticsModifierNode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.SemanticsConfiguration
import androidx.compose.ui.semantics.SemanticsModifier
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.LayoutDirection
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * Robolectric tests verifying real Compose `DrawScope` painting of all 8 `NVE_DRAW1` opcodes,
 * `Modifier.semantics` accessibility nodes, and `NveSceneCanvas` (`SkiaCanvas` + `Image` + `TextInput`).
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class NveSceneCanvasRobolectricTest {

    companion object {
        // Minimal valid 1x1 red PNG as base64 data URI.
        private const val RED_PNG_DATA_URI =
            "data:image/png;base64," +
                "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="
    }

    private fun extractSemantics(modifier: Modifier): SemanticsConfiguration {
        val merged = SemanticsConfiguration()
        modifier.foldIn(Unit) { _, element ->
            if (element is ModifierNodeElement<*>) {
                val node = element.create()
                if (node is SemanticsModifierNode) {
                    with(node) { merged.applySemantics() }
                }
            } else if (element is SemanticsModifier) {
                for ((key, value) in element.semanticsConfiguration) {
                    @Suppress("UNCHECKED_CAST")
                    merged[key as androidx.compose.ui.semantics.SemanticsPropertyKey<Any?>] = value
                }
            }
        }
        return merged
    }

    @Test
    fun drawIntoComposeDrawScopeRendersAll8OpcodesAndRejectsCorruptBuffers() {
        val bitmap = Bitmap.createBitmap(80, 60, Bitmap.Config.ARGB_8888)
        val androidCanvas = AndroidCanvas(bitmap)
        val composeCanvas = ComposeCanvas(androidCanvas)
        val drawScope = CanvasDrawScope()

        val validBytes = TestDrawListBuilder.buildAll8Opcodes()
        var ok = false
        drawScope.draw(
            density = Density(1f),
            layoutDirection = LayoutDirection.Ltr,
            canvas = composeCanvas,
            size = Size(80f, 60f),
        ) {
            ok = NveDrawListRenderer.drawInto(this, validBytes, scale = 1f)
        }
        assertTrue(ok)

        // Corrupt buffer must return false safely and never throw.
        var corruptOk = true
        drawScope.draw(
            density = Density(1f),
            layoutDirection = LayoutDirection.Ltr,
            canvas = composeCanvas,
            size = Size(80f, 60f),
        ) {
            corruptOk = NveDrawListRenderer.drawInto(this, byteArrayOf(1, 2, 3, 4), scale = 1f)
        }
        assertFalse(corruptOk)
        bitmap.recycle()
    }

    @Test
    fun nveSemanticsExposesContentDescriptionRoleDisabledStateDescriptionAndTestTag() {
        val buttonNode = NveRenderNodeSnapshot(
            nodeId = 10,
            kind = NveSceneBuffer.KIND_PRESSABLE,
            flags = NveSceneBuffer.FLAG_INTERACTIVE,
            opacityU8 = 255,
            x = 0f,
            y = 0f,
            width = 120f,
            height = 44f,
            bgRgba = 0,
            textRgba = 0,
            borderRgba = 0,
            borderRadius = 8f,
            borderWidth = 0f,
            fontSize = 14f,
            text = "",
            childIds = IntArray(0),
            zIndex = 0,
            metadata = NveNodeMetadata(
                accessibilityLabel = "Save workspace",
                accessibilityRole = "button",
                accessibilityStateDescription = "Unsaved changes",
                disabled = true,
                testId = "save-btn",
            ),
        )

        val config = extractSemantics(Modifier.nveSemantics(buttonNode))
        assertEquals(listOf("Save workspace"), config[SemanticsProperties.ContentDescription])
        assertEquals(Role.Button, config[SemanticsProperties.Role])
        assertTrue(SemanticsProperties.Disabled in config)
        assertEquals("Unsaved changes", config[SemanticsProperties.StateDescription])
        assertEquals("save-btn", config[SemanticsProperties.TestTag])

        val checkboxNode = buttonNode.copy(
            nodeId = 11,
            kind = NveSceneBuffer.KIND_VIEW,
            metadata = NveNodeMetadata(
                accessibilityLabel = "Enable telemetry",
                accessibilityRole = "checkbox",
                accessibilityHint = "Toggles metrics",
                disabled = false,
            ),
        )
        val checkboxConfig = extractSemantics(Modifier.nveSemantics(checkboxNode))
        assertEquals(Role.Checkbox, checkboxConfig[SemanticsProperties.Role])
        assertFalse(SemanticsProperties.Disabled in checkboxConfig)
        assertEquals("Toggles metrics", checkboxConfig[SemanticsProperties.StateDescription])

        val imageNode = buttonNode.copy(
            nodeId = 12,
            kind = NveSceneBuffer.KIND_IMAGE,
            metadata = NveNodeMetadata(accessibilityLabel = "Studio avatar"),
        )
        val imageConfig = extractSemantics(Modifier.nveSemantics(imageNode))
        assertEquals(Role.Image, imageConfig[SemanticsProperties.Role])
        assertEquals(listOf("Studio avatar"), imageConfig[SemanticsProperties.ContentDescription])
    }

    @Test
    fun sceneCanvasRendersSkiaCanvasImageAndTextInputWithResizeModes() {
        assertEquals(ContentScale.Crop, contentScaleForResizeMode(NveNodeMetadata.RESIZE_MODE_COVER))
        assertEquals(ContentScale.Fit, contentScaleForResizeMode(NveNodeMetadata.RESIZE_MODE_CONTAIN))
        assertEquals(ContentScale.FillBounds, contentScaleForResizeMode(NveNodeMetadata.RESIZE_MODE_STRETCH))
        assertEquals(ContentScale.None, contentScaleForResizeMode(NveNodeMetadata.RESIZE_MODE_CENTER))

        // Valid data URI decodes; malformed URI returns null safely.
        val decodedBitmap = decodeDataUriImageBitmap(RED_PNG_DATA_URI)
        assertNotNull(decodedBitmap)
        assertNull(decodeDataUriImageBitmap("data:image/png;base64,!!!invalid!!!"))
        assertNull(decodeDataUriImageBitmap("https://example.com/unsupported.png"))

        val drawBytes = TestDrawListBuilder.buildAll8Opcodes()
        val nodes = linkedMapOf(
            1 to NveRenderNodeSnapshot(
                nodeId = 1,
                kind = NveSceneBuffer.KIND_ROOT,
                flags = 0,
                opacityU8 = 255,
                x = 0f,
                y = 0f,
                width = 360f,
                height = 640f,
                bgRgba = TestDrawListBuilder.rgba(15, 23, 42, 255),
                textRgba = 0,
                borderRgba = 0,
                borderRadius = 0f,
                borderWidth = 0f,
                fontSize = 14f,
                text = "",
                childIds = intArrayOf(2, 3, 4),
                zIndex = 0,
            ),
            2 to NveRenderNodeSnapshot(
                nodeId = 2,
                kind = NveSceneBuffer.KIND_SKIA_CANVAS,
                flags = 0,
                opacityU8 = 255,
                x = 12f,
                y = 12f,
                width = 160f,
                height = 64f,
                bgRgba = 0,
                textRgba = 0,
                borderRgba = 0,
                borderRadius = 8f,
                borderWidth = 0f,
                fontSize = 14f,
                text = "",
                childIds = IntArray(0),
                zIndex = 1,
                metadata = NveNodeMetadata(
                    accessibilityLabel = "Frame latency chart",
                    accessibilityRole = "image",
                ),
                drawListBytes = drawBytes,
            ),
            3 to NveRenderNodeSnapshot(
                nodeId = 3,
                kind = NveSceneBuffer.KIND_IMAGE,
                flags = 0,
                opacityU8 = 255,
                x = 12f,
                y = 88f,
                width = 48f,
                height = 48f,
                bgRgba = 0,
                textRgba = 0,
                borderRgba = 0,
                borderRadius = 4f,
                borderWidth = 0f,
                fontSize = 14f,
                text = RED_PNG_DATA_URI,
                childIds = IntArray(0),
                zIndex = 1,
                metadata = NveNodeMetadata(
                    accessibilityLabel = "Avatar",
                    resizeMode = NveNodeMetadata.RESIZE_MODE_CONTAIN,
                ),
            ),
            4 to NveRenderNodeSnapshot(
                nodeId = 4,
                kind = NveSceneBuffer.KIND_TEXT_INPUT,
                flags = NveSceneBuffer.FLAG_INTERACTIVE,
                opacityU8 = 255,
                x = 12f,
                y = 148f,
                width = 240f,
                height = 44f,
                bgRgba = TestDrawListBuilder.rgba(30, 41, 59, 255),
                textRgba = TestDrawListBuilder.rgba(248, 250, 252, 255),
                borderRgba = TestDrawListBuilder.rgba(71, 85, 105, 255),
                borderRadius = 6f,
                borderWidth = 1f,
                fontSize = 15f,
                text = "",
                childIds = IntArray(0),
                zIndex = 2,
                metadata = NveNodeMetadata(
                    accessibilityLabel = "API token",
                    placeholder = "Paste secret token",
                    secureTextEntry = true,
                    disabled = false,
                ),
            ),
        )

        assertNotNull(resolveCanvasDrawListBytes(nodes[2]!!))

        val activityController = Robolectric.buildActivity(ComponentActivity::class.java)
        val activity = activityController.create().start().resume().get()
        activity.setContent {
            NveSceneCanvas(
                nodes = nodes,
                imageLoader = { decodedBitmap },
            )
        }
        assertNotNull(activity.window.decorView)
        activityController.pause().stop().destroy()
    }
}
