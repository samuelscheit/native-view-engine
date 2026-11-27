package dev.nve.host;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;

final class TableSceneTest {
    private TableSceneTest() {}

    static final Key64 SCHEMA = Key64.of(0xF00DFACEC0FFEE11L);
    static final Key64 VISUAL = Key64.of("nve.visual");
    static final FieldSpec OPACITY = new FieldSpec(0, 0, 8, FieldKind.U32);
    static final FieldSpec Z = new FieldSpec(0, 8, 12, FieldKind.I32);
    static final FieldSpec VISIBLE = new FieldSpec(0, 20, 1, FieldKind.BOOL);
    static final FieldSpec WIDTH = FieldSpec.word(1, FieldKind.LENGTH);
    static final FieldSpec COLOR = FieldSpec.word(2, FieldKind.COLOR);
    static final FieldSpec LABEL = FieldSpec.word(3, FieldKind.STR);
    static final FieldSpec SCALE = FieldSpec.word(5, FieldKind.F32);

    static final int NODE_STRIDE = 16;  // test node: id u32 @0, text_offset u32 @4, text_len u16 @8, child_count u16 @10, child_offset u32 @12

    static ByteBuffer words(int... w) {
        final ByteBuffer b = ByteBuffer.allocateDirect(w.length * 4).order(ByteOrder.LITTLE_ENDIAN);
        for (int v : w) b.putInt(v);
        b.flip();
        return b;
    }

    static ByteBuffer rows() {
        return words(
                200 | (0xFFD << 8) | (1 << 20), 0x42480001, 0x80402010, 0, 3, Float.floatToRawIntBits(1.5f),
                2047 << 8, 2, 0, 3, 0, 0x80000000);
    }

    static void run(Check c) {
        fieldSpecs(c);
        denseTable(c);
        sparseAndInvalid(c);
        scene(c);
        sceneRejects(c);
    }

    static void fieldSpecs(Check c) {
        c.eq(new FieldSpec(0, 8, 8, FieldKind.U32).extract(0xABCD1234), 0x12, "extract");
        c.eq(FieldSpec.word(0, FieldKind.U32).extract(0xABCD1234), 0xABCD1234, "extract whole");
        c.eq(new FieldSpec(0, 0, 3, FieldKind.I32).extractSigned(7), -1, "sign 3 bits");
        c.eq(new FieldSpec(0, 0, 3, FieldKind.I32).extractSigned(4), -4, "sign min");
        c.eq(FieldSpec.word(0, FieldKind.I32).extractSigned(0x80000000), Integer.MIN_VALUE, "sign 32");
        c.eq(new FieldSpec(0, 31, 1, FieldKind.I32).extractSigned(0x80000000), -1, "sign top bit");
        c.that(LABEL.fits(5) && !LABEL.fits(4), "str fits");
        c.that(!new FieldSpec(0, 24, 16, FieldKind.U32).fits(1), "too wide");
        c.that(!new FieldSpec(0, 0, 0, FieldKind.U32).fits(1), "zero bits");
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
    }

    static void denseTable(Check c) {
        final TableReader t = new TableReader().attach(VISUAL, 4, 6, 2, rows(), null, words(1), 1);
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
        c.eq(new TableReader().attach(VISUAL, 0, 2, 1, shifted, null, null, 0).word(0, 1), 8, "position-relative");
    }

    static void sparseAndInvalid(Check c) {
        final TableReader t = new TableReader().attach(Key64.of(1L), 0, 2, 2, words(1, 2, 3, 4), words(0x00010007, 0x00020003), null, 0);
        c.that(t.isSparse(), "sparse");
        c.eq(t.rowOf(0x00020003), 1, "sparse rowOf");
        c.eq(t.rowOf(1), -1, "sparse miss");
        c.eq(t.owner(0), 0x00010007, "sparse owner");
        c.throwsA(IllegalArgumentException.class, () -> new TableReader().attach(VISUAL, 0, 2, 3, words(1, 2, 3, 4), null, null, 0), "short rows");
        c.throwsA(IllegalArgumentException.class, () -> new TableReader().attach(VISUAL, 0, 33, 0, null, null, null, 0), "33 words");
        c.throwsA(IllegalArgumentException.class, () -> new TableReader().attach(VISUAL, 0, 0, 0, null, null, null, 0), "0 words");
        c.throwsA(IllegalArgumentException.class, () -> new TableReader().attach(VISUAL, 0, 2, 2, words(1, 2, 3, 4), words(1), null, 0), "short owners");
        c.throwsA(IllegalArgumentException.class, () -> new TableReader().attach(VISUAL, 0, 2, 0, null, null, null, 2), "null dirty");
    }

    static ByteBuffer nodes() {
        final ByteBuffer b = ByteBuffer.allocateDirect(3 * NODE_STRIDE).order(ByteOrder.LITTLE_ENDIAN);
        final int[][] n = {{1, 0, 3, 2, 0}, {2, 3, 4, 0, 2}, {3, 7, 0, 0, 2}};
        for (int[] r : n) {
            b.putInt(r[0]).putInt(r[1]).putShort((short) r[2]).putShort((short) r[3]).putInt(r[4]);
        }
        b.flip();
        return b;
    }

