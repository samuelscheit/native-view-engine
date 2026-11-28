package dev.nve.host;

import dev.nve.host.generated.EventRingVectors;
import dev.nve.host.generated.NveAbi;
import java.nio.BufferOverflowException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;

/** {@link EventRingWriter} / {@link EventRingReader} ({@code NVEV} v2) and {@link ImageSource} tests. */
final class EventRingTest {
    private EventRingTest() {}

    static void run(Check c) {
        goldenLayoutAndRead(c);
        copyAndTruncate(c);
        overflowAndValidation(c);
        readerRejects(c);
        steadyState(c);
        imageSources(c);
        androidKeys(c);
    }

    private static int node(int i) {
        return (1 << SceneReader.INDEX_BITS) | i;
    }

    /** Writes {@code golden_events()} (codegen abi/java_host.rs) and compares byte-for-byte. */
    static EventRingWriter golden(EventRingWriter w) {
        final int mods = NveAbi.EVENT_FLAG_MOD_META | NveAbi.EVENT_FLAG_MOD_SHIFT;
        return w.focus(node(1), 10L)
                .press(node(2), 11L, 1.5f, -2.0f)
                .scroll(node(3), 12L, 0.0f, 30.0f, 300.0f, 900.0f)
                .textChange(node(4), 13L, "h\u00e9\ud83d\ude80")
                .composition(NveAbi.TEXT_INPUT_EVENT_COMPOSITION_UPDATE, node(4), 14L, "\u3042")
                .layout(node(5), 15L, 1.0f, 2.0f, 3.0f, 4.0f)
                .custom(node(6), 16L, "submit", new byte[] {9, 8, 7})
                .keyDown(node(7), 17L, "\u00e9", "KeyE", mods | NveAbi.EVENT_FLAG_COMPOSING, 69)
                .keyUp(node(7), 18L, "MediaTrackPrevious", "MediaTrackPrevious", NveAbi.EVENT_FLAG_MOD_REPEAT, 177)
                .pointer(NveAbi.INTERACTION_EVENT_POINTER_DOWN, node(8), 19L, 3.0f, 4.0f, 2,
                        NveAbi.EVENT_FLAG_CAPTURE | EventRingWriter.pointerButtonFlag(1))
                .wheel(node(9), 20L, 0.0f, -12.5f, NveAbi.EVENT_FLAG_MOD_CTRL)
                .tableEvent(EventRingVectors.TABLE, 3, node(10), 21L, 0, 1, 2, 3, 4, 5, 6, new byte[] {1, 2});
    }

    static void goldenLayoutAndRead(Check c) {
        final EventRingWriter w = golden(new EventRingWriter(ByteBuffer.allocateDirect(1024), EventRingVectors.SURFACE));
        final byte[] expected = EventRingVectors.frame();
        c.eq(w.count(), 12, "golden count");
        c.eq(w.length(), expected.length, "golden length");
        final byte[] actual = new byte[w.length()];
        final ByteBuffer b = w.buffer().duplicate();
        ((java.nio.Buffer) b).flip();
        b.get(actual);
        int mismatch = -1;
        for (int i = 0; i < expected.length && mismatch < 0; i++) {
            if (expected[i] != actual[i]) mismatch = i;
        }
        c.eq(mismatch, -1, "first byte differing from Rust golden NVEV v2 frame");
        c.that(java.util.Arrays.equals(expected, actual), "NVEV v2 frame equals nve_abi::EventBatchBuf::to_wire_bytes");

        // Decode the golden frame with EventRingReader and check all 12 events.
        final EventRingReader r = new EventRingReader().attach(expected);
        c.eq(r.count(), 12, "reader count");
        c.eq(r.arenaLength(), 61, "reader arena length");
        c.that(r.table(0).equals(NveAbi.TABLE_INTERACTION) && r.event(0) == NveAbi.INTERACTION_EVENT_FOCUS && r.timestamp(0) == 10L, "focus");
        c.that(r.payloadF32(1, 0) == 1.5f && r.payloadF32(1, 1) == -2.0f && r.surface(1) == EventRingVectors.SURFACE, "press");
        c.that(r.table(2).equals(NveAbi.TABLE_SCROLL) && r.payloadF32(2, 3) == 900.0f, "scroll");
        c.eq(r.text(3), "h\u00e9\ud83d\ude80", "text change utf8");
        c.eq(r.text(4), "\u3042", "composition update utf8");
        c.that(r.table(5).equals(NveAbi.TABLE_LAYOUT) && r.payloadF32(5, 2) == 3.0f && r.payloadF32(5, 3) == 4.0f, "layout");
        c.eq(r.customName(6), "submit", "custom name");
        c.that(java.util.Arrays.equals(r.customData(6), new byte[] {9, 8, 7}), "custom data");
        c.eq(r.key(7), "\u00e9", "inline key");
        c.eq(r.code(7), "KeyE", "inline key code");
        c.eq(r.text(7), "", "inline key physical code never leaks as text()");
        c.eq(r.payloadU32(7, 0), 69, "inline keyCode");
        c.eq(r.key(8), "MediaTrackPrevious", "split key");
        c.eq(r.code(8), "MediaTrackPrevious", "split code");
        c.eq(r.payloadU32(8, 0), 177, "split keyCode");
        c.that(r.event(9) == NveAbi.INTERACTION_EVENT_POINTER_DOWN && r.payloadU32(9, 2) == 2
                && (r.flags(9) & NveAbi.EVENT_FLAG_CAPTURE) != 0, "pointer");
        c.that(r.event(10) == NveAbi.INTERACTION_EVENT_WHEEL && r.payloadF32(10, 1) == -12.5f
                && (r.flags(10) & NveAbi.EVENT_FLAG_MOD_CTRL) != 0, "wheel");
        c.that(r.tableBits(11) == EventRingVectors.TABLE && r.event(11) == 3 && r.payloadU32(11, 5) == 6
                && java.util.Arrays.equals(r.bytes(11), new byte[] {1, 2}), "module table event");

        // Null / empty arena payloads produce RawEventBytes {0, 0}.
        w.tableEvent(EventRingVectors.TABLE, 4, node(10), 22L, NveAbi.EVENT_FLAG_UTF8_BYTES | NveAbi.EVENT_FLAG_CUSTOM_NAMED, 0, 0, 0, 0, 0, 0, null);
        c.eq(w.length(), expected.length + EventRingWriter.EVENT_SIZE, "null table payload adds only 64 bytes");
        r.attach(w.buffer(), w.length());
        c.that(r.bytesOffset(12) == 0 && r.bytesLen(12) == 0 && r.flags(12) == 0, "internal framing flags stripped & empty bytes = (0,0)");
        w.reset();
        c.that(w.count() == 0 && w.arenaLength() == 0 && w.length() == EventRingWriter.HEADER_SIZE, "reset");
        r.attach(w.buffer(), w.length());
        c.eq(r.count(), 0, "empty NVEV v2 frame is valid");
    }

