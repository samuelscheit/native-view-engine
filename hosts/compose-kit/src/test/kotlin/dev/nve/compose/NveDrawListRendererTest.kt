package dev.nve.compose

import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Assertions.assertNotNull
import org.junit.jupiter.api.Assertions.assertNull
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import java.io.ByteArrayOutputStream
import java.io.File
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.StandardCharsets

internal object TestDrawListBuilder {
    fun rgba(r: Int, g: Int, b: Int, a: Int = 255): Int =
        (r and 0xFF) or ((g and 0xFF) shl 8) or ((b and 0xFF) shl 16) or ((a and 0xFF) shl 24)

    fun buildAll8Opcodes(): ByteArray {
        val out = ByteArrayOutputStream()
        out.write("NVE_DRAW1".toByteArray(StandardCharsets.US_ASCII))
        writeU16(out, 1)
        writeU32(out, 8)

        // 1. Clear
        out.write(NveDrawListRenderer.OP_CLEAR)
        writeI32(out, rgba(15, 23, 42, 255))

        // 2. FillRect
        out.write(NveDrawListRenderer.OP_FILL_RECT)
        writeF32(out, 4f); writeF32(out, 8f); writeF32(out, 40f); writeF32(out, 20f)
        writeI32(out, rgba(56, 189, 248, 255))

        // 3. FillRoundedRect
        out.write(NveDrawListRenderer.OP_FILL_ROUNDED_RECT)
        writeF32(out, 10f); writeF32(out, 12f); writeF32(out, 32f); writeF32(out, 16f); writeF32(out, 6f)
        writeI32(out, rgba(34, 197, 94, 255))

        // 4. FillCircle
        out.write(NveDrawListRenderer.OP_FILL_CIRCLE)
        writeF32(out, 50f); writeF32(out, 30f); writeF32(out, 8f)
        writeI32(out, rgba(250, 204, 21, 255))

        // 5. StrokeLine
        out.write(NveDrawListRenderer.OP_STROKE_LINE)
        writeF32(out, 2f); writeF32(out, 4f); writeF32(out, 60f); writeF32(out, 44f); writeF32(out, 2.5f)
        writeI32(out, rgba(248, 113, 113, 255))

        // 6. ClipRect
        out.write(NveDrawListRenderer.OP_CLIP_RECT)
        writeF32(out, 5f); writeF32(out, 5f); writeF32(out, 70f); writeF32(out, 50f)

        // 7. StrokePolyline (3 points)
        out.write(NveDrawListRenderer.OP_STROKE_POLYLINE)
        writeF32(out, 3f)
        writeI32(out, rgba(168, 85, 247, 255))
        writeU32(out, 3)
        writeF32(out, 6f); writeF32(out, 40f)
        writeF32(out, 30f); writeF32(out, 15f)
        writeF32(out, 60f); writeF32(out, 35f)

        // 8. RestoreClip
        out.write(NveDrawListRenderer.OP_RESTORE_CLIP)

        return out.toByteArray()
    }

    fun writeU16(out: ByteArrayOutputStream, v: Int) {
        out.write(v and 0xFF)
        out.write((v ushr 8) and 0xFF)
    }

    fun writeI32(out: ByteArrayOutputStream, v: Int) {
        out.write(v and 0xFF)
        out.write((v ushr 8) and 0xFF)
        out.write((v ushr 16) and 0xFF)
        out.write((v ushr 24) and 0xFF)
    }

    fun writeU32(out: ByteArrayOutputStream, v: Int) = writeI32(out, v)

    fun writeF32(out: ByteArrayOutputStream, v: Float) = writeI32(out, v.toRawBits())

    fun writeI64(out: ByteArrayOutputStream, v: Long) {
        for (i in 0 until 8) {
            out.write(((v ushr (i * 8)) and 0xFFL).toInt())
        }
    }
}

class NveDrawListRendererTest {

