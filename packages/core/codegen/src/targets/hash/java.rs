//! Java (`dev.nve.android.generated.NveHash`) XXH64 mirror.

use super::{banner, byte_list, HASH_VECTORS, STREAM_SPLITS};

const BODY: &str = r#"package dev.nve.android.generated;

import java.nio.charset.StandardCharsets;

/** Canonical XXH64 (bit-identical to Rust {@code nve_hash::xxh64}) plus 64-bit key halves. */
public final class NveHash {
    public static final long PRIME1 = 0x9E3779B185EBCA87L;
    public static final long PRIME2 = 0xC2B2AE3D27D4EB4FL;
    public static final long PRIME3 = 0x165667B19E3779F9L;
    public static final long PRIME4 = 0x85EBCA77C2B2AE63L;
    public static final long PRIME5 = 0x27D4EB2F165667C5L;

    private NveHash() {}

    /** Streaming XXH64; digests equal {@link #xxh64} for any chunking. */
    public static final class Hasher {
        private final long seed;
        private long totalLen;
        private long v1, v2, v3, v4;
        private final byte[] buf = new byte[32];
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

        public Hasher update(byte[] bytes, int offset, int length) {
            totalLen += length;
            int i = offset;
            final int end = offset + length;
            if (bufLen > 0) {
                while (bufLen < 32 && i < end) buf[bufLen++] = bytes[i++];
                if (bufLen < 32) return this;
                stripe(buf, 0);
                bufLen = 0;
            }
            for (; i + 32 <= end; i += 32) stripe(bytes, i);
            while (i < end) buf[bufLen++] = bytes[i++];
            return this;
        }

        public long digest() {
            long h;
            if (Long.compareUnsigned(totalLen, 32) >= 0) {
                h = Long.rotateLeft(v1, 1) + Long.rotateLeft(v2, 7) + Long.rotateLeft(v3, 12)
                        + Long.rotateLeft(v4, 18);
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

    private static long round(long acc, long lane) {
        return Long.rotateLeft(acc + lane * PRIME2, 31) * PRIME1;
    }

    private static long mergeRound(long acc, long val) {
        return (acc ^ round(0, val)) * PRIME1 + PRIME4;
    }

    private static long read64(byte[] b, int i) {
        return (read32(b, i) & 0xFFFFFFFFL) | ((long) read32(b, i + 4) << 32);
    }

    private static int read32(byte[] b, int i) {
        return (b[i] & 0xFF) | (b[i + 1] & 0xFF) << 8 | (b[i + 2] & 0xFF) << 16 | (b[i + 3] & 0xFF) << 24;
    }

    public static long xxh64(byte[] bytes, int offset, int length, long seed) {
        return new Hasher(seed).update(bytes, offset, length).digest();
    }

    public static long xxh64(byte[] bytes, long seed) {
        return xxh64(bytes, 0, bytes.length, seed);
    }

    /** Identity of a UTF-8 name: {@code xxh64(name, 0)} (Rust {@code nve_hash::const_id}). */
    public static long constId(String name) {
        return xxh64(name.getBytes(StandardCharsets.UTF_8), 0L);
    }

    /** Low {@code u32} half of a 64-bit key (as a signed Java int). */
    public static int lo(long key) {
        return (int) key;
    }

    /** High {@code u32} half of a 64-bit key (as a signed Java int). */
    public static int hi(long key) {
        return (int) (key >>> 32);
    }

    public static long fromHalves(int lo, int hi) {
        return ((long) hi << 32) | (lo & 0xFFFFFFFFL);
    }
"#;

/// Generates `hosts/android-shell/.../generated/NveHash.java`.
pub(super) fn generate_java_hash() -> String {
    let mut s = banner();
    s.push_str(BODY);
    s.push_str("\n    private static final byte[][] VECTOR_BYTES = {\n");
    for v in HASH_VECTORS {
        let bytes = byte_list(v.bytes, |b| format!("(byte) 0x{b:02X}"));
        writeln!(s, "        {{{bytes}}}, // {}", v.label);
    }
    s.push_str("    };\n    private static final long[][] VECTOR_SEED_EXPECTED = {\n");
    for v in HASH_VECTORS {
        writeln!(s, "        {{0x{:016X}L, 0x{:016X}L}},", v.seed, v.expected());
    }
    let [a, b, c] = STREAM_SPLITS;
    write!(
        s,
        r#"    }};

    /** {{@code true}} when one-shot and chunked digests of every golden vector match Rust. */
    public static boolean verifyTestVectors() {{
        for (int v = 0; v < VECTOR_BYTES.length; v++) {{
            final byte[] bytes = VECTOR_BYTES[v];
            final long seed = VECTOR_SEED_EXPECTED[v][0];
            final long expected = VECTOR_SEED_EXPECTED[v][1];
            if (xxh64(bytes, seed) != expected) return false;
            final Hasher h = new Hasher(seed);
            int at = 0;
            for (int split : new int[] {{{a}, {b}, {c}}}) {{
                final int end = Math.min(split, bytes.length);
                if (end > at) {{
                    h.update(bytes, at, end - at);
                    at = end;
                }}
            }}
            h.update(bytes, at, bytes.length - at);
            if (h.digest() != expected) return false;
        }}
        return true;
    }}
}}
"#
    );
    s.into_string()
}
