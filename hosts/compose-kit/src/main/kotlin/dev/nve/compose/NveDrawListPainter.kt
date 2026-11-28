package dev.nve.compose

import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import dev.nve.host.DrawList
import java.nio.ByteBuffer

/**
 * Paints `nve.res.draw_list` (`FlatResource`) draw lists ([DrawList], the one JVM codec) onto a
 * Compose [DrawScope]. One painter per canvas node reuses its [Path] and wrapper buffer across
 * frames.
 */
class NveDrawListPainter : DrawList.Sink {
    private val path = Path()
    private var scope: DrawScope? = null
    private var scale = 1f
    private var bytes: ByteArray? = null
    private var buffer: ByteBuffer? = null
    private var validationError: DrawList.Error? = null
    internal var validationCount: Int = 0
        private set
    private var stroke: Stroke? = null
    private var strokeWidth = Float.NaN

    /**
     * Paints [list] in dp units scaled by [scale] (default: the scope's density). Validation is
     * cached per [ByteArray] reference so repeated repaints of an unchanged list stream directly
     * through [DrawList.streamValidated]. A malformed list paints nothing and returns its
     * [DrawList.Error]; clips are always restored.
     */
    fun draw(scope: DrawScope, list: ByteArray, scale: Float = scope.density): DrawList.Error? {
        val buf: ByteBuffer
        val error: DrawList.Error?
        if (list === bytes) {
            buf = buffer!!
            error = validationError
        } else {
            buf = ByteBuffer.wrap(list)
            error = DrawList.validate(buf, 0, list.size)
            validationCount++
            bytes = list
            buffer = buf
            validationError = error
        }
        if (error != null) return error
        this.scope = scope
        this.scale = if (scale.isFinite() && scale > 0f) scale else 1f
        try {
            DrawList.streamValidated(buf, 0, this)
            return null
        } finally {
            this.scope = null
        }
    }

    private val s get() = scope!!

    private fun strokeOf(width: Float): Stroke {
        if (width != strokeWidth) {
            strokeWidth = width
            stroke = Stroke(width = width, cap = StrokeCap.Round, join = StrokeJoin.Round)
        }
        return stroke!!
    }

    override fun onClear(rgba: Int) {
        if (visible(rgba)) s.drawRect(rgbaToColor(rgba), Offset.Zero, s.size)
    }

    override fun onFillRect(x: Float, y: Float, w: Float, h: Float, rgba: Int) {
        if (w > 0f && h > 0f && visible(rgba)) s.drawRect(rgbaToColor(rgba), Offset(x * scale, y * scale), Size(w * scale, h * scale))
    }

    override fun onFillRoundedRect(x: Float, y: Float, w: Float, h: Float, radius: Float, rgba: Int) {
        if (w <= 0f || h <= 0f || !visible(rgba)) return
        val r = radius.coerceAtLeast(0f) * scale
        s.drawRoundRect(rgbaToColor(rgba), Offset(x * scale, y * scale), Size(w * scale, h * scale), CornerRadius(r, r))
    }

    override fun onFillCircle(cx: Float, cy: Float, radius: Float, rgba: Int) {
        if (radius > 0f && visible(rgba)) s.drawCircle(rgbaToColor(rgba), radius * scale, Offset(cx * scale, cy * scale))
    }

    override fun onStrokeLine(x0: Float, y0: Float, x1: Float, y1: Float, width: Float, rgba: Int) {
        if (width > 0f && visible(rgba)) {
            s.drawLine(rgbaToColor(rgba), Offset(x0 * scale, y0 * scale), Offset(x1 * scale, y1 * scale), width * scale, StrokeCap.Round)
        }
    }

    private var polyWidth = 0f
    private var polyRgba = 0
    private var polyDrawn = false

    override fun onStrokePolylineStart(pointCount: Int, width: Float, rgba: Int) {
        polyDrawn = pointCount >= 2 && width > 0f && visible(rgba)
        polyWidth = width
        polyRgba = rgba
        path.reset()
    }

    override fun onStrokePolylinePoint(index: Int, x: Float, y: Float) {
        if (!polyDrawn) return
        if (index == 0) path.moveTo(x * scale, y * scale) else path.lineTo(x * scale, y * scale)
    }

    override fun onStrokePolylineEnd() {
        if (polyDrawn) s.drawPath(path, rgbaToColor(polyRgba), style = strokeOf(polyWidth * scale))
    }

    override fun onClipRect(x: Float, y: Float, w: Float, h: Float) {
        val canvas = s.drawContext.canvas
        canvas.save()
        canvas.clipRect(x * scale, y * scale, (x + w.coerceAtLeast(0f)) * scale, (y + h.coerceAtLeast(0f)) * scale)
    }

    override fun onRestoreClip() {
        s.drawContext.canvas.restore()
    }

    companion object {
        private fun visible(rgba: Int) = rgba ushr 24 != 0

        /** `ColorRgba::to_u32()` (`r | g << 8 | b << 16 | a << 24`) as a Compose [Color]. */
        fun rgbaToColor(rgba: Int): Color =
            Color(red = rgba and 0xFF, green = (rgba ushr 8) and 0xFF, blue = (rgba ushr 16) and 0xFF, alpha = rgba ushr 24)
    }
}
