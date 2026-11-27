package dev.nve.host;

import java.nio.BufferOverflowException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;

/** {@link EventRingWriter} byte layout ({@code nve_abi::encode_events}) and {@link ImageSource} decoding. */
final class EventRingTest {
    private EventRingTest() {}

    static void run(Check c) {
        layout(c);
        overflowLeavesNoTrace(c);
        steadyState(c);
        imageSources(c);
    }

    static void layout(Check c) {
        final EventRingWriter w = new EventRingWriter(ByteBuffer.allocateDirect(512), 7);
        w.point(1, 42, 99L, 1.5f, -2f)
                .text(4, 43, 100L, "h\u00e9\ud83d\ude80")
                .custom(8, 44, 101L, "submit", new byte[] {9, 8})
                .key(12, 45, 102L, "a", "KeyA", 3, 65, true)
                .pointer(15, 46, 103L, 3f, 4f, 1, 2)
                .wheel(20, 47, 104L, 0f, -10f, 4)
                .scroll(3, 48, 105L, 0f, 30f, 300f, 900f)
                .empty(5, 49, 106L);
        c.eq(w.count(), 8, "count");
        final ByteBuffer b = w.buffer().duplicate().order(ByteOrder.LITTLE_ENDIAN);
        b.flip();
        c.eq(b.limit(), w.length(), "length");
        c.eq(b.getInt(), 8, "count prefix");
        header(c, b, 42, 99L, 1, 1);
        c.that(b.getFloat() == 1.5f && b.getFloat() == -2f, "point");
        header(c, b, 43, 100L, 4, 3);
        c.eq(b.getInt(), 7, "text len");
        final byte[] utf8 = new byte[7];
        b.get(utf8);
        c.that(new String(utf8, java.nio.charset.StandardCharsets.UTF_8).equals("h\u00e9\ud83d\ude80"), "text utf8");
        header(c, b, 44, 101L, 8, 5);
        c.eq(b.getShort(), 6, "custom name len");
        b.position(b.position() + 6);
        c.eq(b.getInt(), 2, "custom data len");
        c.that(b.get() == 9 && b.get() == 8, "custom data");
        header(c, b, 45, 102L, 12, 6);
        c.eq(b.getShort(), 1, "key len");
        c.eq(b.get(), 'a', "key");
        c.eq(b.getShort(), 4, "code len");
        b.position(b.position() + 4);
        c.that(b.get() == 3 && b.getInt() == 65 && b.get() == 1, "key modifiers, code, composing");
        header(c, b, 46, 103L, 15, 7);
        c.that(b.getFloat() == 3f && b.getFloat() == 4f && b.get() == 1 && b.get() == 2, "pointer");
        header(c, b, 47, 104L, 20, 8);
        c.that(b.getFloat() == 0f && b.getFloat() == -10f && b.get() == 4, "wheel");
        header(c, b, 48, 105L, 3, 2);
        c.that(b.getFloat() == 0f && b.getFloat() == 30f && b.getFloat() == 300f && b.getFloat() == 900f, "scroll");
        header(c, b, 49, 106L, 5, 0);
        c.eq(b.remaining(), 0, "no trailing bytes");
        w.reset();
        c.that(w.count() == 0 && w.length() == 4 && w.buffer().getInt(0) == 0, "reset");
    }

    static void header(Check c, ByteBuffer b, int node, long ts, int kind, int tag) {
        c.eq(b.getInt(), 7, "surface");
        c.eq(b.getInt(), node, "node");
        c.eq(b.getLong(), ts, "timestamp");
        c.eq(b.get(), kind, "kind");
        c.eq(b.get(), tag, "payload tag");
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
