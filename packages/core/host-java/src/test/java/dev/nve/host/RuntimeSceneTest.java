package dev.nve.host;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;

/** {@link RuntimeSceneReader} over hand-built {@code NveRuntimeSceneView} slabs and the packed JNI frame. */
final class RuntimeSceneTest {
    private RuntimeSceneTest() {}

    static final long SCHEMA = RuntimeSceneReader.SCHEMA.value();
    static final int CAP = 4;
    static final int ROOT = (1 << 24) | 1;      // index 1, generation 1
    static final int CHILD = (3 << 24) | 2;     // index 2, generation 3
    static final int VISUAL_SLOT = 0;
    static final int TEXT_SLOT = 33;            // second `table_set` word
    static final int[] TEXT = {'h', 0xC3, 0xA9, 0xF0, 0x9F, 0x9A, 0x80};
    static final FieldSpec BG = FieldSpec.word(0, FieldKind.COLOR);
    static final Key64 VISUAL_KEY = Key64.of("nve.visual");
    static final Key64 TEXT_KEY = Key64.of("nve.text");
    static final FieldSpec TEXT_FIELD = FieldSpec.word(0, FieldKind.STR);

    static ByteBuffer words(int... w) {
        return TableSceneTest.words(w);
    }

    static ByteBuffer bytes(int... v) {
        final ByteBuffer b = ByteBuffer.allocateDirect(v.length);
        for (int x : v) b.put((byte) x);
        b.flip();
        return b;
    }

    /** Index 0 null, 1 root (visual), 2 text child (visual + text, primary = text), 3 free. */
    static ByteBuffer rows(int childOff, int childLen) {
        final ByteBuffer b = ByteBuffer.allocateDirect(CAP * RuntimeSceneReader.NODE_ROW_SIZE).order(ByteOrder.LITTLE_ENDIAN);
        row(b, 1, 1, 0, childOff, childLen, 1, 0xFF, 0);
        row(b, 2, 1 | 2, ROOT, 0, 0, 3, TEXT_SLOT, 0x5);
        row(b, 3, 0, 0, 0, 0, 2, 0xFF, 0);
        b.put(3 * 32 + 31, (byte) 0);  // free slot
        return b;
    }

    static void row(ByteBuffer b, int index, int tableBits, int parent, int off, int len, int gen, int primary, int effects) {
        final int at = index * 32;
        b.putInt(at, (tableBits & 1) << VISUAL_SLOT);
        b.putInt(at + 4, (tableBits & 2) != 0 ? 1 << (TEXT_SLOT - 32) : 0);
        b.putInt(at + 16, parent).putInt(at + 20, off).putInt(at + 24, len);
        b.put(at + 28, (byte) gen).put(at + 29, (byte) primary).put(at + 30, (byte) effects).put(at + 31, (byte) 1);
    }

    static ByteBuffer rects() {
        final ByteBuffer b = ByteBuffer.allocateDirect(CAP * 16).order(ByteOrder.LITTLE_ENDIAN);
        for (int i = 0; i < CAP * 4; i++) b.putFloat(i * 4, i * 0.5f);
        return b;
    }

    static TableReader[] tables() {
        return new TableReader[] {
            new TableReader().attach(VISUAL_KEY, VISUAL_SLOT, 1, CAP, words(0, 0x11, 0x22, 0x33), null, words(2), 1),
            // Row 0 is owned by a stale generation of index 2; row 1 is the live CHILD.
            new TableReader().attach(TEXT_KEY, TEXT_SLOT, 2, 2, words(0, 1, 0, 3), words((1 << 24) | 2, CHILD), null, 0),
        };
    }

    static RuntimeSceneReader attach(RuntimeSceneReader r, ByteBuffer rows, ByteBuffer children, int childrenLen,
                                     ByteBuffer dirty, int dirtyCount, TableReader[] tables) {
        return r.attach(SCHEMA, 9L, CAP, rows, rects(), bytes(TEXT), TEXT.length, children, childrenLen,
                dirty, dirtyCount, words(0x77), 1, tables, tables.length);
    }

