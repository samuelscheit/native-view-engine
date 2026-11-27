package dev.nve.host;

import java.util.Arrays;

/** Little-endian writer for raw primitives and {@code NveBinValue}; reuse with {@link #clear()}. */
public final class BinWriter {
    /** Largest array the JVM reliably allocates. */
    private static final int MAX_SIZE = Integer.MAX_VALUE - 8;

    private byte[] buf;
    private int len;

    public BinWriter() {
        this(64);
    }

    public BinWriter(int capacity) {
        buf = new byte[Math.max(capacity, 16)];
    }

    public int size() {
        return len;
    }

    /** Copy of the written bytes. */
    public byte[] toByteArray() {
        return Arrays.copyOf(buf, len);
    }

    public void clear() {
        len = 0;
    }

    // ---- raw little-endian primitives ---------------------------------------------------------

    public void u8(int v) {
        ensure(1);
        buf[len++] = (byte) v;
    }

    public void u16(int v) {
        ensure(2);
        buf[len++] = (byte) v;
        buf[len++] = (byte) (v >>> 8);
    }

    public void u32(int v) {
        ensure(4);
        putLe32(len, v);
        len += 4;
    }

    public void u64(long v) {
        u32((int) v);
        u32((int) (v >>> 32));
    }

    public void f32(float v) {
        u32(Float.floatToRawIntBits(v));
    }

    public void f64(double v) {
        u64(Double.doubleToRawLongBits(v));
    }

    public void raw(byte[] b, int offset, int length) {
        ensure(length);
        System.arraycopy(b, offset, buf, len, length);
        len += length;
    }

    // ---- tagged NveBinValue ------------------------------------------------------------------

    public void writeNull() {
        u8(BinTag.NULL.code());
    }

    public void writeBool(boolean v) {
        u8(BinTag.BOOL.code());
        u8(v ? 1 : 0);
    }

    public void writeI64(long v) {
        u8(BinTag.I64.code());
        u64(v);
    }

    public void writeF64(double v) {
        u8(BinTag.F64.code());
        f64(v);
    }

    /** Throws {@link BinCodecException.Kind#INVALID_UTF8} on a lone surrogate, writing nothing. */
    public void writeStr(CharSequence v) {
        final int n = Utf8.encodedLength(v);
        u8(BinTag.STR.code());
        writeEncoded(v, n);
    }

    public void writeBytes(byte[] v) {
        u8(BinTag.BYTES.code());
        u32(v.length);
        raw(v, 0, v.length);
    }

    /** Length-prefixed UTF-8 encoded in place (no temporary array); throws on a lone surrogate. */
    public void writeMapKey(CharSequence key) {
        writeEncoded(key, Utf8.encodedLength(key));
    }

    private void writeEncoded(CharSequence s, int n) {
        ensure(4 + n);
        u32(n);
        len = Utf8.encode(s, buf, len);
    }

    /** Opens an array of {@code count} values; returns the mark for {@link #end(int)}. */
    public int beginArray(int count) {
        return begin(BinTag.ARRAY, count);
    }

    /** Opens a map of {@code count} entries ({@link #writeMapKey} + value each). */
    public int beginMap(int count) {
        return begin(BinTag.MAP, count);
    }

    /** Patches the payload length of the container opened at {@code mark} (stale marks are rejected). */
    public void end(int mark) {
        if (mark < 0 || mark > len - 8) throw new BinCodecException(BinCodecException.Kind.INVALID_MARK, "mark " + mark + " of " + len);
        putLe32(mark, len - mark - 4);
    }

    private int begin(BinTag tag, int count) {
        u8(tag.code());
        final int mark = len;
        u32(0);
        u32(count);
        return mark;
    }

    private void putLe32(int at, int v) {
        buf[at] = (byte) v;
        buf[at + 1] = (byte) (v >>> 8);
        buf[at + 2] = (byte) (v >>> 16);
        buf[at + 3] = (byte) (v >>> 24);
    }

    private void ensure(int extra) {
        if (extra > MAX_SIZE - len) throw new BinCodecException(BinCodecException.Kind.TOO_LARGE, "size " + len + " + " + extra);
        if (len + extra <= buf.length) return;
        final long grown = Math.max((long) buf.length * 2, (long) len + extra);
        buf = Arrays.copyOf(buf, (int) Math.min(grown, MAX_SIZE));
    }
}
