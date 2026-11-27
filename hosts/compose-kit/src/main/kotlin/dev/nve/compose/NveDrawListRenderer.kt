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
import java.nio.ByteBuffer

/**
 * Strict validation error returned by [NveDrawListRenderer] when decoding corrupt or truncated
 * `NVE_DRAW1` binary draw lists.
 */
enum class NveDrawListError {
    TRUNCATED,
    BAD_MAGIC,
    UNSUPPORTED_VERSION,
    OP_COUNT_EXCEEDS_LIMIT,
    POLYLINE_POINTS_EXCEEDS_LIMIT,
    UNKNOWN_OPCODE,
    NON_FINITE_FLOAT,
    TRAILING_BYTES,
    INVALID_BASE64,
}

/**
 * Validated 15-byte `NVE_DRAW1` header.
 */
data class NveDrawListHeader(
    val version: Int,
    val opCount: Int,
)

/**
 * Zero-allocation streaming visitor receiving decoded `NVE_DRAW1` drawing operations.
 */
interface NveDrawOpSink {
    fun onClear(rgba: Int)
    fun onFillRect(x: Float, y: Float, w: Float, h: Float, rgba: Int)
    fun onFillRoundedRect(x: Float, y: Float, w: Float, h: Float, radius: Float, rgba: Int)
    fun onFillCircle(cx: Float, cy: Float, radius: Float, rgba: Int)
    fun onStrokeLine(x0: Float, y0: Float, x1: Float, y1: Float, width: Float, rgba: Int)
    fun onStrokePolylineStart(pointCount: Int, width: Float, rgba: Int)
    fun onStrokePolylinePoint(index: Int, x: Float, y: Float)
    fun onStrokePolylineEnd()
    fun onClipRect(x: Float, y: Float, w: Float, h: Float)
    fun onRestoreClip()
}

/**
 * Zero-copy, binary `NVE_DRAW1` decoder and Jetpack Compose [DrawScope] renderer
 * matching `packages/core/components/src/canvas_ir.rs` byte-for-byte.
 *
 * Wire layout (all integers and IEEE-754 floats little-endian):
 * - `[0..9]`: ASCII magic `"NVE_DRAW1"`
 * - `[9..11]`: `u16` format version (`1`)
 * - `[11..15]`: `u32` op count (`<= 65_536`)
 * - `[15..]`: packed tagged opcodes (`1..=8`):
 *   - `1 = Clear { rgba: u32 }`
 *   - `2 = FillRect { x, y, w, h: f32, rgba: u32 }`
 *   - `3 = FillRoundedRect { x, y, w, h, radius: f32, rgba: u32 }`
 *   - `4 = FillCircle { cx, cy, radius: f32, rgba: u32 }`
 *   - `5 = StrokeLine { x0, y0, x1, y1, width: f32, rgba: u32 }`
 *   - `6 = StrokePolyline { width: f32, rgba: u32, count: u32, points: (f32, f32)[count] }`
 *   - `7 = ClipRect { x, y, w, h: f32 }`
 *   - `8 = RestoreClip`
 */
object NveDrawListRenderer {
    const val HEADER_SIZE: Int = 15
    const val FORMAT_VERSION: Int = 1
    const val MAX_DRAW_OPS: Int = 65_536
    const val MAX_POLYLINE_POINTS: Int = 16_384

    const val OP_CLEAR: Int = 1
    const val OP_FILL_RECT: Int = 2
    const val OP_FILL_ROUNDED_RECT: Int = 3
    const val OP_FILL_CIRCLE: Int = 4
    const val OP_STROKE_LINE: Int = 5
    const val OP_STROKE_POLYLINE: Int = 6
    const val OP_CLIP_RECT: Int = 7
    const val OP_RESTORE_CLIP: Int = 8

    private val MAGIC = byteArrayOf(
        'N'.code.toByte(),
        'V'.code.toByte(),
        'E'.code.toByte(),
        '_'.code.toByte(),
        'D'.code.toByte(),
        'R'.code.toByte(),
        'A'.code.toByte(),
        'W'.code.toByte(),
        '1'.code.toByte(),
    )

    /**
     * Converts a little-endian `ColorRgba::to_u32()` (`[r, g, b, a]`) into a Compose [Color].
     */
    fun rgbaToColor(rgba: Int): Color {
        val r = rgba and 0xFF
        val g = (rgba ushr 8) and 0xFF
        val b = (rgba ushr 16) and 0xFF
        val a = (rgba ushr 24) and 0xFF
        return Color(red = r, green = g, blue = b, alpha = a)
    }

