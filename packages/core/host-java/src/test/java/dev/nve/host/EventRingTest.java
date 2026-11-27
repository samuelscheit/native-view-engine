package dev.nve.host;

import java.nio.BufferOverflowException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;

import dev.nve.host.generated.EventRingVectors;

/** {@link EventRingWriter} byte layout ({@code nve_abi::encode_events}) and {@link ImageSource} decoding. */
final class EventRingTest {
    private EventRingTest() {}

    static void run(Check c) {
        layout(c);
        resumeAndTruncate(c);
        overflowLeavesNoTrace(c);
        steadyState(c);
        imageSources(c);
        androidKeys(c);
    }

    /** Writes {@code nve_abi::golden_events()} (codegen abi/java_host.rs) and compares byte-for-byte. */
    static EventRingWriter golden(EventRingWriter w) {
        return w.empty(5, 1, 10L)
                .point(1, 2, 11L, 1.5f, -2f)
                .scroll(3, 3, 12L, 0f, 30f, 300f, 900f)
                .text(4, 4, 13L, "h\u00e9\ud83d\ude80")
                .layout(7, 5, 14L, 1f, 2f, 3f, 4f)
                .custom(8, 6, 15L, "submit", new byte[] {9, 8, 7})
                .key(12, 7, 16L, "\u00e9", "KeyE", 0x05, 69, true)
                .pointer(15, 8, 17L, 3f, 4f, 0x02, 0x09)
                .wheel(20, 9, 18L, 0f, -12.5f, 0x01)
                .tableEvent(21, 0x8100_0002, 19L, 0x1122_3344_5566_7788L, 42, new byte[] {1, 2});
    }

    static void layout(Check c) {
        final EventRingWriter w = golden(new EventRingWriter(ByteBuffer.allocateDirect(512), EventRingVectors.SURFACE));
        final byte[] expected = EventRingVectors.ring();
        c.eq(w.count(), 10, "count");
        c.eq(w.length(), expected.length, "golden length");
        final byte[] actual = new byte[w.length()];
        final ByteBuffer b = w.buffer().duplicate();
        ((java.nio.Buffer) b).flip();
        b.get(actual);
        int mismatch = -1;
        for (int i = 0; i < expected.length && mismatch < 0; i++) {
            if (expected[i] != actual[i]) mismatch = i;
        }
        c.eq(mismatch, -1, "first byte differing from the Rust golden ring");
        c.that(java.util.Arrays.equals(expected, actual), "ring bytes equal nve_abi::encode_events");
        w.tableEvent(21, 1, 0L, 1L, 2, null);
        c.eq(w.length(), expected.length + 18 + 16, "null table payload = empty");
        w.reset();
        c.that(w.count() == 0 && w.length() == 4 && w.buffer().getInt(0) == 0, "reset");
    }

    static void resumeAndTruncate(Check c) {
        final EventRingWriter w = new EventRingWriter(ByteBuffer.allocate(256), 7);
        w.point(1, 1, 0L, 1f, 2f);
        final int count = w.count();
        final int len = w.length();
        w.text(4, 2, 0L, "abc").empty(5, 3, 0L);
        w.truncate(count, len);
        c.that(w.count() == 1 && w.length() == len && w.buffer().getInt(0) == 1, "truncate rolls back");
        c.throwsA(IllegalArgumentException.class, () -> w.truncate(2, len), "truncate past count");
        c.throwsA(IllegalArgumentException.class, () -> w.truncate(0, 3), "truncate below header");
        final ByteBuffer bigger = ByteBuffer.allocate(512);
        final ByteBuffer pending = w.buffer().duplicate();
        ((java.nio.Buffer) pending).flip();
        bigger.put(pending);
        final EventRingWriter r = EventRingWriter.resume(bigger, 7, w.count()).empty(5, 4, 0L);
        c.that(r.count() == 2 && r.buffer().getInt(0) == 2 && r.length() == len + 18, "resume continues the ring");
    }

    static void overflowLeavesNoTrace(Check c) {
        final EventRingWriter w = new EventRingWriter(ByteBuffer.allocate(4 + 18 + 8 + 10), 1);
        w.point(1, 1, 0L, 0f, 0f);
        final int len = w.length();
        c.throwsA(BufferOverflowException.class, () -> w.text(4, 2, 0L, "too long for the ring"), "text overflow");
        c.throwsA(BufferOverflowException.class, () -> w.point(1, 2, 0L, 0f, 0f), "point overflow");
        c.binError(BinCodecException.Kind.INVALID_UTF8, () -> w.custom(8, 2, 0L, "\ud800", null), "lone surrogate");
        c.that(w.count() == 1 && w.length() == len && w.buffer().getInt(0) == 1, "failed writes leave the ring intact");
        final StringBuilder big = new StringBuilder();
        for (int i = 0; i < 0x10000; i++) big.append('x');
        final EventRingWriter roomy = new EventRingWriter(ByteBuffer.allocate(1 << 18), 1);
        c.throwsA(IllegalArgumentException.class, () -> roomy.custom(8, 1, 0L, big.toString(), null), "name over u16");
        c.eq(roomy.count(), 0, "rejected name leaves no event");
    }

