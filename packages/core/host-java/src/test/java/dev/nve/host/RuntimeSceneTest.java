package dev.nve.host;

import static dev.nve.host.generated.NveJniFrame.*;

import java.nio.ByteBuffer;

/** {@link RuntimeSceneReader} over packed frames from {@link PackedSceneBuilder} (the JNI layout). */
final class RuntimeSceneTest {
    private RuntimeSceneTest() {}

    static final int CAP = 4;
    static final int VISUAL_SLOT = 0;
    static final int TEXT_SLOT = 33;            // second `table_set` word
    static final FieldSpec BG = FieldSpec.word(0, FieldKind.COLOR);
    static final Key64 VISUAL_KEY = Key64.of("nve.visual");
    static final Key64 TEXT_KEY = Key64.of("nve.text");
    static final FieldSpec TEXT_FIELD = FieldSpec.word(0, FieldKind.STR);

    /** Index 1 root (visual), 2 text child of generation 3 (visual + text, primary = text), 3 free. */
    static PackedSceneBuilder builder() {
        final PackedSceneBuilder b = new PackedSceneBuilder(CAP, 9L, 3)
                .generation(2, 3)
                .node(2, 1, 4f, 4.5f, 5f, 5.5f)
                .primary(2, TEXT_SLOT)
                .effects(2, 5)
                .dirty(1, 2)
                .moved()
                .removed(0x77);
        b.table(VISUAL_KEY, VISUAL_SLOT, 1, true).set(1, BG, 0x11).set(2, BG, 0x22).dirtyRows(2);
        b.table(TEXT_KEY, TEXT_SLOT, 2, false).set(2, TEXT_FIELD, "h\u00e9");
        b.appendText("\ud83d\ude80");
        return b;
    }

    static int child(PackedSceneBuilder b) {
        return b.nodeId(2);
    }

    static void run(Check c) {
        nodes(c);
        tablesAndText(c);
        deltasAndScroll(c);
        rejects(c);
        detach(c);
        steadyState(c);
    }

    static void nodes(Check c) {
        final PackedSceneBuilder b = builder();
        final RuntimeSceneReader s = b.attach(new RuntimeSceneReader());
        final int root = b.nodeId(1);
        final int child = child(b);
        c.eq(s.frameId(), 9, "frame id");
        c.eq(s.sceneEpoch(), 3, "scene epoch");
        c.eq(s.nodeCapacity(), CAP, "capacity");
        c.that(!s.isLive(0) && s.isLive(1) && s.isLive(2) && !s.isLive(3) && !s.isLive(4) && !s.isLive(-1), "liveness");
        c.eq(s.nodeId(2), child, "child id");
        c.eq(child, (3 << 24) | 2, "generation 3");
        c.eq(s.nodeId(3), 0, "free slot id");
        c.eq(s.resolve(child), 2, "resolve");
        c.eq(s.resolve((2 << 24) | 2), -1, "stale generation");
        c.eq(s.resolve(3), -1, "free slot");
        c.eq(s.resolve(0), -1, "null id");
        c.eq(s.resolve(CAP), -1, "past capacity");
        c.eq(RuntimeSceneReader.indexOf(child), 2, "indexOf");
        c.eq(RuntimeSceneReader.generationOf(0xFF000001), 255, "generationOf unsigned");
        c.eq(s.parent(2), root, "parent");
        c.eq(s.parent(1), 0, "root parent");
        c.eq(s.childCount(1), 1, "child count");
        c.eq(s.childId(1, 0), child, "child id");
        c.throwsA(IndexOutOfBoundsException.class, () -> s.childId(1, 1), "child oob");
        c.throwsA(IndexOutOfBoundsException.class, () -> s.childId(1, -1), "child negative");
        c.eq(s.generation(2), 3, "generation");
        c.eq(s.effects(2), 5, "effects");
        c.eq(s.flags(2), dev.nve.host.generated.NveRuntimeAbi.NODE_ROW_FLAG_LIVE, "flags");
        c.that(s.hasTable(2, VISUAL_SLOT) && s.hasTable(2, TEXT_SLOT) && !s.hasTable(1, TEXT_SLOT), "table bits");
        c.that(!s.hasTable(2, 127) && !s.hasTable(2, 128) && !s.hasTable(2, -1), "table bits out of range");
        c.that(s.x(2) == 4f && s.y(2) == 4.5f && s.width(2) == 5f && s.height(2) == 5.5f, "rect");
        c.throwsA(IndexOutOfBoundsException.class, () -> s.x(CAP), "rect oob");
        c.throwsA(IndexOutOfBoundsException.class, () -> s.parent(CAP), "row oob");

        // A corrupt child span (off + len past the slab, or `u32` wrap) never reads out of range.
        final ByteBuffer[] slabs = b.slabs();
        slabs[S_ROWS].putInt(32 + 24, 2);
        final RuntimeSceneReader bad = new RuntimeSceneReader().attach(b.header(), slabs);
        c.throwsA(IndexOutOfBoundsException.class, () -> bad.childCount(1), "span past slab");
        slabs[S_ROWS].putInt(32 + 20, -1);
        final RuntimeSceneReader wrap = new RuntimeSceneReader().attach(b.header(), slabs);
        c.throwsA(IndexOutOfBoundsException.class, () -> wrap.childId(1, 0), "span offset wrap");
    }

