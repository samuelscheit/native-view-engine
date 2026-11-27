package dev.nve.android

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
import dev.nve.host.EventRingWriter
import dev.nve.host.FrameHost
import dev.nve.host.ImageSource
import dev.nve.host.Key64
import dev.nve.host.generated.NveAbi
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
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** Records what the renderer sends and serves resources by id. */
private class FakeFrameHost : FrameHost {
    val rings = ArrayList<ByteBuffer>()
    val resources = HashMap<Int, ByteBuffer>()
    val resourceCalls = ArrayList<Int>()
    private var kind = Key64.ZERO

    override fun send(writer: FrameHost.EventWriter): Int {
        val ring = EventRingWriter(ByteBuffer.allocate(1024), 1)
        writer.write(ring)
        rings.add(ByteBuffer.wrap(ring.buffer().array().copyOf(ring.length())).order(ByteOrder.LITTLE_ENDIAN))
        return 0
    }

    override fun scrollBy(nodeId: Int, dx: Float, dy: Float): Int = 0

    override fun resource(id: Int): ByteBuffer? {
        resourceCalls.add(id)
        kind = ImageSource.KIND
        return resources[id]?.duplicate()
    }

    override fun resourceKind(): Key64 = kind

    fun kinds(): List<Int> = rings.map { it.get(20).toInt() }
}

/** `NveSceneRenderer` + `NveViewStyle` on real Android Views (Robolectric). */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
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
        assertTrue("renderer-caused scrolls are not user input", host.kinds().none { it == NveAbi.EVENT_SCROLL })
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
        assertEquals(listOf(NveAbi.EVENT_SUBMIT), host.kinds().filter { it == NveAbi.EVENT_SUBMIT })
        assertEquals(s.id(4), host.rings.last().getInt(8))
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
        assertTrue("renderer writes are not text changes", host.kinds().none { it == NveAbi.EVENT_TEXT_CHANGE })
        edit.setText("typed\uD83D")
        val typed = host.rings.last { it.get(20).toInt() == NveAbi.EVENT_TEXT_CHANGE }
        assertEquals("a lone surrogate is sent as U+FFFD (5 + 3 UTF-8 bytes)", 8, typed.getInt(22))
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
    }
}