    /**
     * Fast check for the 9-byte `NVE_DRAW1` magic prefix in a [ByteBuffer] slice.
     */
    fun hasMagic(
        buffer: ByteBuffer,
        offset: Int = buffer.position(),
        length: Int = buffer.remaining(),
    ): Boolean {
        if (offset < 0 || length < HEADER_SIZE || offset.toLong() + length.toLong() > buffer.limit().toLong()) {
            return false
        }
        for (i in MAGIC.indices) {
            if (buffer.get(offset + i) != MAGIC[i]) return false
        }
        return true
    }

    fun hasMagic(bytes: ByteArray): Boolean {
        if (bytes.size < HEADER_SIZE) return false
        for (i in MAGIC.indices) {
            if (bytes[i] != MAGIC[i]) return false
        }
        return true
    }

    /**
     * Parses and validates only the fixed 15-byte header.
     */
    fun decodeHeader(
        buffer: ByteBuffer,
        offset: Int = buffer.position(),
        length: Int = buffer.remaining(),
    ): NveDrawListHeader? {
        if (offset < 0 || length < HEADER_SIZE || offset.toLong() + length.toLong() > buffer.limit().toLong()) {
            return null
        }
        for (i in MAGIC.indices) {
            if (buffer.get(offset + i) != MAGIC[i]) return null
        }
        val version = readU16Le(buffer, offset + 9)
        if (version != FORMAT_VERSION) return null
        val opCount = readU32Le(buffer, offset + 11)
        if (opCount > MAX_DRAW_OPS.toLong()) return null
        return NveDrawListHeader(version = version, opCount = opCount.toInt())
    }

    fun validate(bytes: ByteArray): NveDrawListError? = validate(ByteBuffer.wrap(bytes), 0, bytes.size)

    /**
     * Strict zero-allocation validation of an `NVE_DRAW1` binary stream.
     * Returns `null` if valid, or the exact [NveDrawListError] on corrupt/truncated input.
     */
    fun validate(
        buffer: ByteBuffer,
        offset: Int = buffer.position(),
        length: Int = buffer.remaining(),
    ): NveDrawListError? {
        if (offset < 0 || length < 0 || offset.toLong() + length.toLong() > buffer.limit().toLong()) {
            return NveDrawListError.TRUNCATED
        }
        if (length < HEADER_SIZE) {
            return NveDrawListError.TRUNCATED
        }
        for (i in MAGIC.indices) {
            if (buffer.get(offset + i) != MAGIC[i]) return NveDrawListError.BAD_MAGIC
        }
        val version = readU16Le(buffer, offset + 9)
        if (version != FORMAT_VERSION) {
            return NveDrawListError.UNSUPPORTED_VERSION
        }
        val opCount = readU32Le(buffer, offset + 11)
        if (opCount > MAX_DRAW_OPS.toLong()) {
            return NveDrawListError.OP_COUNT_EXCEEDS_LIMIT
        }
        val end = offset + length
        var pos = offset + HEADER_SIZE
        if ((end - pos).toLong() < opCount) {
            return NveDrawListError.TRUNCATED
        }

        for (i in 0L until opCount) {
            if (pos >= end) return NveDrawListError.TRUNCATED
            val op = buffer.get(pos++).toInt() and 0xFF
            when (op) {
                OP_CLEAR -> {
                    if (end - pos < 4) return NveDrawListError.TRUNCATED
                    pos += 4
                }
                OP_FILL_RECT -> {
                    if (end - pos < 20) return NveDrawListError.TRUNCATED
                    if (!isFiniteRange(buffer, pos, 4)) return NveDrawListError.NON_FINITE_FLOAT
                    pos += 20
                }
                OP_FILL_ROUNDED_RECT -> {
                    if (end - pos < 24) return NveDrawListError.TRUNCATED
                    if (!isFiniteRange(buffer, pos, 5)) return NveDrawListError.NON_FINITE_FLOAT
                    pos += 24
                }
                OP_FILL_CIRCLE -> {
                    if (end - pos < 16) return NveDrawListError.TRUNCATED
                    if (!isFiniteRange(buffer, pos, 3)) return NveDrawListError.NON_FINITE_FLOAT
                    pos += 16
                }
                OP_STROKE_LINE -> {
                    if (end - pos < 24) return NveDrawListError.TRUNCATED
                    if (!isFiniteRange(buffer, pos, 5)) return NveDrawListError.NON_FINITE_FLOAT
                    pos += 24
                }
                OP_STROKE_POLYLINE -> {
                    if (end - pos < 12) return NveDrawListError.TRUNCATED
                    if (!readF32Le(buffer, pos).isFinite()) return NveDrawListError.NON_FINITE_FLOAT
                    pos += 8 // width + rgba
                    val ptCount = readU32Le(buffer, pos)
                    pos += 4
                    if (ptCount > MAX_POLYLINE_POINTS.toLong()) {
                        return NveDrawListError.POLYLINE_POINTS_EXCEEDS_LIMIT
                    }
                    val neededBytes = ptCount * 8L
                    if (neededBytes > (end - pos).toLong()) return NveDrawListError.TRUNCATED
                    val floatCount = (ptCount * 2L).toInt()
                    if (!isFiniteRange(buffer, pos, floatCount)) return NveDrawListError.NON_FINITE_FLOAT
                    pos += neededBytes.toInt()
                }
                OP_CLIP_RECT -> {
                    if (end - pos < 16) return NveDrawListError.TRUNCATED
                    if (!isFiniteRange(buffer, pos, 4)) return NveDrawListError.NON_FINITE_FLOAT
                    pos += 16
                }
                OP_RESTORE_CLIP -> Unit
                else -> return NveDrawListError.UNKNOWN_OPCODE
            }
        }
        return if (pos != end) NveDrawListError.TRAILING_BYTES else null
    }