    static RuntimeSceneReader scene() {
        return attach(new RuntimeSceneReader(), rows(0, 1), words(CHILD), 1, words(1, 2), 2, tables());
    }

    static void run(Check c) {
        nodes(c);
        tablesAndText(c);
        rejects(c);
        packed(c);
        steadyState(c);
    }

    static void nodes(Check c) {
        final RuntimeSceneReader s = scene();
        c.eq(s.frameId(), 9, "frame id");
        c.eq(s.nodeCapacity(), CAP, "capacity");
        c.that(!s.isLive(0) && s.isLive(1) && s.isLive(2) && !s.isLive(3) && !s.isLive(4) && !s.isLive(-1), "liveness");
        c.eq(s.nodeId(1), ROOT, "root id");
        c.eq(s.nodeId(2), CHILD, "child id");
        c.eq(s.nodeId(3), 0, "free slot id");
        c.eq(s.resolve(CHILD), 2, "resolve");
        c.eq(s.resolve((2 << 24) | 2), -1, "stale generation");
        c.eq(s.resolve(3), -1, "free slot");
        c.eq(s.resolve(0), -1, "null id");
        c.eq(s.resolve(CAP), -1, "past capacity");
        c.eq(RuntimeSceneReader.indexOf(CHILD), 2, "indexOf");
        c.eq(RuntimeSceneReader.generationOf(0xFF000001), 255, "generationOf unsigned");
        c.eq(s.parent(2), ROOT, "parent");
        c.eq(s.parent(1), 0, "root parent");
        c.eq(s.childCount(1), 1, "child count");
        c.eq(s.childId(1, 0), CHILD, "child id");
        c.throwsA(IndexOutOfBoundsException.class, () -> s.childId(1, 1), "child oob");
        c.throwsA(IndexOutOfBoundsException.class, () -> s.childId(1, -1), "child negative");
        c.eq(s.generation(2), 3, "generation");
        c.eq(s.effects(2), 5, "effects");
        c.eq(s.flags(2), RuntimeSceneReader.FLAG_LIVE, "flags");
        c.that(s.hasTable(2, VISUAL_SLOT) && s.hasTable(2, TEXT_SLOT) && !s.hasTable(1, TEXT_SLOT), "table bits");
        c.that(!s.hasTable(2, 127) && !s.hasTable(2, 128) && !s.hasTable(2, -1), "table bits out of range");
        c.that(s.x(2) == 4f && s.y(2) == 4.5f && s.width(2) == 5f && s.height(2) == 5.5f, "rect");
        c.throwsA(IndexOutOfBoundsException.class, () -> s.x(CAP), "rect oob");
        c.throwsA(IndexOutOfBoundsException.class, () -> s.parent(CAP), "row oob");
        c.eq(s.dirtyCount(), 2, "dirty count");
        c.eq(s.dirtyIndex(1), 2, "dirty index");
        c.throwsA(IndexOutOfBoundsException.class, () -> s.dirtyIndex(2), "dirty oob");
        c.eq(s.removedId(0), 0x77, "removed");
        c.throwsA(IndexOutOfBoundsException.class, () -> s.removedId(1), "removed oob");
        // A corrupt child span (off + len past the slab, or `u32` wrap) never reads out of range.
        final RuntimeSceneReader bad = attach(new RuntimeSceneReader(), rows(0, 2), words(CHILD), 1, null, 0, tables());
        c.throwsA(IndexOutOfBoundsException.class, () -> bad.childCount(1), "span past slab");
        final RuntimeSceneReader wrap = attach(new RuntimeSceneReader(), rows(-1, 2), words(CHILD), 1, null, 0, tables());
        c.throwsA(IndexOutOfBoundsException.class, () -> wrap.childId(1, 0), "span offset wrap");
    }

