package dev.nve.compose

import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Canvas
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.drawscope.CanvasDrawScope
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.graphics.toPixelMap
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.LayoutDirection
import dev.nve.host.DrawList
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import java.io.File
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** [NveDrawListPainter] on real Skia pixels (format validation lives in host-java `DrawListTest`). */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class NveDrawListPainterTest {
    private val red = rgba(255, 0, 0)
    private val blue = rgba(0, 0, 255)

    @Test
    fun paintsScaledOpsInsideBalancedClips() {
        val list = DrawListBytes()
            .clip(0f, 0f, 5f, 5f).fillRect(0f, 0f, 20f, 20f, red).restore()
            .fillRect(15f, 15f, 2f, 2f, blue)
            .polyline(1f, blue, 0f, 19f, 19f, 19f)
            .build()
        val image = ImageBitmap(40, 40)
        val canvas = Canvas(image)
        val saves = canvas.nativeCanvas.saveCount
        assertNull(paint(canvas, list, scale = 2f))
        assertEquals("every clip is restored", saves, canvas.nativeCanvas.saveCount)
        val px = image.toPixelMap()
        assertEquals(androidx.compose.ui.graphics.Color.Red, px[4, 4])
        assertEquals("outside the clip", 0f, px[12, 12].alpha)
        assertEquals(androidx.compose.ui.graphics.Color.Blue, px[31, 31])
        assertEquals("polyline stroked at 2x", androidx.compose.ui.graphics.Color.Blue, px[20, 38])
    }

    @Test
    fun openClipsAreClosedAndUnmatchedRestoresDropped() {
        val list = DrawListBytes().restore().clip(0f, 0f, 2f, 2f).clip(0f, 0f, 1f, 1f).build()
        val canvas = Canvas(ImageBitmap(8, 8))
        val saves = canvas.nativeCanvas.saveCount
        assertNull(paint(canvas, list))
        assertEquals(saves, canvas.nativeCanvas.saveCount)
    }

    @Test
    fun aMalformedListPaintsNothing() {
        val good = DrawListBytes().fillRect(0f, 0f, 8f, 8f, red).build()
        val image = ImageBitmap(8, 8)
        assertEquals(DrawList.Error.TRAILING_BYTES, paint(Canvas(image), good + 0.toByte()))
        assertEquals(0f, image.toPixelMap()[4, 4].alpha)
    }

    @Test
    fun composeKitSourcesContainZeroOrgJsonUsage() {
        val kotlinFiles = File("src/main/kotlin").walkTopDown().filter { it.isFile && it.extension == "kt" }.toList()
        assertFalse(kotlinFiles.isEmpty())
        for (file in kotlinFiles) assertFalse(file.path, file.readText().contains("org.json"))
    }

    private fun paint(canvas: Canvas, list: ByteArray, scale: Float = 1f): DrawList.Error? {
        var error: DrawList.Error? = null
        CanvasDrawScope().draw(Density(1f), LayoutDirection.Ltr, canvas, Size(40f, 40f)) {
            error = NveDrawListPainter().draw(this, list, scale)
        }
        return error
    }

    private fun rgba(r: Int, g: Int, b: Int, a: Int = 255) = r or (g shl 8) or (b shl 16) or (a shl 24)

    private class DrawListBytes {
        private val ops = ByteBuffer.allocate(1024).order(ByteOrder.LITTLE_ENDIAN)
        private var count = 0

        private fun op(code: Int, vararg floats: Float): DrawListBytes {
            ops.put(code.toByte())
            floats.forEach { ops.putFloat(it) }
            count++
            return this
        }

        fun clip(x: Float, y: Float, w: Float, h: Float) = op(DrawList.OP_CLIP_RECT, x, y, w, h)
        fun restore() = op(DrawList.OP_RESTORE_CLIP)
        fun fillRect(x: Float, y: Float, w: Float, h: Float, rgba: Int) = op(DrawList.OP_FILL_RECT, x, y, w, h).also { ops.putInt(rgba) }
        fun polyline(width: Float, rgba: Int, vararg xy: Float) = op(DrawList.OP_STROKE_POLYLINE, width).also {
            ops.putInt(rgba).putInt(xy.size / 2)
            xy.forEach { ops.putFloat(it) }
        }

        fun build(): ByteArray {
            val out = ByteBuffer.allocate(DrawList.HEADER_SIZE + ops.position()).order(ByteOrder.LITTLE_ENDIAN)
            out.put("NVE_DRAW1".toByteArray(Charsets.US_ASCII)).putShort(DrawList.FORMAT_VERSION.toShort()).putInt(count)
            out.put(ops.array(), 0, ops.position())
            return out.array()
        }
    }
}
