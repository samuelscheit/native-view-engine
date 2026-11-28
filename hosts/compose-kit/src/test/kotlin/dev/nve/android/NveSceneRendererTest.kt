package dev.nve.android

import android.graphics.Bitmap
import android.graphics.Canvas as AndroidCanvas
import android.graphics.Color as AndroidColor
import android.text.InputType
import android.view.KeyEvent
import android.view.View
import android.view.inputmethod.EditorInfo
import android.widget.EditText
import android.widget.FrameLayout
import android.widget.HorizontalScrollView
import android.widget.ScrollView
import android.widget.TextView
import dev.nve.compose.TestScene
import dev.nve.compose.imageSourceBytes
import dev.nve.compose.isEvent
import dev.nve.host.DrawList
import dev.nve.host.EventRingReader
import dev.nve.host.EventRingWriter
import dev.nve.host.FrameHost
import dev.nve.host.ImageSource
import dev.nve.host.Key64
import dev.nve.host.generated.NveAbi
import dev.nve.host.generated.NveModuleTables
import dev.nve.host.generated.NveTables
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotSame
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** Records what the renderer sends (validated `NVEV` v2 batches) and serves resources by id. */
private class FakeFrameHost : FrameHost {
    val rings = ArrayList<EventRingReader>()
    val resources = HashMap<Int, ByteBuffer>()
    val resourceKinds = HashMap<Int, Key64>()
    val resourceCalls = ArrayList<Int>()
    private var kind = Key64.ZERO
    var synchronous = false
    /** Runs inside [send] after the events are recorded: a local app's frame in answer. */
    var answer: (() -> Unit)? = null

    override fun send(writer: FrameHost.EventWriter): Int {
        val ring = EventRingWriter(ByteBuffer.allocate(1024), 1)
        writer.write(ring)
        rings.add(EventRingReader().attach(ring.buffer().array().copyOf(ring.length())))
        answer?.invoke()
        return 0
    }

    override fun dispatchesSynchronously(): Boolean = synchronous

    override fun scrollBy(nodeId: Int, dx: Float, dy: Float): Int = 0

    override fun resource(id: Int): ByteBuffer? {
        resourceCalls.add(id)
        kind = resourceKinds[id] ?: ImageSource.KIND
        return resources[id]?.duplicate()
    }

    override fun resourceKind(): Key64 = kind

    /** `(x, y)` of every scroll event sent. */
    fun scrolls(): List<Pair<Float, Float>> =
        rings.filter { it.isEvent(NveAbi.TABLE_SCROLL, NveAbi.SCROLL_EVENT_SCROLL) }
            .map { it.payloadF32(0, 0) to it.payloadF32(0, 1) }
}