    static void tablesAndText(Check c) {
        final PackedSceneBuilder b = builder();
        final RuntimeSceneReader s = b.attach(new RuntimeSceneReader());
        final TableReader visual = s.findTable(VISUAL_KEY);
        final TableReader text = s.tableAtSlot(TEXT_SLOT);
        c.that(visual != null && visual == s.tableAtSlot(VISUAL_SLOT) && visual == s.table(0), "visual lookup");
        c.that(text != null && text.key().equals(TEXT_KEY), "text by slot");
        c.that(s.tableAtSlot(5) == null && s.tableAtSlot(-1) == null && s.tableAtSlot(128) == null, "missing slots");
        c.that(s.findTable(Key64.of("nve.missing")) == null, "missing key");
        c.that(s.primaryTable(2) == text && s.primaryTable(1) == null, "primary table");
        c.eq(s.rowOf(visual, 2), 2, "dense row");
        c.eq(visual.color(s.rowOf(visual, 2), BG), 0x22, "dense value");
        c.eq(visual.dirtyRow(0), 2, "dense dirty row");
        c.eq(s.rowOf(text, 2), 0, "sparse row through the published index");
        c.eq(s.rowOf(text, 1), -1, "no table bit");
        c.eq(s.rowOf(text, 3), -1, "free node");
        c.eq(s.rowOf(null, 2), -1, "null table");
        c.that(s.text(text, 0, TEXT_FIELD).equals("h\u00e9"), "string field");
        c.that(s.text(3, 4).equals("\ud83d\ude80"), "astral text");
        c.that(s.text(7, 0).isEmpty(), "empty at end");
        c.throwsA(IndexOutOfBoundsException.class, () -> s.text(2, 1), "split char start");
        c.throwsA(IndexOutOfBoundsException.class, () -> s.text(0, 2), "split char end");
        c.throwsA(IndexOutOfBoundsException.class, () -> s.text(7, 1), "text oob");
        c.throwsA(IndexOutOfBoundsException.class, () -> s.text(8, 0), "offset past slab");
        c.throwsA(IndexOutOfBoundsException.class, () -> s.text(Integer.MAX_VALUE, 2), "text overflow");
        c.throwsA(IndexOutOfBoundsException.class, () -> s.table(2), "table oob");

        // An index entry whose row is owned by another generation of the slot is not the node's row.
        final ByteBuffer[] stale = b.slabs();
        stale[S_TABLES + TABLE_SLAB_STRIDE + TS_OWNERS].putInt(0, (1 << 24) | 2);
        final RuntimeSceneReader st = new RuntimeSceneReader().attach(b.header(), stale);
        c.eq(st.rowOf(st.tableAtSlot(TEXT_SLOT), 2), -1, "stale owner generation is no row");

        // Re-attach without the text table: the slot map is per frame, the old reader detached.
        final PackedSceneBuilder onlyVisual = new PackedSceneBuilder(CAP, 10L, 3).generation(2, 3).node(2, 1, 0f, 0f, 1f, 1f);
        onlyVisual.table(VISUAL_KEY, VISUAL_SLOT, 1, true).set(2, BG, 1);
        onlyVisual.attach(s);
        c.that(s.tableAtSlot(TEXT_SLOT) == null && s.primaryTable(2) == null, "slot map reset");
        c.that(!text.isAttached(), "a pooled table not published this frame is detached");
    }

