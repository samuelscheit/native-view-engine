package dev.nve.compose

import androidx.compose.runtime.MutableState
import androidx.compose.runtime.Stable
import androidx.compose.runtime.State
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshots.Snapshot
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
    /** Runtime scroll offset (dp, `scroll_offsets`) of a scroll container; `(0, 0)` when not scrolled. */
    val scrollOffset: NveScrollOffset = NveScrollOffset.ZERO,
) {
    val scrollsHorizontally: Boolean
        get() = scrollAxes == NveTables.Values.ScrollAxes.HORIZONTAL || scrollAxes == NveTables.Values.ScrollAxes.BOTH
    val scrollsVertically: Boolean
        get() = scrollAxes == NveTables.Values.ScrollAxes.VERTICAL || scrollAxes == NveTables.Values.ScrollAxes.BOTH
}

/**
 * Composition state of one runtime surface, updated in place by [NveSceneCapture] in O(changed).
 *
 * Every node lives in its own [MutableState] slot: a frame writes only the slots of nodes it
 * rebuilt, so only their composables recompose (each `NveNodeContent` reads just its own slot;
 * this holder is one stable instance, so unchanged nodes skip). Also keeps the reverse index
 * `resource id -> node ids` that lets a frame's `dirty_resources` rebuild clean nodes bound to
 * rewritten resources, and the set of containers with a published scroll offset.
 */
@Stable
class NveSceneState {
    /** Last applied frame (`0`: none yet, the next frame reconciles everything). Not observable. */
    var frameId: Long = 0L
        private set

    /** Scene epoch: a new one re-keys every node, so no remembered state survives into it. */
    var sceneEpoch: Int by mutableIntStateOf(0)
        private set

    var rootBox: NveBox by mutableStateOf(NveBox())
        private set

    var rootChildren: List<Int> by mutableStateOf(emptyList())
        private set

    private val slots = HashMap<Int, MutableState<NveNode>>()
    private val users = HashMap<Int, HashSet<Int>>()
    private var scrolled: Set<Int> = emptySet()

    /** Live node ids (not observable; for tests and tooling). */
    val ids: Set<Int> get() = slots.keys

    val size: Int get() = slots.size

    /** Current node [id] (an observable read: the caller recomposes when that node changes). */
    operator fun get(id: Int): NveNode? = slots[id]?.value

    /** Node [id]'s slot, read by its composable only. */
    internal fun slot(id: Int): State<NveNode>? = slots[id]

    /** Ids of nodes bound to resource [resourceId] (image source or draw list). */
    internal fun usersOf(resourceId: Int): Set<Int> = users[resourceId] ?: emptySet()

    internal fun put(n: NveNode) {
        val slot = slots[n.id]
        val old = slot?.value
        if (slot == null) slots[n.id] = mutableStateOf(n) else slot.value = n
        rebind(old?.imageResource ?: 0, n.imageResource, n.id)
        rebind(old?.drawListResource ?: 0, n.drawListResource, n.id)
    }

    internal fun remove(id: Int) {
        val old = slots.remove(id)?.value ?: return
        rebind(old.imageResource, 0, id)
        rebind(old.drawListResource, 0, id)
    }

    /** Drops every node not in [live] (a full reconcile). */
    internal fun retainOnly(live: Set<Int>) {
        if (live.size == slots.size) return
        for (id in slots.keys.filter { it !in live }) remove(id)
    }

    /** A new epoch: fresh slots (no node state may survive into reused ids). */
    internal fun clear() {
        slots.clear()
        users.clear()
        scrolled = emptySet()
    }

    internal fun setRoot(box: NveBox, children: List<Int>) {
        rootBox = box
        rootChildren = children
    }

    internal fun setFrame(frameId: Long, epoch: Int) {
        this.frameId = frameId
        sceneEpoch = epoch
    }

    /** Applies the frame's `scroll_offsets` in O(published + previously scrolled). */
    internal fun applyScrollOffsets(s: SceneReader) {
        if (s.scrollCount() == 0 && scrolled.isEmpty()) return
        val now = HashSet<Int>(s.scrollCount())
        for (k in 0 until s.scrollCount()) {
            val id = s.scrollNodeId(k)
            now.add(id)
            val n = this[id] ?: continue
            val offset = NveScrollOffset(s.scrollX(id), s.scrollY(id))
            if (n.scrollOffset != offset) put(n.copy(scrollOffset = offset))
        }
        for (id in scrolled) {
            if (id in now) continue
            val n = this[id] ?: continue
            if (n.scrollOffset != NveScrollOffset.ZERO) put(n.copy(scrollOffset = NveScrollOffset.ZERO))
        }
        scrolled = now
    }

