package dev.nve.compose

import dev.nve.host.ImageSource
import dev.nve.host.Key64
import dev.nve.host.SceneSemantics
import dev.nve.host.generated.NveModuleTables
import dev.nve.host.generated.NveTables
import org.junit.jupiter.api.Assertions.assertArrayEquals
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Assertions.assertNotSame
import org.junit.jupiter.api.Assertions.assertNull
import org.junit.jupiter.api.Assertions.assertSame
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import java.nio.ByteBuffer

class NveSceneCaptureTest {
    private val drawKind = Key64.of("nve.res.draw_list")
    private val press = NveTables.Values.EventMask.PRESS

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
            .set(visual, 8, NveTables.Visual.VISIBILITY, NveTables.Values.Visibility.HIDDEN)
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
            .set(input, 4, NveTables.TextInput.SECURE, 1).set(input, 4, NveTables.TextInput.INPUT_KIND, NveTables.Values.InputKind.EMAIL)
        val image = scene.table(NveTables.Image.KEY, NveTables.Image.WORDS)
        scene.set(image, 5, NveTables.Image.SRC, 1).set(image, 5, NveTables.Image.FIT, NveTables.Values.ImageFit.CONTAIN)
        val scroll = scene.table(NveTables.Scroll.KEY, NveTables.Scroll.WORDS)
        scene.set(scroll, 6, NveTables.Scroll.AXES, NveTables.Values.ScrollAxes.HORIZONTAL)
        val canvas = scene.table(NveModuleTables.Canvas.KEY, NveModuleTables.Canvas.WORDS)
        scene.set(canvas, 8, NveModuleTables.Canvas.DRAW_LIST, 2)
        val interaction = scene.table(NveTables.Interaction.KEY, NveTables.Interaction.WORDS)
        scene.set(interaction, 9, NveTables.Interaction.EVENT_MASK, press).set(interaction, 3, NveTables.Interaction.EVENT_MASK, press)
            .set(interaction, 3, NveTables.Interaction.DISABLED, 1)
        val a11y = scene.table(NveTables.Accessibility.KEY, NveTables.Accessibility.WORDS)
        scene.set(a11y, 9, NveTables.Accessibility.ROLE, NveTables.Values.AriaRole.BUTTON).set(a11y, 9, NveTables.Accessibility.LABEL, "Submit form")
            .set(a11y, 7, NveTables.Accessibility.HIDDEN, 1)
        val debug = scene.table(NveTables.DebugId.KEY, NveTables.DebugId.WORDS)
        scene.set(debug, 9, NveTables.DebugId.TEST_ID, "submit-btn")
        return scene
    }

    private class Resources(var image: ByteBuffer, var draw: ByteBuffer) : NveResourceLookup {
        val calls = ArrayList<Int>()

        override fun resource(id: Int): NveResource? {
            calls.add(id)
            return when (id) {
                1 -> NveResource(ImageSource.KIND, image)
                2 -> NveResource(Key64.of("nve.res.draw_list"), draw)
                else -> null
            }
        }
    }

    @Test
    fun capturesKindsGeometryStylesAndSemantics() {
        val res = Resources(imageSourceBytes(0, "asset://logo.png".toByteArray()), ByteBuffer.wrap(byteArrayOf(1, 2, 3)))
        val snap = capture(showcase(), res)
        assertEquals(0xFF2A170F.toInt(), snap.rootBox.background)
        assertEquals(listOf(0x1000002), snap.rootChildren)
        val column = snap.at(0x1000002)
        assertEquals(NveNodeKind.CONTAINER, column.kind)
        assertEquals(NveBox(0xFF0000FF.toInt(), 0x80FFFFFF.toInt(), 2f, 12f, 0.5f, -3, true), column.box)
        assertEquals(listOf(3, 4, 5, 6, 8, 9).map { 0x1000000 or it }, column.children)
        val title = snap.at(0x1000003)
        assertEquals(NveNodeKind.TEXT, title.kind)
        assertEquals("Hello \uD83D\uDE80", title.text)
        assertEquals(5f to 15f, title.x to title.y)  // relative to the column at (5, 5)
        assertEquals(NveTextStyle(color = 0xFF00FF00.toInt(), fontSize = 24f, fontWeight = 700), title.textStyle)
        assertEquals(SceneSemantics.DEFAULT_FONT_WEIGHT, snap.at(0x1000009).textStyle.fontWeight, "siblings do not inherit overrides")
        assertFalse(title.pressable, "disabled interaction is not pressable")
        assertTrue(title.semantics.disabled)
        val input = snap.at(0x1000004)
        assertEquals(NveInput("draft", "Type", secure = true, multiline = false, readOnly = false, inputKind = NveTables.Values.InputKind.EMAIL), input.input)
        assertEquals(20f, input.textStyle.fontSize, "inherits the column's size")
        val image = snap.at(0x1000005)
        assertEquals(NveImage.Url("asset://logo.png"), image.image)
        assertEquals(NveTables.Values.ImageFit.CONTAIN, image.imageFit)
        val scroll = snap.at(0x1000006)
        assertEquals(NveNodeKind.SCROLL, scroll.kind)
        assertEquals(NveTables.Values.ScrollAxes.HORIZONTAL, scroll.scrollAxes)
        assertTrue(scroll.scrollsHorizontally && !scroll.scrollsVertically)
        assertEquals(200f to 150f, scroll.scrollContentWidth to scroll.scrollContentHeight)
        val row = snap.at(0x1000007)
        assertEquals(0f to 0f, row.x to row.y, "scroll content is relative to the scroll container")
        assertTrue(row.semantics.hidden)
        val canvas = snap.at(0x1000008)
        assertArrayEquals(byteArrayOf(1, 2, 3), canvas.drawList)
        assertFalse(canvas.box.visible)
        val button = snap.at(0x1000009)
        assertTrue(button.pressable)
        assertEquals(NveSemantics(NveTables.Values.AriaRole.BUTTON, "Submit form", "", "submit-btn"), button.semantics)
        assertEquals(NveNodeKind.TEXT, button.kind)
    }

    @Test
    fun resourcesAreCopiedOutOfTheFrameAndReusedOnlyWhileProvablyUnchanged() {
        val draw = ByteBuffer.allocateDirect(3).put(byteArrayOf(4, 5, 6)).flip() as ByteBuffer
        val res = Resources(imageSourceBytes(1, byteArrayOf(0x89.toByte(), 'P'.code.toByte())), draw)
        val state = capture(showcase(TestScene(frameId = 1)), res)
        val encoded = state.at(0x1000005).image as NveImage.Encoded
        assertArrayEquals(byteArrayOf(0x89.toByte(), 'P'.code.toByte()), encoded.bytes)
        draw.put(0, 9)  // the frame's buffer is reused by the next frame: the state must not alias it
        val firstDraw = state.at(0x1000008).drawList
        assertArrayEquals(byteArrayOf(4, 5, 6), firstDraw)

        res.calls.clear()
        capture(showcase(TestScene(frameId = 2)), res, state)
        assertSame(encoded, state.at(0x1000005).image, "same image id reuses the decoded copy")
        assertSame(firstDraw, state.at(0x1000008).drawList)
        assertTrue(res.calls.isEmpty(), "unchanged resources are not re-read")

        // Same id, new bytes (ephemeral rewrite / recycled slot): listed in dirty_resources.
        capture(showcase(TestScene(frameId = 3)).also { it.dirtyResources.add(2) }, res, state)
        assertArrayEquals(byteArrayOf(9, 5, 6), state.at(0x1000008).drawList, "dirty draw list re-read")
        assertSame(encoded, state.at(0x1000005).image, "clean image still reused")

        // A skipped frame may have redefined anything: re-read everything.
        res.calls.clear()
        capture(showcase(TestScene(frameId = 5)), res, state)
        assertEquals(setOf(1, 2), res.calls.toSet(), "frame gap re-reads every resource")
        // A new epoch reuses ids for unrelated resources.
        res.calls.clear()
        capture(showcase(TestScene(frameId = 6, epoch = 1)), res, state)
        assertEquals(setOf(1, 2), res.calls.toSet(), "epoch change re-reads every resource")
    }

    /**
     * P0: a resource rewritten in place (same id, new bytes) leaves its users' rows clean. Only
     * `dirty_resources` names it; the `resource -> nodes` index must rebuild every clean user.
     */
    @Test
    fun cleanNodesBoundToARewrittenResourceAreRebuilt() {
        val res = Resources(imageSourceBytes(1, byteArrayOf(1)), ByteBuffer.wrap(byteArrayOf(4, 5, 6)))
        val state = capture(showcase(TestScene(frameId = 1)), res)
        val title = state.at(0x1000003)
        res.image = imageSourceBytes(1, byteArrayOf(2, 3))
        res.draw = ByteBuffer.wrap(byteArrayOf(7))
        res.calls.clear()
        capture(showcase(TestScene(frameId = 2)).also { it.dirty = intArrayOf(); it.dirtyResources.add(1) }, res, state)
        assertArrayEquals(byteArrayOf(2, 3), (state.at(0x1000005).image as NveImage.Encoded).bytes, "image user rebuilt")
        assertArrayEquals(byteArrayOf(4, 5, 6), state.at(0x1000008).drawList, "clean draw list kept")
        assertEquals(listOf(1), res.calls, "only the rewritten resource is re-read")
        assertSame(title, state.at(0x1000003), "nodes bound to nothing dirty are untouched")

        capture(showcase(TestScene(frameId = 3)).also { it.dirty = intArrayOf(); it.dirtyResources.add(2) }, res, state)
        assertArrayEquals(byteArrayOf(7), state.at(0x1000008).drawList, "draw-list user rebuilt")

        // A user that is also content-dirty is rebuilt once, not twice.
        res.calls.clear()
        capture(showcase(TestScene(frameId = 4)).also { it.dirty = intArrayOf(5); it.dirtyResources.add(1) }, res, state)
        assertEquals(listOf(1), res.calls, "dirty + resource-dirty node read once")
        // A removed user leaves the index: rewriting its resource rebuilds nothing.
        res.calls.clear()
        capture(showcase(TestScene(frameId = 5)).also { it.dirty = intArrayOf(); it.removed.add(0x1000005) }, res, state)
        capture(showcase(TestScene(frameId = 6)).also { it.dirty = intArrayOf(); it.dirtyResources.add(1) }, res, state)
        assertTrue(res.calls.isEmpty(), "no users left for resource 1: ${res.calls}")
    }

    @Test
    fun nonFiniteGeometryAndPaintAreSanitized() {
        val scene = TestScene().node(2, 1, Float.NaN, 0f, Float.POSITIVE_INFINITY, Float.NaN)
        val visual = scene.table(NveTables.Visual.KEY, NveTables.Visual.WORDS)
        scene.set(visual, 2, NveTables.Visual.OPACITY, Float.NaN).set(visual, 2, NveTables.Visual.BORDER_RADIUS, Float.NaN)
            .set(visual, 2, NveTables.Visual.BORDER_WIDTH, Float.NEGATIVE_INFINITY)
        val typo = scene.table(NveTables.Typography.KEY, NveTables.Typography.WORDS)
        scene.set(typo, 2, NveTables.Typography.SET, NveTables.Values.TypographySet.FONT_SIZE).set(typo, 2, NveTables.Typography.FONT_SIZE, Float.NaN)
        val text = scene.table(NveTables.Text.KEY, NveTables.Text.WORDS)
        scene.set(text, 2, NveTables.Text.TEXT, "x")
        val n = capture(scene, { null }).at(0x1000002)
        assertEquals(listOf(0f, 0f, 0f, 0f), listOf(n.x, n.y, n.width, n.height))
        assertEquals(NveBox(opacity = 1f), n.box)
        assertEquals(SceneSemantics.DEFAULT_FONT_SIZE, n.textStyle.fontSize)
    }

    @Test
    fun brokenOrForeignResourcesRenderEmpty() {
        val res = Resources(imageSourceBytes(7, byteArrayOf(1)), ByteBuffer.allocate(0))
        assertNull(capture(showcase(), res).at(0x1000005).image, "unknown image tag")
        val noLookup = capture(showcase(), { null })
        assertNull(noLookup.at(0x1000005).image)
        assertNull(noLookup.at(0x1000008).drawList)
    }

    @Test
    fun removedNodesLeaveTheState() {
        val res = Resources(imageSourceBytes(0, "x".toByteArray()), ByteBuffer.allocate(0))
        val state = capture(showcase(), res)
        // A frame gap reconciles everything: nodes no longer live are gone.
        capture(TestScene(frameId = 3).node(2, 1, 0f, 0f, 10f, 10f), res, state)
        assertEquals(setOf(0x1000002), state.ids)
        assertEquals(3L, state.frameId)
        assertTrue(state.usersOf(1).isEmpty(), "removed nodes leave the resource index")
    }

    @Test
    fun chainedFramesRebuildOnlyWhatChanged() {
        val res = Resources(imageSourceBytes(0, "asset://logo.png".toByteArray()), ByteBuffer.allocate(0))
        val state = capture(showcase(TestScene(frameId = 1)), res)
        val before = state.ids.associateWith { state.at(it) }
        val rootChildren = state.rootChildren
        assertFalse(NveSceneCapture.capture(showcase(TestScene(frameId = 1)).reader(), res, state), "a redelivered frame is a no-op")

        // Frame 2: only the title is content-dirty, the button moved, row 7 was removed.
        val next = showcase(TestScene(frameId = 2))
        next.node(9, 2, 40f, 330f, 120f, 44f)  // its new rect (parent-relative 35, 325)
        next.set(next.table(NveTables.Text.KEY, NveTables.Text.WORDS), 3, NveTables.Text.TEXT, "Edited")
        next.dirty = intArrayOf(3)
        next.moved = intArrayOf(9)
        next.removed.add(0x1000007)
        capture(next, res, state)
        assertEquals("Edited", state.at(0x1000003).text, "dirty node rebuilt")
        assertSame(before[0x1000004], state.at(0x1000004), "untouched node reused as is")
        assertSame(before[0x1000005], state.at(0x1000005))
        val button = state.at(0x1000009)
        assertNotSame(before[0x1000009], button)
        assertEquals(35f to 325f, button.x to button.y, "moved node gets its new geometry")
        assertSame(before.getValue(0x1000009).semantics, button.semantics, "a moved node keeps its content")
        assertFalse(0x1000007 in state.ids, "removed node dropped")
        assertSame(rootChildren, state.rootChildren, "a clean root is kept")
    }

    @Test
    fun runtimeScrollOffsetsReachTheState() {
        val res = Resources(imageSourceBytes(0, "x".toByteArray()), ByteBuffer.allocate(0))
        val state = capture(showcase().scroll(6, 30f, 0f), res)
        assertEquals(NveScrollOffset(30f, 0f), state.at(0x1000006).scrollOffset)
        assertEquals(NveScrollOffset.ZERO, state.at(0x1000002).scrollOffset, "unscrolled containers are at 0")
        val scroll = state.at(0x1000006)
        capture(showcase(TestScene(frameId = 2)).scroll(6, 30f, 0f).also { it.dirty = intArrayOf() }, res, state)
        assertSame(scroll, state.at(0x1000006), "an unchanged offset leaves the node untouched")
        capture(showcase(TestScene(frameId = 3)).scroll(6, 0f, 45f).also { it.dirty = intArrayOf() }, res, state)
        assertEquals(NveScrollOffset(0f, 45f), state.at(0x1000006).scrollOffset, "a clean container follows its new offset")
        capture(showcase(TestScene(frameId = 4)).also { it.dirty = intArrayOf() }, res, state)
        assertEquals(NveScrollOffset.ZERO, state.at(0x1000006).scrollOffset, "scrolled back to the origin")
    }

    @Test
    fun scrollContentExtentUpdatesOnlyWhenChildOrContainerGeometryChanges() {
        val res = Resources(imageSourceBytes(0, "x".toByteArray()), ByteBuffer.allocate(0))
        val state = capture(showcase(TestScene(frameId = 1)), res)
        val initialScroll = state.at(0x1000006)
        assertEquals(200f to 150f, initialScroll.scrollContentWidth to initialScroll.scrollContentHeight)

        // Child row 7 changes text only (same geometry): scroll container slot stays untouched.
        val textOnly = showcase(TestScene(frameId = 2))
        textOnly.set(textOnly.table(NveTables.Text.KEY, NveTables.Text.WORDS), 7, NveTables.Text.TEXT, "Row updated")
        textOnly.dirty = intArrayOf(7)
        capture(textOnly, res, state)
        assertSame(initialScroll, state.at(0x1000006), "content-only child edit keeps scroll slot unchanged")

        // Child row 7 grows taller via `moved`: scroll container precomputed extent updates.
        val taller = showcase(TestScene(frameId = 3))
        taller.node(7, 6, 10f, 170f, 260f, 240f)
        taller.dirty = intArrayOf()
        taller.moved = intArrayOf(7)
        capture(taller, res, state)
        val grown = state.at(0x1000006)
        assertEquals(260f to 240f, grown.scrollContentWidth to grown.scrollContentHeight)
    }

    @Test
    fun drawListKindIsNotAnImage() {
        assertFalse(drawKind == ImageSource.KIND)
    }
}
