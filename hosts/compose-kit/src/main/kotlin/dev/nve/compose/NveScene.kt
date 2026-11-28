package dev.nve.compose

import dev.nve.host.ImageSource
import dev.nve.host.Key64
import dev.nve.host.SceneReader
import dev.nve.host.SceneSemantics
import dev.nve.host.generated.NveModuleTables
import dev.nve.host.generated.NveTables
import java.nio.ByteBuffer

/** How a node is realized, from the tables it carries (shared with every Java / Kotlin host). */
typealias NveNodeKind = SceneSemantics.Kind

private typealias Tables = SceneSemantics.Tables

/** `nve.visual` box paint (`RGBA8888` colors, dp lengths). */
data class NveBox(
    val background: Int = 0,
    val borderColor: Int = 0,
    val borderWidth: Float = 0f,
    val radius: Float = 0f,
    val opacity: Float = 1f,
    val zIndex: Int = 0,
    val visible: Boolean = true,
)

/** Inherited `nve.typography` text style (resolved by [SceneSemantics.TextStyle]). */
data class NveTextStyle(
    val color: Int = SceneSemantics.DEFAULT_COLOR,
    val fontSize: Float = SceneSemantics.DEFAULT_FONT_SIZE,
    val fontWeight: Int = SceneSemantics.DEFAULT_FONT_WEIGHT,
    val italic: Boolean = false,
    val align: Int = NveTables.Values.TextAlign.START,
)

/** `nve.text_input` state. */
data class NveInput(
    val value: String,
    val placeholder: String,
    val secure: Boolean,
    val multiline: Boolean,
    val readOnly: Boolean,
    val inputKind: Int,
)

/** `nve.accessibility` + `nve.interaction` + `nve.debug_id` semantics. */
data class NveSemantics(
    val role: Int = NveTables.Values.AriaRole.NONE,
    val label: String = "",
    val hint: String = "",
    val testId: String = "",
    val hidden: Boolean = false,
    val disabled: Boolean = false,
)

/** An image source copied out of the frame (resources die at scene release). */
sealed interface NveImage {
    data class Url(val url: String) : NveImage

    class Encoded(val bytes: ByteArray) : NveImage
}

/** One node with its rect relative to its parent (scroll content is unscrolled), in dp. */
data class NveNode(
    val id: Int,
    val kind: NveNodeKind,
    val x: Float,
    val y: Float,
    val width: Float,
    val height: Float,
    val box: NveBox,
    val semantics: NveSemantics,
    val pressable: Boolean,
    val children: List<Int>,
    val text: String = "",
    val textStyle: NveTextStyle = NveTextStyle(),
    val input: NveInput? = null,
    val image: NveImage? = null,
    val imageFit: Int = NveTables.Values.ImageFit.COVER,
    val imageResource: Int = 0,
    val drawList: ByteArray? = null,
    val drawListResource: Int = 0,
    /** `nve.scroll.axes` (`NveTables.Values.ScrollAxes`) of a scroll container. */
    val scrollAxes: Int = NveTables.Values.ScrollAxes.NONE,
) {
    val scrollsHorizontally: Boolean
        get() = scrollAxes == NveTables.Values.ScrollAxes.HORIZONTAL || scrollAxes == NveTables.Values.ScrollAxes.BOTH
    val scrollsVertically: Boolean
        get() = scrollAxes == NveTables.Values.ScrollAxes.VERTICAL || scrollAxes == NveTables.Values.ScrollAxes.BOTH
}

/** Immutable copy of one runtime frame for composition. */
class NveSceneSnapshot(
    val frameId: Long,
    val sceneEpoch: Int,
    val rootBox: NveBox,
    val rootChildren: List<Int>,
    val nodes: Map<Int, NveNode>,
    /** Runtime scroll offsets (dp, `scroll_offsets`) of scrolled containers by node id. */
    val scrollOffsets: Map<Int, NveScrollOffset> = emptyMap(),
) {
    /** Runtime scroll offset of container [id] (`(0, 0)` when not scrolled). */
    fun scrollOffset(id: Int): NveScrollOffset = scrollOffsets[id] ?: NveScrollOffset.ZERO

    companion object {
        /** No frame yet (`frameId` 0: the next frame always reconciles everything). */
        val EMPTY = NveSceneSnapshot(0L, 0, NveBox(), emptyList(), emptyMap())
    }
}

/** A runtime scroll position in dp. */
data class NveScrollOffset(val x: Float, val y: Float) {
    companion object {
        val ZERO = NveScrollOffset(0f, 0f)
    }
}

/** Resource wire bytes of the frame being captured (`null` when the id is not live). */
fun interface NveResourceLookup {
    fun resource(id: Int): NveResource?
}

class NveResource(val kind: Key64, val bytes: ByteBuffer)

/**
 * Builds [NveSceneSnapshot]s from runtime frames in O(changed): on the frame right after
 * [previous] (same epoch) only removed, content-dirty (incl. inherited typography), and moved
 * nodes are rebuilt — every other [NveNode] (and its copied resource bytes) is reused as is; a
 * redelivered frame returns [previous]. Any gap or new epoch rebuilds everything.
 */
