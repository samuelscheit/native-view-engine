package dev.nve.host;

import java.util.Arrays;

final class BinCodecTest {
    private BinCodecTest() {}

    /** Validated by Rust {@code BinValueView::parse}: {"k": [1, "é", null, true, 1.5, b"\xFF"]}. */
    static final String GOLDEN_HEX =
            "07 34000000 01000000 01000000 6b 06 26000000 06000000 02 0100000000000000 04 02000000 c3a9 00 0101 03 "
                    + "000000000000f83f 05 01000000 ff";

    static byte[] fromHex(String hex) {
        final String digits = hex.replace(" ", "");
        final byte[] out = new byte[digits.length() / 2];
        for (int i = 0; i < out.length; i++) out[i] = (byte) Integer.parseInt(digits.substring(2 * i, 2 * i + 2), 16);
        return out;
    }

    static byte[] bytes(int... values) {
        final byte[] out = new byte[values.length];
        for (int i = 0; i < values.length; i++) out[i] = (byte) values[i];
        return out;
    }

    static void run(Check c) {
        final byte[] golden = fromHex(GOLDEN_HEX);
        golden(c, golden);
        primitives(c);
        rejections(c);
        depth(c);
        truncationAndFuzz(c, golden);
        dynamicValue(c, golden);
        utf8(c);
        hardening(c, golden);
    }

