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
    }

    static void golden(Check c, byte[] golden) {
        final BinWriter w = new BinWriter(4);  // forces growth
        final int map = w.beginMap(1);
        w.writeMapKey("k");
        final int arr = w.beginArray(6);
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
        final int[] marks = new int[depth];
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
        HybridRuntime.checkSchema(Key64.of(1L), Key64.of(1L));
        final HybridRuntime.HybridException e = c.throwsA(HybridRuntime.HybridException.class,
                () -> HybridRuntime.checkSchema(Key64.of(1L), Key64.of(2L)), "schema mismatch");
        c.that(e != null && e.status() == HybridRuntime.HybridStatus.SCHEMA_MISMATCH, "schema mismatch status");
        c.that(HybridRuntime.HostStatus.fromCode(-8) == HybridRuntime.HostStatus.INTERNAL_ERROR, "host status -8");
        c.that(HybridRuntime.HostStatus.fromCode(99) == null, "unknown host status");
        c.that(HybridRuntime.HybridStatus.fromCode(4) == HybridRuntime.HybridStatus.NULL_HANDLE, "hybrid status 4");
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
}
