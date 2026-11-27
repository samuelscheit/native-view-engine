package dev.nve.host;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;

final class TableSceneTest {
    private TableSceneTest() {}

    static final Key64 VISUAL = Key64.of("nve.visual");
    static final FieldSpec OPACITY = new FieldSpec(0, 0, 8, FieldKind.U32);
    static final FieldSpec Z = new FieldSpec(0, 8, 12, FieldKind.I32);
    static final FieldSpec VISIBLE = new FieldSpec(0, 20, 1, FieldKind.BOOL);
    static final FieldSpec WIDTH = FieldSpec.word(1, FieldKind.LENGTH);
    static final FieldSpec COLOR = FieldSpec.word(2, FieldKind.COLOR);
    static final FieldSpec LABEL = FieldSpec.word(3, FieldKind.STR);
    static final FieldSpec SCALE = FieldSpec.word(5, FieldKind.F32);

    static ByteBuffer words(int... w) {
        final ByteBuffer b = ByteBuffer.allocateDirect(w.length * 4).order(ByteOrder.LITTLE_ENDIAN);
        for (int v : w) b.putInt(v);
        b.flip();
        return b;
    }

    /** One row-index page: node index {@code owners[r]} maps to {@code r + 1}. */
    static ByteBuffer sparseIndexPage(int... nodeIndexes) {
        final int[] page = new int[dev.nve.host.generated.NveAbi.ROW_INDEX_PAGE_LEN];
        for (int r = 0; r < nodeIndexes.length; r++) page[nodeIndexes[r]] = r + 1;
        return words(page);
    }

    static ByteBuffer rows() {
        return words(
                200 | (0xFFD << 8) | (1 << 20), 0x42480001, 0x80402010, 0, 3, Float.floatToRawIntBits(1.5f),
                2047 << 8, 2, 0, 3, 0, 0x80000000);
    }

    static void run(Check c) {
        hardening(c);
        fieldSpecs(c);
        denseTable(c);
        sparseAndInvalid(c);
    }

    static void hardening(Check c) {
        final int[][] bad = {{0, 24, 16}, {0, 0, 0}, {0, 0, 33}, {0, 31, 2}, {32, 0, 32}, {-1, 0, 8}, {0, -1, 8},
            // `shift + bits` / `word + span` would overflow int and slip past an additive check.
            {0, Integer.MAX_VALUE - 10, 20}, {Integer.MAX_VALUE, 0, 32}, {Integer.MAX_VALUE - 1, 0, 32}};
        for (int[] b : bad) {
            c.throwsA(IllegalArgumentException.class, () -> new FieldSpec(b[0], b[1], b[2], FieldKind.U32), "bad spec " + b[0] + "/" + b[1] + "/" + b[2]);
        }
        c.throwsA(IllegalArgumentException.class, () -> FieldSpec.word(31, FieldKind.STR), "string past word 31");
        c.throwsA(IllegalArgumentException.class, () -> new FieldSpec(0, 0, 8, null), "null kind");
        final TableReader t = new TableReader().attach(VISUAL, 0, 6, 2, rows(), null, null, 0, null, 0, null, 0);
        c.throwsA(IndexOutOfBoundsException.class, () -> t.u32(0, FieldSpec.word(6, FieldKind.U32)), "field past row");
        c.throwsA(IndexOutOfBoundsException.class, () -> t.i32(0, FieldSpec.word(31, FieldKind.I32)), "i32 field past row");
        c.throwsA(IndexOutOfBoundsException.class, () -> t.strOffset(0, FieldSpec.word(3, FieldKind.U32)), "strOffset of a non-string spec");
        c.throwsA(IndexOutOfBoundsException.class, () -> t.strLen(0, FieldSpec.word(5, FieldKind.STR)), "strLen past the row");
        c.throwsA(IndexOutOfBoundsException.class, () -> t.strOffset(0, FieldSpec.word(30, FieldKind.STR)), "strOffset far past the row");
        c.eq(t.strLen(0, LABEL), 3, "valid string spec still reads");
    }

