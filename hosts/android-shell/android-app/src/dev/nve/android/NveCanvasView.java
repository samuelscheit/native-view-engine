package dev.nve.android;

import android.content.Context;
import android.graphics.Canvas;
import android.graphics.Paint;
import android.graphics.Path;
import android.graphics.RectF;
import android.view.View;

import java.nio.ByteBuffer;

/**
 * Zero-allocation `NVE_DRAW1` binary vector draw-list renderer for `KIND_SKIA_CANVAS` nodes.
 *
 * Decodes little-endian `NVE_DRAW1` streams directly from a `ByteBuffer` (either a zero-copy
 * `text_slab` slice or a retained reusable buffer copied from the batched dirty-node metadata slab)
 * and paints all 8 opcodes onto `android.graphics.Canvas` using reusable `Paint`, `Path`, and
 * `RectF` objects.
 */
public final class NveCanvasView extends View {
    public static final int HEADER_SIZE = 15;
    public static final int FORMAT_VERSION = 1;
    public static final int MAX_DRAW_OPS = 65_536;
    public static final int MAX_POLYLINE_POINTS = 16_384;

    public static final int OP_CLEAR = 1;
    public static final int OP_FILL_RECT = 2;
    public static final int OP_FILL_ROUNDED_RECT = 3;
    public static final int OP_FILL_CIRCLE = 4;
    public static final int OP_STROKE_LINE = 5;
    public static final int OP_STROKE_POLYLINE = 6;
    public static final int OP_CLIP_RECT = 7;
    public static final int OP_RESTORE_CLIP = 8;

    private static final byte[] MAGIC = {
        'N', 'V', 'E', '_', 'D', 'R', 'A', 'W', '1'
    };

    private final Paint fillPaint = new Paint(Paint.ANTI_ALIAS_FLAG);
    private final Paint strokePaint = new Paint(Paint.ANTI_ALIAS_FLAG);
    private final Path polyPath = new Path();
    private final RectF scratchRect = new RectF();

    private ByteBuffer drawBuffer;
    private int drawOffset = 0;
    private int drawLength = 0;
    private boolean drawValid = false;
    private ByteBuffer retainedBuffer;
    private float density = 1.0f;

    public NveCanvasView(Context context) {
        super(context);
        fillPaint.setStyle(Paint.Style.FILL);
        strokePaint.setStyle(Paint.Style.STROKE);
        strokePaint.setStrokeCap(Paint.Cap.ROUND);
        strokePaint.setStrokeJoin(Paint.Join.ROUND);
        setWillNotDraw(false);
    }

    public void setDensity(float nextDensity) {
        float safe = (Float.isFinite(nextDensity) && nextDensity > 0.0f) ? nextDensity : 1.0f;
        if (Float.floatToRawIntBits(density) != Float.floatToRawIntBits(safe)) {
            density = safe;
            invalidate();
        }
    }

    /**
     * Zero-copy bind directly to a stable `ByteBuffer` slice (e.g. `text_slab`).
     */
    public void setDrawListSlice(ByteBuffer buffer, int offset, int length) {
        if (buffer == null || length < HEADER_SIZE || !validate(buffer, offset, length)) {
            clearDrawList();
            return;
        }
        drawBuffer = buffer;
        drawOffset = offset;
        drawLength = length;
        drawValid = true;
        invalidate();
    }

    /**
     * Copies `length` bytes from a transient frame buffer (such as the batched dirty-node
     * metadata slab) into a reusable per-view `ByteBuffer` (zero steady-state allocation once
     * warmed up) so subsequent repaints remain memory-safe across frames.
     */
    public void setDrawListCopy(ByteBuffer src, int offset, int length) {
        if (src == null || length < HEADER_SIZE || !validate(src, offset, length)) {
            clearDrawList();
            return;
        }
        if (retainedBuffer == null || retainedBuffer.capacity() < length) {
            int alloc = Math.max(length, retainedBuffer != null ? retainedBuffer.capacity() << 1 : 256);
            retainedBuffer = ByteBuffer.allocateDirect(alloc);
        }
        for (int i = 0; i < length; i++) {
            retainedBuffer.put(i, src.get(offset + i));
        }
        drawBuffer = retainedBuffer;
        drawOffset = 0;
        drawLength = length;
        drawValid = true;
        invalidate();
    }