    static void hardening(Check c, byte[] golden) {
        // Container limit: children never read past their container (Rust `&bytes[..end]`).
        c.binError(BinCodecException.Kind.TRUNCATED,
                () -> BinReader.validate(bytes(6, 5, 0, 0, 0, 1, 0, 0, 0, 4, 2, 0, 0, 0, 0xC0, 0xAF)), "escape container");
        c.binError(BinCodecException.Kind.TRUNCATED,
                () -> BinReader.validate(bytes(6, 10, 0, 0, 0, 1, 0, 0, 0, 6, 9, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0)), "nested escape");
        // BOM is text on both paths.
        for (String s : new String[] {"\ufeff", "\ufeffx", "\ufeff\ufeffx"}) {
            final BinWriter w = new BinWriter();
            final long m = w.beginMap(1);
            w.writeMapKey(s);
            w.writeStr(s);
            w.end(m);
            final BinReader r = new BinReader(w.toByteArray());
            c.that(r.beginMap() == 1 && r.readMapKey().equals(s) && r.readStr().equals(s), "BOM round trip " + s.length());
        }
        // Lone surrogates are rejected, never written as '?' or U+FFFD; the buffer stays untouched.
        for (String lone : new String[] {"\ud800", "a\udc00", "\ud83d", "\ude80\ud83d"}) {
            final BinWriter w = new BinWriter();
            c.binError(BinCodecException.Kind.INVALID_UTF8, () -> w.writeMapKey(lone), "map key lone surrogate");
            c.eq(w.size(), 0, "nothing written for a rejected key");
            c.binError(BinCodecException.Kind.INVALID_UTF8, () -> w.writeStr(lone), "str lone surrogate");
            c.eq(w.size(), 0, "nothing written for a rejected string (not even the tag)");
        }
        // Stale / foreign marks never patch out of bounds.
        final BinWriter w = new BinWriter();
        final long mark = w.beginArray(1);
        w.writeNull();
        w.clear();
        c.binError(BinCodecException.Kind.INVALID_MARK, () -> w.end(mark), "stale mark after clear");
        w.writeNull();
        c.binError(BinCodecException.Kind.INVALID_MARK, () -> w.end(0), "mark without a container");
        c.binError(BinCodecException.Kind.INVALID_MARK, () -> w.end(-1), "negative mark");
        // Stale after clear() + refill: the old offset is in range again; nothing may be patched.
        final BinWriter refill = new BinWriter();
        final long stale = refill.beginArray(1);
        refill.clear();
        refill.writeBytes(new byte[16]);
        final byte[] before = refill.toByteArray();
        c.binError(BinCodecException.Kind.INVALID_MARK, () -> refill.end(stale), "stale mark after refill");
        c.that(Arrays.equals(refill.toByteArray(), before), "stale end writes nothing");
        // end(0) / end(1) on a 9-byte buffer would overwrite the I64 tag and payload.
        final BinWriter nine = new BinWriter();
        nine.writeI64(0x0102030405060708L);
        final byte[] nineBefore = nine.toByteArray();
        c.binError(BinCodecException.Kind.INVALID_MARK, () -> nine.end(0L), "end(0) on foreign bytes");
        c.binError(BinCodecException.Kind.INVALID_MARK, () -> nine.end((1L << 32) | 1), "forged mark on foreign bytes");
        c.that(Arrays.equals(nine.toByteArray(), nineBefore), "foreign bytes untouched");
        final BinWriter staleNine = new BinWriter();
        final long staleMark = staleNine.beginArray(1);
        staleNine.clear();
        staleNine.writeI64(-1);
        final byte[] staleBefore = staleNine.toByteArray();
        c.binError(BinCodecException.Kind.INVALID_MARK, () -> staleNine.end(staleMark), "stale mark after a 9-byte refill");
        c.that(Arrays.equals(staleNine.toByteArray(), staleBefore), "9-byte refill untouched");
        // Double end: the second would re-patch a closed container with the grown length.
        final BinWriter twice = new BinWriter();
        final long open = twice.beginArray(1);
        twice.writeNull();
        twice.end(open);
        twice.writeNull();
        final byte[] closed = twice.toByteArray();
        c.binError(BinCodecException.Kind.INVALID_MARK, () -> twice.end(open), "double end");
        c.that(Arrays.equals(twice.toByteArray(), closed), "closed container untouched");
        // TOO_LARGE is decided before any byte: a zero-RAM 2 GiB string (715,827,880 x U+20AC).
        final BinWriter huge = new BinWriter();
        huge.writeNull();
        c.binError(BinCodecException.Kind.TOO_LARGE, () -> huge.writeStr(new Repeated('\u20ac', 715_827_880)), "2 GiB string");
        c.eq(huge.size(), 1, "rejected oversized writes leave nothing behind");
        // A container recreated at the very same offset: only the generation tells them apart.
        final BinWriter same = new BinWriter();
        final long old = same.beginArray(1);
        same.clear();
        final long fresh = same.beginArray(1);
        same.writeNull();
        c.binError(BinCodecException.Kind.INVALID_MARK, () -> same.end(old), "stale mark on a recreated container");
        same.end(fresh);
        BinReader.validate(same.toByteArray());
        // Utf8 ranges are bounds-checked (no overflowed `offset + length`).
        final byte[] bad = bytes(0x41, 0xC0, 0xAF);
        c.throwsA(IndexOutOfBoundsException.class, () -> Utf8.isValid(bad, 1, Integer.MAX_VALUE), "isValid overflow");
        c.throwsA(IndexOutOfBoundsException.class, () -> Utf8.isValid(bad, 0, -1), "isValid negative length");
        c.throwsA(IndexOutOfBoundsException.class, () -> Utf8.isValid(bad, -1, 1), "isValid negative offset");
        c.throwsA(IndexOutOfBoundsException.class, () -> Utf8.isValid(bad, 2, 2), "isValid past end");
        c.that(!Utf8.isValid(bad, 1, 2) && Utf8.isValid(bad, 0, 1), "isValid slices");
        c.throwsA(IndexOutOfBoundsException.class,
                () -> Utf8.isValid(java.nio.ByteBuffer.wrap(bad), 1, Integer.MAX_VALUE), "isValid(ByteBuffer) overflow");
        // Validation and steady-state writes allocate nothing per call (the probe must be live).
        c.that(Alloc.supported(), "thread allocation counter is supported and live");
        final BinWriter big = new BinWriter(1 << 20);
        final long arr = big.beginArray(3);
        big.writeBytes(new byte[256 * 1024]);
        big.writeStr(new String(new char[64 * 1024]).replace('\0', 'x'));
        big.writeStr("h\u00e9 \ud83d\ude80");
        big.end(arr);
        final byte[] payload = big.toByteArray();
        BinReader.validate(payload);
        // Writer reuse alone, tightly. OpenJ9 reports thread allocation at TLH-refill granularity (tens of
        // KiB), so resolution comes from iterations: < 2.6 B per iteration, while reverting writeStr to
        // getBytes(UTF_8) costs >= 32 B per iteration (>= 3.2 MiB here).
        final BinWriter reuse = new BinWriter(256);
        reuse.writeStr("warm-up \u00e9 \ud83d\ude80");
        final long written = Alloc.bytes(() -> {
            for (int i = 0; i < 100_000; i++) {
                reuse.clear();
                final long a = reuse.beginArray(3);
                reuse.writeI64(i);
                reuse.writeStr("steady \u00e9 \ud83d\ude80");
                reuse.writeMapKey("key");
                reuse.end(a);
            }
        });
        c.that(written < 256 * 1024, "writer reuse allocates nothing per call (" + written + " bytes / 100k iterations)");
        // Validation: 1000 passes over a 320 KiB payload; the only allocation is each BinReader (~32 B),
        // while decoding the 64 Ki-char string or copying the 256 KiB bytes would cost >= 64 KiB per pass.
        // Warm up first so one-time JIT / OSR bookkeeping is not billed to the measured loop.
        for (int i = 0; i < 100; i++) new BinReader(payload).checkValue();
        final long validated = Alloc.bytes(() -> {
            for (int i = 0; i < 1000; i++) new BinReader(payload).checkValue();
        });
        // Only the 1000 readers (~32-48 KiB) plus TLH-granularity noise; a decode or copy is >= 64 MiB.
        c.that(validated < 1024 * 1024, "validation allocates only the reader (" + validated + " bytes / 1000 passes)");
    }