    static void copyAndTruncate(Check c) {
        final EventRingWriter w = new EventRingWriter(ByteBuffer.allocate(512), 7);
        w.press(1, 10L, 1f, 2f);
        final int count = w.count();
        final int arena = w.arenaLength();
        w.textChange(2, 11L, "abc").focus(3, 12L);
        w.truncate(count, arena);
        c.that(w.count() == 1 && w.arenaLength() == 0 && w.length() == EventRingWriter.HEADER_SIZE + EventRingWriter.EVENT_SIZE,
                "truncate rolls back");
        c.throwsA(IllegalArgumentException.class, () -> w.truncate(2, arena), "truncate past count");
        c.throwsA(IllegalArgumentException.class, () -> w.truncate(0, 1), "truncate past arena");
        final EventRingWriter bigger = w.copyTo(ByteBuffer.allocateDirect(1024));
        bigger.submit(4, 13L, "go");
        final EventRingReader r = new EventRingReader().attach(bigger.buffer(), bigger.length());
        c.that(r.count() == 2 && r.event(0) == NveAbi.INTERACTION_EVENT_PRESS && "go".equals(r.text(1)), "copyTo preserves batch");
    }

    static void overflowAndValidation(Check c) {
        c.throwsA(IllegalArgumentException.class, () -> new EventRingWriter(ByteBuffer.allocate(13), 1), "buffer shorter than header");
        final EventRingWriter w = new EventRingWriter(ByteBuffer.allocate(EventRingWriter.HEADER_SIZE + 2 * EventRingWriter.EVENT_SIZE + 4), 1);
        w.press(1, 0L, 0f, 0f);
        final int len = w.length();
        c.throwsA(BufferOverflowException.class, () -> w.textChange(2, 0L, "too long for ring"), "arena overflow");
        c.throwsA(BinCodecException.class, () -> w.custom(2, 0L, "\ud800", null), "lone surrogate");
        c.that(w.count() == 1 && w.arenaLength() == 0 && w.length() == len, "failed writes leave batch intact");
        // Inline key boundary: 16 bytes inline vs 17 bytes split.
        final EventRingWriter k = new EventRingWriter(ByteBuffer.allocate(512), 1);
        k.keyDown(1, 1L, "0123456789abcdef", "", 0, 1);
        k.keyDown(1, 2L, "0123456789abcdefg", "Code", 0, 2);
        final EventRingReader r = new EventRingReader().attach(k.buffer(), k.length());
        c.that((r.flags(0) & NveAbi.EVENT_FLAG_INLINE_KEY_UTF8) != 0 && (r.flags(0) & NveAbi.EVENT_FLAG_UTF8_BYTES) == 0
                && "0123456789abcdef".equals(r.key(0)) && r.code(0).isEmpty(), "16 B inline key without code");
        c.that((r.flags(1) & NveAbi.EVENT_FLAG_KEY_SPLIT_BYTES) != 0
                && "0123456789abcdefg".equals(r.key(1)) && "Code".equals(r.code(1)), "17 B split key");
    }

