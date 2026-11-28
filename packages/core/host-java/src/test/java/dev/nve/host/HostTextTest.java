package dev.nve.host;

/** {@link Utf8#wellFormed} (lone-surrogate repair for platform text widgets) and {@link AndroidKeys}. */
final class HostTextTest {
    private HostTextTest() {}

    static void run(Check c) {
        final String ok = "h\u00e9\ud83d\ude80";
        c.that(Utf8.wellFormed(ok) == ok, "a well-formed string is returned as is (no copy)");
        c.eq(Utf8.wellFormed("a\ud83db"), "a\uFFFDb", "lone high surrogate");
        c.eq(Utf8.wellFormed("\ude80x"), "\uFFFDx", "lone low surrogate first");
        c.eq(Utf8.wellFormed("x\ud83d"), "x\uFFFD", "high surrogate at the end");
        c.eq(Utf8.wellFormed("\ude80\ud83d"), "\uFFFD\uFFFD", "reversed pair");
        c.eq(Utf8.wellFormed("\ud83d\ud83d\ude80"), "\uFFFD\ud83d\ude80", "a pair after a lone high surrogate survives");
        c.eq(Utf8.wellFormed(""), "", "empty");
        c.eq(Utf8.encodedLength(Utf8.wellFormed("a\ud83db")), 5, "the repaired string encodes");

        c.eq(AndroidKeys.key(0, 'a'), "a", "printable");
        c.eq(AndroidKeys.key(0, 0x1F680), "\ud83d\ude80", "astral printable");
        c.eq(AndroidKeys.key(66, 0xD83D), "Enter", "a surrogate code point is not a character");
        c.eq(AndroidKeys.key(0, 0xDC00), "Unidentified", "lone low surrogate");
        c.eq(AndroidKeys.key(0, 0x80000041), "Unidentified", "COMBINING_ACCENT flag");
        c.eq(AndroidKeys.key(0, 0x7F), "Unidentified", "DEL control");
        c.eq(AndroidKeys.key(131, 0), "F1", "function key");

        c.eq(new String(ImageSource.base64("TWFu", 0), java.nio.charset.StandardCharsets.ISO_8859_1), "Man", "base64 full quad");
        c.eq(new String(ImageSource.base64("TWE=", 0), java.nio.charset.StandardCharsets.ISO_8859_1), "Ma", "base64 one pad");
        c.eq(new String(ImageSource.base64("TQ==", 0), java.nio.charset.StandardCharsets.ISO_8859_1), "M", "base64 two pads");
        c.eq(new String(ImageSource.base64("TW Fu\n", 0), java.nio.charset.StandardCharsets.ISO_8859_1), "Man", "base64 whitespace");
        c.that(ImageSource.base64("", 0).length == 0, "empty base64");
        c.that(ImageSource.base64("TWF", 0) == null, "truncated quad");
        c.that(ImageSource.base64("T===", 0) == null, "three pads");
        c.that(ImageSource.base64("TQ==TWFu", 0) == null, "data after padding");
        c.that(ImageSource.base64("TW!u", 0) == null, "bad character");
        c.that(ImageSource.base64("=AAA", 0) == null, "leading pad");
        c.that(ImageSource.base64("TR==", 0) == null, "non-canonical: low bits under two pads");
        c.that(ImageSource.base64("TWF=", 0) == null, "non-canonical: low bits under one pad");
        c.that(ImageSource.base64("TQ== =", 0) == null, "pad after a complete quantum");
        c.eq(new String(source(0, "DATA:Image/PNG;Base64,TQ==").bytes(), java.nio.charset.StandardCharsets.ISO_8859_1), "M",
                "data URL scheme / type / base64 token are case-insensitive");
        c.that(source(0, "data:text/plain;base64,TQ==").bytes() == null, "non-image data URL");
        c.that(source(0, ";base64,TQ==").bytes() == null, "no scheme");
        c.eq(new String(source(0, "data:image/png;base64,TWFu").bytes(), java.nio.charset.StandardCharsets.ISO_8859_1), "Man", "data URL bytes");
        c.that(source(0, "data:image/png,TWFu").bytes() == null, "non-base64 data URL");
        c.that(source(0, "asset://logo.png").bytes() == null, "asset URL is loaded by the host");
        c.eq(new String(source(1, "PNG!").bytes(), java.nio.charset.StandardCharsets.ISO_8859_1), "PNG!", "inline bytes");
    }

    static ImageSource source(int tag, String payload) {
        final byte[] p = payload.getBytes(java.nio.charset.StandardCharsets.UTF_8);
        final java.nio.ByteBuffer b = java.nio.ByteBuffer.allocate(5 + p.length).order(java.nio.ByteOrder.LITTLE_ENDIAN);
        b.put((byte) tag).putInt(p.length).put(p);
        b.flip();
        return ImageSource.decode(b);
    }
}