    public void setDrawListBytes(byte[] bytes) {
        if (bytes == null || bytes.length < HEADER_SIZE) {
            clearDrawList();
            return;
        }
        setDrawListSlice(ByteBuffer.wrap(bytes), 0, bytes.length);
    }

    public void clearDrawList() {
        boolean hadData = drawValid || drawLength != 0;
        drawBuffer = null;
        drawOffset = 0;
        drawLength = 0;
        drawValid = false;
        if (hadData) {
            invalidate();
        }
    }

    public boolean hasValidDrawList() {
        return drawValid && drawBuffer != null && drawLength >= HEADER_SIZE;
    }

    @Override
    protected void onDraw(Canvas canvas) {
        super.onDraw(canvas);
        if (!drawValid || drawBuffer == null || drawLength < HEADER_SIZE) {
            return;
        }
        decodeAndDraw(
                canvas,
                drawBuffer,
                drawOffset,
                drawLength,
                density,
                getWidth(),
                getHeight(),
                fillPaint,
                strokePaint,
                polyPath,
                scratchRect);
    }

    /**
     * Fast zero-allocation check for the 9-byte `NVE_DRAW1` header magic.
     */
    public static boolean hasDrawMagic(ByteBuffer buf, int offset, int length) {
        if (buf == null || offset < 0 || length < HEADER_SIZE || offset + length > buf.limit()) {
            return false;
        }
        for (int i = 0; i < MAGIC.length; i++) {
            if (buf.get(offset + i) != MAGIC[i]) {
                return false;
            }
        }
        return true;
    }

    /**
     * Strict zero-allocation bounds, version, op-count, polyline-vertex, and finite-float
     * validation of an `NVE_DRAW1` byte slice.
     */
    public static boolean validate(ByteBuffer buf, int offset, int length) {
        if (!hasDrawMagic(buf, offset, length)) {
            return false;
        }
        final int end = offset + length;
        final int version = readU16Le(buf, offset + 9);
        if (version != FORMAT_VERSION) {
            return false;
        }
        final long opCount = readU32Le(buf, offset + 11);
        if (opCount > MAX_DRAW_OPS) {
            return false;
        }
        int pos = offset + HEADER_SIZE;
        if ((long) (end - pos) < opCount) {
            return false;
        }

        for (long i = 0; i < opCount; i++) {
            if (pos >= end) {
                return false;
            }
            int op = buf.get(pos++) & 0xFF;
            switch (op) {
                case OP_CLEAR:
                    if (end - pos < 4) return false;
                    pos += 4;
                    break;
                case OP_FILL_RECT:
                    if (end - pos < 20) return false;
                    if (!isFiniteRange(buf, pos, 4)) return false;
                    pos += 20;
                    break;
                case OP_FILL_ROUNDED_RECT:
                    if (end - pos < 24) return false;
                    if (!isFiniteRange(buf, pos, 5)) return false;
                    pos += 24;
                    break;
                case OP_FILL_CIRCLE:
                    if (end - pos < 16) return false;
                    if (!isFiniteRange(buf, pos, 3)) return false;
                    pos += 16;
                    break;
                case OP_STROKE_LINE:
                    if (end - pos < 24) return false;
                    if (!isFiniteRange(buf, pos, 5)) return false;
                    pos += 24;
                    break;
                case OP_STROKE_POLYLINE: {
                    if (end - pos < 12) return false;
                    if (!Float.isFinite(readF32Le(buf, pos))) return false;
                    pos += 8; // width + color
                    long ptCount = readU32Le(buf, pos);
                    pos += 4;
                    if (ptCount > MAX_POLYLINE_POINTS) return false;
                    long neededBytes = ptCount * 8L;
                    if (neededBytes > (long) (end - pos)) return false;
                    int floats = (int) (ptCount * 2L);
                    if (!isFiniteRange(buf, pos, floats)) return false;
                    pos += (int) neededBytes;
                    break;
                }
                case OP_CLIP_RECT:
                    if (end - pos < 16) return false;
                    if (!isFiniteRange(buf, pos, 4)) return false;
                    pos += 16;
                    break;
                case OP_RESTORE_CLIP:
                    break;
                default:
                    return false;
            }
        }
        return pos == end;
    }

