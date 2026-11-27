package dev.nve.compose

import dev.nve.host.ImageSource
import dev.nve.host.Key64
import dev.nve.host.RuntimeSceneReader
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
    val align: Int = NveTables.Typography.TEXT_ALIGN_START,
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
    val role: Int = NveTables.Accessibility.ROLE_NONE,
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
    val imageFit: Int = NveTables.Image.FIT_COVER,
    val imageResource: Int = 0,
    val drawList: ByteArray? = null,
    val drawListResource: Int = 0,
    val scrollHorizontal: Boolean = false,
)

/** Immutable copy of one runtime frame for composition. */
class NveSceneSnapshot(
    val frameId: Long,
    val sceneEpoch: Int,
    val rootBox: NveBox,
    val rootChildren: List<Int>,
    val nodes: Map<Int, NveNode>,
) {
    companion object {
        val EMPTY = NveSceneSnapshot(-1L, 0, NveBox(), emptyList(), emptyMap())
    }
}

/** Resource wire bytes of the frame being captured (`null` when the id is not live). */
fun interface NveResourceLookup {
    fun resource(id: Int): NveResource?
}

class NveResource(val kind: Key64, val bytes: ByteBuffer)

/** Builds [NveSceneSnapshot]s from runtime frames; unchanged resources are reused, not re-copied. */
object NveSceneCapture {
    fun capture(s: RuntimeSceneReader, resources: NveResourceLookup, previous: NveSceneSnapshot): NveSceneSnapshot {
        val t = SceneSemantics.Tables().bind(s)
        // Copied resources stay valid only along an unbroken frame chain of one epoch, minus the
        // ids this frame (re)defined: a skipped frame's dirty list is lost, a new epoch reuses ids.
        val chained = previous.sceneEpoch == s.sceneEpoch() && s.frameId() == previous.frameId + 1
        val dirty = HashSet<Int>(s.dirtyResourceCount())
        for (k in 0 until s.dirtyResourceCount()) dirty.add(s.dirtyResource(k))
        val reusable = { id: Int -> chained && id != 0 && id !in dirty }
        val style = SceneSemantics.TextStyle()
        val nodes = HashMap<Int, NveNode>(s.nodeCapacity())
        for (i in SceneSemantics.ROOT_INDEX + 1 until s.nodeCapacity()) {
            if (s.isLive(i)) nodes[s.nodeId(i)] = node(s, t, i, resources, previous.nodes[s.nodeId(i)], reusable, style)
        }
        val root = SceneSemantics.ROOT_INDEX
        val rootChildren = if (s.isLive(root)) children(s, root) else emptyList()
        return NveSceneSnapshot(s.frameId(), s.sceneEpoch(), if (s.isLive(root)) box(s, t, root) else NveBox(), rootChildren, nodes)
    }

    private fun children(s: RuntimeSceneReader, i: Int): List<Int> = List(s.childCount(i)) { s.childId(i, it) }

    private fun node(
        s: RuntimeSceneReader,
        t: Tables,
        i: Int,
        resources: NveResourceLookup,
        prev: NveNode?,
        reusable: (Int) -> Boolean,
        style: SceneSemantics.TextStyle,
    ): NveNode {
        val kind = SceneSemantics.kindOf(s, t, i)
        val parentId = s.parent(i)
        val p = if (parentId == 0) -1 else s.resolve(parentId)
        val nested = p > SceneSemantics.ROOT_INDEX
        var out = NveNode(
            id = s.nodeId(i), kind = kind,
            x = SceneSemantics.finite(s.x(i) - (if (nested) s.x(p) else 0f), 0f),
            y = SceneSemantics.finite(s.y(i) - (if (nested) s.y(p) else 0f), 0f),
            width = maxOf(0f, SceneSemantics.finite(s.width(i), 0f)),
            height = maxOf(0f, SceneSemantics.finite(s.height(i), 0f)),
            box = box(s, t, i), semantics = semantics(s, t, i, SceneSemantics.disabled(s, t, i)),
            pressable = SceneSemantics.pressable(s, t, i),
            children = children(s, i),
        )
        when (kind) {
            NveNodeKind.TEXT -> out = out.copy(
                text = s.text(t.text, s.rowOf(t.text, i), NveTables.Text.TEXT), textStyle = textStyle(s, t, i, style),
            )
            NveNodeKind.INPUT -> {
                val r = s.rowOf(t.input, i)
                val input = t.input!!
                out = out.copy(
                    textStyle = textStyle(s, t, i, style),
                    input = NveInput(
                        value = s.text(input, r, NveTables.TextInput.VALUE),
                        placeholder = s.text(input, r, NveTables.TextInput.PLACEHOLDER),
                        secure = input.bool(r, NveTables.TextInput.SECURE),
                        multiline = input.bool(r, NveTables.TextInput.MULTILINE),
                        readOnly = input.bool(r, NveTables.TextInput.READ_ONLY),
                        inputKind = input.u32(r, NveTables.TextInput.INPUT_KIND),
                    ),
                )
            }
            NveNodeKind.IMAGE -> {
                val r = s.rowOf(t.image, i)
                val id = t.image!!.u32(r, NveTables.Image.SRC)
                val image = if (prev != null && prev.imageResource == id && reusable(id)) prev.image else imageOf(resources.resource(id))
                out = out.copy(image = image, imageResource = id, imageFit = t.image.u32(r, NveTables.Image.FIT))
            }
            NveNodeKind.CANVAS -> {
                val id = t.canvas!!.u32(s.rowOf(t.canvas, i), NveModuleTables.Canvas.DRAW_LIST)
                val bytes = if (prev?.drawList != null && prev.drawListResource == id && reusable(id)) {
                    prev.drawList
                } else {
                    (if (id == 0) null else resources.resource(id))?.let { copy(it.bytes) }
                }
                out = out.copy(drawList = bytes, drawListResource = id)
            }
            NveNodeKind.SCROLL -> out = out.copy(
                scrollHorizontal = t.scroll!!.u32(s.rowOf(t.scroll, i), NveTables.Scroll.AXES) == NveTables.Scroll.AXES_HORIZONTAL,
            )
            NveNodeKind.CONTAINER -> Unit
        }
        return out
    }

    private fun imageOf(res: NveResource?): NveImage? {
        if (res == null || res.kind != ImageSource.KIND) return null
        val src = try {
            ImageSource.decode(res.bytes)
        } catch (_: RuntimeException) {
            return null
        }
        return src.url?.let { NveImage.Url(it) } ?: NveImage.Encoded(copy(src.encoded))
    }

    private fun copy(b: ByteBuffer): ByteArray = ByteArray(b.remaining()).also { b.duplicate().get(it) }

    private fun box(s: RuntimeSceneReader, t: Tables, i: Int): NveBox {
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
            visible = v.u32(r, NveTables.Visual.VISIBILITY) != NveTables.Visual.VISIBILITY_HIDDEN,
        )
    }

    private fun textStyle(s: RuntimeSceneReader, t: Tables, i: Int, st: SceneSemantics.TextStyle): NveTextStyle {
        st.resolve(s, t, i)
        return NveTextStyle(st.color, st.fontSize, st.fontWeight, st.italic, st.align)
    }

    private fun semantics(s: RuntimeSceneReader, t: Tables, i: Int, disabled: Boolean): NveSemantics {
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