    static void deltasAndScroll(Check c) {
        final PackedSceneBuilder b = builder().moved(3, 1).dirty(2).dirtyResources(3, 9, 0x80000000)
                .scroll(1, 0f, 30f).scroll((3 << 24) | 2, 4f, 0f);
        final RuntimeSceneReader s = b.attach(new RuntimeSceneReader());
        c.eq(s.dirtyCount(), 1, "dirty count");
        c.eq(s.dirtyIndex(0), 2, "dirty index");
        c.throwsA(IndexOutOfBoundsException.class, () -> s.dirtyIndex(1), "dirty oob");
        c.eq(s.movedCount(), 2, "moved count");
        c.that(s.movedIndex(0) == 3 && s.movedIndex(1) == 1, "moved indexes");
        c.throwsA(IndexOutOfBoundsException.class, () -> s.movedIndex(2), "moved oob");
        c.eq(s.removedId(0), 0x77, "removed");
        c.throwsA(IndexOutOfBoundsException.class, () -> s.removedId(1), "removed oob");
        c.that(s.dirtyResourceCount() == 3 && s.dirtyResource(1) == 9, "dirty resources");
        c.that(s.isResourceDirty(3) && s.isResourceDirty(0x80000000) && !s.isResourceDirty(4), "dirty resource lookup (unsigned)");
        c.eq(s.scrollCount(), 2, "scroll count");
        c.that(s.scrollY(1) == 30f && s.scrollX(1) == 0f, "root scroll");
        c.that(s.scrollX((3 << 24) | 2) == 4f, "child scroll");
        c.that(s.scrollX(2) == 0f && s.scrollY(99) == 0f && s.scrollIndexOf(0) == -1, "unscrolled nodes are zero");

        c.eq(s.relationTo(0L, 3), RuntimeSceneReader.FRAME_RESET, "no previous frame");
        c.eq(s.relationTo(8L, 3), RuntimeSceneReader.FRAME_NEXT, "the next frame");
        c.eq(s.relationTo(9L, 3), RuntimeSceneReader.FRAME_SAME, "a redelivered frame");
        c.eq(s.relationTo(7L, 3), RuntimeSceneReader.FRAME_RESET, "a skipped frame");
        c.eq(s.relationTo(8L, 2), RuntimeSceneReader.FRAME_RESET, "another epoch");
    }

    static SceneException.Kind kindOf(Check c, Runnable body, String what) {
        final SceneException e = c.throwsA(SceneException.class, body, what);
        return e == null ? null : e.kind();
    }

    interface Corrupt {
        void apply(long[] h, ByteBuffer[] s);
    }

    static SceneException.Kind reject(Check c, PackedSceneBuilder b, Corrupt corrupt, String what) {
        final long[] h = b.header();
        final ByteBuffer[] s = b.slabs();
        corrupt.apply(h, s);
        return kindOf(c, () -> new RuntimeSceneReader().attach(h, s), what);
    }

    static void rejects(Check c) {
        final PackedSceneBuilder b = builder();
        final int text = S_TABLES + TABLE_SLAB_STRIDE;
        final int textH = H_TABLES + TABLE_HEADER_STRIDE;
        c.that(reject(c, b, (h, s) -> h[H_SCHEMA] = 1L, "schema") == SceneException.Kind.SCHEMA_MISMATCH, "schema kind");
        c.that(reject(c, b, (h, s) -> s[S_ROWS] = RuntimeSceneTestBuffers.words(1), "short rows") == SceneException.Kind.MISSING_SLAB, "rows kind");
        reject(c, b, (h, s) -> s[S_RECTS] = RuntimeSceneTestBuffers.words(1), "short rects");
        reject(c, b, (h, s) -> s[S_TEXT] = null, "null text");
        reject(c, b, (h, s) -> h[H_CHILDREN_LEN] = 2, "short children");
        reject(c, b, (h, s) -> h[H_DIRTY_COUNT] = 3, "short dirty");
        reject(c, b, (h, s) -> h[H_SCROLL_COUNT] = 1, "short scroll offsets");
        reject(c, b, (h, s) -> h[H_NODE_CAPACITY] = -1, "negative capacity");
        reject(c, b, (h, s) -> h[H_NODE_CAPACITY] = (1 << 24) + 1, "capacity past index space");
        c.that(reject(c, builder().dirty(0), (h, s) -> { }, "dirty null slot") == SceneException.Kind.INVALID_NODE, "dirty kind");
        reject(c, builder().dirty(CAP), (h, s) -> { }, "dirty past capacity");
        reject(c, builder().dirty(-1), (h, s) -> { }, "dirty u32 max");
        reject(c, builder().moved(0), (h, s) -> { }, "moved null slot");
        reject(c, builder().moved(CAP), (h, s) -> { }, "moved past capacity");
        reject(c, builder().dirtyResources(9, 3), (h, s) -> { }, "unsorted dirty resources");
        reject(c, builder().dirtyResources(3, 3), (h, s) -> { }, "duplicate dirty resources");
        reject(c, builder().dirtyResources(0x80000000, 1), (h, s) -> { }, "dirty resources sort unsigned");
        reject(c, builder().scroll(5, 0f, 1f).scroll(2, 0f, 1f), (h, s) -> { }, "unsorted scroll offsets");
        reject(c, builder().scroll(5, 0f, 1f).scroll(5, 0f, 1f), (h, s) -> { }, "duplicate scroll offsets");
        c.that(reject(c, b, (h, s) -> h[textH + TH_SLOT] = VISUAL_SLOT, "duplicate slot") == SceneException.Kind.INVALID_TABLE, "duplicate kind");
        reject(c, b, (h, s) -> h[textH + TH_SLOT] = 128, "slot 128");
        reject(c, b, (h, s) -> h[H_TABLES + TH_ROW_COUNT] = CAP + 1, "dense rows past capacity");
        reject(c, b, (h, s) -> h[H_TABLES + TH_WORDS] = 0, "zero-word table");
        reject(c, b, (h, s) -> h[H_TABLE_COUNT] = MAX_TABLES + 1, "129 tables");
        reject(c, b, (h, s) -> h[textH + TH_ROW_COUNT] = 1L << 32, "row count past int");
        c.that(reject(c, b, (h, s) -> s[text + TS_ROWS] = null, "table rows missing") == SceneException.Kind.INVALID_TABLE, "table rows kind");
        reject(c, b, (h, s) -> s[text + TS_ROW_INDEX_PAGES] = null, "row index pages missing");
        reject(c, b, (h, s) -> h[textH + TH_ROW_INDEX_DIR_LEN] = 5, "row index dir past its slab");
        kindOf(c, () -> new RuntimeSceneReader().attach(new long[3], b.slabs()), "short header");
        kindOf(c, () -> new RuntimeSceneReader().attach(b.header(), new ByteBuffer[7]), "short slabs");
        kindOf(c, () -> new RuntimeSceneReader().attach(null, null), "null arrays");
        final RuntimeSceneReader empty = new PackedSceneBuilder(0, 1L, 0).dirty().attach(new RuntimeSceneReader());
        c.that(empty.nodeCapacity() == 0 && !empty.isLive(0) && empty.resolve((1 << 24) | 1) == -1, "empty frame");
    }

