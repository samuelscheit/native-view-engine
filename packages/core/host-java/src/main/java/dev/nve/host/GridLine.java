package dev.nve.host;

/** Decodes packed 16-bit {@code nve_store::GridLine}: 2-bit tag in bits 14..15, 14-bit payload below. */
public final class GridLine {
    public static final int AUTO = 0;
    public static final int LINE = 1;
    public static final int SPAN = 2;

    public static final int MIN_LINE = -8192;
    public static final int MAX_LINE = 8191;
    public static final int MAX_SPAN = 16383;

    static final int TAG_SHIFT = 14;
    static final int TAG_NICHE = 0x3 << TAG_SHIFT;

    private GridLine() {}

    /** {@link #AUTO}, {@link #LINE}, or {@link #SPAN} (unused tag {@code 11} normalizes to auto). */
    public static int kind(int bits) {
        final int tag = (bits >>> TAG_SHIFT) & 0x3;
        return tag == LINE || tag == SPAN ? tag : AUTO;
    }

    /** Signed 14-bit line index when {@link #kind} is {@link #LINE} ({@code 0} otherwise). */
    public static int line(int bits) {
        return kind(bits) == LINE ? ((bits & 0x3FFF) << 18) >> 18 : 0;
    }

    /** Unsigned 14-bit span count when {@link #kind} is {@link #SPAN} ({@code 0} otherwise). */
    public static int span(int bits) {
        return kind(bits) == SPAN ? bits & 0x3FFF : 0;
    }

    /** Encodes a 16-bit {@link #LINE} (clamped to {@code [MIN_LINE, MAX_LINE]}). */
    public static int encodeLine(int value) {
        final int clamped = Math.max(MIN_LINE, Math.min(MAX_LINE, value));
        return (LINE << TAG_SHIFT) | (clamped & 0x3FFF);
    }

    /** Encodes a 16-bit {@link #SPAN} (clamped to {@code [0, MAX_SPAN]}). */
    public static int encodeSpan(int value) {
        final int clamped = Math.max(0, Math.min(MAX_SPAN, value));
        return (SPAN << TAG_SHIFT) | clamped;
    }
}
