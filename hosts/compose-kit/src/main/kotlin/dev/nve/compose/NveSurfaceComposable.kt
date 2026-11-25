package dev.nve.compose

import android.content.Context
import android.graphics.BitmapFactory
import android.view.MotionEvent
import android.view.View
import android.widget.FrameLayout
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicText
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.disabled
import androidx.compose.ui.semantics.invisibleToUser
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.zIndex
import java.util.Locale

/**
 * Custom Android host container performing native `onMeasure` intrinsic layout negotiation,
 * viewport resize forwarding, and `requestDisallowInterceptTouchEvent` nested-scroll arbitration
 * with parent Compose containers.
 */
class NveSurfaceView(
    context: Context,
    private val controller: NveSurfaceController,
) : FrameLayout(context) {

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        val density = resources.displayMetrics.density.coerceAtLeast(1f)
        val widthMode = View.MeasureSpec.getMode(widthMeasureSpec)
        val heightMode = View.MeasureSpec.getMode(heightMeasureSpec)
        val widthPx = View.MeasureSpec.getSize(widthMeasureSpec)
        val heightPx = View.MeasureSpec.getSize(heightMeasureSpec)

        val minW = if (widthMode == View.MeasureSpec.EXACTLY) widthPx / density else 0f
        val maxW = if (widthMode == View.MeasureSpec.UNSPECIFIED) Float.POSITIVE_INFINITY else widthPx / density
        val minH = if (heightMode == View.MeasureSpec.EXACTLY) heightPx / density else 0f
        val maxH = if (heightMode == View.MeasureSpec.UNSPECIFIED) Float.POSITIVE_INFINITY else heightPx / density

        val measured = controller.measureIntrinsic(minW, minH, maxW, maxH)
        if (measured != null) {
            val resolvedW = View.resolveSize((measured.first * density).toInt(), widthMeasureSpec)
            val resolvedH = View.resolveSize((measured.second * density).toInt(), heightMeasureSpec)
            setMeasuredDimension(resolvedW, resolvedH)
        } else {
            super.onMeasure(widthMeasureSpec, heightMeasureSpec)
        }
    }

    override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) {
        super.onSizeChanged(w, h, oldw, oldh)
        if (w > 0 && h > 0) {
            val density = resources.displayMetrics.density.coerceAtLeast(1f)
            controller.resize(
                width = w / density,
                height = h / density,
                scaleFactor = density,
            )
            controller.requestRender()
        }
    }

    override fun onTouchEvent(event: MotionEvent): Boolean {
        val phase = when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> 1
            MotionEvent.ACTION_MOVE -> 2
            MotionEvent.ACTION_UP -> 3
            else -> 4
        }
        val pointerId = event.getPointerId(event.actionIndex)
        val disposition = controller.arbitrateTouch(
            pointerId = pointerId,
            phase = phase,
            x = event.x,
            y = event.y,
        )
        return when (disposition) {
            NveGestureDisposition.CLAIM -> {
                parent?.requestDisallowInterceptTouchEvent(true)
                if (event.actionMasked == MotionEvent.ACTION_UP) {
                    val density = resources.displayMetrics.density.coerceAtLeast(1f)
                    controller.dispatchPress(
                        nodeId = 0,
                        x = event.x / density,
                        y = event.y / density,
                        timestampUs = event.eventTime * 1_000L,
                    )
                } else {
                    controller.drainAndForwardEvents()
                }
                true
            }
            NveGestureDisposition.DEFER_TO_PARENT -> {
                parent?.requestDisallowInterceptTouchEvent(false)
                false
            }
            NveGestureDisposition.PASS_THROUGH -> false
        }
    }
}

/**
 * Maps an NVE accessibility role string or node kind to a Compose [Role].
 */
fun resolveComposeAccessibilityRole(node: NveRenderNodeSnapshot): Role? {
    val explicit = node.metadata.accessibilityRole.trim().lowercase(Locale.US)
    if (explicit.isNotEmpty()) {
        return when (explicit) {
            "button", "pressable", "link" -> Role.Button
            "checkbox" -> Role.Checkbox
            "switch", "toggle" -> Role.Switch
            "radio", "radiobutton" -> Role.RadioButton
            "tab" -> Role.Tab
            "image", "img" -> Role.Image
            "dropdown", "dropdownlist", "combobox" -> Role.DropdownList
            else -> null
        }
    }
    return when (node.kind) {
        NveSceneBuffer.KIND_PRESSABLE -> Role.Button
        NveSceneBuffer.KIND_IMAGE -> Role.Image
        else -> null
    }
}

