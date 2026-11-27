package dev.nve.compose

import dev.nve.android.generated.NveCoreAbi
import dev.nve.android.generated.NveTables
import dev.nve.host.ImageSource
import dev.nve.host.Key64
import dev.nve.host.NativeRuntime
import dev.nve.host.RuntimeSceneReader
import dev.nve.host.TableReader
import java.nio.ByteBuffer

/** How a node is realized, from the tables it carries. */
enum class NveNodeKind { CONTAINER, TEXT, INPUT, IMAGE, CANVAS, SCROLL }

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

/** Inherited `nve.typography` text style. */
data class NveTextStyle(
    val color: Int = NveTables.Typography.DEFAULT_COLOR,
    val fontSize: Float = Float.fromBits(NveTables.Typography.DEFAULT_FONT_SIZE),
    val fontWeight: Int = NveTables.Typography.DEFAULT_FONT_WEIGHT,
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
    val rootBox: NveBox,
    val rootChildren: List<Int>,
    val nodes: Map<Int, NveNode>,
) {
    companion object {
        val EMPTY = NveSceneSnapshot(-1L, NveBox(), emptyList(), emptyMap())
    }
}

/** Resource wire bytes of the frame being captured (`null` when the id is not live). */
fun interface NveResourceLookup {
    fun resource(id: Int): NveResource?
}

class NveResource(val kind: Key64, val flags: Long, val bytes: ByteBuffer) {
    val ephemeral: Boolean get() = (flags and NativeRuntime.RESOURCE_FLAG_EPHEMERAL) != 0L
}

/** Builds [NveSceneSnapshot]s from runtime frames; unchanged resources are reused, not re-copied. */
object NveSceneCapture {
    /** Node index of `NodeId::ROOT`. */
    const val ROOT_INDEX = 1

    /** `Typography::SET_*` bits: color, font size, font weight, font style, text align. */
    private const val TYPOGRAPHY_FIELDS = (1 shl 0) or (1 shl 1) or (1 shl 6) or (1 shl 7) or (1 shl 11)

    /** `EventMask::PRESS`. */
    private const val PRESS_MASK = 1 shl (NveCoreAbi.EVENT_PRESS - 1)

    private class Tables(s: RuntimeSceneReader) {
        val visual: TableReader? = s.findTable(NveTables.Visual.KEY)
        val typography: TableReader? = s.findTable(NveTables.Typography.KEY)
        val text: TableReader? = s.findTable(NveTables.Text.KEY)
        val input: TableReader? = s.findTable(NveTables.TextInput.KEY)
        val image: TableReader? = s.findTable(NveTables.Image.KEY)
        val canvas: TableReader? = s.findTable(NveTables.Canvas.KEY)
        val scroll: TableReader? = s.findTable(NveTables.Scroll.KEY)
        val interaction: TableReader? = s.findTable(NveTables.Interaction.KEY)
        val accessibility: TableReader? = s.findTable(NveTables.Accessibility.KEY)
        val debugId: TableReader? = s.findTable(NveTables.DebugId.KEY)
    }

    fun capture(s: RuntimeSceneReader, resources: NveResourceLookup, previous: NveSceneSnapshot): NveSceneSnapshot {
        val t = Tables(s)
        val nodes = HashMap<Int, NveNode>(s.nodeCapacity())
        for (i in ROOT_INDEX + 1 until s.nodeCapacity()) {
            if (s.isLive(i)) nodes[s.nodeId(i)] = node(s, t, i, resources, previous.nodes[s.nodeId(i)])
        }
        val root = ROOT_INDEX
        val rootChildren = if (s.isLive(root)) children(s, root) else emptyList()
        return NveSceneSnapshot(s.frameId(), if (s.isLive(root)) box(s, t, root) else NveBox(), rootChildren, nodes)
    }

    private fun children(s: RuntimeSceneReader, i: Int): List<Int> = List(s.childCount(i)) { s.childId(i, it) }

    private fun kind(s: RuntimeSceneReader, t: Tables, i: Int): NveNodeKind = when {
        s.rowOf(t.input, i) >= 0 -> NveNodeKind.INPUT
        s.rowOf(t.image, i) >= 0 -> NveNodeKind.IMAGE
        s.rowOf(t.canvas, i) >= 0 -> NveNodeKind.CANVAS
        s.rowOf(t.scroll, i) >= 0 -> NveNodeKind.SCROLL
        s.rowOf(t.text, i) >= 0 && s.childCount(i) == 0 -> NveNodeKind.TEXT
        else -> NveNodeKind.CONTAINER
    }