    static void tablesAndText(Check c) {
        final RuntimeSceneReader s = scene();
        final TableReader visual = s.findTable(VISUAL_KEY);
        final TableReader text = s.tableAtSlot(TEXT_SLOT);
        c.that(visual != null && visual == s.tableAtSlot(VISUAL_SLOT) && visual == s.table(0), "visual lookup");
        c.that(text != null && text.key().equals(TEXT_KEY), "text by slot");
        c.that(s.tableAtSlot(5) == null && s.tableAtSlot(-1) == null && s.tableAtSlot(128) == null, "missing slots");
        c.that(s.findTable(Key64.of("nve.missing")) == null, "missing key");
        c.that(s.primaryTable(2) == text && s.primaryTable(1) == null, "primary table");
        c.eq(s.rowOf(visual, 2), 2, "dense row");
        c.eq(visual.color(s.rowOf(visual, 2), BG), 0x22, "dense value");
        c.eq(s.rowOf(text, 2), 1, "sparse row skips stale owner");
        c.eq(s.rowOf(text, 1), -1, "no table bit");
        c.eq(s.rowOf(text, 3), -1, "free node");
        c.eq(s.rowOf(null, 2), -1, "null table");
        c.that(s.text(text, 1, TEXT_FIELD).equals("h\u00e9"), "string field");
        c.that(s.text(3, 4).equals("\ud83d\ude80"), "astral text");
        c.that(s.text(7, 0).isEmpty(), "empty at end");
        c.throwsA(IndexOutOfBoundsException.class, () -> s.text(2, 1), "split char start");
        c.throwsA(IndexOutOfBoundsException.class, () -> s.text(0, 2), "split char end");
        c.throwsA(IndexOutOfBoundsException.class, () -> s.text(7, 1), "text oob");
        c.throwsA(IndexOutOfBoundsException.class, () -> s.text(8, 0), "offset past slab");
        c.throwsA(IndexOutOfBoundsException.class, () -> s.text(Integer.MAX_VALUE, 2), "text overflow");
        c.throwsA(IndexOutOfBoundsException.class, () -> s.table(2), "table oob");
        // Re-attach without the text table: slot map and sparse index are per frame.
        final TableReader[] onlyVisual = {tables()[0]};
        attach(s, rows(0, 1), words(CHILD), 1, null, 0, onlyVisual);
        c.that(s.tableAtSlot(TEXT_SLOT) == null && s.primaryTable(2) == null, "slot map reset");
        final TableReader[] moved = {new TableReader().attach(TEXT_KEY, TEXT_SLOT, 2, 1, words(0, 3), words(CHILD), null, 0)};
        attach(s, rows(0, 1), words(CHILD), 1, null, 0, moved);
        c.eq(s.rowOf(s.tableAtSlot(TEXT_SLOT), 2), 0, "sparse index rebuilt per frame");
    }

    static SceneException.Kind kindOf(Check c, Runnable body, String what) {
        final SceneException e = c.throwsA(SceneException.class, body, what);
        return e == null ? null : e.kind();
    }

