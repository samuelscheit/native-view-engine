package dev.nve.android;

import android.content.Context;
import android.graphics.Canvas;
import android.graphics.Paint;
import android.graphics.Path;
import android.graphics.RectF;
import android.view.View;

import dev.nve.host.DrawList;
import dev.nve.host.Rgba;

import java.nio.Buffer;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;

/**
 * {@code nve.res.draw_list} ({@code FlatResource}) vector draw-list view of {@code nve.canvas}
 * nodes, painted through the shared {@link DrawList} codec.
 *
 * <p>{@link #setDrawList} copies the frame's draw-list resource (its native buffer dies when the
 * frame is released) into a reusable buffer (no steady-state allocation once warm) and validates
 * it once; {@link #onDraw} streams it with reusable {@code Paint}, {@code Path}, and {@code RectF}.
 */
public final class NveCanvasView extends View implements DrawList.Sink {
    private final Paint fillPaint = new Paint(Paint.ANTI_ALIAS_FLAG);
    private final Paint strokePaint = new Paint(Paint.ANTI_ALIAS_FLAG);
    private final Path polyPath = new Path();
    private final RectF scratchRect = new RectF();

    /** Validated draw list (bytes from 0), or {@code null}. */
    private ByteBuffer drawList;
    private ByteBuffer retainedBuffer;
    private float density = 1.0f;

    /** Paint state of the current {@link #onDraw}. */
    private Canvas canvas;
    private boolean polyDrawn;

    public NveCanvasView(Context context) {
        super(context);
        fillPaint.setStyle(Paint.Style.FILL);
        strokePaint.setStyle(Paint.Style.STROKE);
        strokePaint.setStrokeCap(Paint.Cap.ROUND);
        strokePaint.setStrokeJoin(Paint.Join.ROUND);
        setWillNotDraw(false);
    }

    public void setDensity(float nextDensity) {
        final float safe = Float.isFinite(nextDensity) && nextDensity > 0.0f ? nextDensity : 1.0f;
        if (Float.floatToRawIntBits(density) != Float.floatToRawIntBits(safe)) {
            density = safe;
            invalidate();
        }
    }

    /** Copies and validates draw list {@code src} (its remaining bytes); an invalid one clears the view. */
    public void setDrawList(ByteBuffer src) {
        final int length = src == null ? 0 : src.remaining();
        if (length < DrawList.HEADER_SIZE || (length & 3) != 0) {
            clearDrawList();
            return;
        }
        if (retainedBuffer == null || retainedBuffer.capacity() < length) {
            final int alloc = Math.max(length, retainedBuffer != null ? retainedBuffer.capacity() << 1 : 256);
            retainedBuffer = ByteBuffer.allocateDirect(alloc).order(ByteOrder.LITTLE_ENDIAN);
        }
        ((Buffer) retainedBuffer).clear();
        retainedBuffer.put(src.duplicate());  // one bulk copy
        if (DrawList.validate(retainedBuffer, 0, length) != null) {
            clearDrawList();
            return;
        }
        drawList = retainedBuffer;
        invalidate();
    }

    public void clearDrawList() {
        final boolean had = drawList != null;
        drawList = null;
        if (had) invalidate();
    }

    @Override
    protected void onDraw(Canvas c) {
        super.onDraw(c);
        if (drawList == null) return;
        canvas = c;
        try {
            DrawList.streamValidated(drawList, 0, this);
        } finally {
            canvas = null;
        }
    }

    private static boolean visible(int rgba) {
        return rgba >>> 24 != 0;
    }

    private void rect(float x, float y, float w, float h) {
        final float s = density;
        scratchRect.set(x * s, y * s, (x + Math.max(0.0f, w)) * s, (y + Math.max(0.0f, h)) * s);
    }

    @Override
    public void onClear(int rgba) {
        if (!visible(rgba)) return;
        fillPaint.setColor(Rgba.toArgb(rgba));
        canvas.drawRect(0.0f, 0.0f, getWidth(), getHeight(), fillPaint);
    }

    @Override
    public void onFillRect(float x, float y, float w, float h, int rgba) {
        if (w <= 0.0f || h <= 0.0f || !visible(rgba)) return;
        fillPaint.setColor(Rgba.toArgb(rgba));
        rect(x, y, w, h);
        canvas.drawRect(scratchRect, fillPaint);
    }

    @Override
    public void onFillRoundedRect(float x, float y, float w, float h, float radius, int rgba) {
        if (w <= 0.0f || h <= 0.0f || !visible(rgba)) return;
        fillPaint.setColor(Rgba.toArgb(rgba));
        final float r = Math.max(0.0f, radius) * density;
        rect(x, y, w, h);
        canvas.drawRoundRect(scratchRect, r, r, fillPaint);
    }

    @Override
    public void onFillCircle(float cx, float cy, float radius, int rgba) {
        if (radius <= 0.0f || !visible(rgba)) return;
        fillPaint.setColor(Rgba.toArgb(rgba));
        canvas.drawCircle(cx * density, cy * density, radius * density, fillPaint);
    }

    @Override
    public void onStrokeLine(float x0, float y0, float x1, float y1, float width, int rgba) {
        if (width <= 0.0f || !visible(rgba)) return;
        strokePaint.setColor(Rgba.toArgb(rgba));
        strokePaint.setStrokeWidth(width * density);
        canvas.drawLine(x0 * density, y0 * density, x1 * density, y1 * density, strokePaint);
    }

    @Override
    public void onStrokePolylineStart(int pointCount, float width, int rgba) {
        polyDrawn = pointCount >= 2 && width > 0.0f && visible(rgba);
        if (!polyDrawn) return;
        polyPath.rewind();
        strokePaint.setColor(Rgba.toArgb(rgba));
        strokePaint.setStrokeWidth(width * density);
    }

    @Override
    public void onStrokePolylinePoint(int index, float x, float y) {
        if (!polyDrawn) return;
        if (index == 0) {
            polyPath.moveTo(x * density, y * density);
        } else {
            polyPath.lineTo(x * density, y * density);
        }
    }

    @Override
    public void onStrokePolylineEnd() {
        if (polyDrawn) canvas.drawPath(polyPath, strokePaint);
    }

    @Override
    public void onClipRect(float x, float y, float w, float h) {
        canvas.save();
        rect(x, y, w, h);
        canvas.clipRect(scratchRect);
    }

    @Override
    public void onRestoreClip() {
        canvas.restore();
    }
}
