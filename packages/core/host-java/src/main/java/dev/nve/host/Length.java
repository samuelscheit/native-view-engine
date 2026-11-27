package dev.nve.host;

/** Decodes {@code nve_store::Length}: f32 bits with a 2-bit unit tag in the low mantissa bits. */
public final class Length {
    public static final int POINTS = 0;
    public static final int PERCENT = 1;
    public static final int AUTO = 2;

    private Length() {}

    /** {@link #POINTS}, {@link #PERCENT} or {@link #AUTO} (the unused tag {@code 11} normalizes to auto). */
    public static int unit(int word) {
        final int tag = word & 3;
        return tag == 3 ? AUTO : tag;
    }

    /** Magnitude ({@code 0} for the unused tag {@code 11}). */
    public static float value(int word) {
        return (word & 3) == 3 ? 0f : Float.intBitsToFloat(word & ~3);
    }
}