    static void golden(Check c, byte[] golden) {
        final BinWriter w = new BinWriter(4);  // forces growth
        final long map = w.beginMap(1);
        w.writeMapKey("k");
        final long arr = w.beginArray(6);
        w.writeI64(1);
        w.writeStr("\u00e9");
        w.writeNull();
        w.writeBool(true);
        w.writeF64(1.5);
        w.writeBytes(new byte[] {(byte) 0xFF});
        w.end(arr);
        w.end(map);
        c.that(Arrays.equals(w.toByteArray(), golden), "golden encoding");
        BinReader.validate(golden);
        final BinReader r = new BinReader(golden);
        c.eq(r.beginMap(), 1, "map count");
        c.that(r.readMapKey().equals("k"), "map key");
        c.eq(r.beginArray(), 6, "array count");
        c.eq(r.readI64(), 1, "i64");
        c.that(r.readStr().equals("\u00e9"), "str");
        c.that(r.readNull(), "null");
        c.that(!r.readNull(), "non-null stays");
        c.that(r.readBool(), "bool");
        c.that(r.readF64() == 1.5, "f64");
        c.that(Arrays.equals(r.readBytes(), new byte[] {(byte) 0xFF}), "bytes");
        c.that(r.atEnd(), "at end");
        w.clear();
        c.eq(w.size(), 0, "clear");
    }

    static void primitives(Check c) {
        final BinWriter w = new BinWriter();
        w.u8(0xAA);
        w.u16(0xBEEF);
        w.u32(0xDEADBEEF);
        w.u64(0x0123456789ABCDEFL);
        w.f32(-0.0f);
        w.f64(Double.POSITIVE_INFINITY);
        w.writeI64(-7);
        w.writeI64(Long.MIN_VALUE);
        w.writeF64(Double.NaN);
        final BinReader r = new BinReader(w.toByteArray());
        c.eq(r.u8(), 0xAA, "u8");
        c.eq(r.u16(), 0xBEEF, "u16");
        c.eq(r.u32(), 0xDEADBEEF, "u32");
        c.eq(r.u64(), 0x0123456789ABCDEFL, "u64");
        c.eq(Float.floatToRawIntBits(r.f32()), 0x80000000, "-0.0f");
        c.that(r.f64() == Double.POSITIVE_INFINITY, "inf");
        c.that(r.readF64() == -7.0, "readF64 widens i64");
        c.eq(r.readI64(), Long.MIN_VALUE, "i64 min");
        c.that(Double.isNaN(r.readF64()), "nan");
        c.binError(BinCodecException.Kind.TRUNCATED, r::u8, "read past end");
        final BinReader window = new BinReader(bytes(9, 9, 0, 9), 2, 1);
        c.that(window.readNull() && window.atEnd(), "offset window");
        c.throwsA(IndexOutOfBoundsException.class, () -> new BinReader(new byte[2], 1, 2), "bad window");
    }

    static void rejections(Check c) {
        c.binError(BinCodecException.Kind.INVALID_BOOL, () -> BinReader.validate(bytes(1, 2)), "bool 2");
        c.binError(BinCodecException.Kind.INVALID_TAG, () -> BinReader.validate(bytes(8)), "tag 8");
        c.binError(BinCodecException.Kind.INVALID_UTF8, () -> BinReader.validate(bytes(4, 2, 0, 0, 0, 0xC0, 0xAF)), "overlong");
        c.binError(BinCodecException.Kind.TRUNCATED, () -> BinReader.validate(bytes(4, 0xFF, 0xFF, 0xFF, 0xFF, 0x61)), "huge len");
        c.binError(BinCodecException.Kind.TRUNCATED, () -> BinReader.validate(bytes(6, 4, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF)), "count bomb");
        c.binError(BinCodecException.Kind.TRUNCATED, () -> BinReader.validate(bytes(7, 8, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0)), "map bomb");
        c.binError(BinCodecException.Kind.TRAILING_BYTES, () -> BinReader.validate(bytes(6, 6, 0, 0, 0, 1, 0, 0, 0, 0, 0)), "slack");
        c.binError(BinCodecException.Kind.TRAILING_BYTES, () -> BinReader.validate(bytes(0, 0)), "two values");
        c.binError(BinCodecException.Kind.TRUNCATED,
                () -> BinReader.validate(bytes(6, 5, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0)), "overrun");
        c.binError(BinCodecException.Kind.UNEXPECTED_TAG, () -> new BinReader(bytes(0)).readI64(), "wrong tag");
    }