/** `NveSceneRenderer` + `NveViewStyle` on real Android Views (Robolectric). */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class NveSceneRendererTest {
    private val context: android.content.Context = RuntimeEnvironment.getApplication()
    private val root = FrameLayout(context)
    private val renderer = NveSceneRenderer(context, root, 1f)
    private val host = FakeFrameHost()

    /** Root(1) → column(2) → [title(3), input(4), label(5), scroll(6) → [row(7)], image(8)]. */
    private fun scene(frameId: Long, epoch: Int = 0, axes: Int = NveTables.Values.ScrollAxes.VERTICAL): TestScene {
        val s = TestScene(frameId = frameId, epoch = epoch)
            .node(2, 1, 0f, 0f, 300f, 400f)
            .node(3, 2, 10f, 10f, 200f, 30f)
            .node(4, 2, 10f, 50f, 200f, 40f)
            .node(5, 2, 10f, 100f, 200f, 30f)
            .node(6, 2, 10f, 150f, 200f, 80f)
            .node(7, 6, 10f, 150f, 200f, 300f)
            .node(8, 2, 10f, 240f, 64f, 64f)
        val text = s.table(NveTables.Text.KEY, NveTables.Text.WORDS)
        s.set(text, 3, NveTables.Text.TEXT, "Title").set(text, 5, NveTables.Text.TEXT, "Label").set(text, 7, NveTables.Text.TEXT, "Row")
        val input = s.table(NveTables.TextInput.KEY, NveTables.TextInput.WORDS)
        s.set(input, 4, NveTables.TextInput.VALUE, "draft")
        val scroll = s.table(NveTables.Scroll.KEY, NveTables.Scroll.WORDS)
        s.set(scroll, 6, NveTables.Scroll.AXES, axes)
        val image = s.table(NveTables.Image.KEY, NveTables.Image.WORDS)
        s.set(image, 8, NveTables.Image.SRC, 9)
        return s
    }

    private fun apply(s: TestScene) = renderer.apply(s.reader(), host)

    private fun view(s: TestScene, index: Int): View = renderer.viewOf(s.id(index))!!

    @Test
    fun aChainedFrameTouchesOnlyItsDirtyMovedAndRemovedNodes() {
        val first = scene(1)
        apply(first)
        val label = view(first, 5) as TextView
        assertEquals("Label", label.text.toString())
        label.text = "tampered"  // only a re-apply of node 5 could restore it

        val next = scene(2).also {
            it.set(it.table(NveTables.Text.KEY, NveTables.Text.WORDS), 3, NveTables.Text.TEXT, "Edited")
            it.node(8, 2, 40f, 260f, 64f, 64f)
            it.dirty = intArrayOf(3)
            it.moved = intArrayOf(8)
        }
        apply(next)
        assertEquals("Edited", (view(next, 3) as TextView).text.toString())
        assertEquals("untouched nodes are not re-applied", "tampered", label.text.toString())
        val lp = view(next, 8).layoutParams as FrameLayout.LayoutParams
        assertEquals(40 to 260, lp.leftMargin to lp.topMargin)

        apply(scene(2))  // redelivered frame: a no-op
        assertEquals("tampered", label.text.toString())

        val gap = scene(9)
        apply(gap)
        assertEquals("a frame gap reconciles everything", "Label", label.text.toString())

        val removal = scene(10).also { it.dirty = intArrayOf(2); it.removed.add(it.id(5)) }
        apply(removal)
        assertEquals(null, renderer.viewOf(removal.id(5)))
        assertTrue("removed view detached", label.parent == null)
    }

    @Test
    fun aNewSceneEpochDropsEveryViewAndItsState() {
        val first = scene(1, epoch = 0)
        apply(first)
        val edit = view(first, 4) as EditText
        val scroller = view(first, 6)
        apply(scene(2, epoch = 1))
        val again = scene(2, epoch = 1)
        assertNotSame("same id, new epoch: a fresh view", edit, view(again, 4))
        assertNotSame(scroller, view(again, 6))
        assertTrue("old views left the tree", edit.parent == null && scroller.parent == null)
        assertEquals(1, root.childCount)
    }

    @Test
    fun scrollAxesSelectTheScrollViewAndSwitchingRecreatesIt() {
        apply(scene(1, axes = NveTables.Values.ScrollAxes.VERTICAL))
        val s = scene(2, axes = NveTables.Values.ScrollAxes.HORIZONTAL).also { it.dirty = intArrayOf(6) }
        assertTrue(renderer.viewOf(s.id(6)) is ScrollView)
        apply(s)
        val horizontal = renderer.viewOf(s.id(6))
        assertTrue("vertical -> horizontal recreates the view", horizontal is HorizontalScrollView)
        assertSame("the recreated view sits in its parent", (view(s, 2) as FrameLayout), horizontal!!.parent)
        assertEquals("children follow the new content", 1, ((horizontal as HorizontalScrollView).getChildAt(0) as FrameLayout).childCount)

        apply(scene(3, axes = NveTables.Values.ScrollAxes.BOTH).also { it.dirty = intArrayOf(6) })
        val both = renderer.viewOf(s.id(6)) as ScrollView
        assertTrue("both axes: horizontal scroller inside a vertical one", both.getChildAt(0) is HorizontalScrollView)

        apply(scene(4, axes = NveTables.Values.ScrollAxes.NONE).also { it.dirty = intArrayOf(6) })
        val none = renderer.viewOf(s.id(6))!!
        assertFalse("no axes: no scroll view", none is ScrollView || none is HorizontalScrollView)
    }

    @Test
    fun runtimeScrollOffsetsScrollTheViews() {
        apply(scene(1).scroll(6, 0f, 40f))
        val sv = renderer.viewOf(scene(1).id(6)) as ScrollView
        sv.measure(View.MeasureSpec.makeMeasureSpec(200, View.MeasureSpec.EXACTLY), View.MeasureSpec.makeMeasureSpec(80, View.MeasureSpec.EXACTLY))
        sv.layout(0, 0, 200, 80)
        apply(scene(2).scroll(6, 0f, 60f).also { it.dirty = intArrayOf() })
        assertEquals(60, sv.scrollY)
        apply(scene(3).also { it.dirty = intArrayOf() })
        assertEquals("scrolled back to the origin when the offset is gone", 0, sv.scrollY)
        assertTrue("renderer-caused scrolls are not user input", host.scrolls().isEmpty())
    }

    @Test
    fun aScrollOffsetPublishedBeforeLayoutIsShownAfterIt() {
        val s = scene(1).scroll(6, 0f, 40f)
        apply(s)
        val sv = view(s, 6) as ScrollView
        assertEquals("not laid out: a scroll view would clamp to 0", 0, sv.scrollY)
        layout(sv, 200, 80)
        assertEquals(40, sv.scrollY)
        assertTrue("the restore is not user input", host.scrolls().isEmpty())

        val none = scene(2, axes = NveTables.Values.ScrollAxes.NONE).scroll(6, 0f, 30f).also { it.dirty = intArrayOf(6) }
        apply(none)
        val frame = view(none, 6)
        layout(frame, 200, 80)
        assertEquals("NONE is a clipping frame the runtime still scrolls", 30, frame.scrollY)
    }

    @Test
    fun aBothAxesContainerReportsBothAxesInEveryScrollEvent() {
        val s = scene(1, axes = NveTables.Values.ScrollAxes.BOTH).node(7, 6, 10f, 150f, 500f, 300f)
        apply(s)
        val outer = view(s, 6) as ScrollView
        layout(outer, 200, 80)
        val inner = outer.getChildAt(0) as HorizontalScrollView
        inner.scrollTo(30, 0)
        outer.scrollTo(0, 20)
        inner.scrollTo(45, 0)
        assertEquals(listOf(30f to 0f, 30f to 20f, 45f to 20f), host.scrolls())
    }

    @Test
    fun aLocalAppRejectingAnEditShowsItsValueAgainButPairedTypingStays() {
        val s = scene(1)
        apply(s)
        val edit = view(s, 4) as EditText
        host.synchronous = true
        edit.setText("draftx")
        assertEquals("rejected: the app kept its value", "draft", edit.text.toString())

        host.answer = {
            apply(scene(2).also {
                it.set(it.table(NveTables.TextInput.KEY, NveTables.TextInput.WORDS), 4, NveTables.TextInput.VALUE, "drafty")
                it.dirty = intArrayOf(4)
            })
        }
        edit.setText("drafty")
        assertEquals("accepted within send", "drafty", edit.text.toString())

        host.answer = null
        host.synchronous = false
        edit.setText("typed")
        assertEquals("paired: the echo comes later", "typed", edit.text.toString())
        assertEquals(3, host.rings.count { it.isEvent(NveAbi.TABLE_TEXT_INPUT, NveAbi.TEXT_INPUT_EVENT_CHANGE) })
    }

    @Test
    fun secureInputsAreMaskedForEveryKind() {
        val number = InputType.TYPE_CLASS_NUMBER
        val password = InputType.TYPE_NUMBER_VARIATION_PASSWORD
        val decimal = InputType.TYPE_NUMBER_FLAG_DECIMAL
        assertEquals(number or decimal, NveViewStyle.inputType(false, false, NveTables.Values.InputKind.NUMBER))
        assertEquals(number or decimal or password, NveViewStyle.inputType(true, false, NveTables.Values.InputKind.NUMBER))
        assertEquals(InputType.TYPE_CLASS_PHONE, NveViewStyle.inputType(false, false, NveTables.Values.InputKind.TEL))
        assertEquals(number or password, NveViewStyle.inputType(true, false, NveTables.Values.InputKind.TEL))
        assertEquals(
            InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD,
            NveViewStyle.inputType(true, true, NveTables.Values.InputKind.EMAIL) and InputType.TYPE_TEXT_FLAG_MULTI_LINE.inv(),
        )

        val s = scene(1)
        s.set(s.table(NveTables.TextInput.KEY, NveTables.TextInput.WORDS), 4, NveTables.TextInput.SECURE, 1)
            .set(s.table(NveTables.TextInput.KEY, NveTables.TextInput.WORDS), 4, NveTables.TextInput.INPUT_KIND, NveTables.Values.InputKind.TEL)
        apply(s)
        val edit = view(s, 4) as EditText
        assertEquals(number or password, edit.inputType)
        assertTrue("digits are masked", edit.transformationMethod is android.text.method.PasswordTransformationMethod)
    }

    @Test
    fun losingItsAccessibilityRowClearsSelection() {
        val s = scene(1)
        s.set(s.table(NveTables.Accessibility.KEY, NveTables.Accessibility.WORDS), 5, NveTables.Accessibility.SELECTED, NveTables.Values.AriaTriState.TRUE)
        apply(s)
        assertTrue(view(s, 5).isSelected)
        val next = scene(2).also { it.dirty = intArrayOf(5) }
        apply(next)
        assertFalse(view(next, 5).isSelected)
    }

    @Test
    fun disabledAndHiddenNodesTakeNoInput() {
        val s = scene(1)
        val interaction = s.table(NveTables.Interaction.KEY, NveTables.Interaction.WORDS)
        s.set(interaction, 4, NveTables.Interaction.DISABLED, 1)
        val visual = s.table(NveTables.Visual.KEY, NveTables.Visual.WORDS)
        s.set(visual, 5, NveTables.Visual.VISIBILITY, NveTables.Values.Visibility.HIDDEN).set(visual, 5, NveTables.Visual.OPACITY, 1f)
        apply(s)
        assertFalse("interaction.disabled disables the EditText", view(s, 4).isEnabled)
        assertEquals(View.INVISIBLE, view(s, 5).visibility)
        val enabled = scene(2).also { it.dirty = intArrayOf(4) }
        apply(enabled)
        assertTrue(view(enabled, 4).isEnabled)
    }

    @Test
    fun hardwareEnterAndImeActionsSubmitExactlyOnce() {
        assertTrue(NveSceneRenderer.isSubmit(EditorInfo.IME_ACTION_DONE, null))
        assertFalse(NveSceneRenderer.isSubmit(EditorInfo.IME_NULL, null))
        assertTrue(NveSceneRenderer.isSubmit(EditorInfo.IME_NULL, KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_ENTER)))
        assertFalse("the ACTION_UP half is not a second submit", NveSceneRenderer.isSubmit(EditorInfo.IME_NULL, KeyEvent(KeyEvent.ACTION_UP, KeyEvent.KEYCODE_ENTER)))
        assertFalse(NveSceneRenderer.isSubmit(EditorInfo.IME_NULL, KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_A)))

        val s = scene(1)
        apply(s)
        val edit = view(s, 4) as EditText
        edit.onEditorAction(EditorInfo.IME_ACTION_DONE)
        val submits = host.rings.filter { it.isEvent(NveAbi.TABLE_TEXT_INPUT, NveAbi.TEXT_INPUT_EVENT_SUBMIT) }
        assertEquals(1, submits.size)
        assertEquals(s.id(4), submits.last().node(0))
    }

    @Test
    fun appRewritesKeepTheCaretAndRendererWritesAreNotInput() {
        val s = scene(1)
        apply(s)
        val edit = view(s, 4) as EditText
        edit.setSelection(2)
        val rewritten = scene(2).also {
            it.set(it.table(NveTables.TextInput.KEY, NveTables.TextInput.WORDS), 4, NveTables.TextInput.VALUE, "DRAFT")
            it.dirty = intArrayOf(4)
        }
        apply(rewritten)
        assertEquals("DRAFT", edit.text.toString())
        assertEquals("the caret stays where it was", 2, edit.selectionStart)
        assertTrue("renderer writes are not text changes", host.rings.none { it.isEvent(NveAbi.TABLE_TEXT_INPUT, NveAbi.TEXT_INPUT_EVENT_CHANGE) })
        edit.setText("typed\uD83D")
        val typed = host.rings.last { it.isEvent(NveAbi.TABLE_TEXT_INPUT, NveAbi.TEXT_INPUT_EVENT_CHANGE) }
        assertEquals("a lone surrogate is sent as U+FFFD (5 + 3 UTF-8 bytes)", 8, typed.bytesLen(0))
        assertEquals("typed\uFFFD", typed.text(0))
    }

    @Test
    fun dirtyResourcesRebindOnlyTheirViews() {
        host.resources[9] = imageSourceBytes(1, byteArrayOf(1, 2, 3))
        apply(scene(1))
        assertEquals(listOf(9), host.resourceCalls)
        apply(scene(2).also { it.dirty = intArrayOf(3) })
        assertEquals("clean resources are not re-read", listOf(9), host.resourceCalls)
        apply(scene(3).also { it.dirty = intArrayOf(); it.dirtyResources.add(9) })
        assertEquals("a dirty id rebinds its image", listOf(9, 9), host.resourceCalls)
        apply(scene(4).also { it.dirty = intArrayOf(); it.dirtyResources.add(11) })
        assertEquals("unrelated dirty ids touch nothing", listOf(9, 9), host.resourceCalls)
        apply(scene(5).also { it.dirty = intArrayOf(8); it.dirtyResources.add(9) })
        assertEquals("a dirty image showing a dirty resource decodes once", listOf(9, 9, 9), host.resourceCalls)

        // Canvas node: FlatResource draw list paints, dirty rewrite updates it, foreign kind clears it.
        fun fillRectDrawList(rgba: Int): ByteBuffer = ByteBuffer.allocate(28).order(ByteOrder.LITTLE_ENDIAN)
            .putInt(1).putInt(DrawList.OP_FILL_RECT).putFloat(0f).putFloat(0f).putFloat(16f).putFloat(16f).putInt(rgba)
            .also { it.flip() }
        fun canvasScene(frameId: Long): TestScene = TestScene(frameId = frameId, epoch = 2)
            .node(2, 1, 0f, 0f, 16f, 16f)
            .also { it.set(it.table(NveModuleTables.Canvas.KEY, NveModuleTables.Canvas.WORDS), 2, NveModuleTables.Canvas.DRAW_LIST, 12) }
        fun sample(v: View): Int = Bitmap.createBitmap(16, 16, Bitmap.Config.ARGB_8888).let { bmp ->
            layout(v, 16, 16)
            v.draw(AndroidCanvas(bmp))
            bmp.getPixel(8, 8)
        }
        host.resources[12] = fillRectDrawList(0xFF0000FF.toInt()) // red in RGBA wire order
        host.resourceKinds[12] = DrawList.KIND
        apply(canvasScene(1))
        val canvasView = view(canvasScene(1), 2) as NveCanvasView
        assertEquals("canvas paints FlatResource FillRect", AndroidColor.RED, sample(canvasView))
        host.resources[12] = fillRectDrawList(0xFFFF0000.toInt()) // blue in RGBA wire order
        apply(canvasScene(2).also { it.dirty = intArrayOf(); it.dirtyResources.add(12) })
        assertEquals("dirty draw-list resource rebinds canvas", AndroidColor.BLUE, sample(canvasView))
        host.resourceKinds[12] = ImageSource.KIND
        apply(canvasScene(3).also { it.dirty = intArrayOf(); it.dirtyResources.add(12) })
        assertEquals("foreign resource kind clears canvas", 0, sample(canvasView))

        // Android NveCanvasView caps nested clip depth at 64 with overflow tracking.
        val deepOps = DrawList.MAX_CLIP_DEPTH + 2 + 2 + 1 + 1 + 1
        val deepBuf = ByteBuffer.allocate(DrawList.HEADER_SIZE + (DrawList.MAX_CLIP_DEPTH + 2) * 20 + 3 * 4 + 2 * 24)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(deepOps)
        repeat(DrawList.MAX_CLIP_DEPTH - 1) {
            deepBuf.putInt(DrawList.OP_CLIP_RECT).putFloat(0f).putFloat(0f).putFloat(16f).putFloat(16f)
        }
        deepBuf.putInt(DrawList.OP_CLIP_RECT).putFloat(0f).putFloat(0f).putFloat(6f).putFloat(6f)
            .putInt(DrawList.OP_CLIP_RECT).putFloat(0f).putFloat(0f).putFloat(2f).putFloat(2f)
            .putInt(DrawList.OP_CLIP_RECT).putFloat(0f).putFloat(0f).putFloat(1f).putFloat(1f)
            .putInt(DrawList.OP_RESTORE_CLIP)
            .putInt(DrawList.OP_RESTORE_CLIP)
            .putInt(DrawList.OP_FILL_RECT).putFloat(0f).putFloat(0f).putFloat(16f).putFloat(16f).putInt(0xFF0000FF.toInt())
            .putInt(DrawList.OP_RESTORE_CLIP)
            .putInt(DrawList.OP_FILL_RECT).putFloat(10f).putFloat(10f).putFloat(4f).putFloat(4f).putInt(0xFFFF0000.toInt())
            .also { it.flip() }
        canvasView.setDrawList(deepBuf)
        val bmp = Bitmap.createBitmap(16, 16, Bitmap.Config.ARGB_8888)
        val c = AndroidCanvas(bmp)
        val beforeSaves = c.saveCount
        layout(canvasView, 16, 16)
        canvasView.draw(c)
        assertEquals("canvas saveCount balanced after 66 nested clips", beforeSaves, c.saveCount)
        assertEquals("inside clip #64", AndroidColor.RED, bmp.getPixel(3, 3))
        assertEquals("clip #64 stays active after 2 overflow restores", 0, bmp.getPixel(8, 8))
        assertEquals("3rd restore pops clip #64", AndroidColor.BLUE, bmp.getPixel(12, 12))
    }

    private fun layout(v: View, w: Int, h: Int) {
        v.measure(View.MeasureSpec.makeMeasureSpec(w, View.MeasureSpec.EXACTLY), View.MeasureSpec.makeMeasureSpec(h, View.MeasureSpec.EXACTLY))
        v.layout(0, 0, w, h)
    }
}