    static void readerRejects(Check c) {
        final byte[] valid = EventRingVectors.frame();
        final EventRingReader r = new EventRingReader();
        c.binError(BinCodecException.Kind.TRUNCATED, () -> r.attach(new byte[13]), "short header");
        final byte[] badMagic = valid.clone();
        badMagic[0] = 'X';
        c.binError(BinCodecException.Kind.INVALID_TAG, () -> r.attach(badMagic), "bad magic");
        final byte[] v1 = valid.clone();
        v1[4] = 1;
        c.binError(BinCodecException.Kind.INVALID_TAG, () -> r.attach(v1), "v1 rejected");
        c.binError(BinCodecException.Kind.TRAILING_BYTES,
                () -> r.attach(java.util.Arrays.copyOf(valid, valid.length + 1)), "trailing byte");
        c.binError(BinCodecException.Kind.TRUNCATED,
                () -> r.attach(java.util.Arrays.copyOf(valid, valid.length - 1)), "truncated arena");

        // Single-event mutations for ingress validation parity with Rust validate_one_event.
        final EventRingWriter single = new EventRingWriter(ByteBuffer.allocate(256), 1).textChange(1, 0L, "ab");
        final byte[] base = new byte[single.length()];
        final ByteBuffer dup = single.buffer().duplicate();
        ((java.nio.Buffer) dup).flip();
        dup.get(base);

        final byte[] badSpan = base.clone();
        ByteBuffer.wrap(badSpan).order(ByteOrder.LITTLE_ENDIAN).putInt(14 + 56, 2);
        c.binError(BinCodecException.Kind.TRUNCATED, () -> r.attach(badSpan), "span oob");

        final byte[] conflict = base.clone();
        ByteBuffer.wrap(conflict).order(ByteOrder.LITTLE_ENDIAN)
                .putInt(14 + 28, NveAbi.EVENT_FLAG_INLINE_KEY_UTF8 | NveAbi.EVENT_FLAG_KEY_SPLIT_BYTES);
        c.binError(BinCodecException.Kind.INVALID_TAG, () -> r.attach(conflict), "conflicting flags");

        final byte[] badUtf8 = base.clone();
        badUtf8[badUtf8.length - 1] = (byte) 0xFF;
        c.binError(BinCodecException.Kind.INVALID_UTF8, () -> r.attach(badUtf8), "invalid utf8 bytes");

        final byte[] bigInline = base.clone();
        ByteBuffer.wrap(bigInline).order(ByteOrder.LITTLE_ENDIAN)
                .putInt(14 + 28, NveAbi.EVENT_FLAG_INLINE_KEY_UTF8)
                .putInt(14 + 36, 17);
        c.binError(BinCodecException.Kind.TOO_LARGE, () -> r.attach(bigInline), "inline key > 16");

        // Split key cutting through a 2-byte UTF-8 sequence.
        final EventRingWriter split = new EventRingWriter(ByteBuffer.allocate(256), 1)
                .keyDown(1, 0L, "\u00e9\u00e9\u00e9\u00e9\u00e9\u00e9\u00e9\u00e9\u00e9", "X", 0, 1);
        final byte[] badSplit = new byte[split.length()];
        final ByteBuffer sdup = split.buffer().duplicate();
        ((java.nio.Buffer) sdup).flip();
        sdup.get(badSplit);
        ByteBuffer.wrap(badSplit).order(ByteOrder.LITTLE_ENDIAN).putInt(14 + 36, 1);  // mid-codepoint
        c.binError(BinCodecException.Kind.INVALID_UTF8, () -> r.attach(badSplit), "split key mid-utf8 boundary");
        c.eq(r.count(), 0, "failed attach leaves reader detached");

        // Non-canonical index-0 handles (`0x01000000`) canonicalize to `0` on read & write, and
        // `CUSTOM_NAMED | UTF8_BYTES` never leaks through `text()`.
        final EventRingWriter canonW = new EventRingWriter(ByteBuffer.allocate(256), 0x01000000)
                .custom(0x01000000, 1L, "ping", "data".getBytes(java.nio.charset.StandardCharsets.UTF_8));
        final byte[] canonBytes = new byte[canonW.length()];
        final ByteBuffer cdup = canonW.buffer().duplicate();
        ((java.nio.Buffer) cdup).flip();
        cdup.get(canonBytes);
        r.attach(canonBytes);
        c.that(r.surface(0) == 0 && r.node(0) == 0, "EventRingWriter canonicalizes index-0 handles");
        // Patch non-canonical handles & CUSTOM_NAMED | UTF8_BYTES directly onto wire bytes.
        ByteBuffer.wrap(canonBytes).order(ByteOrder.LITTLE_ENDIAN)
                .putInt(14 + NveAbi.EVENT_OFFSET_SURFACE, 0x01000000)
                .putInt(14 + NveAbi.EVENT_OFFSET_NODE, 0x01000000)
                .putInt(14 + NveAbi.EVENT_OFFSET_FLAGS, NveAbi.EVENT_FLAG_CUSTOM_NAMED | NveAbi.EVENT_FLAG_UTF8_BYTES);
        r.attach(canonBytes);
        c.eq(r.surface(0), 0, "EventRingReader canonicalizes surface 0x01000000 -> 0");
        c.eq(r.node(0), 0, "EventRingReader canonicalizes node 0x01000000 -> 0");
        c.eq(r.text(0), "", "CUSTOM_NAMED | UTF8_BYTES never leaks through text()");
        c.eq(r.customName(0), "ping", "customName intact");
        c.that(java.util.Arrays.equals(r.customData(0), "data".getBytes(java.nio.charset.StandardCharsets.UTF_8)), "customData intact");
    }

