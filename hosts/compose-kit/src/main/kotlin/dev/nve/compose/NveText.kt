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
import dev.nve.android.generated.NveTables
import dev.nve.host.NativeRuntime
import kotlin.math.ceil

/** Runtime text style → Compose: sizes are dp (logical points), independent of the font scale. */
internal fun NveTextStyle.toCompose(density: Density): TextStyle = with(density) {
    TextStyle(
        color = argb(color),
        fontSize = (if (fontSize.isFinite() && fontSize > 0f) fontSize else 16f).dp.toSp(),
        fontWeight = FontWeight(fontWeight.coerceIn(1, 1000)),
        fontStyle = if (italic) FontStyle.Italic else FontStyle.Normal,
        textAlign = when (align) {
            NveTables.Typography.TEXT_ALIGN_CENTER -> TextAlign.Center
            NveTables.Typography.TEXT_ALIGN_RIGHT -> TextAlign.Right
            NveTables.Typography.TEXT_ALIGN_LEFT -> TextAlign.Left
            NveTables.Typography.TEXT_ALIGN_END -> TextAlign.End
            NveTables.Typography.TEXT_ALIGN_JUSTIFY -> TextAlign.Justify
            else -> TextAlign.Start
        },
    )
}

/**
 * Host text measurer over Compose's [TextMeasurer], so runtime layout sizes text exactly as the
 * [androidx.compose.foundation.text.BasicText]s that draw it.
 */
class NveComposeTextMeasurer(private val measurer: TextMeasurer, private val density: Density) : NativeRuntime.TextMeasurer {
    override fun measure(text: String, fontSize: Float, fontWeight: Int, maxWidth: Float, out: FloatArray): Boolean {
        val style = NveTextStyle(fontSize = fontSize, fontWeight = fontWeight).toCompose(density)
        val maxPx = when {
            maxWidth.isInfinite() -> Constraints.Infinity
            // Min-content: the widest unbreakable word, as CSS does.
            maxWidth <= 0f -> text.split(Regex("\\s+")).maxOfOrNull { word ->
                if (word.isEmpty()) 0 else measurer.measure(word, style).size.width
            }?.coerceAtLeast(1) ?: 1
            else -> ceil(maxWidth * density.density).toInt().coerceAtLeast(1)
        }
        val result = measurer.measure(text, style, constraints = Constraints(maxWidth = maxPx))
        val widest = (0 until result.lineCount).maxOfOrNull { result.getLineRight(it) - result.getLineLeft(it) } ?: 0f
        out[0] = ceil(widest / density.density)
        out[1] = ceil(result.size.height / density.density)
        return true
    }
}

@Composable
internal fun NveTextField(node: NveNode, base: Modifier, controller: NveSurfaceController) {
    val input = node.input ?: return
    // The runtime value is the source of truth; local state only bridges keystrokes until it echoes.
    var text by remember(node.id) { mutableStateOf(input.value) }
    LaunchedEffect(node.id, input.value) { text = input.value }
    val style = node.textStyle.toCompose(LocalDensity.current)
    BasicTextField(
        value = text,
        onValueChange = {
            text = it
            controller.textChanged(node.id, it)
        },
        modifier = base.onFocusChanged { controller.focusChanged(node.id, it.isFocused) },
        readOnly = input.readOnly,
        singleLine = !input.multiline,
        textStyle = style,
        visualTransformation = if (input.secure) PasswordVisualTransformation() else VisualTransformation.None,
        keyboardOptions = KeyboardOptions(
            keyboardType = keyboardType(input),
            imeAction = if (input.multiline) ImeAction.Default else ImeAction.Done,
        ),
        keyboardActions = KeyboardActions(onDone = { controller.submit(node.id, text) }),
        decorationBox = { field ->
            if (text.isEmpty() && input.placeholder.isNotEmpty()) {
                BasicText(input.placeholder, style = style.copy(color = style.color.copy(alpha = 0.5f)))
            }
            field()
        },
    )
}

internal fun keyboardType(input: NveInput): KeyboardType = when {
    input.secure || input.inputKind == NveTables.TextInput.INPUT_KIND_PASSWORD -> KeyboardType.Password
    input.inputKind == NveTables.TextInput.INPUT_KIND_EMAIL -> KeyboardType.Email
    input.inputKind == NveTables.TextInput.INPUT_KIND_URL -> KeyboardType.Uri
    input.inputKind == NveTables.TextInput.INPUT_KIND_NUMBER -> KeyboardType.Decimal
    input.inputKind == NveTables.TextInput.INPUT_KIND_TEL -> KeyboardType.Phone
    else -> KeyboardType.Text
}