    @Test
    fun decodesAll8OpcodesZeroCopyFromByteBufferSlice() {
        val drawBytes = TestDrawListBuilder.buildAll8Opcodes()
        val padded = ByteBuffer.allocateDirect(drawBytes.size + 19).order(ByteOrder.LITTLE_ENDIAN)
        for (i in 0 until 11) padded.put(0x55.toByte())
        padded.put(drawBytes)
        for (i in 0 until 8) padded.put(0xAA.toByte())
        padded.position(3) // non-zero initial position must stay untouched!
        padded.limit(drawBytes.size + 19)

        assertTrue(NveDrawListRenderer.hasMagic(padded, 11, drawBytes.size))
        assertNull(NveDrawListRenderer.validate(padded, 11, drawBytes.size))
        val header = NveDrawListRenderer.decodeHeader(padded, 11, drawBytes.size)
        assertNotNull(header)
        assertEquals(1, header!!.version)
        assertEquals(8, header.opCount)

        val log = mutableListOf<String>()
        val polyPts = mutableListOf<Pair<Float, Float>>()
        val err = NveDrawListRenderer.decode(
            buffer = padded,
            offset = 11,
            length = drawBytes.size,
            sink = object : NveDrawOpSink {
                override fun onClear(rgba: Int) { log.add("clear:$rgba") }
                override fun onFillRect(x: Float, y: Float, w: Float, h: Float, rgba: Int) {
                    log.add("rect:$x,$y,$w,$h,$rgba")
                }
                override fun onFillRoundedRect(
                    x: Float,
                    y: Float,
                    w: Float,
                    h: Float,
                    radius: Float,
                    rgba: Int,
                ) {
                    log.add("rrect:$x,$y,$w,$h,$radius,$rgba")
                }
                override fun onFillCircle(cx: Float, cy: Float, radius: Float, rgba: Int) {
                    log.add("circle:$cx,$cy,$radius,$rgba")
                }
                override fun onStrokeLine(
                    x0: Float,
                    y0: Float,
                    x1: Float,
                    y1: Float,
                    width: Float,
                    rgba: Int,
                ) {
                    log.add("line:$x0,$y0,$x1,$y1,$width,$rgba")
                }
                override fun onStrokePolylineStart(pointCount: Int, width: Float, rgba: Int) {
                    log.add("polyStart:$pointCount,$width,$rgba")
                }
                override fun onStrokePolylinePoint(index: Int, x: Float, y: Float) {
                    polyPts.add(x to y)
                }
                override fun onStrokePolylineEnd() { log.add("polyEnd") }
                override fun onClipRect(x: Float, y: Float, w: Float, h: Float) {
                    log.add("clip:$x,$y,$w,$h")
                }
                override fun onRestoreClip() { log.add("restore") }
            },
        )
        assertNull(err)
        assertEquals(3, padded.position()) // zero-copy slice read did not mutate position!
        assertEquals(9, log.size) // 7 single ops + polyStart + polyEnd
        assertTrue(log[0].startsWith("clear:"))
        assertTrue(log[1].startsWith("rect:4.0,8.0,40.0,20.0"))
        assertTrue(log[2].startsWith("rrect:10.0,12.0,32.0,16.0,6.0"))
        assertTrue(log[3].startsWith("circle:50.0,30.0,8.0"))
        assertTrue(log[4].startsWith("line:2.0,4.0,60.0,44.0,2.5"))
        assertEquals("clip:5.0,5.0,70.0,50.0", log[5])
        assertTrue(log[6].startsWith("polyStart:3,3.0"))
        assertEquals("polyEnd", log[7])
        assertEquals("restore", log[8])
        assertEquals(listOf(6f to 40f, 30f to 15f, 60f to 35f), polyPts)
    }

