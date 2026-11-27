package dev.nve.host;

/**
 * 64-bit identity key (table keys, schema hashes, interface ids; Rust {@code nve_hash::Key64}).
 *
 * <p>The unsigned value lives in a {@code long}; it crosses 53-bit hosts as {@code (lo, hi)} halves.
 */
public final class Key64 {
    public static final Key64 ZERO = new Key64(0L);

    private final long value;

    private Key64(long value) {
        this.value = value;
    }

    public static Key64 of(long value) {
        return new Key64(value);
    }

    /** Key of a reverse-DNS or plain name (Rust {@code Key64::of}). */
    public static Key64 of(String name) {
        return new Key64(Xxh64.constId(name));
    }

    public static Key64 fromHalves(int lo, int hi) {
        return new Key64(((long) hi << 32) | (lo & 0xFFFFFFFFL));
    }

    /** Raw unsigned bits (compare with {@link Long#compareUnsigned}). */
    public long value() {
        return value;
    }

    /** Low {@code u32} half (as a signed Java int; use {@link Integer#toUnsignedLong}). */
    public int lo() {
        return (int) value;
    }

    /** High {@code u32} half (as a signed Java int). */
    public int hi() {
        return (int) (value >>> 32);
    }

    /** 16 lowercase hex digits. */
    public String toHex() {
        final String digits = Long.toHexString(value);
        return "0000000000000000".substring(digits.length()) + digits;
    }

    @Override
    public boolean equals(Object other) {
        return other instanceof Key64 && ((Key64) other).value == value;
    }

    @Override
    public int hashCode() {
        return Long.hashCode(value);
    }

    @Override
    public String toString() {
        return "Key64(0x" + toHex() + ")";
    }
}