    static ByteBuffer text(int... bytes) {
        final ByteBuffer b = ByteBuffer.allocateDirect(bytes.length);
        for (int v : bytes) b.put((byte) v);
        b.flip();
        return b;
    }

    static final int[] TEXT = {'h', 0xC3, 0xA9, 0xF0, 0x9F, 0x9A, 0x80};

    static SceneReader attach(Key64 expected, ByteBuffer textSlab, ByteBuffer children, TableReader[] tables) {
        return new SceneReader().attach(expected, SCHEMA.value(), 7L, nodes(), 3, NODE_STRIDE, textSlab, 7,
                children, 2, words(0, 2, 99), 3, words(42), 1, tables, tables.length);
    }

    static void scene(Check c) {
        final TableReader[] tables = {new TableReader().attach(VISUAL, 0, 6, 2, rows(), null, null, 0)};
        final SceneReader s = attach(SCHEMA, text(TEXT), words(2, 3), tables);
        c.eq(s.frameId(), 7, "frame id");
        c.eq(s.nodeCount(), 3, "node count");
        c.eq(s.nodeI32(1, 0), 2, "node id");
        c.that(s.text(s.nodeI32(0, 4), s.nodeU16(0, 8)).equals("h\u00e9"), "node text");
        c.that(s.text(3, 4).equals("\ud83d\ude80"), "rocket");
        c.that(s.text(7, 0).isEmpty(), "empty at end");
        c.throwsA(IndexOutOfBoundsException.class, () -> s.text(2, 1), "split char");
        c.throwsA(IndexOutOfBoundsException.class, () -> s.text(4, 3), "mid rocket");
        c.throwsA(IndexOutOfBoundsException.class, () -> s.text(7, 1), "text oob");
        c.throwsA(IndexOutOfBoundsException.class, () -> s.text(Integer.MAX_VALUE, 2), "text overflow");
        c.eq(s.childId(s.nodeI32(0, 12) + 1), 3, "child");
        c.throwsA(IndexOutOfBoundsException.class, () -> s.childId(2), "child oob");
        c.throwsA(IndexOutOfBoundsException.class, () -> s.nodeI32(3, 0), "node oob");
        c.throwsA(IndexOutOfBoundsException.class, () -> s.nodeI32(0, 13), "field past stride");
        c.eq(s.nodeU8(0, 0), 1, "node u8");
        c.that(s.nodeF32(0, 0) == Float.intBitsToFloat(1), "node f32 bits");
        int visited = 0;
        for (int i = 0; i < s.dirtyCount(); i++) {
            final int idx = s.dirtyIndex(i);
            if (idx < s.nodeCount()) {
                c.eq(s.nodeI32(idx, 0), idx + 1, "dirty node id");
                visited++;
            }
        }
        c.eq(visited, 2, "dirty visits");
        c.eq(s.removedId(0), 42, "removed");
        c.that(s.findTable(VISUAL) == tables[0], "find table");
        c.that(s.findTable(Key64.of(1L)) == null, "missing table");
        c.eq(s.findTable(VISUAL).u32(0, OPACITY), 200, "table via scene");
        // Heap (non-direct) text slab takes the array fast path.
        final ByteBuffer heap = ByteBuffer.wrap(new byte[] {0, 'h', (byte) 0xC3, (byte) 0xA9, (byte) 0xF0, (byte) 0x9F, (byte) 0x9A, (byte) 0x80});
        heap.position(1);
        c.that(attach(SCHEMA, heap, words(2, 3), tables).text(0, 3).equals("h\u00e9"), "heap text slab");
    }

    static void sceneRejects(Check c) {
        final TableReader[] none = {};
        final SceneException mismatch = c.throwsA(SceneException.class, () -> attach(Key64.of(1L), text(TEXT), words(2, 3), none), "schema");
        c.that(mismatch != null && mismatch.kind() == SceneException.Kind.SCHEMA_MISMATCH, "schema kind");
        final SceneException missing = c.throwsA(SceneException.class, () -> attach(SCHEMA, text(TEXT), null, none), "null children");
        c.that(missing != null && missing.kind() == SceneException.Kind.MISSING_SLAB, "missing kind");
        c.throwsA(SceneException.class, () -> attach(SCHEMA, text(TEXT), words(2), none), "short children");
        final SceneException utf = c.throwsA(SceneException.class, () -> attach(SCHEMA, text(0xC0, 0xAF, 0, 0, 0, 0, 0), words(2, 3), none), "bad utf8");
        c.that(utf != null && utf.kind() == SceneException.Kind.INVALID_UTF8, "utf8 kind");
        final SceneException table = c.throwsA(SceneException.class,
                () -> new SceneReader().attach(SCHEMA, SCHEMA.value(), 1L, null, 0, NODE_STRIDE, null, 0, null, 0, null, 0, null, 0, new TableReader[1], 1),
                "null table");
        c.that(table != null && table.kind() == SceneException.Kind.INVALID_TABLE, "table kind");
        final SceneReader empty = new SceneReader().attach(SCHEMA, SCHEMA.value(), 1L, null, 0, NODE_STRIDE, null, 0, null, 0, null, 0, null, 0, null, 0);
        c.eq(empty.nodeCount(), 0, "empty frame");
    }
}