    static void steadyState(Check c) {
        if (!Alloc.supported()) return;
        final EventRingWriter w = new EventRingWriter(ByteBuffer.allocateDirect(4096), 1);
        final EventRingReader r = new EventRingReader();
        final Runnable frames = () -> {
            for (int i = 0; i < 100; i++) {
                w.reset()
                        .press(1, i, 1f, 2f)
                        .textChange(1, i, "\u00e9t\u00e9 ok")
                        .keyDown(1, i, "\u00e9", "KeyE", 0, 69)
                        .custom(1, i, "submit", null);
                r.attach(w.buffer(), w.length());
            }
        };
        frames.run();  // warm scratch and JIT
        final long bytes = Alloc.bytes(frames);
        c.eq(bytes, 0, "steady-state NVEV v2 writes + validation allocate nothing");
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
        final int pad = (4 - (payload.length & 3)) & 3;
        final ByteBuffer b = ByteBuffer.allocate(8 + payload.length + pad + trailing).order(ByteOrder.LITTLE_ENDIAN);
        b.putInt(tag).putInt(declaredLen).put(payload).put(new byte[pad + trailing]);
        b.flip();
        return b;
    }

    static void imageSources(Check c) {
        final byte[] url = "asset://logo.png".getBytes(java.nio.charset.StandardCharsets.UTF_8);
        final ImageSource u = ImageSource.decode(image(0, url, url.length, 0));
        c.that("asset://logo.png".equals(u.url) && u.encoded == null, "url source");
        final ImageSource e = ImageSource.decode(image(1, new byte[] {(byte) 0x89, 'P'}, 2, 0));
        c.that(e.url == null && e.encoded.remaining() == 2 && e.encoded.get(1) == 'P', "encoded source (padded to 4 B)");
        final ByteBuffer badPad = image(1, new byte[] {(byte) 0x89, 'P'}, 2, 0);
        badPad.put(badPad.limit() - 1, (byte) 1);
        c.binError(BinCodecException.Kind.TRAILING_BYTES, () -> ImageSource.decode(badPad), "non-zero pad byte");
        final ByteBuffer unaligned = ByteBuffer.allocate(10).order(ByteOrder.LITTLE_ENDIAN)
                .putInt(1).putInt(2).put((byte) 0x89).put((byte) 'P');
        unaligned.flip();
        c.binError(BinCodecException.Kind.TRUNCATED, () -> ImageSource.decode(unaligned), "missing pad bytes");
        c.binError(BinCodecException.Kind.TRUNCATED, () -> ImageSource.decode(image(0, url, url.length + 1, 0)), "declared past end");
        c.binError(BinCodecException.Kind.TRAILING_BYTES, () -> ImageSource.decode(image(0, url, url.length, 3)), "trailing bytes");
        c.binError(BinCodecException.Kind.INVALID_TAG, () -> ImageSource.decode(image(2, url, url.length, 0)), "unknown tag");
        c.binError(BinCodecException.Kind.INVALID_UTF8, () -> ImageSource.decode(image(0, new byte[] {(byte) 0xC0, (byte) 0xAF}, 2, 0)), "bad url utf8");
        c.binError(BinCodecException.Kind.TRUNCATED, () -> ImageSource.decode(ByteBuffer.allocate(7)), "short header");
        c.binError(BinCodecException.Kind.TRUNCATED, () -> ImageSource.decode(image(1, new byte[0], -1, 0)), "u32 length past int");
        c.that(ImageSource.KIND.equals(Key64.of("nve.res.image_src")), "kind key");
    }
}