object NveSceneCapture {
    fun capture(s: SceneReader, resources: NveResourceLookup, previous: NveSceneSnapshot): NveSceneSnapshot {
        val relation = s.relationTo(previous.frameId, previous.sceneEpoch)
        if (relation == SceneReader.FRAME_SAME) return previous
        val t = SceneSemantics.Tables().bind(s)
        val style = SceneSemantics.TextStyle()
        val root = SceneSemantics.ROOT_INDEX
        val chained = relation == SceneReader.FRAME_NEXT
        // Copied resources stay valid along an unbroken chain of one epoch, minus the ids this
        // frame (re)defined: a skipped frame's dirty list is lost, a new epoch reuses ids.
        val reusable = { id: Int -> chained && id != 0 && !s.isResourceDirty(id) }
        val nodes: HashMap<Int, NveNode>
        var rootDirty = !chained
        if (chained) {
            nodes = HashMap(previous.nodes)
            for (k in 0 until s.removedCount()) nodes.remove(s.removedId(k))
            for (d in 0 until s.dirtyCount()) {
                val i = s.dirtyIndex(d)
                if (i == root) rootDirty = true else nodes[s.nodeId(i)] = node(s, t, i, resources, previous.nodes[s.nodeId(i)], reusable, style)
            }
            for (m in 0 until s.movedCount()) {
                val i = s.movedIndex(m)
                if (i == root) continue
                val id = s.nodeId(i)
                val prev = nodes[id]
                nodes[id] = prev?.let { geometry(s, i, it) } ?: node(s, t, i, resources, null, reusable, style)
            }
        } else {
            nodes = HashMap(s.nodeCapacity())
            for (i in root + 1 until s.nodeCapacity()) {
                if (s.isLive(i)) nodes[s.nodeId(i)] = node(s, t, i, resources, previous.nodes[s.nodeId(i)], reusable, style)
            }
        }
        val live = s.isLive(root)
        return NveSceneSnapshot(
            s.frameId(), s.sceneEpoch(),
            rootBox = if (!live) NveBox() else if (rootDirty) box(s, t, root) else previous.rootBox,
            rootChildren = if (!live) emptyList() else if (rootDirty) children(s, root) else previous.rootChildren,
            nodes = nodes,
            scrollOffsets = scrollOffsets(s, previous),
        )
    }

    /** O(scrolled containers); the previous map is reused when nothing moved. */
    private fun scrollOffsets(s: SceneReader, previous: NveSceneSnapshot): Map<Int, NveScrollOffset> {
        if (s.scrollCount() == 0) return emptyMap()
        val out = HashMap<Int, NveScrollOffset>(s.scrollCount())
        for (k in 0 until s.scrollCount()) {
            val id = s.scrollNodeId(k)
            out[id] = NveScrollOffset(s.scrollX(id), s.scrollY(id))
        }
        return if (out == previous.scrollOffsets) previous.scrollOffsets else out
    }

    private fun children(s: SceneReader, i: Int): List<Int> = List(s.childCount(i)) { s.childId(i, it) }

    private fun parentRelative(s: SceneReader, i: Int): FloatArray {
        val parentId = s.parent(i)
        val p = if (parentId == 0) -1 else s.resolve(parentId)
        val nested = p > SceneSemantics.ROOT_INDEX
        return floatArrayOf(
            SceneSemantics.finite(s.x(i) - (if (nested) s.x(p) else 0f), 0f),
            SceneSemantics.finite(s.y(i) - (if (nested) s.y(p) else 0f), 0f),
            maxOf(0f, SceneSemantics.finite(s.width(i), 0f)),
            maxOf(0f, SceneSemantics.finite(s.height(i), 0f)),
        )
    }

    /** [prev] with node [i]'s new geometry (a moved node's content is unchanged). */
    private fun geometry(s: SceneReader, i: Int, prev: NveNode): NveNode {
        val g = parentRelative(s, i)
        return prev.copy(x = g[0], y = g[1], width = g[2], height = g[3])
    }