    /**
     * Decodes a validated `NVE_DRAW1` slice directly from `buf` onto `canvas` with zero per-op
     * allocations. Returns `false` safely on corrupt/truncated input without throwing.
     */
    public static boolean decodeAndDraw(
            Canvas canvas,
            ByteBuffer buf,
            int offset,
            int length,
            float scale,
            float viewWidthPx,
            float viewHeightPx,
            Paint fillPaint,
            Paint strokePaint,
            Path polyPath,
            RectF scratchRect) {
        if (canvas == null || !validate(buf, offset, length)) {
            return false;
        }
        final float s = (Float.isFinite(scale) && scale > 0.0f) ? scale : 1.0f;
        final int opCount = (int) readU32Le(buf, offset + 11);
        int pos = offset + HEADER_SIZE;
        int clipDepth = 0;

        try {
            for (int i = 0; i < opCount; i++) {
                int op = buf.get(pos++) & 0xFF;
                switch (op) {
                    case OP_CLEAR: {
                        int rgba = readI32Le(buf, pos);
                        pos += 4;
                        if ((rgba >>> 24) != 0) {
                            fillPaint.setColor(rgbaToArgb(rgba));
                            canvas.drawRect(0.0f, 0.0f, viewWidthPx, viewHeightPx, fillPaint);
                        }
                        break;
                    }
                    case OP_FILL_RECT: {
                        float x = readF32Le(buf, pos);
                        float y = readF32Le(buf, pos + 4);
                        float w = readF32Le(buf, pos + 8);
                        float h = readF32Le(buf, pos + 12);
                        int rgba = readI32Le(buf, pos + 16);
                        pos += 20;
                        if (w > 0.0f && h > 0.0f && (rgba >>> 24) != 0) {
                            fillPaint.setColor(rgbaToArgb(rgba));
                            scratchRect.set(x * s, y * s, (x + w) * s, (y + h) * s);
                            canvas.drawRect(scratchRect, fillPaint);
                        }
                        break;
                    }
                    case OP_FILL_ROUNDED_RECT: {
                        float x = readF32Le(buf, pos);
                        float y = readF32Le(buf, pos + 4);
                        float w = readF32Le(buf, pos + 8);
                        float h = readF32Le(buf, pos + 12);
                        float radius = readF32Le(buf, pos + 16);
                        int rgba = readI32Le(buf, pos + 20);
                        pos += 24;
                        if (w > 0.0f && h > 0.0f && (rgba >>> 24) != 0) {
                            fillPaint.setColor(rgbaToArgb(rgba));
                            float r = Math.max(0.0f, radius) * s;
                            scratchRect.set(x * s, y * s, (x + w) * s, (y + h) * s);
                            canvas.drawRoundRect(scratchRect, r, r, fillPaint);
                        }
                        break;
                    }
                    case OP_FILL_CIRCLE: {
                        float cx = readF32Le(buf, pos);
                        float cy = readF32Le(buf, pos + 4);
                        float radius = readF32Le(buf, pos + 8);
                        int rgba = readI32Le(buf, pos + 12);
                        pos += 16;
                        if (radius > 0.0f && (rgba >>> 24) != 0) {
                            fillPaint.setColor(rgbaToArgb(rgba));
                            canvas.drawCircle(cx * s, cy * s, radius * s, fillPaint);
                        }
                        break;
                    }
                    case OP_STROKE_LINE: {
                        float x0 = readF32Le(buf, pos);
                        float y0 = readF32Le(buf, pos + 4);
                        float x1 = readF32Le(buf, pos + 8);
                        float y1 = readF32Le(buf, pos + 12);
                        float width = readF32Le(buf, pos + 16);
                        int rgba = readI32Le(buf, pos + 20);
                        pos += 24;
                        if (width > 0.0f && (rgba >>> 24) != 0) {
                            strokePaint.setColor(rgbaToArgb(rgba));
                            strokePaint.setStrokeWidth(width * s);
                            canvas.drawLine(x0 * s, y0 * s, x1 * s, y1 * s, strokePaint);
                        }
                        break;
                    }
                    case OP_STROKE_POLYLINE: {
                        float width = readF32Le(buf, pos);
                        int rgba = readI32Le(buf, pos + 4);
                        int ptCount = (int) readU32Le(buf, pos + 8);
                        pos += 12;
                        if (ptCount >= 2 && width > 0.0f && (rgba >>> 24) != 0) {
                            polyPath.rewind();
                            float x0 = readF32Le(buf, pos);
                            float y0 = readF32Le(buf, pos + 4);
                            polyPath.moveTo(x0 * s, y0 * s);
                            for (int p = 1; p < ptCount; p++) {
                                int ptOff = pos + (p << 3);
                                polyPath.lineTo(
                                        readF32Le(buf, ptOff) * s,
                                        readF32Le(buf, ptOff + 4) * s);
                            }
                            strokePaint.setColor(rgbaToArgb(rgba));
                            strokePaint.setStrokeWidth(width * s);
                            canvas.drawPath(polyPath, strokePaint);
                        }
                        pos += ptCount << 3;
                        break;
                    }
                    case OP_CLIP_RECT: {
                        float x = readF32Le(buf, pos);
                        float y = readF32Le(buf, pos + 4);
                        float w = readF32Le(buf, pos + 8);
                        float h = readF32Le(buf, pos + 12);
                        pos += 16;
                        canvas.save();
                        clipDepth++;
                        scratchRect.set(x * s, y * s, (x + Math.max(0.0f, w)) * s, (y + Math.max(0.0f, h)) * s);
                        canvas.clipRect(scratchRect);
                        break;
                    }
                    case OP_RESTORE_CLIP: {
                        if (clipDepth > 0) {
                            canvas.restore();
                            clipDepth--;
                        }
                        break;
                    }
                    default:
                        return false;
                }
            }
            return true;
        } finally {
            while (clipDepth > 0) {
                canvas.restore();
                clipDepth--;
            }
        }
    }