    static void fieldSpecs(Check c) {
        c.eq(new FieldSpec(0, 8, 8, FieldKind.U32).extract(0xABCD1234), 0x12, "extract");
        c.eq(FieldSpec.word(0, FieldKind.U32).extract(0xABCD1234), 0xABCD1234, "extract whole");
        c.eq(new FieldSpec(0, 0, 3, FieldKind.I32).extractSigned(7), -1, "sign 3 bits");
        c.eq(new FieldSpec(0, 0, 3, FieldKind.I32).extractSigned(4), -4, "sign min");
        c.eq(FieldSpec.word(0, FieldKind.I32).extractSigned(0x80000000), Integer.MIN_VALUE, "sign 32");
        c.eq(new FieldSpec(0, 31, 1, FieldKind.I32).extractSigned(0x80000000), -1, "sign top bit");
        c.that(LABEL.fits(5) && !LABEL.fits(4), "str fits");
        c.eq(Length.unit(0x3F800000), Length.POINTS, "points");
        c.that(Length.value(0x3F800000) == 1f, "points value");
        c.eq(Length.unit(0x42480001), Length.PERCENT, "percent");
        c.that(Length.value(0x42480001) == 50f, "percent value");
        c.eq(Length.unit(2), Length.AUTO, "auto");
        c.eq(Length.unit(0x3F800003), Length.AUTO, "tag 11");
        c.that(Length.value(0x3F800003) == 0f, "tag 11 value");
        c.eq(Rgba.r(0x80402010), 0x10, "r");
        c.eq(Rgba.g(0x80402010), 0x20, "g");
        c.eq(Rgba.b(0x80402010), 0x40, "b");
        c.eq(Rgba.a(0x80402010), 0x80, "a");
        c.eq(Rgba.toArgb(0x80402010), 0x80102040, "RGBA8888 -> ARGB8888");
        c.eq(Rgba.toArgb(0xFF0000FF), 0xFFFF0000, "opaque red");
    }

    static void denseTable(Check c) {
        final TableReader t = new TableReader().attach(VISUAL, 4, 6, 2, rows(), null, words(1), 1, null, 0, null, 0);
        c.that(!t.isSparse() && t.key().equals(VISUAL) && t.slot() == 4, "dense header");
        c.eq(t.u32(0, OPACITY), 200, "opacity");
        c.eq(t.i32(0, Z), -3, "z -3");
        c.eq(t.i32(1, Z), 2047, "z max");
        c.that(t.bool(0, VISIBLE) && !t.bool(1, VISIBLE), "visible");
        c.eq(Length.unit(t.length(0, WIDTH)), Length.PERCENT, "width unit");
        c.that(Length.value(t.length(0, WIDTH)) == 50f, "width value");
        c.eq(Length.unit(t.length(1, WIDTH)), Length.AUTO, "width auto");
        c.eq(Rgba.a(t.color(0, COLOR)), 0x80, "color alpha");
        c.eq(t.strOffset(0, LABEL), 0, "label offset");
        c.eq(t.strLen(0, LABEL), 3, "label len");
        c.that(t.f32(0, SCALE) == 1.5f, "scale");
        c.eq(Float.floatToRawIntBits(t.f32(1, SCALE)), 0x80000000, "-0.0 scale");
        c.eq(t.dirtyRow(0), 1, "dirty row");
        c.eq(t.rowOf(1), 1, "dense rowOf");
        c.eq(t.rowOf(2), -1, "dense rowOf miss");
        c.eq(t.owner(1), 1, "dense owner");
        c.throwsA(IndexOutOfBoundsException.class, () -> t.u32(2, OPACITY), "row oob");
        c.throwsA(IndexOutOfBoundsException.class, () -> t.word(0, 6), "word oob");
        c.throwsA(IndexOutOfBoundsException.class, () -> t.strLen(0, FieldSpec.word(5, FieldKind.STR)), "str second word oob");
        c.throwsA(IndexOutOfBoundsException.class, () -> t.dirtyRow(1), "dirty oob");
        // Non-zero buffer position: reads are relative to it.
        final ByteBuffer shifted = words(0xDEAD, 7, 8);
        shifted.position(4);
        c.eq(new TableReader().attach(VISUAL, 0, 2, 1, shifted, null, null, 0, null, 0, null, 0).word(0, 1), 8, "position-relative");
    }