    private fun node(s: RuntimeSceneReader, t: Tables, i: Int, resources: NveResourceLookup, prev: NveNode?): NveNode {
        val kind = kind(s, t, i)
        val parentId = s.parent(i)
        val p = if (parentId == 0) -1 else s.resolve(parentId)
        val px = if (p > 1) s.x(p) else 0f
        val py = if (p > 1) s.y(p) else 0f
        val irow = s.rowOf(t.interaction, i)
        val disabled = irow >= 0 && t.interaction!!.bool(irow, NveTables.Interaction.DISABLED)
        val pressable = irow >= 0 && !disabled &&
            (t.interaction!!.u32(irow, NveTables.Interaction.EVENT_MASK) and PRESS_MASK) != 0
        var out = NveNode(
            id = s.nodeId(i), kind = kind, x = s.x(i) - px, y = s.y(i) - py,
            width = maxOf(0f, s.width(i)), height = maxOf(0f, s.height(i)),
            box = box(s, t, i), semantics = semantics(s, t, i, disabled), pressable = pressable,
            children = children(s, i),
        )
        when (kind) {
            NveNodeKind.TEXT -> out = out.copy(
                text = s.text(t.text, s.rowOf(t.text, i), NveTables.Text.TEXT), textStyle = textStyle(s, t, i),
            )
            NveNodeKind.INPUT -> {
                val r = s.rowOf(t.input, i)
                val input = t.input!!
                out = out.copy(
                    textStyle = textStyle(s, t, i),
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
                val image = if (prev != null && prev.imageResource == id && id != 0) prev.image else imageOf(resources.resource(id))
                out = out.copy(image = image, imageResource = id, imageFit = t.image.u32(r, NveTables.Image.FIT))
            }
            NveNodeKind.CANVAS -> {
                val id = t.canvas!!.u32(s.rowOf(t.canvas, i), NveTables.Canvas.DRAW_LIST)
                val res = if (id == 0) null else resources.resource(id)
                val bytes = if (prev?.drawList != null && prev.drawListResource == id && res?.ephemeral != true) {
                    prev.drawList
                } else {
                    res?.let { copy(it.bytes) }
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
            borderWidth = maxOf(0f, v.f32(r, NveTables.Visual.BORDER_WIDTH)),
            radius = maxOf(0f, v.f32(r, NveTables.Visual.BORDER_RADIUS)),
            opacity = v.f32(r, NveTables.Visual.OPACITY).coerceIn(0f, 1f),
            zIndex = v.i32(r, NveTables.Visual.Z_INDEX),
            visible = v.u32(r, NveTables.Visual.VISIBILITY) != NveTables.Visual.VISIBILITY_HIDDEN,
        )
    }

    /** Explicit fields win; the rest inherit from the nearest ancestor that sets them (O(depth)). */
    fun textStyle(s: RuntimeSceneReader, typography: TableReader?, index: Int): NveTextStyle {
        var style = NveTextStyle()
        var pending = TYPOGRAPHY_FIELDS
        var i = index
        while (i > 0 && pending != 0 && typography != null) {
            val r = s.rowOf(typography, i)
            if (r >= 0) {
                val set = typography.u32(r, NveTables.Typography.SET) and pending
                if (set and (1 shl 0) != 0) style = style.copy(color = typography.color(r, NveTables.Typography.COLOR))
                if (set and (1 shl 1) != 0) style = style.copy(fontSize = typography.f32(r, NveTables.Typography.FONT_SIZE))
                if (set and (1 shl 6) != 0) style = style.copy(fontWeight = typography.u32(r, NveTables.Typography.FONT_WEIGHT))
                if (set and (1 shl 7) != 0) {
                    style = style.copy(italic = typography.u32(r, NveTables.Typography.FONT_STYLE) == NveTables.Typography.FONT_STYLE_ITALIC)
                }
                if (set and (1 shl 11) != 0) style = style.copy(align = typography.u32(r, NveTables.Typography.TEXT_ALIGN))
                pending = pending and set.inv()
            }
            val parent = s.parent(i)
            i = if (parent == 0) 0 else s.resolve(parent)
        }
        return style
    }

    private fun textStyle(s: RuntimeSceneReader, t: Tables, i: Int): NveTextStyle = textStyle(s, t.typography, i)

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
