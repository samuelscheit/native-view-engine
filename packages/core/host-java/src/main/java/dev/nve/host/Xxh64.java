package dev.nve.host;

/** Canonical XXH64, bit-identical to the Rust {@code nve-hash} crate (unsigned 64-bit in a {@code long}). */
public final class Xxh64 {
    public static final long PRIME1 = 0x9E3779B185EBCA87L;
    public static final long PRIME2 = 0xC2B2AE3D27D4EB4FL;
    public static final long PRIME3 = 0x165667B19E3779F9L;
    public static final long PRIME4 = 0x85EBCA77C2B2AE63L;
    public static final long PRIME5 = 0x27D4EB2F165667C5L;

    private Xxh64() {}

    /** Streaming XXH64; digests equal {@link #hash} for any chunking. Allocation-free after construction. */
    public static final class Hasher {
        private final long seed;
        private long totalLen;
        private long v1;
        private long v2;
        private long v3;
        private long v4;
        private final byte[] buf = new byte[32];
        private final byte[] scratch = new byte[8];
        private int bufLen;

        public Hasher(long seed) {
            this.seed = seed;
            v1 = seed + PRIME1 + PRIME2;
            v2 = seed + PRIME2;
            v3 = seed;
            v4 = seed - PRIME1;
        }

        public Hasher update(byte[] bytes) {
            return update(bytes, 0, bytes.length);
        }

        /** Feeds {@code bytes[offset .. offset + length)}. */
        public Hasher update(byte[] bytes, int offset, int length) {
            if (offset < 0 || length < 0 || length > bytes.length - offset) {
                throw new IndexOutOfBoundsException("range [" + offset + ", +" + length + ") of " + bytes.length);
            }
            totalLen += length;
            int i = offset;
            final int end = offset + length;
            if (bufLen > 0) {
                while (bufLen < 32 && i < end) buf[bufLen++] = bytes[i++];
                if (bufLen < 32) return this;
                stripe(buf, 0);
                bufLen = 0;
            }
            for (; end - i >= 32; i += 32) stripe(bytes, i); // `i + 32` could overflow near Integer.MAX_VALUE
            while (i < end) buf[bufLen++] = bytes[i++];
            return this;
        }

        /**
         * Feeds the UTF-8 encoding of {@code text} without allocating; throws
         * {@link BinCodecException.Kind#INVALID_UTF8} on a lone surrogate (never hashes {@code ?}).
         */
        public Hasher updateUtf8(CharSequence text) {
            Utf8.encodedLength(text); // validate everything first: a rejected string feeds nothing
            for (int i = 0; i < text.length(); i += Character.isHighSurrogate(text.charAt(i)) ? 2 : 1) {
                update(scratch, 0, Utf8.encodeAt(text, i, scratch, 0));
            }
            return this;
        }

        /** Feeds {@code v} as 4 little-endian bytes. */
        public Hasher updateU32(int v) {
            for (int k = 0; k < 4; k++) scratch[k] = (byte) (v >>> (8 * k));
            return update(scratch, 0, 4);
        }

        /** Feeds {@code v} as 8 little-endian bytes. */
        public Hasher updateU64(long v) {
            for (int k = 0; k < 8; k++) scratch[k] = (byte) (v >>> (8 * k));
            return update(scratch, 0, 8);
        }

        /** Digest of everything fed so far (the hasher stays usable). */
        public long digest() {
            long h;
            if (Long.compareUnsigned(totalLen, 32) >= 0) {
                h = Long.rotateLeft(v1, 1) + Long.rotateLeft(v2, 7) + Long.rotateLeft(v3, 12) + Long.rotateLeft(v4, 18);
                h = mergeRound(mergeRound(mergeRound(mergeRound(h, v1), v2), v3), v4);
            } else {
                h = seed + PRIME5;
            }
            h += totalLen;
            int i = 0;
            for (; i + 8 <= bufLen; i += 8) {
                h ^= round(0, read64(buf, i));
                h = Long.rotateLeft(h, 27) * PRIME1 + PRIME4;
            }
            if (i + 4 <= bufLen) {
                h ^= (read32(buf, i) & 0xFFFFFFFFL) * PRIME1;
                h = Long.rotateLeft(h, 23) * PRIME2 + PRIME3;
                i += 4;
            }
            for (; i < bufLen; i++) {
                h ^= (buf[i] & 0xFFL) * PRIME5;
                h = Long.rotateLeft(h, 11) * PRIME1;
            }
            h ^= h >>> 33;
            h *= PRIME2;
            h ^= h >>> 29;
            h *= PRIME3;
            return h ^ (h >>> 32);
        }

        private void stripe(byte[] b, int i) {
            v1 = round(v1, read64(b, i));
            v2 = round(v2, read64(b, i + 8));
            v3 = round(v3, read64(b, i + 16));
            v4 = round(v4, read64(b, i + 24));
        }
    }

    public static long hash(byte[] bytes, int offset, int length, long seed) {
        return new Hasher(seed).update(bytes, offset, length).digest();
    }

    public static long hash(byte[] bytes, long seed) {
        return hash(bytes, 0, bytes.length, seed);
    }

    /**
     * Identity of a UTF-8 name: {@code xxh64(name, 0)} (Rust {@code nve_hash::const_id}); throws
     * {@link BinCodecException.Kind#INVALID_UTF8} for a name with a lone surrogate.
     */
    public static long constId(CharSequence name) {
        return new Hasher(0L).updateUtf8(name).digest();
    }

    private static long round(long acc, long lane) {
        return Long.rotateLeft(acc + lane * PRIME2, 31) * PRIME1;
    }

    private static long mergeRound(long acc, long val) {
        return (acc ^ round(0, val)) * PRIME1 + PRIME4;
    }

    static long read64(byte[] b, int i) {
        return (read32(b, i) & 0xFFFFFFFFL) | ((long) read32(b, i + 4) << 32);
    }

    static int read32(byte[] b, int i) {
        return (b[i] & 0xFF) | (b[i + 1] & 0xFF) << 8 | (b[i + 2] & 0xFF) << 16 | (b[i + 3] & 0xFF) << 24;
    }
}