    static byte[] nested(int depth) {
        final BinWriter w = new BinWriter();
        final long[] marks = new long[depth];
        for (int i = 0; i < depth; i++) marks[i] = w.beginArray(1);
        w.writeNull();
        for (int i = depth - 1; i >= 0; i--) w.end(marks[i]);
        return w.toByteArray();
    }

    static void depth(Check c) {
        BinReader.validate(nested(BinTag.MAX_DEPTH));
        c.binError(BinCodecException.Kind.MAX_DEPTH, () -> BinReader.validate(nested(BinTag.MAX_DEPTH + 1)), "depth 65");
    }

    static void truncationAndFuzz(Check c, byte[] golden) {
        for (int n = 0; n < golden.length; n++) {
            final byte[] prefix = Arrays.copyOf(golden, n);
            c.throwsA(BinCodecException.class, () -> BinReader.validate(prefix), "prefix " + n);
            c.throwsA(BinCodecException.class, () -> new BinReader(prefix).skipValue(), "skip prefix " + n);
        }
        final byte[] two = Arrays.copyOf(golden, golden.length + 1);
        final BinReader r = new BinReader(two);
        c.eq(r.readRawValue().length, golden.length, "raw value");
        c.that(r.readNull() && r.atEnd(), "trailing null");

        long s = 0x9E3779B97F4A7C15L;
        for (int iter = 0; iter < 20000; iter++) {
            byte[] buf;
            s = xorshift(s);
            if (iter % 2 == 0) {
                buf = golden.clone();
                buf[(int) Long.remainderUnsigned(s, buf.length)] = (byte) (s >>> 40);
            } else {
                buf = new byte[(int) Long.remainderUnsigned(s, 48)];
                for (int i = 0; i < buf.length; i++) {
                    s = xorshift(s);
                    buf[i] = (byte) Long.remainderUnsigned(s, 9);
                }
            }
            boolean valid;
            try {
                BinReader.validate(buf);
                valid = true;
            } catch (BinCodecException e) {
                valid = false;
            }
            try {
                final BinReader fr = new BinReader(buf);
                fr.readRawValue();
                if (valid) c.that(fr.atEnd(), "valid fuzz input skips fully");
            } catch (BinCodecException e) {
                if (valid) c.that(false, "valid fuzz input failed to skip");
            }
        }
    }

    static long xorshift(long s) {
        s ^= s << 13;
        s ^= s >>> 7;
        return s ^ (s << 17);
    }

    static void dynamicValue(Check c, byte[] golden) {
        c.that(HybridRuntime.DynamicValue.NULL.isNull(), "null dynamic value");
        final HybridRuntime.DynamicValue v = HybridRuntime.DynamicValue.of(golden);
        c.that(v.tag() == BinTag.MAP, "dynamic tag");
        c.that(v.equals(HybridRuntime.DynamicValue.of(golden.clone())), "dynamic equals");
        c.binError(BinCodecException.Kind.TRUNCATED, () -> HybridRuntime.DynamicValue.of(Arrays.copyOf(golden, 10)), "dynamic prefix");
        c.binError(BinCodecException.Kind.TRUNCATED, () -> HybridRuntime.DynamicValue.of(new byte[0]), "dynamic empty");
        jniBridge(c, golden);
        HybridRuntime.checkSchema(Key64.of(1L), Key64.of(1L));
        final HybridRuntime.HybridException e = c.throwsA(HybridRuntime.HybridException.class,
                () -> HybridRuntime.checkSchema(Key64.of(1L), Key64.of(2L)), "schema mismatch");
        c.that(e != null && e.status() == HybridRuntime.HybridStatus.SCHEMA_MISMATCH, "schema mismatch status");
        c.that(HybridRuntime.HostStatus.fromCode(-8) == HybridRuntime.HostStatus.INTERNAL_ERROR, "host status -8");
        c.that(HybridRuntime.HostStatus.fromCode(99) == null, "unknown host status");
        c.that(HybridRuntime.HybridStatus.fromCode(4) == HybridRuntime.HybridStatus.NULL_HANDLE, "hybrid status 4");
    }