    static void steadyState(Check c) {
        if (!Alloc.supported()) return;
        final EventRingWriter w = new EventRingWriter(ByteBuffer.allocateDirect(4096), 1);
        final Runnable frames = () -> {
            for (int i = 0; i < 100; i++) {
                w.reset().point(1, 1, i, 1f, 2f).text(4, 1, i, "\u00e9t\u00e9 ok").custom(8, 1, i, "submit", null);
            }
        };
        frames.run();  // warm the scratch and the JIT
        final long bytes = Alloc.bytes(frames);
        c.eq(bytes, 0, "steady-state ring writes allocate nothing");
    }

    static void androidKeys(Check c) {
        c.that(AndroidKeys.key(29, 'a').equals("a") && AndroidKeys.code(29).equals("KeyA"), "letter");
        c.that(AndroidKeys.key(29, 'A').equals("A"), "shifted letter keeps the produced character");
        c.that(AndroidKeys.key(7, '0').equals("0") && AndroidKeys.code(16).equals("Digit9"), "digits");
        c.that(AndroidKeys.key(66, 0).equals("Enter") && AndroidKeys.key(66, '\n').equals("Enter"), "enter (control char ignored)");
        c.that(AndroidKeys.key(67, 0).equals("Backspace") && AndroidKeys.code(67).equals("Backspace"), "backspace");
        c.that(AndroidKeys.key(62, ' ').equals(" ") && AndroidKeys.code(62).equals("Space"), "space");
        c.that(AndroidKeys.key(21, 0).equals("ArrowLeft") && AndroidKeys.code(22).equals("ArrowRight"), "arrows");
        c.that(AndroidKeys.key(59, 0).equals("Shift") && AndroidKeys.code(60).equals("ShiftRight"), "modifiers");
        c.that(AndroidKeys.key(131, 0).equals("F1") && AndroidKeys.code(142).equals("F12"), "function keys");
        c.that(AndroidKeys.key(9999, 0).equals("Unidentified") && AndroidKeys.code(9999).isEmpty(), "unknown key");
        c.that(AndroidKeys.key(29, 0x1F600).equals("\ud83d\ude00"), "astral character");
        c.that(AndroidKeys.key(9999, 0x80000041).equals("Unidentified"), "dead-key combining flag is not a character");
        c.that(AndroidKeys.key(9999, 0x110000).equals("Unidentified"), "invalid code point");
    }

    static ByteBuffer image(int tag, byte[] payload, int declaredLen, int trailing) {
        final ByteBuffer b = ByteBuffer.allocate(5 + payload.length + trailing).order(ByteOrder.LITTLE_ENDIAN);
        b.put((byte) tag).putInt(declaredLen).put(payload).put(new byte[trailing]);
        b.flip();
        return b;
    }

    static void imageSources(Check c) {
        final byte[] url = "asset://logo.png".getBytes(java.nio.charset.StandardCharsets.UTF_8);
        final ImageSource u = ImageSource.decode(image(0, url, url.length, 0));
        c.that("asset://logo.png".equals(u.url) && u.encoded == null, "url source");
        final ImageSource e = ImageSource.decode(image(1, new byte[] {(byte) 0x89, 'P'}, 2, 0));
        c.that(e.url == null && e.encoded.remaining() == 2 && e.encoded.get(1) == 'P', "encoded source");
        c.binError(BinCodecException.Kind.TRUNCATED, () -> ImageSource.decode(image(0, url, url.length + 1, 0)), "declared past end");
        c.binError(BinCodecException.Kind.TRAILING_BYTES, () -> ImageSource.decode(image(0, url, url.length, 3)), "trailing bytes");
        c.binError(BinCodecException.Kind.INVALID_TAG, () -> ImageSource.decode(image(2, url, url.length, 0)), "unknown tag");
        c.binError(BinCodecException.Kind.INVALID_UTF8, () -> ImageSource.decode(image(0, new byte[] {(byte) 0xC0, (byte) 0xAF}, 2, 0)), "bad url utf8");
        c.binError(BinCodecException.Kind.TRUNCATED, () -> ImageSource.decode(ByteBuffer.allocate(3)), "short header");
        c.binError(BinCodecException.Kind.TRUNCATED, () -> ImageSource.decode(image(1, new byte[0], -1, 0)), "u32 length past int");
        c.that(ImageSource.KIND.equals(Key64.of("nve.res.image_src")), "kind key");
    }
}