/**
 * Maps an NVE `resizeMode` wire code (`0=cover, 1=contain, 2=stretch, 3=center`) to Compose [ContentScale].
 */
fun contentScaleForResizeMode(resizeMode: Int): ContentScale = when (resizeMode) {
    NveNodeMetadata.RESIZE_MODE_CONTAIN -> ContentScale.Fit
    NveNodeMetadata.RESIZE_MODE_STRETCH -> ContentScale.FillBounds
    NveNodeMetadata.RESIZE_MODE_CENTER -> ContentScale.None
    else -> ContentScale.Crop
}

/**
 * Applies TalkBack / Jetpack Compose [Modifier.semantics] (`contentDescription`, `role`,
 * `disabled`, `stateDescription`, and optional `testTag`) for an [NveRenderNodeSnapshot].
 */
@Suppress("DEPRECATION")
fun Modifier.nveSemantics(node: NveRenderNodeSnapshot): Modifier {
    val meta = node.metadata
    val resolvedRole = resolveComposeAccessibilityRole(node)
    val effectiveStateDesc = when {
        meta.accessibilityStateDescription.isNotEmpty() -> meta.accessibilityStateDescription
        meta.accessibilityHint.isNotEmpty() -> meta.accessibilityHint
        else -> ""
    }
    val hasSemantics = meta.accessibilityLabel.isNotEmpty() ||
        resolvedRole != null ||
        meta.disabled ||
        effectiveStateDesc.isNotEmpty() ||
        meta.accessibilityHidden

    var result = this
    if (meta.testId.isNotEmpty()) {
        result = result.testTag(meta.testId)
    }
    if (!hasSemantics) {
        return result
    }
    return result.semantics {
        if (meta.accessibilityLabel.isNotEmpty()) {
            contentDescription = meta.accessibilityLabel
        }
        if (resolvedRole != null) {
            role = resolvedRole
        }
        if (meta.disabled) {
            disabled()
        }
        if (effectiveStateDesc.isNotEmpty()) {
            stateDescription = effectiveStateDesc
        }
        if (meta.accessibilityHidden) {
            invisibleToUser()
        }
    }
}

/**
 * Resolves binary `NVE_DRAW1` bytes for a [NveSceneBuffer.KIND_SKIA_CANVAS] node from either
 * direct zero-copy `drawListBytes`, batched metadata bytes, or a `nve-draw1;b64,...` text attribute.
 */
fun resolveCanvasDrawListBytes(node: NveRenderNodeSnapshot): ByteArray? {
    val direct = node.drawListBytes
    if (direct != null && NveDrawListRenderer.validate(direct) == null) {
        return direct
    }
    val fromMeta = node.metadata.drawListBytes
    if (fromMeta != null && NveDrawListRenderer.validate(fromMeta) == null) {
        return fromMeta
    }
    if (node.text.isNotEmpty()) {
        return NveDrawListRenderer.decodeBase64Attr(node.text)
    }
    return null
}

/**
 * Decodes a `data:image/<mime>;base64,...` URI into a Compose [ImageBitmap] with zero JSON parsing.
 */
fun decodeDataUriImageBitmap(src: String): ImageBitmap? {
    if (src.isEmpty()) return null
    val lower = src.lowercase(Locale.US)
    if (!lower.startsWith("data:image/")) return null
    val comma = src.indexOf(',')
    if (comma <= 0 || comma + 1 >= src.length) return null
    val header = lower.substring(0, comma)
    if (!header.contains(";base64")) return null
    val bytes = NveDrawListRenderer.decodeBase64Strict(src.substring(comma + 1)) ?: return null
    if (bytes.isEmpty()) return null
    return try {
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size)?.asImageBitmap()
    } catch (_: Throwable) {
        null
    }
}

