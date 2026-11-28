package dev.nve.compose

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.ScrollState
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicText
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.focus.focusProperties
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.RectangleShape
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.disabled
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTag
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.dp
import androidx.compose.ui.zIndex
import dev.nve.host.Rgba
import dev.nve.host.generated.NveTables
import kotlin.math.roundToInt

/**
 * Renders a runtime surface: every node of [NveSurfaceController.scene] as a positioned
 * composable (text, text field, image, canvas draw list, scroll container, box), with runtime
 * layout measured by Compose's own text stack and input sent back as runtime events.
 */
@Composable
fun NveSurface(controller: NveSurfaceController, modifier: Modifier = Modifier) {
    val density = LocalDensity.current
    val measurer = rememberTextMeasurer()
    DisposableEffect(controller, measurer, density) {
        controller.setTextMeasurer(NveComposeTextMeasurer(measurer, density))
        onDispose { controller.setTextMeasurer(null) }
    }
    val scene = controller.scene
    val root = scene.rootBox
    Box(
        modifier
            .onSizeChanged { controller.setViewport(it.width / density.density, it.height / density.density, density.density) }
            .nveLayer(root)
            .nveBox(root),
    ) {
        for (id in scene.rootChildren) NveNodeContent(scene, id, controller, root.visible)
    }
}

/** Observes which nodes (re)compose; tests only (`null` in apps). */
internal val LocalNodeCompositions = staticCompositionLocalOf<((Int) -> Unit)?> { null }

/**
 * One node, keyed by `(scene epoch, node id)`: a new epoch reuses ids for unrelated nodes, so no
 * remembered state (scroll position, field text, focus) survives into it. Reads only its own
 * slot, so a frame that rebuilt other nodes skips it. [parentVisible]: every ancestor is visible.
 */
@Composable
private fun NveNodeContent(scene: NveSceneState, id: Int, controller: NveSurfaceController, parentVisible: Boolean) {
    val slot = scene.slot(id) ?: return
    key(scene.sceneEpoch, id) {
        val node = slot.value
        LocalNodeCompositions.current?.invoke(id)
        val density = LocalDensity.current
        // `visibility: hidden` (here or on any ancestor) keeps the box but takes no input: no
        // click, no focus, no typing, no scrolling, no semantics.
        val visible = parentVisible && node.box.visible
        var base = Modifier
            .offset(node.x.dp, node.y.dp)
            .size(node.width.dp, node.height.dp)
        if (node.box.zIndex != 0) base = base.zIndex(node.box.zIndex.toFloat())
        base = base
            .nveLayer(node.box)
            .nveBox(node.box)
            .nveSemantics(node, visible)
            .then(if (visible && node.pressable) Modifier.clickable { controller.press(id, node.width / 2f, node.height / 2f) } else Modifier)
            .then(if (visible) Modifier else Modifier.focusProperties { canFocus = false })
        when (node.kind) {
            NveNodeKind.TEXT -> BasicText(node.text, base, style = node.textStyle.toCompose(density))
            NveNodeKind.INPUT -> NveTextField(node, base, controller, enabled = visible && !node.semantics.disabled)
            NveNodeKind.IMAGE -> NveImageContent(node, base)
            NveNodeKind.CANVAS -> {
                val painter = remember { NveDrawListPainter() }
                Canvas(base) { node.drawList?.let { painter.draw(this, it) } }
            }
            NveNodeKind.SCROLL -> NveScrollContainer(scene, node, base, controller, visible)
            NveNodeKind.CONTAINER -> Box(base) { for (child in node.children) NveNodeContent(scene, child, controller, visible) }
        }
    }
}

@Composable
private fun NveScrollContainer(scene: NveSceneState, node: NveNode, base: Modifier, controller: NveSurfaceController, visible: Boolean) {
    val across = rememberScrollState()
    val down = rememberScrollState()
    val contentW = node.scrollContentWidth
    val contentH = node.scrollContentHeight
    val density = LocalDensity.current.density
    // The collector outlives recompositions: it must read the latest content size, not the first.
    val content by rememberUpdatedState(floatArrayOf(contentW, contentH))
    // The position (dp, both axes) the runtime is known to be at: user deltas are measured
    // against it, so a missed emission never lets the runtime drift from the native position.
    val reported = remember { FloatArray(2) }
    LaunchedEffect(node.id, node.scrollAxes, density) {
        trackScroll(across, down, node, controller, density, reported) { content }
    }
    // Runtime / app scroll-to and restore: follow `scroll_offsets` unless the user is scrolling.
    val target = node.scrollOffset
    LaunchedEffect(node.id, node.scrollAxes, target, density) {
        if (node.scrollsHorizontally && !across.isScrollInProgress) {
            reported[0] = target.x
            val x = (target.x * density).roundToInt()
            if (across.value != x) across.scrollTo(x)
        }
        if (node.scrollsVertically && !down.isScrollInProgress) {
            reported[1] = target.y
            val y = (target.y * density).roundToInt()
            if (down.value != y) down.scrollTo(y)
        }
    }
    var scroller = base
    if (node.scrollsVertically) scroller = scroller.verticalScroll(down, enabled = visible)
    if (node.scrollsHorizontally) scroller = scroller.horizontalScroll(across, enabled = visible)
    // `ScrollAxes.NONE` (`overflow: hidden`): no user scrolling, but it clips its content and the
    // runtime still scrolls it programmatically (a web scroll-to), so the content shifts by it.
    val clipped = node.scrollAxes == NveTables.Values.ScrollAxes.NONE
    if (clipped) scroller = scroller.clipToBounds()
    val box = Modifier.size(contentW.dp, contentH.dp)
    Box(scroller) {
        Box(if (clipped) box.offset((-target.x).dp, (-target.y).dp) else box) {
            for (child in node.children) NveNodeContent(scene, child, controller, visible)
        }
    }
}

