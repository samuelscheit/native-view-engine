package dev.nve.host;

import java.nio.charset.StandardCharsets;
import java.util.Arrays;

/**
 * Bounds-checked little-endian cursor over {@code byte[]} plus the compact tagged {@code NveBinValue}
 * format (byte-identical to Rust {@code nve_abi::encode_bin_value}).
 *
 * <p>Wire layout: Null [0] | Bool [1, 0|1] | I64 [2, i64] | F64 [3, f64] | Str [4, len u32, utf8] |
 * Bytes [5, len u32, raw] | Array [6, payload u32, count u32, ...] | Map [7, payload u32, count u32,
 * (klen u32, key, value)...]; {@code payload} counts every byte after the payload field.
 */
public final class BinReader {
    private final byte[] data;
    /** Reads stop here: the window end, or the enclosing container's end while validating. */
    private int limit;
    private int pos;
    /** End offset of the container most recently opened by {@link #openContainer}. */
    private int containerEnd;

    public BinReader(byte[] data) {
        this(data, 0, data.length);
    }

    /** Reads {@code data[offset .. offset + length)}. */
    public BinReader(byte[] data, int offset, int length) {
        if (offset < 0 || length < 0 || length > data.length - offset) {
            throw new IndexOutOfBoundsException("range [" + offset + ", +" + length + ") of " + data.length);
        }
        this.data = data;
        this.pos = offset;
        this.limit = offset + length;
    }

    public int position() {
        return pos;
    }

    public int remaining() {
        return limit - pos;
    }

    public boolean atEnd() {
        return pos == limit;
    }

    // ---- raw little-endian primitives ---------------------------------------------------------

    public int u8() {
        return data[take(1)] & 0xFF;
    }

    public int u16() {
        final int at = take(2);
        return (data[at] & 0xFF) | (data[at + 1] & 0xFF) << 8;
    }

    /** Raw {@code u32} bits (use {@link Integer#toUnsignedLong} for the unsigned value). */
    public int u32() {
        return Xxh64.read32(data, take(4));
    }

    /** Raw {@code u64} bits (unsigned in a {@code long}). */
    public long u64() {
        return Xxh64.read64(data, take(8));
    }

    public float f32() {
        return Float.intBitsToFloat(u32());
    }

    public double f64() {
        return Double.longBitsToDouble(u64());
    }

    /** Copy of the next {@code n} bytes. */
    public byte[] bytes(int n) {
        final int at = take(n);
        return Arrays.copyOfRange(data, at, at + n);
    }

    /** Next {@code n} bytes decoded as strictly validated UTF-8. */
    public String utf8(int n) {
        final int at = take(n);
        if (!Utf8.isValid(data, at, n)) throw new BinCodecException(BinCodecException.Kind.INVALID_UTF8, "at " + at);
        return new String(data, at, n, StandardCharsets.UTF_8);
    }

    // ---- tagged NveBinValue ------------------------------------------------------------------

    public BinTag peekTag() {
        if (pos >= limit) throw truncated(1);
        return BinTag.fromCode(data[pos] & 0xFF);
    }

    /** Consumes a {@code null} and returns {@code true}; leaves any other value in place. */
    public boolean readNull() {
        if (peekTag() != BinTag.NULL) return false;
        pos++;
        return true;
    }

    public boolean readBool() {
        expect(BinTag.BOOL);
        final int b = u8();
        if (b > 1) throw new BinCodecException(BinCodecException.Kind.INVALID_BOOL, "byte " + b);
        return b == 1;
    }

    public long readI64() {
        expect(BinTag.I64);
        return u64();
    }

    /** {@code F64}, or an {@code I64} widened to {@code double} (Rust {@code BinValueView::as_number}). */
    public double readF64() {
        if (peekTag() == BinTag.I64) return (double) readI64();
        expect(BinTag.F64);
        return f64();
    }

    public String readStr() {
        expect(BinTag.STR);
        return utf8(length());
    }

    public byte[] readBytes() {
        expect(BinTag.BYTES);
        return bytes(length());
    }

    /** A container being read: its entry count, and the bounds {@link #end(ReadMark)} checks and restores. */
    public static final class ReadMark {
        public final int count;
        final int end;
        final int outerLimit;

        ReadMark(int count, int end, int outerLimit) {
            this.count = count;
            this.end = end;
            this.outerLimit = outerLimit;
        }
    }

    /**
     * Opens an array (counts its payload cannot hold are rejected). Until {@link #end(ReadMark)}, no
     * read can pass its payload, so a crafted entry never reads its parent's bytes.
     */
    public ReadMark beginArray() {
        return open(BinTag.ARRAY, 1);
    }