/**
 * Renders a retained NVE scene graph ([NveSurfaceState.mountedNodes]) directly with Jetpack Compose
 * primitives, including zero-alloc `NVE_DRAW1` [Canvas] (`KIND_SKIA_CANVAS`), [Image] (`KIND_IMAGE`),
 * [BasicTextField] (`KIND_TEXT_INPUT`), [BasicText] (`KIND_TEXT`), and [Modifier.semantics].
 */
@Composable
fun NveSceneCanvas(
    state: NveSurfaceState,
    modifier: Modifier = Modifier,
    rootNodeId: Int = 1,
    controller: NveSurfaceController? = null,
    imageLoader: ((String) -> ImageBitmap?)? = null,
) {
    NveSceneCanvas(
        nodes = state.mountedNodes,
        modifier = modifier,
        rootNodeId = rootNodeId,
        onPress = controller?.let { ctrl ->
            { nodeId, x, y -> ctrl.dispatchPress(nodeId, x, y) }
        },
        onTextChange = controller?.let { ctrl ->
            { nodeId, text -> ctrl.dispatchTextChange(nodeId, text) }
        },
        onSubmit = controller?.let { ctrl ->
            { nodeId, text ->
                ctrl.enqueueEvent(
                    NveEventRingCodec.custom(
                        surfaceId = ctrl.surfaceId,
                        nodeId = nodeId,
                        name = "submit",
                        data = text.toByteArray(Charsets.UTF_8),
                    ),
                )
                ctrl.flushEvents()
            }
        },
        imageLoader = imageLoader,
    )
}

/**
 * Declarative Jetpack Compose renderer for an NVE scene node map (`nodeId -> NveRenderNodeSnapshot`).
 */
@Composable
fun NveSceneCanvas(
    nodes: Map<Int, NveRenderNodeSnapshot>,
    modifier: Modifier = Modifier,
    rootNodeId: Int = 1,
    onPress: ((nodeId: Int, x: Float, y: Float) -> Unit)? = null,
    onTextChange: ((nodeId: Int, text: String) -> Unit)? = null,
    onSubmit: ((nodeId: Int, text: String) -> Unit)? = null,
    imageLoader: ((String) -> ImageBitmap?)? = null,
) {
    val root = nodes[rootNodeId] ?: nodes.values.firstOrNull { it.kind == NveSceneBuffer.KIND_ROOT }
    Box(modifier = modifier) {
        if (root != null) {
            NveSceneNodeComposable(
                node = root,
                nodes = nodes,
                isRoot = true,
                onPress = onPress,
                onTextChange = onTextChange,
                onSubmit = onSubmit,
                imageLoader = imageLoader,
            )
        }
    }
}