    static void rejects(Check c) {
        final RuntimeSceneReader r = new RuntimeSceneReader();
        c.that(kindOf(c, () -> r.attach(1L, 0, 0, null, null, null, 0, null, 0, null, 0, null, 0, null, 0), "schema")
                == SceneException.Kind.SCHEMA_MISMATCH, "schema kind");
        c.that(kindOf(c, () -> r.attach(SCHEMA, 0, CAP, words(1), rects(), null, 0, null, 0, null, 0, null, 0, null, 0), "short rows")
                == SceneException.Kind.MISSING_SLAB, "rows kind");
        kindOf(c, () -> r.attach(SCHEMA, 0, CAP, rows(0, 1), words(1), null, 0, null, 0, null, 0, null, 0, null, 0), "short rects");
        kindOf(c, () -> r.attach(SCHEMA, 0, CAP, rows(0, 1), rects(), null, 3, null, 0, null, 0, null, 0, null, 0), "null text");
        kindOf(c, () -> attach(r, rows(0, 1), words(CHILD), 2, null, 0, tables()), "short children");
        kindOf(c, () -> attach(r, rows(0, 1), words(CHILD), 1, words(1), 2, tables()), "short dirty");
        kindOf(c, () -> r.attach(SCHEMA, 0, -1, null, null, null, 0, null, 0, null, 0, null, 0, null, 0), "negative capacity");
        kindOf(c, () -> r.attach(SCHEMA, 0, (1 << 24) + 1, null, null, null, 0, null, 0, null, 0, null, 0, null, 0), "capacity past index space");
        c.that(kindOf(c, () -> attach(r, rows(0, 1), words(CHILD), 1, words(0), 1, tables()), "dirty null slot")
                == SceneException.Kind.INVALID_NODE, "dirty kind");
        kindOf(c, () -> attach(r, rows(0, 1), words(CHILD), 1, words(CAP), 1, tables()), "dirty past capacity");
        kindOf(c, () -> attach(r, rows(0, 1), words(CHILD), 1, words(-1), 1, tables()), "dirty u32 max");
        final TableReader dup = new TableReader().attach(Key64.of(5L), TEXT_SLOT, 1, 0, null, words(), null, 0);
        c.that(kindOf(c, () -> attach(r, rows(0, 1), words(CHILD), 1, null, 0, new TableReader[] {tables()[1], dup}), "duplicate slot")
                == SceneException.Kind.INVALID_TABLE, "duplicate kind");
        final TableReader far = new TableReader().attach(Key64.of(5L), 128, 1, 0, null, null, null, 0);
        kindOf(c, () -> attach(r, rows(0, 1), words(CHILD), 1, null, 0, new TableReader[] {far}), "slot 128");
        final TableReader wide = new TableReader().attach(Key64.of(5L), 1, 1, CAP + 1, words(0, 0, 0, 0, 0), null, null, 0);
        kindOf(c, () -> attach(r, rows(0, 1), words(CHILD), 1, null, 0, new TableReader[] {wide}), "dense rows past capacity");
        kindOf(c, () -> attach(r, rows(0, 1), words(CHILD), 1, null, 0, new TableReader[] {new TableReader()}), "unattached table");
        kindOf(c, () -> attach(r, rows(0, 1), words(CHILD), 1, null, 0, new TableReader[1]), "null table");
        // A rejected dirty frame leaves no stale slot entries behind for the next good frame.
        kindOf(c, () -> attach(r, rows(0, 1), words(CHILD), 1, words(0), 1, tables()), "rejected frame");
        attach(r, rows(0, 1), words(CHILD), 1, null, 0, new TableReader[0]);
        c.that(r.tableAtSlot(VISUAL_SLOT) == null, "no stale slot after rejection");
        final RuntimeSceneReader empty = new RuntimeSceneReader().attach(SCHEMA, 1L, 0, null, null, null, 0, null, 0, null, 0, null, 0, null, 0);
        c.that(empty.nodeCapacity() == 0 && !empty.isLive(0) && empty.resolve(ROOT) == -1, "empty frame");
    }

    static long[] header(int tableCount) {
        final long[] h = new long[RuntimeSceneReader.HEADER_LEN];
        h[RuntimeSceneReader.H_SCHEMA] = SCHEMA;
        h[RuntimeSceneReader.H_FRAME_ID] = 11;
        h[RuntimeSceneReader.H_NODE_CAPACITY] = CAP;
        h[RuntimeSceneReader.H_DIRTY_COUNT] = 1;
        h[RuntimeSceneReader.H_REMOVED_COUNT] = 0;
        h[RuntimeSceneReader.H_TEXT_LEN] = TEXT.length;
        h[RuntimeSceneReader.H_CHILDREN_LEN] = 1;
        h[RuntimeSceneReader.H_TABLE_COUNT] = tableCount;
        final long[][] t = {{VISUAL_KEY.value(), VISUAL_SLOT, 1, CAP, 1}, {TEXT_KEY.value(), TEXT_SLOT, 2, 2, 0}};
        for (int i = 0; i < tableCount; i++) {
            System.arraycopy(t[i], 0, h, RuntimeSceneReader.H_TABLES + i * RuntimeSceneReader.H_TABLE_STRIDE, 5);
        }
        return h;
    }

