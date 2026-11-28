package dev.nve.compose

import androidx.compose.foundation.text.BasicText
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.TextMeasurer
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import dev.nve.host.NativeRuntime
import dev.nve.host.SceneSemantics
import dev.nve.host.Utf8
import dev.nve.host.generated.NveTables
import kotlin.math.ceil

/** Runtime text style → Compose: sizes are dp (logical points), independent of the font scale. */
internal fun NveTextStyle.toCompose(density: Density): TextStyle = with(density) {
    TextStyle(
        color = argb(color),
        fontSize = fontSize.dp.toSp(),  // sanitized by SceneSemantics.TextStyle
        fontWeight = FontWeight(fontWeight.coerceIn(1, 1000)),
        fontStyle = if (italic) FontStyle.Italic else FontStyle.Normal,
        textAlign = when (align) {
            NveTables.Values.TextAlign.CENTER -> TextAlign.Center
            NveTables.Values.TextAlign.RIGHT -> TextAlign.Right
            NveTables.Values.TextAlign.LEFT -> TextAlign.Left
            NveTables.Values.TextAlign.END -> TextAlign.End
            NveTables.Values.TextAlign.JUSTIFY -> TextAlign.Justify
            else -> TextAlign.Start
        },
    )
}

/**
 * Host text measurer over Compose's [TextMeasurer], so runtime layout sizes text exactly as the
 * [androidx.compose.foundation.text.BasicText]s that draw it.
 */
class NveComposeTextMeasurer(private val measurer: TextMeasurer, private val density: Density) : NativeRuntime.TextMeasurer {
    override fun measure(text: String, fontSize: Float, fontWeight: Int, maxWidth: Float, out: FloatArray) {
        val size = if (fontSize.isFinite() && fontSize > 0f) fontSize else SceneSemantics.DEFAULT_FONT_SIZE
        val style = NveTextStyle(fontSize = size, fontWeight = fontWeight).toCompose(density)
        val maxPx = when {
            // Unconstrained: +Inf, NaN, or wider than `Constraints` can encode.
            maxWidth.isNaN() || maxWidth * density.density >= MAX_CONSTRAINT_PX -> Constraints.Infinity
            // Min-content: the widest unbreakable word, as CSS does.
            maxWidth <= 0f -> minContentPx(text, style)
            else -> ceil(maxWidth * density.density).toInt().coerceAtLeast(1)
        }
        // An empty string still measures one line height (an empty paragraph, like a caret line).
        val result = measurer.measure(text, style, constraints = Constraints(maxWidth = maxPx))
        val widest = (0 until result.lineCount).maxOfOrNull { result.getLineRight(it) - result.getLineLeft(it) } ?: 0f
        out[0] = ceil(widest / density.density)
        out[1] = ceil(result.size.height / density.density)
    }

    private fun minContentPx(text: String, style: TextStyle): Int {
        var widest = 1
        var start = 0
        for (i in 0..text.length) {
            if (i == text.length || text[i].isWhitespace()) {
                if (i > start) widest = maxOf(widest, measurer.measure(text.substring(start, i), style).size.width)
                start = i + 1
            }
        }
        return widest
    }

    private companion object {
        /** Widest finite `Constraints.maxWidth` used (larger widths encode as unbounded). */
        const val MAX_CONSTRAINT_PX = (1 shl 18) - 2f
    }
}

/**
 * Controlled text field: the runtime value is the source of truth. A keystroke is sent (lone
 * surrogates repaired first); with a local app ([NveSurfaceController.dispatchIsSynchronous])
 * the frame is applied before `textChanged` returns, so the field shows the app's verdict at
 * once and a rejected or rewritten keystroke snaps back. While paired the echo is asynchronous:
 * the field keeps the typing, a late echo of a value it sent never clobbers newer typing, and
 * only a value it never sent (the app rewrote it) replaces the text.
 */
@Composable
internal fun NveTextField(node: NveNode, base: Modifier, controller: NveSurfaceController, enabled: Boolean) {
    val input = node.input ?: return
    var text by remember(node.id) { mutableStateOf(input.value) }
    // Values sent while paired and not yet echoed, oldest first.
    val inFlight = remember(node.id) { ArrayDeque<String>() }
    LaunchedEffect(node.id, input.value) {
        val echoed = inFlight.indexOf(input.value)
        if (echoed >= 0) {
            repeat(echoed + 1) { inFlight.removeFirst() }
        } else {
            inFlight.clear()
            text = input.value
        }
    }
    // `onFocusChanged` reports the initial (unfocused) state on attach: only transitions are events.
    var focused by remember(node.id) { mutableStateOf(false) }
    val style = node.textStyle.toCompose(LocalDensity.current)
    BasicTextField(
        value = text,
        onValueChange = {
            val typed = Utf8.wellFormed(it)
            if (typed == text) return@BasicTextField  // selection / composition only
            if (controller.dispatchIsSynchronous) {
                controller.textChanged(node.id, typed)
                text = controller.scene[node.id]?.input?.value ?: typed
            } else {
                inFlight.addLast(typed)
                text = typed
                controller.textChanged(node.id, typed)
            }
        },
        enabled = enabled,
        modifier = base.onFocusChanged {
            if (it.isFocused != focused) {
                focused = it.isFocused
                controller.focusChanged(node.id, it.isFocused)
            }
        },
        readOnly = input.readOnly,
        singleLine = !input.multiline,
        textStyle = style,
        visualTransformation = if (input.secure) PasswordVisualTransformation() else VisualTransformation.None,
        keyboardOptions = KeyboardOptions(
            keyboardType = keyboardType(input),
            imeAction = if (input.multiline) ImeAction.Default else ImeAction.Done,
        ),
        keyboardActions = KeyboardActions(onDone = { controller.submit(node.id, Utf8.wellFormed(text)) }),
        decorationBox = { field ->
            if (text.isEmpty() && input.placeholder.isNotEmpty()) {
                BasicText(input.placeholder, style = style.copy(color = style.color.copy(alpha = 0.5f)))
            }
            field()
        },
    )
}

internal fun keyboardType(input: NveInput): KeyboardType = when {
    input.secure || input.inputKind == NveTables.Values.InputKind.PASSWORD -> KeyboardType.Password
    input.inputKind == NveTables.Values.InputKind.EMAIL -> KeyboardType.Email
    input.inputKind == NveTables.Values.InputKind.URL -> KeyboardType.Uri
    input.inputKind == NveTables.Values.InputKind.NUMBER -> KeyboardType.Decimal
    input.inputKind == NveTables.Values.InputKind.TEL -> KeyboardType.Phone
    else -> KeyboardType.Text
}