@Composable
private fun NveSceneNodeComposable(
    node: NveRenderNodeSnapshot,
    nodes: Map<Int, NveRenderNodeSnapshot>,
    isRoot: Boolean,
    onPress: ((nodeId: Int, x: Float, y: Float) -> Unit)?,
    onTextChange: ((nodeId: Int, text: String) -> Unit)?,
    onSubmit: ((nodeId: Int, text: String) -> Unit)?,
    imageLoader: ((String) -> ImageBitmap?)?,
) {
    val safeW = if (node.width.isFinite() && node.width > 0f) node.width else 0f
    val safeH = if (node.height.isFinite() && node.height > 0f) node.height else 0f
    val safeX = if (node.x.isFinite()) node.x else 0f
    val safeY = if (node.y.isFinite()) node.y else 0f
    val safeRadius = if (node.borderRadius.isFinite() && node.borderRadius > 0f) node.borderRadius else 0f
    val safeBorderW = if (node.borderWidth.isFinite() && node.borderWidth > 0f) node.borderWidth else 0f
    val shape = RoundedCornerShape(safeRadius.dp)

    var mod: Modifier = if (isRoot) {
        Modifier.fillMaxSize()
    } else {
        Modifier
            .offset(x = safeX.dp, y = safeY.dp)
            .size(width = safeW.dp, height = safeH.dp)
    }

    if (node.zIndex != 0) {
        mod = mod.zIndex(node.zIndex.toFloat())
    }
    if (node.opacityU8 < 255) {
        mod = mod.alpha(node.opacity.coerceIn(0f, 1f))
    }
    if (node.clipsBounds || safeRadius > 0f) {
        mod = mod.clip(shape)
    }
    if ((node.bgRgba ushr 24) != 0) {
        mod = mod.background(NveDrawListRenderer.rgbaToColor(node.bgRgba), shape)
    }
    if (safeBorderW > 0f && (node.borderRgba ushr 24) != 0) {
        mod = mod.border(safeBorderW.dp, NveDrawListRenderer.rgbaToColor(node.borderRgba), shape)
    }

    mod = mod.nveSemantics(node)

    val canClick = node.isInteractive &&
        !node.metadata.disabled &&
        node.kind != NveSceneBuffer.KIND_TEXT_INPUT &&
        node.kind != NveSceneBuffer.KIND_SCROLL_VIEW
    if (canClick && onPress != null) {
        mod = mod.clickable(
            enabled = !node.metadata.disabled,
            role = resolveComposeAccessibilityRole(node),
        ) {
            onPress(node.nodeId, safeW * 0.5f, safeH * 0.5f)
        }
    }

    when (node.kind) {
        NveSceneBuffer.KIND_SKIA_CANVAS -> {
            val drawBytes = remember(node.drawListBytes, node.metadata.drawListBytes, node.text) {
                resolveCanvasDrawListBytes(node)
            }
            val reusablePath = remember { Path() }
            Canvas(modifier = mod) {
                if (drawBytes != null) {
                    NveDrawListRenderer.drawInto(
                        drawScope = this,
                        bytes = drawBytes,
                        scale = density,
                        reusablePath = reusablePath,
                    )
                }
            }
        }
        NveSceneBuffer.KIND_IMAGE -> {
            val bitmap = remember(node.text, imageLoader) {
                imageLoader?.invoke(node.text) ?: decodeDataUriImageBitmap(node.text)
            }
            val contentScale = contentScaleForResizeMode(node.metadata.resizeMode)
            if (bitmap != null) {
                Image(
                    bitmap = bitmap,
                    contentDescription = node.metadata.accessibilityLabel.ifEmpty { null },
                    modifier = mod,
                    contentScale = contentScale,
                )
            } else {
                Box(modifier = mod)
            }
        }
        NveSceneBuffer.KIND_TEXT_INPUT -> {
            val textColor = if ((node.textRgba ushr 24) != 0) {
                NveDrawListRenderer.rgbaToColor(node.textRgba)
            } else {
                Color.White
            }
            val fontSize = if (node.fontSize.isFinite() && node.fontSize > 0f) node.fontSize else 16f
            val placeholder = node.metadata.placeholder
            BasicTextField(
                value = node.text,
                onValueChange = { next ->
                    if (!node.metadata.disabled) {
                        onTextChange?.invoke(node.nodeId, next)
                    }
                },
                modifier = mod,
                enabled = !node.metadata.disabled,
                singleLine = !node.metadata.multiline,
                textStyle = TextStyle(color = textColor, fontSize = fontSize.sp),
                cursorBrush = SolidColor(textColor),
                visualTransformation = if (node.metadata.secureTextEntry) {
                    PasswordVisualTransformation()
                } else {
                    VisualTransformation.None
                },
                keyboardOptions = KeyboardOptions(
                    imeAction = if (node.metadata.multiline) ImeAction.Default else ImeAction.Done,
                ),
                keyboardActions = KeyboardActions(
                    onDone = { onSubmit?.invoke(node.nodeId, node.text) },
                    onSend = { onSubmit?.invoke(node.nodeId, node.text) },
                ),
                decorationBox = { innerTextField ->
                    Box {
                        if (node.text.isEmpty() && placeholder.isNotEmpty()) {
                            BasicText(
                                text = placeholder,
                                style = TextStyle(
                                    color = textColor.copy(alpha = 0.5f),
                                    fontSize = fontSize.sp,
                                ),
                            )
                        }
                        innerTextField()
                    }
                },
            )
        }
        NveSceneBuffer.KIND_TEXT -> {
            if (node.childIds.isEmpty()) {
                val textColor = if ((node.textRgba ushr 24) != 0) {
                    NveDrawListRenderer.rgbaToColor(node.textRgba)
                } else {
                    Color.Black
                }
                val fontSize = if (node.fontSize.isFinite() && node.fontSize > 0f) node.fontSize else 16f
                BasicText(
                    text = node.text,
                    modifier = mod,
                    style = TextStyle(color = textColor, fontSize = fontSize.sp),
                )
            } else {
                Box(modifier = mod) {
                    RenderSortedChildren(node, nodes, onPress, onTextChange, onSubmit, imageLoader)
                }
            }
        }
        NveSceneBuffer.KIND_SCROLL_VIEW -> {
            val scrollState = rememberScrollState()
            Box(modifier = mod.verticalScroll(scrollState)) {
                RenderSortedChildren(node, nodes, onPress, onTextChange, onSubmit, imageLoader)
            }
        }
        else -> {
            Box(modifier = mod) {
                RenderSortedChildren(node, nodes, onPress, onTextChange, onSubmit, imageLoader)
            }
        }
    }
}