    fun decode(bytes: ByteArray, sink: NveDrawOpSink): NveDrawListError? =
        decode(ByteBuffer.wrap(bytes), 0, bytes.size, sink)

    /**
     * Validates and streams every `NVE_DRAW1` operation directly from [buffer] into [sink]
     * without allocating intermediate operation or vertex lists.
     */
    fun decode(
        buffer: ByteBuffer,
        offset: Int = buffer.position(),
        length: Int = buffer.remaining(),
        sink: NveDrawOpSink,
    ): NveDrawListError? {
        val error = validate(buffer, offset, length)
        if (error != null) return error

        val opCount = readU32Le(buffer, offset + 11).toInt()
        var pos = offset + HEADER_SIZE
        for (i in 0 until opCount) {
            when (buffer.get(pos++).toInt() and 0xFF) {
                OP_CLEAR -> {
                    val rgba = readI32Le(buffer, pos)
                    pos += 4
                    sink.onClear(rgba)
                }
                OP_FILL_RECT -> {
                    val x = readF32Le(buffer, pos)
                    val y = readF32Le(buffer, pos + 4)
                    val w = readF32Le(buffer, pos + 8)
                    val h = readF32Le(buffer, pos + 12)
                    val rgba = readI32Le(buffer, pos + 16)
                    pos += 20
                    sink.onFillRect(x, y, w, h, rgba)
                }
                OP_FILL_ROUNDED_RECT -> {
                    val x = readF32Le(buffer, pos)
                    val y = readF32Le(buffer, pos + 4)
                    val w = readF32Le(buffer, pos + 8)
                    val h = readF32Le(buffer, pos + 12)
                    val radius = readF32Le(buffer, pos + 16)
                    val rgba = readI32Le(buffer, pos + 20)
                    pos += 24
                    sink.onFillRoundedRect(x, y, w, h, radius, rgba)
                }
                OP_FILL_CIRCLE -> {
                    val cx = readF32Le(buffer, pos)
                    val cy = readF32Le(buffer, pos + 4)
                    val radius = readF32Le(buffer, pos + 8)
                    val rgba = readI32Le(buffer, pos + 12)
                    pos += 16
                    sink.onFillCircle(cx, cy, radius, rgba)
                }
                OP_STROKE_LINE -> {
                    val x0 = readF32Le(buffer, pos)
                    val y0 = readF32Le(buffer, pos + 4)
                    val x1 = readF32Le(buffer, pos + 8)
                    val y1 = readF32Le(buffer, pos + 12)
                    val width = readF32Le(buffer, pos + 16)
                    val rgba = readI32Le(buffer, pos + 20)
                    pos += 24
                    sink.onStrokeLine(x0, y0, x1, y1, width, rgba)
                }
                OP_STROKE_POLYLINE -> {
                    val width = readF32Le(buffer, pos)
                    val rgba = readI32Le(buffer, pos + 4)
                    val ptCount = readU32Le(buffer, pos + 8).toInt()
                    pos += 12
                    sink.onStrokePolylineStart(ptCount, width, rgba)
                    for (p in 0 until ptCount) {
                        val ptOff = pos + (p shl 3)
                        sink.onStrokePolylinePoint(p, readF32Le(buffer, ptOff), readF32Le(buffer, ptOff + 4))
                    }
                    sink.onStrokePolylineEnd()
                    pos += ptCount shl 3
                }
                OP_CLIP_RECT -> {
                    val x = readF32Le(buffer, pos)
                    val y = readF32Le(buffer, pos + 4)
                    val w = readF32Le(buffer, pos + 8)
                    val h = readF32Le(buffer, pos + 12)
                    pos += 16
                    sink.onClipRect(x, y, w, h)
                }
                OP_RESTORE_CLIP -> sink.onRestoreClip()
                else -> return NveDrawListError.UNKNOWN_OPCODE
            }
        }
        return null
    }