    /** Opens a map like {@link #beginArray()}; read each entry as {@link #readMapKey()} then a value. */
    public ReadMark beginMap() {
        return open(BinTag.MAP, 5);
    }

    /** Closes {@code mark}'s container: its payload must have been read exactly ({@code TRAILING_BYTES}). */
    public void end(ReadMark mark) {
        limit = mark.outerLimit;
        if (pos != mark.end) throw new BinCodecException(BinCodecException.Kind.TRAILING_BYTES, (mark.end - pos) + " bytes");
    }

    private ReadMark open(BinTag tag, int minEntryBytes) {
        final int outer = limit;
        final int count = openContainer(tag, minEntryBytes);
        limit = containerEnd;
        return new ReadMark(count, containerEnd, outer);
    }

    public String readMapKey() {
        return utf8(length());
    }

    /** Skips one value in O(1). */
    public void skipValue() {
        final BinTag tag = peekTag();
        pos++;
        switch (tag) {
            case NULL:
                return;
            case BOOL:
                take(1);
                return;
            case I64:
            case F64:
                take(8);
                return;
            default:
                take(length());
        }
    }

    /** Copy of the next complete encoded value. */
    public byte[] readRawValue() {
        final int start = pos;
        skipValue();
        return Arrays.copyOfRange(data, start, pos);
    }

    /** Fully validates and consumes the next value (Rust {@code BinValueView::parse} rules). */
    public void checkValue() {
        checkValue(0);
    }

    /** Copy of the next value, fully validated like {@link #checkValue()}. */
    byte[] readCheckedValue() {
        final int start = pos;
        checkValue();
        return Arrays.copyOfRange(data, start, pos);
    }

    /** Throws {@link BinCodecException.Kind#TRAILING_BYTES} unless every byte was consumed. */
    public void expectEnd() {
        if (!atEnd()) throw new BinCodecException(BinCodecException.Kind.TRAILING_BYTES, remaining() + " bytes");
    }

    /** Throws unless {@code bytes} holds exactly one valid {@code NveBinValue}. */
    public static void validate(byte[] bytes) {
        final BinReader r = new BinReader(bytes);
        r.checkValue();
        r.expectEnd();
    }

    private int take(int n) {
        if (n < 0 || n > limit - pos) throw truncated(n);
        final int at = pos;
        pos += n;
        return at;
    }

    /** A {@code u32} length; anything above {@code Integer.MAX_VALUE} cannot fit and is truncated input. */
    private int length() {
        final long len = Integer.toUnsignedLong(u32());
        if (len > limit - pos) throw truncated(len);
        return (int) len;
    }

    private BinCodecException truncated(long need) {
        return new BinCodecException(BinCodecException.Kind.TRUNCATED, "need " + need + " at " + pos + ", have " + (limit - pos));
    }

    private void expect(BinTag tag) {
        final BinTag actual = peekTag();
        if (actual != tag) {
            throw new BinCodecException(BinCodecException.Kind.UNEXPECTED_TAG, "expected " + tag + ", got " + actual);
        }
        pos++;
    }

    private int openContainer(BinTag tag, int minEntryBytes) {
        expect(tag);
        final int payload = length();
        if (payload < 4) throw truncated(4);
        containerEnd = pos + payload;
        final long count = Integer.toUnsignedLong(u32());
        if (count > (payload - 4) / minEntryBytes) throw truncated(count);
        return (int) count;
    }

    /** Validates a length-prefixed UTF-8 run in place (no String, no copy). */
    private void checkUtf8Run() {
        final int n = length();
        final int at = take(n);
        if (!Utf8.isValid(data, at, n)) throw new BinCodecException(BinCodecException.Kind.INVALID_UTF8, "at " + at);
    }

    private void checkValue(int depth) {
        final BinTag tag = peekTag();
        switch (tag) {
            case NULL:
            case I64:
            case F64:
            case BYTES:
                skipValue();
                return;
            case BOOL:
                readBool();
                return;
            case STR:
                expect(BinTag.STR);
                checkUtf8Run();
                return;
            default:
                if (depth >= BinTag.MAX_DEPTH) throw new BinCodecException(BinCodecException.Kind.MAX_DEPTH, "depth " + depth);
                final boolean isMap = tag == BinTag.MAP;
                final int count = openContainer(tag, isMap ? 5 : 1);
                final int stop = containerEnd;
                // Like Rust's `&bytes[..container_end]`: no child read may escape its container.
                final int outer = limit;
                limit = stop;
                try {
                    for (int i = 0; i < count; i++) {
                        if (isMap) checkUtf8Run();
                        checkValue(depth + 1);
                    }
                } finally {
                    limit = outer;
                }
                if (pos != stop) throw new BinCodecException(BinCodecException.Kind.TRAILING_BYTES, (stop - pos) + " bytes");
        }
    }
}