    private static boolean isFiniteRange(ByteBuffer buf, int pos, int floatCount) {
        for (int i = 0; i < floatCount; i++) {
            if (!Float.isFinite(readF32Le(buf, pos + (i << 2)))) {
                return false;
            }
        }
        return true;
    }

    private static int readU16Le(ByteBuffer buf, int pos) {
        return (buf.get(pos) & 0xFF) | ((buf.get(pos + 1) & 0xFF) << 8);
    }

    private static int readI32Le(ByteBuffer buf, int pos) {
        return (buf.get(pos) & 0xFF)
                | ((buf.get(pos + 1) & 0xFF) << 8)
                | ((buf.get(pos + 2) & 0xFF) << 16)
                | ((buf.get(pos + 3) & 0xFF) << 24);
    }

    private static long readU32Le(ByteBuffer buf, int pos) {
        return readI32Le(buf, pos) & 0xFFFF_FFFFL;
    }

    private static float readF32Le(ByteBuffer buf, int pos) {
        return Float.intBitsToFloat(readI32Le(buf, pos));
    }

    private static int rgbaToArgb(int rgba) {
        int r = rgba & 0xFF;
        int g = (rgba >>> 8) & 0xFF;
        int b = (rgba >>> 16) & 0xFF;
        int a = (rgba >>> 24) & 0xFF;
        return (a << 24) | (r << 16) | (g << 8) | b;
    }
}
