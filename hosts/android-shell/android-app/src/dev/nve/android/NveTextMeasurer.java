package dev.nve.android;

import android.graphics.Paint;
import android.graphics.Typeface;
import android.os.Build;
import android.text.StaticLayout;
import android.text.TextPaint;

import dev.nve.host.NativeRuntime;
import dev.nve.host.SceneSemantics;

/**
 * Host text measurer backed by Android's text stack ({@link StaticLayout} over a {@link TextPaint}),
 * so runtime layout sizes text exactly as the {@code TextView}s that render it. Sizes are in dp
 * (logical points): the paint runs at the dp font size, and glyph metrics scale linearly.
 */
final class NveTextMeasurer implements NativeRuntime.TextMeasurer {
    /** Widest `StaticLayout` wrap width used (an int pixel width; larger means unconstrained). */
    static final float MAX_WRAP = 1 << 20;

    private final TextPaint paint = new TextPaint(Paint.ANTI_ALIAS_FLAG);

    @Override
    public void measure(String text, float fontSize, int fontWeight, float maxWidth, float[] out) {
        paint.setTextSize(Float.isFinite(fontSize) && fontSize > 0f ? fontSize : SceneSemantics.DEFAULT_FONT_SIZE);
        paint.setTypeface(typeface(fontWeight, false));
        if (text.isEmpty()) {
            out[0] = 0f;
            out[1] = (float) Math.ceil(paint.getFontMetrics(null));  // an empty line still has a line height
            return;
        }
        // max-content when unconstrained (+Inf, NaN, or wider than any layout), min-content when not
        // positive, else wrapped at the given width.
        final float wrapAt;
        if (!(maxWidth > 0f)) {
            wrapAt = Float.isNaN(maxWidth) ? paint.measureText(text) : longestWord(text);
        } else if (maxWidth >= MAX_WRAP) {
            wrapAt = paint.measureText(text);
        } else {
            wrapAt = maxWidth;
        }
        final int wrap = (int) Math.max(1f, Math.min(MAX_WRAP, (float) Math.ceil(wrapAt)));
        final StaticLayout layout = StaticLayout.Builder.obtain(text, 0, text.length(), paint, wrap)
                .setIncludePad(false)
                .build();
        float widest = 0f;
        for (int i = 0; i < layout.getLineCount(); i++) widest = Math.max(widest, layout.getLineWidth(i));
        out[0] = (float) Math.ceil(widest);
        out[1] = (float) Math.ceil(layout.getHeight());
    }

    private float longestWord(String text) {
        float longest = 0f;
        int start = 0;
        for (int i = 0; i <= text.length(); i++) {
            if (i == text.length() || Character.isWhitespace(text.charAt(i))) {
                if (i > start) longest = Math.max(longest, paint.measureText(text, start, i));
                start = i + 1;
            }
        }
        return longest;
    }

    /** CSS weight ({@code 100..900}) and italic as an Android typeface. */
    static Typeface typeface(int weight, boolean italic) {
        if (Build.VERSION.SDK_INT >= 28) {
            return Typeface.create(Typeface.DEFAULT, Math.max(1, Math.min(1000, weight)), italic);
        }
        final int style = (weight >= 600 ? Typeface.BOLD : Typeface.NORMAL) | (italic ? Typeface.ITALIC : 0);
        return Typeface.create(Typeface.DEFAULT, style);
    }
}
