package dev.nve.host;

import dev.nve.host.generated.HashVectors;

/** XXH64 against the golden vectors of the canonical Rust {@code nve-hash} crate. */
final class Xxh64Test {
    private Xxh64Test() {}

    static final long WIRE_SEED = 0x4E56455749524532L;

    static byte[] seq(int n) {
        final byte[] out = new byte[n];
        for (int i = 0; i < n; i++) out[i] = (byte) (i * 31 + 7);
        return out;
    }

    /** Golden vectors generated from Rust {@code nve_hash::xxh64} ({@code HashVectors.java}). */
    static final Object[][] VECTORS = fromGenerated();

    private static Object[][] fromGenerated() {
        final HashVectors.Vector[] all = HashVectors.all();
        final Object[][] out = new Object[all.length][];
        for (int i = 0; i < all.length; i++) {
            out[i] = new Object[] {all[i].label, fromHex(all[i].hex), all[i].seed, all[i].expected};
        }
        return out;
    }

    static byte[] fromHex(String hex) {
        final byte[] out = new byte[hex.length() / 2];
        for (int i = 0; i < out.length; i++) out[i] = (byte) Integer.parseInt(hex.substring(2 * i, 2 * i + 2), 16);
        return out;
    }

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
        // OpenJ9 reports thread allocation at TLH-refill granularity (tens of KiB): warm up, then take
        // resolution from iterations (< 2.6 B per call), as in BinCodecTest.
        final Xxh64.Hasher warm = new Xxh64.Hasher(0);
        for (int i = 0; i < 1000; i++) warm.updateUtf8("warm \u00e9 \ud83d\ude80");
        final long allocated = Alloc.bytes(() -> {
            for (int i = 0; i < 100_000; i++) warm.updateUtf8("steady \u00e9 \ud83d\ude80");
        });
        c.that(allocated < 256 * 1024, "updateUtf8 allocates nothing per call (" + allocated + " bytes / 100k calls)");
    }
}