    /**
     * Renders a binary `NVE_DRAW1` [ByteArray] directly onto a Compose [DrawScope].
     * Returns `false` safely on corrupt or truncated input without throwing.
     */
    fun drawInto(
        drawScope: DrawScope,
        bytes: ByteArray,
        scale: Float = drawScope.density,
        reusablePath: Path = Path(),
    ): Boolean = drawInto(drawScope, ByteBuffer.wrap(bytes), 0, bytes.size, scale, reusablePath)

    /**
     * Zero-copy renders a binary `NVE_DRAW1` [ByteBuffer] slice directly onto a Compose [DrawScope],
     * painting all 8 opcodes with strict bounds/NaN guards and balanced clip save/restore.
     */
    fun drawInto(
        drawScope: DrawScope,
        buffer: ByteBuffer,
        offset: Int = buffer.position(),
        length: Int = buffer.remaining(),
        scale: Float = drawScope.density,
        reusablePath: Path = Path(),
    ): Boolean {
        if (validate(buffer, offset, length) != null) {
            return false
        }
        val s = if (scale.isFinite() && scale > 0f) scale else 1f
        val opCount = readU32Le(buffer, offset + 11).toInt()
        var pos = offset + HEADER_SIZE
        var clipDepth = 0
        val canvas = drawScope.drawContext.canvas

        try {
            for (i in 0 until opCount) {
                when (buffer.get(pos++).toInt() and 0xFF) {
                    OP_CLEAR -> {
                        val rgba = readI32Le(buffer, pos)
                        pos += 4
                        if ((rgba ushr 24) != 0) {
                            drawScope.drawRect(
                                color = rgbaToColor(rgba),
                                topLeft = Offset.Zero,
                                size = drawScope.size,
                            )
                        }
                    }
                    OP_FILL_RECT -> {
                        val x = readF32Le(buffer, pos)
                        val y = readF32Le(buffer, pos + 4)
                        val w = readF32Le(buffer, pos + 8)
                        val h = readF32Le(buffer, pos + 12)
                        val rgba = readI32Le(buffer, pos + 16)
                        pos += 20
                        if (w > 0f && h > 0f && (rgba ushr 24) != 0) {
                            drawScope.drawRect(
                                color = rgbaToColor(rgba),
                                topLeft = Offset(x * s, y * s),
                                size = Size(w * s, h * s),
                            )
                        }
                    }
                    OP_FILL_ROUNDED_RECT -> {
                        val x = readF32Le(buffer, pos)
                        val y = readF32Le(buffer, pos + 4)
                        val w = readF32Le(buffer, pos + 8)
                        val h = readF32Le(buffer, pos + 12)
                        val radius = readF32Le(buffer, pos + 16)
                        val rgba = readI32Le(buffer, pos + 20)
                        pos += 24
                        if (w > 0f && h > 0f && (rgba ushr 24) != 0) {
                            val r = radius.coerceAtLeast(0f) * s
                            drawScope.drawRoundRect(
                                color = rgbaToColor(rgba),
                                topLeft = Offset(x * s, y * s),
                                size = Size(w * s, h * s),
                                cornerRadius = CornerRadius(r, r),
                            )
                        }
                    }
                    OP_FILL_CIRCLE -> {
                        val cx = readF32Le(buffer, pos)
                        val cy = readF32Le(buffer, pos + 4)
                        val radius = readF32Le(buffer, pos + 8)
                        val rgba = readI32Le(buffer, pos + 12)
                        pos += 16
                        if (radius > 0f && (rgba ushr 24) != 0) {
                            drawScope.drawCircle(
                                color = rgbaToColor(rgba),
                                radius = radius * s,
                                center = Offset(cx * s, cy * s),
                            )
                        }
                    }
                    OP_STROKE_LINE -> {
                        val x0 = readF32Le(buffer, pos)
                        val y0 = readF32Le(buffer, pos + 4)
                        val x1 = readF32Le(buffer, pos + 8)
                        val y1 = readF32Le(buffer, pos + 12)
                        val width = readF32Le(buffer, pos + 16)
                        val rgba = readI32Le(buffer, pos + 20)
                        pos += 24
                        if (width > 0f && (rgba ushr 24) != 0) {
                            drawScope.drawLine(
                                color = rgbaToColor(rgba),
                                start = Offset(x0 * s, y0 * s),
                                end = Offset(x1 * s, y1 * s),
                                strokeWidth = width * s,
                                cap = StrokeCap.Round,
                            )
                        }
                    }
                    OP_STROKE_POLYLINE -> {
                        val width = readF32Le(buffer, pos)
                        val rgba = readI32Le(buffer, pos + 4)
                        val ptCount = readU32Le(buffer, pos + 8).toInt()
                        pos += 12
                        if (ptCount >= 2 && width > 0f && (rgba ushr 24) != 0) {
                            reusablePath.reset()
                            reusablePath.moveTo(readF32Le(buffer, pos) * s, readF32Le(buffer, pos + 4) * s)
                            for (p in 1 until ptCount) {
                                val ptOff = pos + (p shl 3)
                                reusablePath.lineTo(
                                    readF32Le(buffer, ptOff) * s,
                                    readF32Le(buffer, ptOff + 4) * s,
                                )
                            }
                            drawScope.drawPath(
                                path = reusablePath,
                                color = rgbaToColor(rgba),
                                style = Stroke(
                                    width = width * s,
                                    cap = StrokeCap.Round,
                                    join = StrokeJoin.Round,
                                ),
                            )
                        }
                        pos += ptCount shl 3
                    }
                    OP_CLIP_RECT -> {
                        val x = readF32Le(buffer, pos)
                        val y = readF32Le(buffer, pos + 4)
                        val w = readF32Le(buffer, pos + 8)
                        val h = readF32Le(buffer, pos + 12)
                        pos += 16
                        canvas.save()
                        clipDepth++
                        canvas.clipRect(
                            left = x * s,
                            top = y * s,
                            right = (x + w.coerceAtLeast(0f)) * s,
                            bottom = (y + h.coerceAtLeast(0f)) * s,
                        )
                    }
                    OP_RESTORE_CLIP -> {
                        if (clipDepth > 0) {
                            canvas.restore()
                            clipDepth--
                        }
                    }
                    else -> return false
                }
            }
            return true
        } finally {
            while (clipDepth > 0) {
                canvas.restore()
                clipDepth--
            }
        }
    }

    private fun isFiniteRange(buffer: ByteBuffer, pos: Int, floatCount: Int): Boolean {
        for (i in 0 until floatCount) {
            if (!readF32Le(buffer, pos + (i shl 2)).isFinite()) return false
        }
        return true
    }

    private fun readU16Le(buffer: ByteBuffer, pos: Int): Int =
        (buffer.get(pos).toInt() and 0xFF) or
            ((buffer.get(pos + 1).toInt() and 0xFF) shl 8)

    private fun readI32Le(buffer: ByteBuffer, pos: Int): Int =
        (buffer.get(pos).toInt() and 0xFF) or
            ((buffer.get(pos + 1).toInt() and 0xFF) shl 8) or
            ((buffer.get(pos + 2).toInt() and 0xFF) shl 16) or
            ((buffer.get(pos + 3).toInt() and 0xFF) shl 24)

    private fun readU32Le(buffer: ByteBuffer, pos: Int): Long =
        readI32Le(buffer, pos).toLong() and 0xFFFF_FFFFL

    private fun readF32Le(buffer: ByteBuffer, pos: Int): Float =
        Float.fromBits(readI32Le(buffer, pos))
}
