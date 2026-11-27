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
import androidx.compose.runtime.key
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.focusProperties
import androidx.compose.ui.graphics.Color
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
    Box(
        modifier
            .onSizeChanged { controller.setViewport(it.width / density.density, it.height / density.density, density.density) }
            .nveBox(scene.rootBox),
    ) {
        for (id in scene.rootChildren) NveNodeContent(scene, id, controller)
    }
}

/**
 * One node, keyed by `(scene epoch, node id)`: a new epoch reuses ids for unrelated nodes, so no
 * remembered state (scroll position, field text, focus) survives into it.
 */
@Composable
private fun NveNodeContent(scene: NveSceneSnapshot, id: Int, controller: NveSurfaceController) {
    val node = scene.nodes[id] ?: return
    key(scene.sceneEpoch, id) {
        val density = LocalDensity.current
        val visible = node.box.visible
        val base = Modifier
            .offset(node.x.dp, node.y.dp)
            .size(node.width.dp, node.height.dp)
            .zIndex(node.box.zIndex.toFloat())
            .graphicsLayer { alpha = if (visible) node.box.opacity else 0f }
            .nveBox(node.box)
            .nveSemantics(node)
            // `visibility: hidden` keeps the box but takes no input: no click, no focus, no semantics.
            .then(if (visible && node.pressable) Modifier.clickable { controller.press(id, node.width / 2f, node.height / 2f) } else Modifier)
            .then(if (visible) Modifier else Modifier.focusProperties { canFocus = false })
        when (node.kind) {
            NveNodeKind.TEXT -> BasicText(node.text, base, style = node.textStyle.toCompose(density))
            NveNodeKind.INPUT -> NveTextField(node, base, controller, enabled = visible && !node.semantics.disabled)
            NveNodeKind.IMAGE -> NveImageContent(node, base)
            NveNodeKind.CANVAS -> Canvas(base) { node.drawList?.let { NveDrawListRenderer.drawInto(this, it) } }
            NveNodeKind.SCROLL -> NveScrollContainer(scene, node, base, controller)
            NveNodeKind.CONTAINER -> Box(base) { for (child in node.children) NveNodeContent(scene, child, controller) }
        }
    }
}

@Composable
private fun NveScrollContainer(scene: NveSceneSnapshot, node: NveNode, base: Modifier, controller: NveSurfaceController) {
    val across = rememberScrollState()
    var contentW = node.width
    var contentH = node.height
    for (child in node.children) {
        scene.nodes[child]?.let {
            contentW = maxOf(contentW, it.x + it.width)
            contentH = maxOf(contentH, it.y + it.height)
        }
    }
    val density = LocalDensity.current.density
    // The collector outlives recompositions: it must read the latest content size, not the first.
    val content by rememberUpdatedState(floatArrayOf(contentW, contentH))
    val down = rememberScrollState()
    LaunchedEffect(scene.sceneEpoch, node.id, across, node.scrollAxes, density) {
        if (node.scrollsHorizontally) trackScroll(across, node, controller, density, horizontal = true) { content }
    }
    LaunchedEffect(scene.sceneEpoch, node.id, down, node.scrollAxes, density) {
        if (node.scrollsVertically) trackScroll(down, node, controller, density, horizontal = false) { content }
    }
    // Runtime / app scroll-to and restore: follow `scroll_offsets` unless the user is scrolling.
    val target = scene.scrollOffset(node.id)
    LaunchedEffect(scene.sceneEpoch, node.id, target, density) {
        val x = (target.x * density).toInt()
        val y = (target.y * density).toInt()
        if (node.scrollsHorizontally && !across.isScrollInProgress && across.value != x) across.scrollTo(x)
        if (node.scrollsVertically && !down.isScrollInProgress && down.value != y) down.scrollTo(y)
    }
    var scroller = base
    if (node.scrollsVertically) scroller = scroller.verticalScroll(down)
    if (node.scrollsHorizontally) scroller = scroller.horizontalScroll(across)
    if (node.scrollAxes == NveTables.Values.ScrollAxes.NONE) scroller = scroller.clip(RoundedCornerShape(0.dp))
    Box(scroller) {
        Box(Modifier.size(contentW.dp, contentH.dp)) {
            for (child in node.children) NveNodeContent(scene, child, controller)
        }
    }
}

private suspend fun trackScroll(
    state: ScrollState,
    node: NveNode,
    controller: NveSurfaceController,
    density: Float,
    horizontal: Boolean,
    content: () -> FloatArray,
) {
    var last = state.value
    snapshotFlow { state.value }.collect { px ->
        val delta = (px - last) / density
        last = px
        // Clamps from content resizes are not user scrolls (the runtime clamps on its own).
        if (delta == 0f || !state.isScrollInProgress) return@collect
        val pos = px / density
        val (w, h) = content()
        if (horizontal) {
            controller.scrolled(node.id, delta, 0f, pos, 0f, w, h)
        } else {
            controller.scrolled(node.id, 0f, delta, 0f, pos, w, h)
        }
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

private fun Modifier.nveBox(box: NveBox): Modifier {
    val shape = RoundedCornerShape(box.radius.dp)
    var m = this
    if ((box.background ushr 24) != 0) m = m.background(argb(box.background), shape)
    if (box.borderWidth > 0f && (box.borderColor ushr 24) != 0) m = m.border(box.borderWidth.dp, argb(box.borderColor), shape)
    if (box.radius > 0f) m = m.clip(shape)
    return m
}

private fun Modifier.nveSemantics(node: NveNode): Modifier {
    val s = node.semantics
    if (s.hidden || !node.box.visible) return clearAndSetSemantics {}
    if (s.label.isEmpty() && s.testId.isEmpty() && !s.disabled && s.role == NveTables.Values.AriaRole.NONE) return this
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