    /** The helpers generated Kotlin / Java hybrid clients cross JNI with. */
    static void jniBridge(Check c, byte[] golden) {
        // `read` takes exactly one value from a window, leaves the rest, and validates it.
        final byte[] framed = new byte[golden.length + 3];
        framed[0] = 0x55;
        System.arraycopy(golden, 0, framed, 1, golden.length);
        framed[golden.length + 1] = (byte) BinTag.NULL.code();
        final BinReader r = new BinReader(framed, 1, golden.length + 1);
        final HybridRuntime.DynamicValue v = HybridRuntime.DynamicValue.read(r);
        c.that(Arrays.equals(v.bytes(), golden), "dynamic read bytes");
        c.binError(BinCodecException.Kind.TRAILING_BYTES, r::expectEnd, "expectEnd with a value left");
        c.that(r.readNull(), "value after dynamic read");
        r.expectEnd();
        c.binError(BinCodecException.Kind.INVALID_BOOL, () -> HybridRuntime.DynamicValue.read(new BinReader(bytes(1, 2))), "dynamic read bool 2");
        c.binError(BinCodecException.Kind.INVALID_UTF8, () -> HybridRuntime.DynamicValue.read(new BinReader(bytes(4, 1, 0, 0, 0, 0xFF))), "dynamic read bad utf8");
        final BinReader cut = new BinReader(Arrays.copyOf(golden, golden.length - 1));
        c.binError(BinCodecException.Kind.TRUNCATED, () -> HybridRuntime.DynamicValue.read(cut), "dynamic read truncated");

        final BinWriter w = new BinWriter();
        final long arr = w.beginArray(2);
        v.writeTo(w);
        HybridRuntime.DynamicValue.NULL.writeTo(w);
        w.end(arr);
        final BinReader back = new BinReader(w.toByteArray());
        c.eq(back.beginArray(), 2, "writeTo array count");
        c.that(HybridRuntime.DynamicValue.read(back).equals(v) && back.readNull(), "writeTo round trip");
        back.expectEnd();

        final java.nio.ByteBuffer direct = w.toDirectBuffer();
        c.that(direct.isDirect() && direct.order() == java.nio.ByteOrder.LITTLE_ENDIAN, "direct LE buffer");
        c.eq(direct.capacity(), w.size(), "direct capacity == size");
        c.eq(direct.position(), 0, "direct position");
        c.eq(direct.limit(), w.size(), "direct limit");
        final byte[] copied = new byte[direct.remaining()];
        direct.get(copied);
        c.that(Arrays.equals(copied, w.toByteArray()), "direct contents");
        c.eq(HybridRuntime.directBuffer(new byte[0]).capacity(), 0, "empty direct buffer");
        final java.nio.ByteBuffer part = HybridRuntime.directBuffer(golden, 1, 4);
        c.that(part.capacity() == 4 && part.getInt() == 0x34, "direct window, little-endian");
        c.eq(v.toDirectBuffer().capacity(), golden.length, "dynamic direct buffer");
    }

    static void utf8(Check c) {
        final byte[] ok = "plain \u00e9 \u2014 \ud83d\ude80 \udbff\udfff".getBytes(java.nio.charset.StandardCharsets.UTF_8);
        c.that(Utf8.isValid(ok, 0, ok.length), "valid utf8");
        final int[][] bad = {
            {0xC0, 0xAF}, {0xE0, 0x80, 0xAF}, {0xF0, 0x80, 0x80, 0xAF}, {0xED, 0xA0, 0x80},
            {0xF4, 0x90, 0x80, 0x80}, {0xE2, 0x80}, {0xE2, 0x28, 0xA1}, {0x80}, {0xF5, 0x80, 0x80, 0x80},
        };
        for (int[] b : bad) c.that(!Utf8.isValid(bytes(b), 0, b.length), "invalid utf8 " + Arrays.toString(b));
    }

    /** A {@code CharSequence} of {@code length} copies of {@code c} that stores nothing. */
    static final class Repeated implements CharSequence {
        private final char c;
        private final int length;

        Repeated(char c, int length) {
            this.c = c;
            this.length = length;
        }

        @Override
        public int length() {
            return length;
        }

        @Override
        public char charAt(int index) {
            return c;
        }

        @Override
        public CharSequence subSequence(int start, int end) {
            return new Repeated(c, end - start);
        }

        @Override
        public String toString() {
            return "Repeated(" + c + " x " + length + ")";
        }
    }
}
