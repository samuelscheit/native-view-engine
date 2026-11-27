package dev.nve.compose

import dev.nve.android.generated.NveCoreAbi
import dev.nve.android.generated.NveTables
import dev.nve.host.ImageSource
import dev.nve.host.Key64
import dev.nve.host.NativeRuntime
import org.junit.jupiter.api.Assertions.assertArrayEquals
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Assertions.assertNull
import org.junit.jupiter.api.Assertions.assertSame
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import java.nio.ByteBuffer

class NveSceneCaptureTest {
    private val drawKind = Key64.of("nve.res.draw_list")
    private val press = 1 shl (NveCoreAbi.EVENT_PRESS - 1)

    /** Root(1) → column(2) → [title(3), input(4), image(5), scroll(6) → [row(7)], canvas(8), button(9)]. */
    private fun showcase(scene: TestScene = TestScene()): TestScene {
        scene.node(2, 1, 5f, 5f, 300f, 400f)
            .node(3, 2, 10f, 20f, 90f, 24f)
            .node(4, 2, 10f, 50f, 200f, 40f)
            .node(5, 2, 10f, 100f, 64f, 64f)
            .node(6, 2, 10f, 170f, 200f, 80f)
            .node(7, 6, 10f, 170f, 200f, 150f)
            .node(8, 2, 10f, 260f, 120f, 40f)
            .node(9, 2, 10f, 310f, 120f, 44f)
        val visual = scene.table(NveTables.Visual.KEY, NveTables.Visual.WORDS)
        scene.set(visual, 1, NveTables.Visual.BACKGROUND, 0xFF2A170F.toInt())
        scene.set(visual, 2, NveTables.Visual.BACKGROUND, 0xFF0000FF.toInt())
            .set(visual, 2, NveTables.Visual.BORDER_COLOR, 0x80FFFFFF.toInt())
            .set(visual, 2, NveTables.Visual.BORDER_WIDTH, 2f)
            .set(visual, 2, NveTables.Visual.BORDER_RADIUS, 12f)
            .set(visual, 2, NveTables.Visual.OPACITY, 0.5f)
            .set(visual, 2, NveTables.Visual.Z_INDEX, -3)
        scene.set(visual, 8, NveTables.Visual.OPACITY, 1f)
            .set(visual, 8, NveTables.Visual.VISIBILITY, NveTables.Visual.VISIBILITY_HIDDEN)
        val typo = scene.table(NveTables.Typography.KEY, NveTables.Typography.WORDS)
        scene.set(typo, 2, NveTables.Typography.SET, 0b11)  // color + size on the column
            .set(typo, 2, NveTables.Typography.COLOR, 0xFF00FF00.toInt())
            .set(typo, 2, NveTables.Typography.FONT_SIZE, 20f)
            .set(typo, 3, NveTables.Typography.SET, (1 shl 6) or (1 shl 1))  // weight + size override
            .set(typo, 3, NveTables.Typography.FONT_WEIGHT, 700)
            .set(typo, 3, NveTables.Typography.FONT_SIZE, 24f)
        val text = scene.table(NveTables.Text.KEY, NveTables.Text.WORDS)
        scene.set(text, 3, NveTables.Text.TEXT, "Hello \uD83D\uDE80").set(text, 7, NveTables.Text.TEXT, "Row").set(text, 9, NveTables.Text.TEXT, "Submit")
        val input = scene.table(NveTables.TextInput.KEY, NveTables.TextInput.WORDS)
        scene.set(input, 4, NveTables.TextInput.VALUE, "draft").set(input, 4, NveTables.TextInput.PLACEHOLDER, "Type")
            .set(input, 4, NveTables.TextInput.SECURE, 1).set(input, 4, NveTables.TextInput.INPUT_KIND, NveTables.TextInput.INPUT_KIND_EMAIL)
        val image = scene.table(NveTables.Image.KEY, NveTables.Image.WORDS)
        scene.set(image, 5, NveTables.Image.SRC, 1).set(image, 5, NveTables.Image.FIT, NveTables.Image.FIT_CONTAIN)
        val scroll = scene.table(NveTables.Scroll.KEY, NveTables.Scroll.WORDS)
        scene.set(scroll, 6, NveTables.Scroll.AXES, NveTables.Scroll.AXES_HORIZONTAL)
        val canvas = scene.table(NveTables.Canvas.KEY, NveTables.Canvas.WORDS)
        scene.set(canvas, 8, NveTables.Canvas.DRAW_LIST, 2)
        val interaction = scene.table(NveTables.Interaction.KEY, NveTables.Interaction.WORDS)
        scene.set(interaction, 9, NveTables.Interaction.EVENT_MASK, press).set(interaction, 3, NveTables.Interaction.EVENT_MASK, press)
            .set(interaction, 3, NveTables.Interaction.DISABLED, 1)
        val a11y = scene.table(NveTables.Accessibility.KEY, NveTables.Accessibility.WORDS)
        scene.set(a11y, 9, NveTables.Accessibility.ROLE, NveTables.Accessibility.ROLE_BUTTON).set(a11y, 9, NveTables.Accessibility.LABEL, "Submit form")
            .set(a11y, 7, NveTables.Accessibility.HIDDEN, 1)
        val debug = scene.table(NveTables.DebugId.KEY, NveTables.DebugId.WORDS)
        scene.set(debug, 9, NveTables.DebugId.TEST_ID, "submit-btn")
        return scene
    }

