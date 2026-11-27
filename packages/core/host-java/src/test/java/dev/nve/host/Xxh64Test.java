package dev.nve.host;

import java.nio.charset.StandardCharsets;

/** Golden XXH64 vectors computed by the canonical Rust {@code nve-hash} crate. */
final class Xxh64Test {
    private Xxh64Test() {}

    static final long PRIME32 = 0x9E3779B1L;
    static final long WIRE_SEED = 0x4E56455749524532L;

    static byte[] utf8(String s) {
        return s.getBytes(StandardCharsets.UTF_8);
    }

    static byte[] seq(int n) {
        final byte[] out = new byte[n];
        for (int i = 0; i < n; i++) out[i] = (byte) (i * 31 + 7);
        return out;
    }

    /** Official Cyan4973 xxHash sanity buffer. */
    static byte[] cyan(int n) {
        final byte[] out = new byte[n];
        long gen = PRIME32;
        for (int i = 0; i < n; i++) {
            out[i] = (byte) (gen >>> 56);
            gen *= Xxh64.PRIME1;
        }
        return out;
    }

    static final Object[][] VECTORS = {
        {"empty", new byte[0], 0L, 0xEF46DB3751D8E999L},
        {"a", utf8("a"), 0L, 0xD24EC4F1A98C6E5BL},
        {"abc", utf8("abc"), 0L, 0x44BC2CF5AD770999L},
        {"nve.visual", utf8("nve.visual"), 0L, 0x33E73115985A4FD6L},
        {"high_bits", new byte[] {0x00, 0x01, 0x7F, (byte) 0x80, (byte) 0xFF}, 0L, 0x8BABCAB499A141EAL},
        {"utf8", utf8("h\u00e9llo w\u00f6rld \u2014 \u65e5\u672c\u8a9e \ud83d\ude80"), 0L, 0x72AC442FF213F592L},
        {"seq101", seq(101), 0L, 0xFBFDF3FA1A53BC7FL},
        {"seq101_wire", seq(101), WIRE_SEED, 0x277E092A8C3DB1BCL},
        {"seq37_prime1", seq(37), Xxh64.PRIME1, 0x33DDB9A885FC58A4L},
        {"seq1024", seq(1024), 0L, 0x149AA44972CDAE00L},
        {"cyan1", cyan(1), 0L, 0xE934A84ADB052768L},
        {"cyan4", cyan(4), 0L, 0xE244DDBCF81E29E4L},
        {"cyan14", cyan(14), 0L, 0xB89B3598E0BD0A0AL},
        {"cyan222", cyan(222), 0L, 0x06CC5BD930BCEC3AL},
        {"cyan1024", cyan(1024), 0L, 0x2E307F2FE063991AL},
        {"cyan1_p32", cyan(1), PRIME32, 0x5014607643A9B4C3L},
        {"cyan222_p32", cyan(222), PRIME32, 0x0B105469DF89AF66L},
        {"cyan32_p1", cyan(32), Xxh64.PRIME1, 0x6D0242B0B7E278EDL},
        {"cyan1024_p1", cyan(1024), Xxh64.PRIME1, 0xF464C3F69A39421CL},
    };

    static void run(Check c) {
        final int[] steps = {1, 3, 7, 13, 24, 31, 32, 33, 64, 100};
        for (Object[] v : VECTORS) {
            final String label = (String) v[0];
            final byte[] bytes = (byte[]) v[1];
            final long seed = (Long) v[2];
            final long expected = (Long) v[3];
            c.eq(Xxh64.hash(bytes, seed), expected, label);
            for (int step : steps) {
                final Xxh64.Hasher h = new Xxh64.Hasher(seed);
                for (int at = 0; at < bytes.length; at += step) h.update(bytes, at, Math.min(step, bytes.length - at));
                c.eq(h.digest(), expected, label + " step " + step);
            }
        }
        // Offset windows inside a larger array.
        final byte[] padded = new byte[110];
        System.arraycopy(seq(101), 0, padded, 5, 101);
        c.eq(Xxh64.hash(padded, 5, 101, WIRE_SEED), 0x277E092A8C3DB1BCL, "offset window");
        c.throwsA(IndexOutOfBoundsException.class, () -> Xxh64.hash(padded, 100, 11, 0), "window past end");
        c.throwsA(IndexOutOfBoundsException.class, () -> Xxh64.hash(padded, -1, 1, 0), "negative offset");

        final Xxh64.Hasher h = new Xxh64.Hasher(7).updateU32(0x04030201);
        c.eq(h.digest(), h.digest(), "digest is non-consuming");
        h.updateU64(0x0C0B0A0908070605L);
        c.eq(h.digest(), Xxh64.hash(new byte[] {1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12}, 7), "typed feeds");

        final Key64 k = Key64.of("nve.visual");
        c.eq(k.value(), 0x33E73115985A4FD6L, "Key64.of");
        c.eq(k.lo() & 0xFFFFFFFFL, 0x985A4FD6L, "Key64.lo");
        c.eq(k.hi() & 0xFFFFFFFFL, 0x33E73115L, "Key64.hi");
        c.that(Key64.fromHalves(k.lo(), k.hi()).equals(k), "Key64 halves round trip");
        c.eq(Key64.fromHalves(-1, -1).value(), -1L, "Key64 all ones");
        c.that(Key64.of(1L).toHex().equals("0000000000000001"), "Key64.toHex pads");
        c.that(Key64.of(0xF00DFACEC0FFEE11L).toHex().equals("f00dfacec0ffee11"), "Key64.toHex high bit");
        c.eq(Xxh64.constId("abc"), 0x44BC2CF5AD770999L, "constId");
        c.eq(Xxh64.constId("h\u00e9llo w\u00f6rld \u2014 \u65e5\u672c\u8a9e \ud83d\ude80"), 0x72AC442FF213F592L, "constId encodes pairs in place");
        for (String lone : new String[] {"\ud800", "x\udc00", "\ud83d", "\ude80\ud83d"}) {
            c.binError(BinCodecException.Kind.INVALID_UTF8, () -> Xxh64.constId(lone), "constId lone surrogate");
        }
        // A rejected string feeds nothing: the hasher state is unchanged and stays usable.
        final Xxh64.Hasher atomic = new Xxh64.Hasher(0).updateUtf8("ab");
        final long before = atomic.digest();
        c.binError(BinCodecException.Kind.INVALID_UTF8, () -> atomic.updateUtf8("abc\ud800"), "updateUtf8 lone surrogate");
        c.eq(atomic.digest(), before, "updateUtf8 is atomic");
        c.eq(atomic.updateUtf8("c").digest(), Xxh64.constId("abc"), "hasher usable after a rejection");
        final Xxh64.Hasher warm = new Xxh64.Hasher(0);
        warm.updateUtf8("warm \u00e9 \ud83d\ude80");
        final long allocated = Alloc.bytes(() -> {
            for (int i = 0; i < 1000; i++) warm.updateUtf8("steady \u00e9 \ud83d\ude80");
        });
        c.that(allocated < 1024, "updateUtf8 allocates nothing per call (" + allocated + " bytes)");
    }
}