    private fun node(
        s: SceneReader,
        t: Tables,
        i: Int,
        resources: NveResourceLookup,
        prev: NveNode?,
        reusable: (Int) -> Boolean,
        style: SceneSemantics.TextStyle,
    ): NveNode {
        val kind = SceneSemantics.kindOf(s, t, i)
        val g = parentRelative(s, i)
        var text = ""
        var textStyle = NveTextStyle()
        var input: NveInput? = null
        var image: NveImage? = null
        var imageFit = NveTables.Values.ImageFit.COVER
        var imageResource = 0
        var drawList: ByteArray? = null
        var drawListResource = 0
        var scrollAxes = NveTables.Values.ScrollAxes.NONE
        when (kind) {
            NveNodeKind.TEXT -> {
                text = s.text(t.text, s.rowOf(t.text, i), NveTables.Text.TEXT)
                textStyle = textStyle(s, t, i, style)
            }
            NveNodeKind.INPUT -> {
                val r = s.rowOf(t.input, i)
                val table = t.input!!
                textStyle = textStyle(s, t, i, style)
                input = NveInput(
                    value = s.text(table, r, NveTables.TextInput.VALUE),
                    placeholder = s.text(table, r, NveTables.TextInput.PLACEHOLDER),
                    secure = table.bool(r, NveTables.TextInput.SECURE),
                    multiline = table.bool(r, NveTables.TextInput.MULTILINE),
                    readOnly = table.bool(r, NveTables.TextInput.READ_ONLY),
                    inputKind = table.u32(r, NveTables.TextInput.INPUT_KIND),
                )
            }
            NveNodeKind.IMAGE -> {
                val r = s.rowOf(t.image, i)
                imageResource = t.image!!.u32(r, NveTables.Image.SRC)
                image = if (prev != null && prev.imageResource == imageResource && reusable(imageResource)) {
                    prev.image
                } else {
                    imageOf(resources.resource(imageResource))
                }
                imageFit = t.image.u32(r, NveTables.Image.FIT)
            }
            NveNodeKind.CANVAS -> {
                drawListResource = t.canvas!!.u32(s.rowOf(t.canvas, i), NveModuleTables.Canvas.DRAW_LIST)
                drawList = if (prev?.drawList != null && prev.drawListResource == drawListResource && reusable(drawListResource)) {
                    prev.drawList
                } else {
                    (if (drawListResource == 0) null else resources.resource(drawListResource))?.let { copy(it.bytes) }
                }
            }
            NveNodeKind.SCROLL -> scrollAxes = t.scroll!!.u32(s.rowOf(t.scroll, i), NveTables.Scroll.AXES)
            NveNodeKind.CONTAINER -> Unit
        }
        return NveNode(
            id = s.nodeId(i), kind = kind, x = g[0], y = g[1], width = g[2], height = g[3],
            box = box(s, t, i), semantics = semantics(s, t, i, SceneSemantics.disabled(s, t, i)),
            pressable = SceneSemantics.pressable(s, t, i), children = children(s, i),
            text = text, textStyle = textStyle, input = input, image = image, imageFit = imageFit,
            imageResource = imageResource, drawList = drawList, drawListResource = drawListResource, scrollAxes = scrollAxes,
        )
    }

    /** Image bytes copied out of the frame: inline / `data:` bytes, or a URL the host loads. */
    private fun imageOf(res: NveResource?): NveImage? {
        if (res == null || res.kind != ImageSource.KIND) return null
        val src = try {
            ImageSource.decode(res.bytes)
        } catch (_: RuntimeException) {
            return null
        }
        src.bytes()?.let { return NveImage.Encoded(it) }
        return src.url?.let { NveImage.Url(it) }
    }

    private fun copy(b: ByteBuffer): ByteArray = ByteArray(b.remaining()).also { b.duplicate().get(it) }
    private fun box(s: SceneReader, t: Tables, i: Int): NveBox {
        val r = s.rowOf(t.visual, i)
        if (r < 0) return NveBox()
        val v = t.visual!!
        return NveBox(
            background = v.color(r, NveTables.Visual.BACKGROUND),
            borderColor = v.color(r, NveTables.Visual.BORDER_COLOR),
            borderWidth = maxOf(0f, SceneSemantics.finite(v.f32(r, NveTables.Visual.BORDER_WIDTH), 0f)),
            radius = maxOf(0f, SceneSemantics.finite(v.f32(r, NveTables.Visual.BORDER_RADIUS), 0f)),
            opacity = SceneSemantics.finite(v.f32(r, NveTables.Visual.OPACITY), 1f).coerceIn(0f, 1f),
            zIndex = v.i32(r, NveTables.Visual.Z_INDEX),
            visible = v.u32(r, NveTables.Visual.VISIBILITY) != NveTables.Values.Visibility.HIDDEN,
        )
    }

    private fun textStyle(s: SceneReader, t: Tables, i: Int, st: SceneSemantics.TextStyle): NveTextStyle {
        st.resolve(s, t, i)
        return NveTextStyle(st.color, st.fontSize, st.fontWeight, st.italic, st.align)
    }

    private fun semantics(s: SceneReader, t: Tables, i: Int, disabled: Boolean): NveSemantics {
        val testRow = s.rowOf(t.debugId, i)
        val testId = if (testRow >= 0) s.text(t.debugId, testRow, NveTables.DebugId.TEST_ID) else ""
        val r = s.rowOf(t.accessibility, i)
        if (r < 0) return NveSemantics(testId = testId, disabled = disabled)
        val a = t.accessibility!!
        return NveSemantics(
            role = a.u32(r, NveTables.Accessibility.ROLE),
            label = s.text(a, r, NveTables.Accessibility.LABEL),
            hint = s.text(a, r, NveTables.Accessibility.HINT),
            testId = testId,
            hidden = a.bool(r, NveTables.Accessibility.HIDDEN),
            disabled = disabled,
        )
    }
}