    private class Resources(var image: ByteBuffer, var draw: ByteBuffer, var drawFlags: Long = 0L) : NveResourceLookup {
        val calls = ArrayList<Int>()

        override fun resource(id: Int): NveResource? {
            calls.add(id)
            return when (id) {
                1 -> NveResource(ImageSource.KIND, 0L, image)
                2 -> NveResource(Key64.of("nve.res.draw_list"), drawFlags, draw)
                else -> null
            }
        }
    }

    @Test
    fun capturesKindsGeometryStylesAndSemantics() {
        val res = Resources(imageSourceBytes(0, "asset://logo.png".toByteArray()), ByteBuffer.wrap(byteArrayOf(1, 2, 3)))
        val snap = NveSceneCapture.capture(showcase().reader(), res, NveSceneSnapshot.EMPTY)
        assertEquals(0xFF2A170F.toInt(), snap.rootBox.background)
        assertEquals(listOf(0x1000002), snap.rootChildren)
        val column = snap.nodes.getValue(0x1000002)
        assertEquals(NveNodeKind.CONTAINER, column.kind)
        assertEquals(NveBox(0xFF0000FF.toInt(), 0x80FFFFFF.toInt(), 2f, 12f, 0.5f, -3, true), column.box)
        assertEquals(listOf(3, 4, 5, 6, 8, 9).map { 0x1000000 or it }, column.children)
        val title = snap.nodes.getValue(0x1000003)
        assertEquals(NveNodeKind.TEXT, title.kind)
        assertEquals("Hello \uD83D\uDE80", title.text)
        assertEquals(5f to 15f, title.x to title.y)  // relative to the column at (5, 5)
        assertEquals(NveTextStyle(color = 0xFF00FF00.toInt(), fontSize = 24f, fontWeight = 700), title.textStyle)
        assertFalse(title.pressable, "disabled interaction is not pressable")
        assertTrue(title.semantics.disabled)
        val input = snap.nodes.getValue(0x1000004)
        assertEquals(NveInput("draft", "Type", secure = true, multiline = false, readOnly = false, inputKind = NveTables.TextInput.INPUT_KIND_EMAIL), input.input)
        assertEquals(20f, input.textStyle.fontSize, "inherits the column's size")
        val image = snap.nodes.getValue(0x1000005)
        assertEquals(NveImage.Url("asset://logo.png"), image.image)
        assertEquals(NveTables.Image.FIT_CONTAIN, image.imageFit)
        val scroll = snap.nodes.getValue(0x1000006)
        assertEquals(NveNodeKind.SCROLL, scroll.kind)
        assertTrue(scroll.scrollHorizontal)
        val row = snap.nodes.getValue(0x1000007)
        assertEquals(0f to 0f, row.x to row.y, "scroll content is relative to the scroll container")
        assertTrue(row.semantics.hidden)
        val canvas = snap.nodes.getValue(0x1000008)
        assertArrayEquals(byteArrayOf(1, 2, 3), canvas.drawList)
        assertFalse(canvas.box.visible)
        val button = snap.nodes.getValue(0x1000009)
        assertTrue(button.pressable)
        assertEquals(NveSemantics(NveTables.Accessibility.ROLE_BUTTON, "Submit form", "", "submit-btn"), button.semantics)
        assertEquals(NveNodeKind.TEXT, button.kind)
    }