@Composable
private fun RenderSortedChildren(
    parent: NveRenderNodeSnapshot,
    nodes: Map<Int, NveRenderNodeSnapshot>,
    onPress: ((nodeId: Int, x: Float, y: Float) -> Unit)?,
    onTextChange: ((nodeId: Int, text: String) -> Unit)?,
    onSubmit: ((nodeId: Int, text: String) -> Unit)?,
    imageLoader: ((String) -> ImageBitmap?)?,
) {
    val childIds = parent.childIds
    if (childIds.isEmpty()) return
    val children = ArrayList<NveRenderNodeSnapshot>(childIds.size)
    var needsSort = false
    for (cid in childIds) {
        val child = nodes[cid] ?: continue
        if (child.zIndex != 0) needsSort = true
        children.add(child)
    }
    if (children.isEmpty()) return
    if (needsSort && children.size > 1) {
        children.sortBy { it.zIndex }
    }
    for (child in children) {
        NveSceneNodeComposable(
            node = child,
            nodes = nodes,
            isRoot = false,
            onPress = onPress,
            onTextChange = onTextChange,
            onSubmit = onSubmit,
            imageLoader = imageLoader,
        )
    }
}

/**
 * Creates and remembers an [NveSurfaceController] tied to the composition lifecycle.
 */
@Composable
fun rememberNveSurfaceController(
    componentName: String,
    vtableSymbol: String = "nve_get_interface",
    backend: NveRenderBackend = NveRenderBackend.FABRIC,
    bridge: NveBridgeBackend = NveNativeBridge,
): NveSurfaceController {
    return remember(componentName, vtableSymbol, backend, bridge) {
        NveSurfaceController(
            componentName = componentName,
            vtableSymbol = vtableSymbol,
            backend = backend,
            bridge = bridge,
        )
    }
}

/**
 * Embeds a Native View Engine (NVE) Rust component natively inside Jetpack Compose.
 *
 * Bridges Compose state updates (`props`) via compact binary [NveBinValue], participates in
 * Compose sizing via `Modifier` + intrinsic `onMeasure`, arbitrates nested scroll gestures with
 * parent Compose scrollables, and delivers outbound binary `EventRing` events via [onEvent].
 */
@Composable
fun NveSurface(
    componentName: String,
    modifier: Modifier = Modifier,
    vtableSymbol: String = "nve_get_interface",
    backend: NveRenderBackend = NveRenderBackend.FABRIC,
    props: Map<String, Any?> = emptyMap(),
    controller: NveSurfaceController? = null,
    onEvent: ((NveEvent) -> Unit)? = null,
) {
    val ownedController = controller == null
    val activeController = controller ?: rememberNveSurfaceController(
        componentName = componentName,
        vtableSymbol = vtableSymbol,
        backend = backend,
    )

    activeController.onEventCallback = onEvent

    DisposableEffect(activeController) {
        activeController.mountIfNeeded()
        activeController.updateProps(props)
        onDispose {
            if (ownedController) {
                activeController.dispose()
            } else {
                activeController.onEventCallback = null
            }
        }
    }

    LaunchedEffect(activeController, props) {
        activeController.updateProps(props)
    }

    AndroidView(
        modifier = modifier,
        factory = { context ->
            activeController.mountIfNeeded()
            NveSurfaceView(context, activeController)
        },
        update = { view ->
            activeController.onEventCallback = onEvent
            activeController.updateProps(props)
            view.requestLayout()
        },
    )
}