    @Test
    fun rejectsCorruptTruncatedNanOversizedAndTrailingDrawListsSafely() {
        // Truncated header
        assertEquals(NveDrawListError.TRUNCATED, NveDrawListRenderer.validate(ByteArray(0)))
        assertEquals(NveDrawListError.TRUNCATED, NveDrawListRenderer.validate(ByteArray(14)))

        // Bad magic
        val badMagic = "BAD_MAGIC".toByteArray(StandardCharsets.US_ASCII) + ByteArray(6)
        assertEquals(NveDrawListError.BAD_MAGIC, NveDrawListRenderer.validate(badMagic))

        // Unsupported version
        val badVer = ByteArrayOutputStream().apply {
            write("NVE_DRAW1".toByteArray(StandardCharsets.US_ASCII))
            TestDrawListBuilder.writeU16(this, 2)
            TestDrawListBuilder.writeU32(this, 0)
        }.toByteArray()
        assertEquals(NveDrawListError.UNSUPPORTED_VERSION, NveDrawListRenderer.validate(badVer))

        // Op count exceeds MAX_DRAW_OPS
        val tooManyOps = ByteArrayOutputStream().apply {
            write("NVE_DRAW1".toByteArray(StandardCharsets.US_ASCII))
            TestDrawListBuilder.writeU16(this, 1)
            TestDrawListBuilder.writeU32(this, NveDrawListRenderer.MAX_DRAW_OPS + 1)
        }.toByteArray()
        assertEquals(NveDrawListError.OP_COUNT_EXCEEDS_LIMIT, NveDrawListRenderer.validate(tooManyOps))

        // Unknown opcode
        val unknownOp = ByteArrayOutputStream().apply {
            write("NVE_DRAW1".toByteArray(StandardCharsets.US_ASCII))
            TestDrawListBuilder.writeU16(this, 1)
            TestDrawListBuilder.writeU32(this, 1)
            write(99)
        }.toByteArray()
        assertEquals(NveDrawListError.UNKNOWN_OPCODE, NveDrawListRenderer.validate(unknownOp))

        // Non-finite float (NaN & +Inf)
        val nanRect = ByteArrayOutputStream().apply {
            write("NVE_DRAW1".toByteArray(StandardCharsets.US_ASCII))
            TestDrawListBuilder.writeU16(this, 1)
            TestDrawListBuilder.writeU32(this, 1)
            write(NveDrawListRenderer.OP_FILL_RECT)
            TestDrawListBuilder.writeF32(this, Float.NaN)
            TestDrawListBuilder.writeF32(this, 0f)
            TestDrawListBuilder.writeF32(this, 10f)
            TestDrawListBuilder.writeF32(this, 10f)
            TestDrawListBuilder.writeI32(this, -1)
        }.toByteArray()
        assertEquals(NveDrawListError.NON_FINITE_FLOAT, NveDrawListRenderer.validate(nanRect))

        val infCircle = ByteArrayOutputStream().apply {
            write("NVE_DRAW1".toByteArray(StandardCharsets.US_ASCII))
            TestDrawListBuilder.writeU16(this, 1)
            TestDrawListBuilder.writeU32(this, 1)
            write(NveDrawListRenderer.OP_FILL_CIRCLE)
            TestDrawListBuilder.writeF32(this, 1f)
            TestDrawListBuilder.writeF32(this, Float.POSITIVE_INFINITY)
            TestDrawListBuilder.writeF32(this, 5f)
            TestDrawListBuilder.writeI32(this, -1)
        }.toByteArray()
        assertEquals(NveDrawListError.NON_FINITE_FLOAT, NveDrawListRenderer.validate(infCircle))

        // Polyline point count exceeds MAX_POLYLINE_POINTS
        val tooManyPts = ByteArrayOutputStream().apply {
            write("NVE_DRAW1".toByteArray(StandardCharsets.US_ASCII))
            TestDrawListBuilder.writeU16(this, 1)
            TestDrawListBuilder.writeU32(this, 1)
            write(NveDrawListRenderer.OP_STROKE_POLYLINE)
            TestDrawListBuilder.writeF32(this, 2f)
            TestDrawListBuilder.writeI32(this, -1)
            TestDrawListBuilder.writeU32(this, NveDrawListRenderer.MAX_POLYLINE_POINTS + 1)
        }.toByteArray()
        assertEquals(
            NveDrawListError.POLYLINE_POINTS_EXCEEDS_LIMIT,
            NveDrawListRenderer.validate(tooManyPts),
        )

        // Every prefix truncation of a valid stream fails with TRUNCATED
        val valid = TestDrawListBuilder.buildAll8Opcodes()
        for (cut in 1 until valid.size) {
            val err = NveDrawListRenderer.validate(valid.copyOf(cut))
            assertEquals(NveDrawListError.TRUNCATED, err, "Expected TRUNCATED at cut=$cut")
        }

        // Trailing bytes rejected
        val trailing = valid + byteArrayOf(0x00)
        assertEquals(NveDrawListError.TRAILING_BYTES, NveDrawListRenderer.validate(trailing))
    }

    @Test
    fun composeKitSourcesContainZeroOrgJsonUsage() {
        val srcRoot = File("src/main/kotlin")
        assertTrue(srcRoot.isDirectory, "Expected compose-kit Kotlin source directory at ${srcRoot.absolutePath}")
        val kotlinFiles = srcRoot.walkTopDown().filter { it.isFile && it.extension == "kt" }.toList()
        assertTrue(kotlinFiles.isNotEmpty())
        for (file in kotlinFiles) {
            val content = file.readText()
            assertFalse(
                content.contains("org.json"),
                "Found forbidden org.json reference in ${file.path}",
            )
        }
    }
}