    private fun rebind(old: Int, new: Int, id: Int) {
        if (old == new) return
        if (old != 0) users[old]?.let { if (it.remove(id) && it.isEmpty()) users.remove(old) }
        if (new != 0) users.getOrPut(new) { HashSet(2) }.add(id)
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
 * Applies runtime frames to a [NveSceneState] in O(changed): on the frame right after the last
 * applied one (same epoch) only removed, content-dirty (incl. inherited typography), moved, and
 * resource-dirty nodes (clean nodes bound to an id in `dirty_resources`) are rebuilt — every other
 * slot keeps its [NveNode] (and copied resource bytes) untouched. A redelivered frame is a no-op;
 * a gap rebuilds every node in place, a new epoch from fresh slots.
 */
object NveSceneCapture {
    /** Applies [s] to [into]; `false` when [s] was already applied. */
    fun capture(s: SceneReader, resources: NveResourceLookup, into: NveSceneState): Boolean {
        val relation = s.relationTo(into.frameId, into.sceneEpoch)
        if (relation == SceneReader.FRAME_SAME) return false
        // One atomic snapshot: composition never observes a half-applied frame.
        Snapshot.withMutableSnapshot { apply(s, resources, into, relation == SceneReader.FRAME_NEXT) }
        return true
    }

    private fun apply(s: SceneReader, resources: NveResourceLookup, into: NveSceneState, chained: Boolean) {
        val t = SceneSemantics.Tables().bind(s)
        val style = SceneSemantics.TextStyle()
        val root = SceneSemantics.ROOT_INDEX
        // Copied resources stay valid along an unbroken chain of one epoch, minus the ids this
        // frame (re)defined: a skipped frame's dirty list is lost, a new epoch reuses ids.
        val reusable = { id: Int -> chained && id != 0 && !s.isResourceDirty(id) }
        var rootDirty = !chained
        if (chained) {
            for (k in 0 until s.removedCount()) into.remove(s.removedId(k))
            val rebuilt = HashSet<Int>(s.dirtyCount())
            for (d in 0 until s.dirtyCount()) {
                val i = s.dirtyIndex(d)
                if (i == root) {
                    rootDirty = true
                    continue
                }
                val id = s.nodeId(i)
                into.put(node(s, t, i, resources, into[id], reusable, style))
                rebuilt.add(id)
            }
            for (m in 0 until s.movedCount()) {
                val i = s.movedIndex(m)
                if (i == root) continue
                val prev = into[s.nodeId(i)]
                into.put(prev?.let { geometry(s, i, it) } ?: node(s, t, i, resources, null, reusable, style))
            }
            // A resource rewritten in place leaves its users' rows clean: rebuild them from the index.
            for (k in 0 until s.dirtyResourceCount()) {
                for (id in into.usersOf(s.dirtyResource(k)).toList()) {
                    val i = s.resolve(id)
                    if (i <= root || !rebuilt.add(id)) continue
                    into.put(node(s, t, i, resources, into[id], reusable, style))
                }
            }
        } else {
            if (s.sceneEpoch() != into.sceneEpoch) into.clear()
            val live = HashSet<Int>(s.nodeCapacity())
            for (i in root + 1 until s.nodeCapacity()) {
                if (!s.isLive(i)) continue
                val id = s.nodeId(i)
                into.put(node(s, t, i, resources, into[id], reusable, style))
                live.add(id)
            }
            into.retainOnly(live)
        }
        if (!s.isLive(root)) {
            into.setRoot(NveBox(), emptyList())
        } else if (rootDirty) {
            into.setRoot(box(s, t, root), children(s, root))
        }
        into.applyScrollOffsets(s)
        into.setFrame(s.frameId(), s.sceneEpoch())
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
        var scrollOffset = NveScrollOffset.ZERO
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
            NveNodeKind.SCROLL -> {
                scrollAxes = t.scroll!!.u32(s.rowOf(t.scroll, i), NveTables.Scroll.AXES)
                val id = s.nodeId(i)
                scrollOffset = NveScrollOffset(s.scrollX(id), s.scrollY(id))
            }
            NveNodeKind.CONTAINER -> Unit
        }
        return NveNode(
            id = s.nodeId(i), kind = kind, x = g[0], y = g[1], width = g[2], height = g[3],
            box = box(s, t, i), semantics = semantics(s, t, i, SceneSemantics.disabled(s, t, i)),
            pressable = SceneSemantics.pressable(s, t, i), children = children(s, i),
            text = text, textStyle = textStyle, input = input, image = image, imageFit = imageFit,
            imageResource = imageResource, drawList = drawList, drawListResource = drawListResource, scrollAxes = scrollAxes,
            scrollOffset = scrollOffset,
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
