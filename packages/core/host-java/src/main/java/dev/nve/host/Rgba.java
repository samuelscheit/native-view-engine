package dev.nve.host;

/** Channels of a packed {@code RGBA8888} word ({@code r | g<<8 | b<<16 | a<<24}). */
public final class Rgba {
    private Rgba() {}

    public static int r(int packed) {
        return packed & 0xFF;
    }

    public static int g(int packed) {
        return (packed >>> 8) & 0xFF;
    }

    public static int b(int packed) {
        return (packed >>> 16) & 0xFF;
    }

    public static int a(int packed) {
        return packed >>> 24;
    }
}
