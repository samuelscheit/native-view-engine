package dev.nve.android;

import android.graphics.Paint;
import android.graphics.Typeface;
import android.os.Build;
import android.text.StaticLayout;
import android.text.TextPaint;

import dev.nve.host.NativeRuntime;

/**
 * Host text measurer backed by Android's text stack ({@link StaticLayout} over a {@link TextPaint}),
 * so runtime layout sizes text exactly as the {@code TextView}s that render it. Sizes are in dp
 * (logical points): the paint runs at the dp font size, and glyph metrics scale linearly.
 */
final class NveTextMeasurer implements NativeRuntime.TextMeasurer {
    private final TextPaint paint = new TextPaint(Paint.ANTI_ALIAS_FLAG);

    @Override
    public boolean measure(String text, float fontSize, int fontWeight, float maxWidth, float[] out) {
        if (text.isEmpty()) {
            out[0] = 0f;
            out[1] = 0f;
            return true;
        }
        paint.setTextSize(Float.isFinite(fontSize) && fontSize > 0f ? fontSize : 16f);
        paint.setTypeface(typeface(fontWeight, false));
        final int wrap;
        if (Float.isInfinite(maxWidth)) {
            wrap = Math.max(1, (int) Math.ceil(paint.measureText(text)));
        } else if (maxWidth <= 0f) {
            wrap = Math.max(1, (int) Math.ceil(longestWord(text)));  // min-content
        } else {
            wrap = Math.max(1, (int) Math.ceil(maxWidth));
        }
        final StaticLayout layout = StaticLayout.Builder.obtain(text, 0, text.length(), paint, wrap)
                .setIncludePad(false)
                .build();
        float widest = 0f;
        for (int i = 0; i < layout.getLineCount(); i++) widest = Math.max(widest, layout.getLineWidth(i));
        out[0] = (float) Math.ceil(widest);
        out[1] = (float) Math.ceil(layout.getHeight());
        return true;
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
