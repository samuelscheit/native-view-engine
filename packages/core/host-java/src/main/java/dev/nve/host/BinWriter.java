package dev.nve.host;

import java.util.Arrays;

/** Little-endian writer for raw primitives and {@code NveBinValue}; reuse with {@link #clear()}. */
public final class BinWriter {
    /** Largest array the JVM reliably allocates. */
    private static final int MAX_SIZE = Integer.MAX_VALUE - 8;

    private byte[] buf;
    private int len;
    /** Bumped by {@link #clear()}; marks carry it so stale ones are rejected after a refill. */
    private int generation = 1;

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

    /** Empties the buffer (keeping capacity) and invalidates every outstanding container mark. */
    public void clear() {
        len = 0;
        if (++generation == 0) generation = 1; // never 0: a raw mark (generation bits 0) must not match
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
        ensure(5, n); // tag + length + bytes fit, or nothing is written
        u8(BinTag.STR.code());
        writeEncoded(v, n);
    }

    /** Throws {@link BinCodecException.Kind#TOO_LARGE} (writing nothing) when the buffer cannot grow enough. */
    public void writeBytes(byte[] v) {
        ensure(5, v.length);
        u8(BinTag.BYTES.code());
        u32(v.length);
        raw(v, 0, v.length);
    }

    /** Length-prefixed UTF-8 encoded in place (no temporary array); throws on a lone surrogate. */
    public void writeMapKey(CharSequence key) {
        writeEncoded(key, Utf8.encodedLength(key));
    }

    private void writeEncoded(CharSequence s, int n) {
        ensure(4, n);
        u32(n);
        len = Utf8.encode(s, buf, len);
    }

    /** Opens an array of {@code count} values; returns the opaque mark for {@link #end(long)}. */
    public long beginArray(int count) {
        return begin(BinTag.ARRAY, count);
    }

    /** Opens a map of {@code count} entries ({@link #writeMapKey} + value each). */
    public long beginMap(int count) {
        return begin(BinTag.MAP, count);
    }

    /**
     * Patches the payload length of the container opened at {@code mark}. A mark from before
     * {@link #clear()} (even once the buffer refilled), out of range, or not right after an array /
     * map tag throws {@link BinCodecException.Kind#INVALID_MARK} and writes nothing.
     */
    public void end(long mark) {
        final int pos = (int) mark;
        // An open container's length field still holds 0; a closed one holds its payload (>= 4).
        final boolean valid = (int) (mark >>> 32) == generation && pos >= 1 && pos <= len - 8
                && (buf[pos - 1] == BinTag.ARRAY.code() || buf[pos - 1] == BinTag.MAP.code())
                && Xxh64.read32(buf, pos) == 0;
        if (!valid) throw new BinCodecException(BinCodecException.Kind.INVALID_MARK, "mark 0x" + Long.toHexString(mark) + " of " + len);
        putLe32(pos, len - pos - 4);
    }

    private long begin(BinTag tag, int count) {
        u8(tag.code());
        final long mark = ((long) generation << 32) | len;
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

    /** Room for {@code header + payload} more bytes, computed without {@code int} overflow. */
    private void ensure(int header, int payload) {
        if (payload < 0 || payload > MAX_SIZE - header - len) {
            throw new BinCodecException(BinCodecException.Kind.TOO_LARGE, "size " + len + " + " + header + " + " + payload);
        }
        ensure(header + payload);
    }

    private void ensure(int extra) {
        if (extra < 0 || extra > MAX_SIZE - len) {
            throw new BinCodecException(BinCodecException.Kind.TOO_LARGE, "size " + len + " + " + extra);
        }
        if (len + extra <= buf.length) return;
        final long grown = Math.max((long) buf.length * 2, (long) len + extra);
        buf = Arrays.copyOf(buf, (int) Math.min(grown, MAX_SIZE));
    }
}
