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
        optional64AndGridLine(c);
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
        c.throwsA(IllegalArgumentException.class, () -> FieldSpec.word(31, FieldKind.U64), "u64 past word 31");
        c.throwsA(IllegalArgumentException.class, () -> FieldSpec.word(31, FieldKind.I64), "i64 past word 31");
        c.throwsA(IllegalArgumentException.class, () -> FieldSpec.word(31, FieldKind.F64), "f64 past word 31");
        c.throwsA(IllegalArgumentException.class, () -> new FieldSpec(0, 1, 16, FieldKind.U64), "two-word sub-word");
        for (FieldKind k : new FieldKind[] {FieldKind.STR, FieldKind.U64, FieldKind.I64, FieldKind.F64}) {
            c.throwsA(IllegalArgumentException.class,
                    () -> new FieldSpec(0, 0, 32, k, OptEncoding.ZERO_NICHE, 0xFF, 0xFF, false),
                    "two-word rejects non-REQUIRED " + k);
        }
        c.throwsA(IllegalArgumentException.class, () -> new FieldSpec(0, 0, 8, null), "null kind");
        c.throwsA(IllegalArgumentException.class, () -> new FieldSpec(0, 0, 8, FieldKind.U32, null, 0xFF, 0xFF, false), "null optEncoding");
        c.throwsA(IllegalArgumentException.class, () -> new FieldSpec(0, 0, 2, FieldKind.F32, OptEncoding.WIDE_BIT, 1, 0xFF, false), "WIDE_BIT bits != 1");
        c.throwsA(IllegalArgumentException.class, () -> new FieldSpec(1, 0, 1, FieldKind.F32, OptEncoding.WIDE_BIT, 1, 0xFF, false), "WIDE_BIT valWord == word");
        c.throwsA(IllegalArgumentException.class, () -> new FieldSpec(0, 0, 1, FieldKind.F32, OptEncoding.WIDE_BIT, 32, 0xFF, false), "WIDE_BIT valWord >= 32");
        c.throwsA(IllegalArgumentException.class, () -> new FieldSpec(0, 0, 32, FieldKind.F32, OptEncoding.REQUIRED, 1, 0xFF, false), "REQUIRED with valWord");
        c.throwsA(IllegalArgumentException.class, () -> new FieldSpec(0, 0, 1, FieldKind.U32, OptEncoding.BIT0_PRESENT, 0xFF, 0xFF, false), "BIT0_PRESENT bits < 2");
        c.throwsA(IllegalArgumentException.class, () -> new FieldSpec(0, 0, 32, FieldKind.U32, OptEncoding.TAG_XOR, 0xFF, 0xFF, false), "TAG_XOR non-Length/GridLine");
        c.throwsA(IllegalArgumentException.class, () -> new FieldSpec(0, 0, 32, FieldKind.U32, OptEncoding.REQUIRED, 0xFF, 0, true), "inherited overlaps word 0");
        c.throwsA(IllegalArgumentException.class, () -> new FieldSpec(1, 0, 32, FieldKind.U32, OptEncoding.REQUIRED, 0xFF, 0xFF, true), "inherits without explicitBit");
        c.throwsA(IllegalArgumentException.class, () -> new FieldSpec(1, 0, 32, FieldKind.U32, OptEncoding.REQUIRED, 0xFF, 32, false), "explicitBit >= 32");

        final TableReader t = new TableReader().attach(VISUAL, 0, 6, 2, rows(), null, null, 0, null, 0, null, 0);
        c.throwsA(IndexOutOfBoundsException.class, () -> t.u32(0, FieldSpec.word(6, FieldKind.U32)), "field past row");
        c.throwsA(IndexOutOfBoundsException.class, () -> t.i32(0, FieldSpec.word(31, FieldKind.I32)), "i32 field past row");
        c.throwsA(IndexOutOfBoundsException.class, () -> t.strOffset(0, FieldSpec.word(3, FieldKind.U32)), "strOffset of a non-string spec");
        c.throwsA(IndexOutOfBoundsException.class, () -> t.strLen(0, FieldSpec.word(5, FieldKind.STR)), "strLen past the row");
        c.throwsA(IndexOutOfBoundsException.class, () -> t.strOffset(0, FieldSpec.word(30, FieldKind.STR)), "strOffset far past the row");
        c.throwsA(IndexOutOfBoundsException.class, () -> t.u64(0, FieldSpec.word(5, FieldKind.U64)), "u64 past the row");
        c.throwsA(IndexOutOfBoundsException.class, () -> t.f64(0, FieldSpec.word(0, FieldKind.STR)), "f64 of non-64-bit spec");
        c.eq(t.strLen(0, LABEL), 3, "valid string spec still reads");
    }

    static void fieldSpecs(Check c) {
        c.eq(FieldKind.U32.ordinal(), 0, "FieldKind.U32");
        c.eq(FieldKind.NODE.ordinal(), 9, "FieldKind.NODE");
        c.eq(FieldKind.U64.ordinal(), 10, "FieldKind.U64");
        c.eq(FieldKind.I64.ordinal(), 11, "FieldKind.I64");
        c.eq(FieldKind.F64.ordinal(), 12, "FieldKind.F64");
        c.eq(FieldKind.GRID_LINE.ordinal(), 13, "FieldKind.GRID_LINE");
        c.eq(FieldKind.values().length, 14, "FieldKind count");
        c.eq(OptEncoding.REQUIRED.ordinal(), 0, "OptEncoding.REQUIRED");
        c.eq(OptEncoding.WIDE_BIT.ordinal(), 5, "OptEncoding.WIDE_BIT");
        c.eq(OptEncoding.values().length, 6, "OptEncoding count");

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
        c.eq(GridLine.kind(0), GridLine.AUTO, "grid auto");
        c.eq(GridLine.kind(0xC000), GridLine.AUTO, "grid unused tag 11");
        c.eq(GridLine.kind(GridLine.encodeLine(-3)), GridLine.LINE, "grid line kind");
        c.eq(GridLine.line(GridLine.encodeLine(-3)), -3, "grid line -3");
        c.eq(GridLine.line(GridLine.encodeLine(99999)), GridLine.MAX_LINE, "grid line clamp max");
        c.eq(GridLine.line(GridLine.encodeLine(-99999)), GridLine.MIN_LINE, "grid line clamp min");
        c.eq(GridLine.kind(GridLine.encodeSpan(5)), GridLine.SPAN, "grid span kind");
        c.eq(GridLine.span(GridLine.encodeSpan(5)), 5, "grid span 5");
        c.eq(GridLine.span(GridLine.encodeSpan(99999)), GridLine.MAX_SPAN, "grid span clamp max");
        c.eq(Rgba.r(0x80402010), 0x10, "r");
        c.eq(Rgba.g(0x80402010), 0x20, "g");
        c.eq(Rgba.b(0x80402010), 0x40, "b");
        c.eq(Rgba.a(0x80402010), 0x80, "a");
        c.eq(Rgba.toArgb(0x80402010), 0x80102040, "RGBA8888 -> ARGB8888");
        c.eq(Rgba.toArgb(0xFF0000FF), 0xFFFF0000, "opaque red");
    }

    static void optional64AndGridLine(Check c) {
        final FieldSpec optNz = new FieldSpec(1, 0, 16, FieldKind.U32, OptEncoding.ZERO_NICHE, 0xFF, 0, true);
        final FieldSpec optPlus = new FieldSpec(1, 16, 6, FieldKind.ENUM, OptEncoding.PLUS_ONE, 0xFF, 1, true);
        final FieldSpec optWide = new FieldSpec(1, 22, 1, FieldKind.F32, OptEncoding.WIDE_BIT, 2, 2, false);
        final FieldSpec optI16 = new FieldSpec(3, 0, 17, FieldKind.I32, OptEncoding.BIT0_PRESENT, 0xFF, 0xFF, false);
        final FieldSpec optU8 = new FieldSpec(3, 17, 9, FieldKind.U32, OptEncoding.BIT0_PRESENT, 0xFF, 0xFF, false);
        final FieldSpec optLen = new FieldSpec(4, 0, 32, FieldKind.LENGTH, OptEncoding.TAG_XOR, 0xFF, 0xFF, false);
        final FieldSpec optGrid = new FieldSpec(5, 0, 16, FieldKind.GRID_LINE, OptEncoding.TAG_XOR, 0xFF, 0xFF, false);
        final FieldSpec reqGrid = new FieldSpec(5, 16, 16, FieldKind.GRID_LINE, OptEncoding.REQUIRED, 0xFF, 0xFF, false);
        final FieldSpec wideU64 = new FieldSpec(6, 0, 32, FieldKind.U64);
        final FieldSpec wideI64 = new FieldSpec(8, 0, 32, FieldKind.I64);
        final FieldSpec wideF64 = new FieldSpec(10, 0, 32, FieldKind.F64);

        final PackedSceneBuilder b = new PackedSceneBuilder(4, 1L, 1).node(2, 1, 0, 0, 10, 10);
        final Key64 key = Key64.of("nve.test.opt");
        final PackedSceneBuilder.Table tb = b.table(key, 0, 12, true);
        // Node 1: all optional fields absent (row words 0).
        tb.row(1);
        // Node 2: all optional fields present + 64-bit values.
        tb.set(2, optNz, 42)
                .set(2, optPlus, 0)
                .set(2, optWide, 1.75f)
                .set(2, optI16, -32768)
                .set(2, optU8, 255)
                .set(2, optLen, 0x42480001)  // 50%
                .set(2, optGrid, GridLine.encodeLine(-7))
                .set(2, reqGrid, GridLine.encodeSpan(4))
                .set(2, wideU64, 0xFEDC_BA98_7654_3210L)
                .set(2, wideI64, -9_000_000_000_000L)
                .set(2, wideF64, -3.25);
        final SceneReader s = b.attach(new SceneReader());
        final TableReader r = s.findTable(key);
        c.that(!r.isPresent(1, optNz) && !r.isPresent(1, optPlus) && !r.isPresent(1, optWide)
                && !r.isPresent(1, optI16) && !r.isPresent(1, optU8) && !r.isPresent(1, optLen)
                && !r.isPresent(1, optGrid), "zero row decodes every optional as absent");
        c.eq(r.u32Or(1, optNz, 99), 99, "u32Or fallback");
        c.eq(r.i32Or(1, optI16, -5), -5, "i32Or fallback");
        c.that(r.f32Or(1, optWide, 2.5f) == 2.5f, "f32Or fallback");
        c.that(!r.isExplicit(1, optNz) && r.isExplicit(2, optNz) && r.isExplicit(2, optPlus) && r.isExplicit(2, optWide),
                "inherited explicit bits");
        c.that(r.isExplicit(1, optI16) && r.isExplicit(2, optI16), "non-inherited field isExplicit returns true");
        c.throwsA(IndexOutOfBoundsException.class, () -> r.isExplicit(99, optI16), "isExplicit validates row bounds");
        c.throwsA(IndexOutOfBoundsException.class, () -> r.isExplicit(1, FieldSpec.word(12, FieldKind.U32)), "isExplicit validates field fit");
        c.that(r.isPresent(2, optNz) && r.u32(2, optNz) == 42, "ZERO_NICHE");
        c.that(r.isPresent(2, optPlus) && r.u32(2, optPlus) == 0, "PLUS_ONE discriminant 0");
        c.that(r.isPresent(2, optWide) && r.f32(2, optWide) == 1.75f, "WIDE_BIT f32");
        c.that(r.isPresent(2, optI16) && r.i32(2, optI16) == -32768, "BIT0_PRESENT i16 min");
        c.that(r.isPresent(2, optU8) && r.u32(2, optU8) == 255, "BIT0_PRESENT u8 max");
        c.that(r.isPresent(2, optLen) && Length.unit(r.length(2, optLen)) == Length.PERCENT
                && Length.value(r.length(2, optLen)) == 50f, "TAG_XOR Length");
        c.that(r.isPresent(2, optGrid) && GridLine.kind(r.gridLine(2, optGrid)) == GridLine.LINE
                && GridLine.line(r.gridLine(2, optGrid)) == -7, "TAG_XOR GridLine");
        c.that(GridLine.kind(r.gridLine(2, reqGrid)) == GridLine.SPAN && GridLine.span(r.gridLine(2, reqGrid)) == 4, "REQUIRED GridLine");
        c.eq(r.u64(2, wideU64), 0xFEDC_BA98_7654_3210L, "u64");
        c.eq(r.i64(2, wideI64), -9_000_000_000_000L, "i64");
        c.that(r.f64(2, wideF64) == -3.25, "f64");

        // Option<bool> under PLUS_ONE: raw 0b11 (3) is invalid -> behaves as None (both BOOL and 2-bit ENUM specs).
        final FieldSpec optBool = new FieldSpec(0, 0, 2, FieldKind.BOOL, OptEncoding.PLUS_ONE, 0xFF, 0xFF, false);
        final FieldSpec optBoolEnum = new FieldSpec(0, 2, 2, FieldKind.ENUM, OptEncoding.PLUS_ONE, 0xFF, 0xFF, false);
        final TableReader bt = new TableReader().attach(
                Key64.of("nve.test.optbool"), 0, 1, 4,
                words(0 | (0 << 2), 1 | (1 << 2), 2 | (2 << 2), 3 | (3 << 2)),
                null, null, 0, null, 0, null, 0);
        c.that(!bt.isPresent(0, optBool) && !bt.bool(0, optBool) && bt.boolOr(0, optBool, true) && !bt.boolOr(0, optBool, false), "PLUS_ONE bool raw 0");
        c.that(bt.isPresent(1, optBool) && !bt.bool(1, optBool) && !bt.boolOr(1, optBool, true) && !bt.boolOr(1, optBool, false), "PLUS_ONE bool raw 1 (false)");
        c.that(bt.isPresent(2, optBool) && bt.bool(2, optBool) && bt.boolOr(2, optBool, true) && bt.boolOr(2, optBool, false), "PLUS_ONE bool raw 2 (true)");
        c.that(!bt.isPresent(3, optBool) && !bt.bool(3, optBool) && bt.boolOr(3, optBool, true) && !bt.boolOr(3, optBool, false) && bt.u32(3, optBool) == 0,
                "PLUS_ONE bool raw 0b11 is absent");
        c.that(!bt.bool(3, optBoolEnum) && bt.boolOr(3, optBoolEnum, true) && !bt.boolOr(3, optBoolEnum, false),
                "PLUS_ONE 2-bit ENUM boolOr raw 0b11 returns fallback");
        c.that(bt.isPresent(3, optBoolEnum) && bt.u32(3, optBoolEnum) == 2, "3-variant PLUS_ONE ENUM raw 3 stays discriminant 2");
        // 32-bit PLUS_ONE BOOL with bit 31 set (negative signed int in Java) must decode as absent (0), not raw - 1.
        final FieldSpec optBool32 = new FieldSpec(0, 0, 32, FieldKind.BOOL, OptEncoding.PLUS_ONE, 0xFF, 0xFF, false);
        final TableReader bt32 = new TableReader().attach(
                Key64.of("nve.test.optbool32"), 0, 1, 1,
                words(0x8000_0000), null, null, 0, null, 0, null, 0);
        c.that(!bt32.isPresent(0, optBool32) && !bt32.bool(0, optBool32) && bt32.u32(0, optBool32) == 0 && bt32.i32(0, optBool32) == 0,
                "32-bit PLUS_ONE bool with bit 31 set is absent and returns 0");
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