    static ByteBuffer[] slabs() {
        final ByteBuffer[] s = new ByteBuffer[RuntimeSceneReader.SLABS_LEN];
        s[RuntimeSceneReader.S_ROWS] = rows(0, 1);
        s[RuntimeSceneReader.S_RECTS] = rects();
        s[RuntimeSceneReader.S_TEXT] = bytes(TEXT);
        s[RuntimeSceneReader.S_CHILDREN] = words(CHILD);
        s[RuntimeSceneReader.S_DIRTY] = words(2);
        final int t0 = RuntimeSceneReader.S_TABLES;
        s[t0] = words(0, 0x11, 0x22, 0x33);
        s[t0 + 2] = words(2);
        s[t0 + RuntimeSceneReader.S_TABLE_STRIDE] = words(0, 1, 0, 3);
        s[t0 + RuntimeSceneReader.S_TABLE_STRIDE + 1] = words((1 << 24) | 2, CHILD);
        return s;
    }

    static void packed(Check c) {
        final RuntimeSceneReader s = new RuntimeSceneReader().attach(header(2), slabs());
        c.eq(s.frameId(), 11, "packed frame");
        c.eq(s.tableCount(), 2, "packed tables");
        c.that(s.text(s.tableAtSlot(TEXT_SLOT), 1, TEXT_FIELD).equals("h\u00e9"), "packed sparse string");
        c.eq(s.tableAtSlot(VISUAL_SLOT).dirtyRow(0), 2, "packed dirty row");
        c.that(!s.tableAtSlot(VISUAL_SLOT).isSparse() && s.tableAtSlot(TEXT_SLOT).isSparse(), "packed owners");
        c.eq(RuntimeSceneReader.HEADER_LEN, 8 + 128 * 5, "header len");
        c.eq(RuntimeSceneReader.SLABS_LEN, 6 + 128 * 3, "slabs len");
        final long[] tooMany = header(0);
        tooMany[RuntimeSceneReader.H_TABLE_COUNT] = RuntimeSceneReader.MAX_TABLES + 1;
        c.that(kindOf(c, () -> new RuntimeSceneReader().attach(tooMany, slabs()), "129 tables") == SceneException.Kind.INVALID_TABLE, "too many kind");
        final long[] negative = header(2);
        negative[RuntimeSceneReader.H_CHILDREN_LEN] = -1;
        kindOf(c, () -> new RuntimeSceneReader().attach(negative, slabs()), "negative count");
        final long[] huge = header(2);
        huge[RuntimeSceneReader.H_TABLES + 3] = 1L << 32;
        kindOf(c, () -> new RuntimeSceneReader().attach(huge, slabs()), "row count past int");
        final ByteBuffer[] noRows = slabs();
        noRows[RuntimeSceneReader.S_TABLES] = null;
        c.that(kindOf(c, () -> new RuntimeSceneReader().attach(header(2), noRows), "table rows missing") == SceneException.Kind.INVALID_TABLE, "table rows kind");
        kindOf(c, () -> new RuntimeSceneReader().attach(new long[3], slabs()), "short header");
        kindOf(c, () -> new RuntimeSceneReader().attach(header(2), new ByteBuffer[7]), "short slabs");
        kindOf(c, () -> new RuntimeSceneReader().attach(null, null), "null arrays");
    }

    static void steadyState(Check c) {
        if (!Alloc.supported()) return;
        final RuntimeSceneReader s = new RuntimeSceneReader();
        final long[] h = header(2);
        final ByteBuffer[] slabs = slabs();
        s.attach(h, slabs);
        s.rowOf(s.tableAtSlot(TEXT_SLOT), 2);  // warm the pooled sparse index
        final int[] sink = new int[1];
        final long bytes = Alloc.bytes(() -> {
            for (int frame = 0; frame < 64; frame++) {
                s.attach(h, slabs);
                for (int d = 0; d < s.dirtyCount(); d++) {
                    final int i = s.dirtyIndex(d);
                    sink[0] += s.nodeId(i) + s.childCount(i) + Float.floatToRawIntBits(s.width(i))
                            + s.rowOf(s.tableAtSlot(TEXT_SLOT), i) + s.rowOf(s.tableAtSlot(VISUAL_SLOT), i);
                }
            }
        });
        c.eq(bytes, 0, "steady-state packed attach + reads allocate nothing");
    }
}