/**
 * Reports user scrolls of [node] with both axes read together (a `BOTH` container never sends
 * a stale `0` for the other axis), as deltas from [reported]. The emission right after a
 * gesture / fling ends still counts; any other change (runtime scroll-to, a clamp from a content
 * resize, which the runtime applies on its own) is adopted without an event.
 */
private suspend fun trackScroll(
    across: ScrollState,
    down: ScrollState,
    node: NveNode,
    controller: NveSurfaceController,
    density: Float,
    reported: FloatArray,
    content: () -> FloatArray,
) {
    if (!node.scrollsHorizontally && !node.scrollsVertically) return
    var wasScrolling = false
    snapshotFlow { Triple(across.value, down.value, across.isScrollInProgress || down.isScrollInProgress) }
        .collect { (ax, ay, scrolling) ->
            val x = if (node.scrollsHorizontally) ax / density else reported[0]
            val y = if (node.scrollsVertically) ay / density else reported[1]
            val user = scrolling || wasScrolling
            wasScrolling = scrolling
            val dx = x - reported[0]
            val dy = y - reported[1]
            reported[0] = x
            reported[1] = y
            if (!user || (dx == 0f && dy == 0f)) return@collect
            val (w, h) = content()
            controller.scrolled(node.id, dx, dy, x, y, w, h)
        }
}

@Composable
private fun NveImageContent(node: NveNode, base: Modifier) {
    val context = LocalContext.current
    val bitmap = remember(node.imageResource, node.image) { node.image?.let { decodeImage(context, it) } }
    if (bitmap == null) {
        Box(base)
        return
    }
    Image(bitmap.asImageBitmap(), contentDescription = null, modifier = base, contentScale = contentScale(node.imageFit))
}

internal fun contentScale(fit: Int): ContentScale = when (fit) {
    NveTables.Values.ImageFit.CONTAIN -> ContentScale.Fit
    NveTables.Values.ImageFit.FILL -> ContentScale.FillBounds
    NveTables.Values.ImageFit.NONE -> ContentScale.None
    NveTables.Values.ImageFit.SCALE_DOWN -> ContentScale.Inside
    else -> ContentScale.Crop
}

/**
 * Inline / `data:` bytes ([dev.nve.host.ImageSource.bytes], decoded at capture), `asset://`, and
 * `file://` URLs; `null` when unreadable.
 */
internal fun decodeImage(context: Context, image: NveImage): Bitmap? = try {
    when (image) {
        is NveImage.Encoded -> BitmapFactory.decodeByteArray(image.bytes, 0, image.bytes.size)
        is NveImage.Url -> {
            val url = image.url
            when {
                url.startsWith("asset://") -> context.assets.open(url.removePrefix("asset://")).use(BitmapFactory::decodeStream)
                url.startsWith("file://") -> BitmapFactory.decodeFile(url.removePrefix("file://"))
                else -> null
            }
        }
    }
} catch (_: Exception) {
    null
}

internal fun argb(rgba: Int): Color = Color(Rgba.toArgb(rgba))

/** Opacity / `visibility: hidden` as one layer, only when the box needs it. */
private fun Modifier.nveLayer(box: NveBox): Modifier {
    val alpha = if (box.visible) box.opacity else 0f
    return if (alpha >= 1f) this else graphicsLayer { this.alpha = alpha }
}

/** Background, border, and corner clip; a plain unstyled box adds nothing (no shape allocated). */
private fun Modifier.nveBox(box: NveBox): Modifier {
    val fill = (box.background ushr 24) != 0
    val stroke = box.borderWidth > 0f && (box.borderColor ushr 24) != 0
    if (!fill && !stroke && box.radius <= 0f) return this
    val shape: Shape = if (box.radius > 0f) RoundedCornerShape(box.radius.dp) else RectangleShape
    var m = this
    if (fill) m = m.background(argb(box.background), shape)
    if (stroke) m = m.border(box.borderWidth.dp, argb(box.borderColor), shape)
    if (box.radius > 0f) m = m.clip(shape)
    return m
}

private fun Modifier.nveSemantics(node: NveNode, visible: Boolean): Modifier {
    val s = node.semantics
    if (s.hidden || !visible) return clearAndSetSemantics {}
    if (s.label.isEmpty() && s.testId.isEmpty() && !s.disabled && s.role == null) return this
    return semantics {
        if (s.label.isNotEmpty()) contentDescription = s.label
        if (s.testId.isNotEmpty()) testTag = s.testId
        if (s.disabled) disabled()
        when (s.role) {
            NveTables.Values.AriaRole.BUTTON, NveTables.Values.AriaRole.LINK -> role = Role.Button
            NveTables.Values.AriaRole.CHECKBOX -> role = Role.Checkbox
            NveTables.Values.AriaRole.SWITCH -> role = Role.Switch
            NveTables.Values.AriaRole.RADIO -> role = Role.RadioButton
            NveTables.Values.AriaRole.IMAGE -> role = Role.Image
            NveTables.Values.AriaRole.TAB -> role = Role.Tab
            NveTables.Values.AriaRole.HEADING -> heading()
        }
    }
}