    @Test
    fun resourcesAreCopiedOutOfTheFrameAndReusedWhileUnchanged() {
        val draw = ByteBuffer.allocateDirect(3).put(byteArrayOf(4, 5, 6)).flip() as ByteBuffer
        val res = Resources(imageSourceBytes(1, byteArrayOf(0x89.toByte(), 'P'.code.toByte())), draw)
        val first = NveSceneCapture.capture(showcase().reader(), res, NveSceneSnapshot.EMPTY)
        val encoded = first.nodes.getValue(0x1000005).image as NveImage.Encoded
        assertArrayEquals(byteArrayOf(0x89.toByte(), 'P'.code.toByte()), encoded.bytes)
        draw.put(0, 9)  // the frame's buffer is reused by the next frame: the snapshot must not alias it
        assertArrayEquals(byteArrayOf(4, 5, 6), first.nodes.getValue(0x1000008).drawList)

        res.calls.clear()
        val second = NveSceneCapture.capture(showcase().reader(), res, first)
        assertSame(encoded, second.nodes.getValue(0x1000005).image, "same image id reuses the decoded copy")
        assertSame(first.nodes.getValue(0x1000008).drawList, second.nodes.getValue(0x1000008).drawList)
        assertFalse(1 in res.calls, "unchanged image resource is not re-read")

        res.drawFlags = NativeRuntime.RESOURCE_FLAG_EPHEMERAL
        val third = NveSceneCapture.capture(showcase().reader(), res, second)
        assertArrayEquals(byteArrayOf(9, 5, 6), third.nodes.getValue(0x1000008).drawList, "ephemeral draw lists re-read every frame")
    }

    @Test
    fun brokenOrForeignResourcesRenderEmpty() {
        val res = Resources(imageSourceBytes(7, byteArrayOf(1)), ByteBuffer.allocate(0))
        val snap = NveSceneCapture.capture(showcase().reader(), res, NveSceneSnapshot.EMPTY)
        assertNull(snap.nodes.getValue(0x1000005).image, "unknown image tag")
        val noLookup = NveSceneCapture.capture(showcase().reader(), { null }, NveSceneSnapshot.EMPTY)
        assertNull(noLookup.nodes.getValue(0x1000005).image)
        assertNull(noLookup.nodes.getValue(0x1000008).drawList)
    }

    @Test
    fun removedNodesLeaveTheSnapshot() {
        val res = Resources(imageSourceBytes(0, "x".toByteArray()), ByteBuffer.allocate(0))
        val first = NveSceneCapture.capture(showcase().reader(), res, NveSceneSnapshot.EMPTY)
        val smaller = TestScene().node(2, 1, 0f, 0f, 10f, 10f)
        val second = NveSceneCapture.capture(smaller.reader(), res, first)
        assertEquals(setOf(0x1000002), second.nodes.keys)
        assertEquals(1L, second.frameId)
    }

    @Test
    fun drawListKindIsNotAnImage() {
        assertFalse(drawKind == ImageSource.KIND)
    }
}
