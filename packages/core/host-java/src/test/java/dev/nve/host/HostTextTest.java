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
    }
}