    static void sparseAndInvalid(Check c) {
        // Rows owned by index 7 (generation 1) and index 3 (generation 2), with the published index.
        final TableReader t = new TableReader().attach(Key64.of(1L), 0, 2, 2, words(1, 2, 3, 4),
                words(0x01000007, 0x02000003), null, 0, words(0), 1, sparseIndexPage(7, 3), 1);
        c.that(t.isSparse() && t.isAttached(), "sparse");
        c.eq(t.rowOf(0x02000003), 1, "sparse rowOf");
        c.eq(t.rowOf(0x01000003), -1, "sparse rowOf checks the generation");
        c.eq(t.rowOf(1), -1, "sparse miss");
        c.eq(t.rowOfNode(7), 0, "O(1) row of node index");
        c.eq(t.rowOfNode(256), -1, "index past the directory");
        c.eq(t.rowOfNode(-1), -1, "negative index");
        c.eq(t.owner(0), 0x01000007, "sparse owner");
        final TableReader noPage = new TableReader().attach(Key64.of(1L), 0, 2, 2, words(1, 2, 3, 4),
                words(0x01000007, 0x02000003), null, 0, words(-1), 1, sparseIndexPage(7, 3), 1);
        c.eq(noPage.rowOfNode(7), -1, "NO_PAGE directory slot");
        final TableReader lying = new TableReader().attach(Key64.of(1L), 0, 2, 1, words(1, 2),
                words(0x01000007), null, 0, words(0), 1, sparseIndexPage(3, 7), 1);
        c.eq(lying.rowOfNode(7), -1, "an index entry past row_count is rejected");
        c.throwsA(IllegalArgumentException.class, () -> new TableReader().attach(Key64.of(1L), 0, 2, 2, words(1, 2, 3, 4),
                words(0x01000007, 0x02000003), null, 0, words(0), 1, words(0, 1), 1), "short index pages");
        c.throwsA(IllegalArgumentException.class, () -> new TableReader().attach(Key64.of(1L), 0, 2, 2, words(1, 2, 3, 4),
                words(0x01000007, 0x02000003), null, 0, null, 1, sparseIndexPage(7, 3), 1), "null index dir");

        // Detached: every read throws, even through a reference retained past the frame.
        t.detach();
        c.that(!t.isAttached() && t.rowCount() == 0 && !t.isSparse(), "detach drops counts and buffers");
        c.throwsA(IndexOutOfBoundsException.class, () -> t.word(0, 0), "detached word");
        c.throwsA(IndexOutOfBoundsException.class, () -> t.owner(0), "detached owner");
        c.throwsA(IndexOutOfBoundsException.class, () -> t.dirtyRow(0), "detached dirty row");
        c.eq(t.rowOf(0x02000003), -1, "detached rowOf misses");
        c.throwsA(IllegalArgumentException.class, () -> new TableReader().attach(VISUAL, 0, 2, 3, words(1, 2, 3, 4), null, null, 0, null, 0, null, 0), "short rows");
        c.throwsA(IllegalArgumentException.class, () -> new TableReader().attach(VISUAL, 0, 33, 0, null, null, null, 0, null, 0, null, 0), "33 words");
        c.throwsA(IllegalArgumentException.class, () -> new TableReader().attach(VISUAL, 0, 0, 0, null, null, null, 0, null, 0, null, 0), "0 words");
        c.throwsA(IllegalArgumentException.class, () -> new TableReader().attach(VISUAL, 0, 2, 2, words(1, 2, 3, 4), words(1), null, 0, null, 0, null, 0), "short owners");
        c.throwsA(IllegalArgumentException.class, () -> new TableReader().attach(VISUAL, 0, 2, 0, null, null, null, 2, null, 0, null, 0), "null dirty");
    }
}