    /** A rejected or released frame leaves nothing readable, through retained references too. */
    static void detach(Check c) {
        final PackedSceneBuilder b = builder();
        final RuntimeSceneReader live = b.attach(new RuntimeSceneReader());
        final TableReader retained = live.findTable(TEXT_KEY);
        final PackedSceneBuilder bad = builder().dirty(0);
        kindOf(c, () -> live.attach(bad.header(), bad.slabs()), "reject over a live frame");
        c.that(live.nodeCapacity() == 0 && live.tableCount() == 0 && live.tableAtSlot(VISUAL_SLOT) == null && !live.isLive(1),
                "rejected frame leaves the reader detached");
        c.throwsA(IndexOutOfBoundsException.class, () -> live.x(1), "detached reads throw instead of touching freed buffers");
        c.that(!retained.isAttached(), "a retained table reader is detached with the frame");
        c.throwsA(IndexOutOfBoundsException.class, () -> retained.word(0, 0), "retained table reader refuses reads");

        b.attach(live);
        final TableReader visual = live.findTable(VISUAL_KEY);
        live.detach();
        c.that(live.nodeCapacity() == 0 && live.dirtyCount() == 0 && live.movedCount() == 0 && live.scrollCount() == 0
                && live.findTable(VISUAL_KEY) == null, "detach");
        c.throwsA(IndexOutOfBoundsException.class, () -> visual.color(2, BG), "released dense table refuses reads");
        c.throwsA(IndexOutOfBoundsException.class, () -> live.dirtyResource(0), "released deltas refuse reads");
    }

    static void steadyState(Check c) {
        if (!Alloc.supported()) return;
        final RuntimeSceneReader s = new RuntimeSceneReader();
        final PackedSceneBuilder b = builder().moved(3).scroll(1, 0f, 2f);
        final long[] h = b.header();
        final ByteBuffer[] slabs = b.slabs();
        s.attach(h, slabs);
        final int[] sink = new int[1];
        final long bytes = Alloc.bytes(() -> {
            for (int frame = 0; frame < 64; frame++) {
                s.attach(h, slabs);
                for (int d = 0; d < s.dirtyCount(); d++) {
                    final int i = s.dirtyIndex(d);
                    sink[0] += s.nodeId(i) + s.childCount(i) + Float.floatToRawIntBits(s.width(i))
                            + s.rowOf(s.tableAtSlot(TEXT_SLOT), i) + s.rowOf(s.tableAtSlot(VISUAL_SLOT), i)
                            + Float.floatToRawIntBits(s.scrollY(s.nodeId(i)));
                }
                for (int m = 0; m < s.movedCount(); m++) sink[0] += s.movedIndex(m);
                s.detach();
            }
        });
        c.eq(bytes, 0, "steady-state packed attach + O(1) reads + detach allocate nothing");
    }
}

/** Small direct little-endian buffers for corrupting packed frames. */
final class RuntimeSceneTestBuffers {
    private RuntimeSceneTestBuffers() {}

    static ByteBuffer words(int... w) {
        return TableSceneTest.words(w);
    }
}
