package dev.nve.host;

import static dev.nve.host.generated.NveJniFrame.*;

import dev.nve.host.generated.NveAbi;
import dev.nve.host.generated.NveRuntimeAbi;
import java.io.ByteArrayOutputStream;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * Builds a packed {@code NveRuntimeSceneView} frame ({@link dev.nve.host.generated.NveJniFrame}
 * layout, direct little-endian buffers) without a native runtime: JVM tests, previews, and
 * renderer suites feed it to {@link RuntimeSceneReader#attach(long[], ByteBuffer[])} exactly like
 * a JNI-acquired frame. Sparse tables publish the same paged row index the runtime does.
 *
 * <p>Node index {@code i} has {@code NodeId = generation << 24 | i} (generation 1 unless set);
 * index 1 is the root. {@link #header()} / {@link #slabs()} build fresh arrays each call, so a test
 * may corrupt them before attaching.
 */
public final class PackedSceneBuilder {
    private static final int ROW = NveRuntimeAbi.SIZEOF_NODE_ROW;
    private static final int PAGE_BITS = NveAbi.ROW_INDEX_PAGE_BITS;
    private static final int PAGE_LEN = NveAbi.ROW_INDEX_PAGE_LEN;

    /** One published word table under construction. */
    public final class Table {
        final Key64 key;
        final int slot;
        final int words;
        final boolean dense;
        final Map<Integer, int[]> rows = new LinkedHashMap<>();
        int[] dirtyRows = new int[0];

        Table(Key64 key, int slot, int words, boolean dense) {
            this.key = key;
            this.slot = slot;
            this.words = words;
            this.dense = dense;
        }

        /** Row words of node {@code index} (created zeroed; sets the node's table bit). */
        public int[] row(int index) {
            return rows.computeIfAbsent(index, i -> new int[words]);
        }

        /** Writes raw {@code bits} into field {@code f} of node {@code index}. */
        public Table set(int index, FieldSpec f, int bits) {
            final int[] row = row(index);
            final int mask = f.bits >= 32 ? -1 : (1 << f.bits) - 1;
            row[f.word] = (row[f.word] & ~(mask << f.shift)) | ((bits & mask) << f.shift);
            return this;
        }

        public Table set(int index, FieldSpec f, float value) {
            return set(index, f, Float.floatToRawIntBits(value));
        }

        public Table set(int index, FieldSpec f, boolean value) {
            return set(index, f, value ? 1 : 0);
        }

        /** Appends {@code value} to the text slab and points {@code STR} field {@code f} at it. */
        public Table set(int index, FieldSpec f, String value) {
            if (f.kind != FieldKind.STR) throw new IllegalArgumentException(f.kind + " is not STR");
            final byte[] bytes = value.getBytes(StandardCharsets.UTF_8);
            final int[] row = row(index);
            row[f.word] = text.size();
            row[f.word + 1] = bytes.length;
            text.write(bytes, 0, bytes.length);
            return this;
        }

        /** Published {@code dirty_rows} (row indices) of this frame. */
        public Table dirtyRows(int... rows) {
            dirtyRows = rows.clone();
            return this;
        }
    }

    private final int capacity;
    private final long frameId;
    private final int epoch;
    private final int[] generation;
    private final int[] parent;
    private final int[] primary;
    private final int[] effects;
    private final boolean[] live;
    private final float[] rects;
    private final List<List<Integer>> children = new ArrayList<>();
    private final List<Table> tables = new ArrayList<>();
    private final ByteArrayOutputStream text = new ByteArrayOutputStream();
    private int[] dirty;
    private int[] moved = new int[0];
    private int[] removed = new int[0];
    private int[] dirtyResources = new int[0];
    private final List<float[]> scroll = new ArrayList<>();

    public PackedSceneBuilder(int capacity, long frameId, int sceneEpoch) {
        this.capacity = capacity;
        this.frameId = frameId;
        this.epoch = sceneEpoch;
        generation = new int[capacity];
        parent = new int[capacity];
        primary = new int[capacity];
        effects = new int[capacity];
        live = new boolean[capacity];
        rects = new float[capacity * 4];
        java.util.Arrays.fill(generation, 1);
        java.util.Arrays.fill(primary, NveAbi.PRIMARY_SLOT_NONE);
        for (int i = 0; i < capacity; i++) children.add(new ArrayList<>());
        if (capacity > 1) live[1] = true;
    }

    public int nodeId(int index) {
        return generation[index] << RuntimeSceneReader.INDEX_BITS | index;
    }

    /** Makes {@code index} a live child of {@code parentIndex} (appended) with rect {@code (x, y, w, h)}. */
    public PackedSceneBuilder node(int index, int parentIndex, float x, float y, float w, float h) {
        live[index] = true;
        parent[index] = parentIndex;
        children.get(parentIndex).add(index);
        return rect(index, x, y, w, h);
    }

    public PackedSceneBuilder rect(int index, float x, float y, float w, float h) {
        rects[index * 4] = x;
        rects[index * 4 + 1] = y;
        rects[index * 4 + 2] = w;
        rects[index * 4 + 3] = h;
        return this;
    }

    public PackedSceneBuilder generation(int index, int gen) {
        generation[index] = gen;
        return this;
    }

    public PackedSceneBuilder primary(int index, int slot) {
        primary[index] = slot;
        return this;
    }

    public PackedSceneBuilder effects(int index, int bits) {
        effects[index] = bits;
        return this;
    }

    /** The table with {@code key} in {@code slot} ({@code words} per row), created on first use. */
    public Table table(Key64 key, int slot, int words, boolean dense) {
        for (Table t : tables) {
            if (t.key.equals(key)) return t;
        }
        final Table t = new Table(key, slot, words, dense);
        tables.add(t);
        return t;
    }

    /** Appends raw UTF-8 of {@code value} to the text slab; returns its offset. */
    public int appendText(String value) {
        final int at = text.size();
        final byte[] bytes = value.getBytes(StandardCharsets.UTF_8);
        text.write(bytes, 0, bytes.length);
        return at;
    }

    /** Content-dirty node indexes (default: every live non-root node). */
    public PackedSceneBuilder dirty(int... indexes) {
        dirty = indexes.clone();
        return this;
    }

    public PackedSceneBuilder moved(int... indexes) {
        moved = indexes.clone();
        return this;
    }

    public PackedSceneBuilder removed(int... nodeIds) {
        removed = nodeIds.clone();
        return this;
    }

    public PackedSceneBuilder dirtyResources(int... ids) {
        dirtyResources = ids.clone();
        return this;
    }

    /** Published visual shift of scroll container {@code nodeId} (call in ascending id order). */
    public PackedSceneBuilder scroll(int nodeId, float x, float y) {
        scroll.add(new float[] {Float.intBitsToFloat(nodeId), x, y});
        return this;
    }

    public long[] header() {
        final long[] h = new long[HEADER_LEN];
        h[H_SCHEMA] = RuntimeSceneReader.SCHEMA.value();
        h[H_FRAME_ID] = frameId;
        h[H_NODE_CAPACITY] = capacity;
        h[H_DIRTY_COUNT] = dirtyIndexes().length;
        h[H_MOVED_COUNT] = moved.length;
        h[H_REMOVED_COUNT] = removed.length;
        h[H_TEXT_LEN] = text.size();
        h[H_CHILDREN_LEN] = childSlab().length;
        h[H_TABLE_COUNT] = tables.size();
        h[H_SCROLL_COUNT] = scroll.size();
        h[H_SCENE_EPOCH] = epoch;
        h[H_DIRTY_RESOURCE_COUNT] = dirtyResources.length;
        for (int t = 0; t < tables.size(); t++) {
            final Table tb = tables.get(t);
            final int at = H_TABLES + t * TABLE_HEADER_STRIDE;
            h[at + TH_KEY] = tb.key.value();
            h[at + TH_SLOT] = tb.slot;
            h[at + TH_WORDS] = tb.words;
            h[at + TH_ROW_COUNT] = tb.dense ? capacity : tb.rows.size();
            h[at + TH_DIRTY_COUNT] = tb.dirtyRows.length;
            if (!tb.dense) {
                final int[][] index = rowIndex(tb);
                h[at + TH_ROW_INDEX_DIR_LEN] = index[0].length;
                h[at + TH_ROW_INDEX_PAGE_COUNT] = index[1].length / PAGE_LEN;
            }
        }
        return h;
    }

    public ByteBuffer[] slabs() {
        final ByteBuffer[] s = new ByteBuffer[SLABS_LEN];
        final ByteBuffer rows = buf(capacity * ROW);
        final int[] childSlab = childSlab();
        int childOff = 0;
        for (int i = 1; i < capacity; i++) {
            if (!live[i]) continue;
            final int at = i * ROW;
            for (Table t : tables) {
                if (t.rows.containsKey(i)) {
                    final int w = at + NveRuntimeAbi.NODE_ROW_OFFSET_TABLE_SET + (t.slot >>> 5) * 4;
                    rows.putInt(w, rows.getInt(w) | 1 << (t.slot & 31));
                }
            }
            rows.putInt(at + NveRuntimeAbi.NODE_ROW_OFFSET_PARENT, i == 1 || parent[i] == 0 ? 0 : nodeId(parent[i]));
            rows.putInt(at + NveRuntimeAbi.NODE_ROW_OFFSET_CHILD_OFF, childOff);
            rows.putInt(at + NveRuntimeAbi.NODE_ROW_OFFSET_CHILD_LEN, children.get(i).size());
            childOff += children.get(i).size();
            rows.put(at + NveRuntimeAbi.NODE_ROW_OFFSET_GENERATION, (byte) generation[i]);
            rows.put(at + NveRuntimeAbi.NODE_ROW_OFFSET_PRIMARY_SLOT, (byte) primary[i]);
            rows.put(at + NveRuntimeAbi.NODE_ROW_OFFSET_EFFECTS, (byte) effects[i]);
            rows.put(at + NveRuntimeAbi.NODE_ROW_OFFSET_FLAGS, (byte) NveRuntimeAbi.NODE_ROW_FLAG_LIVE);
        }
        final ByteBuffer rectBuf = buf(capacity * NveRuntimeAbi.SIZEOF_LAYOUT_RECT);
        for (int k = 0; k < rects.length; k++) rectBuf.putFloat(k * 4, rects[k]);
        final byte[] textBytes = text.toByteArray();
        final ByteBuffer textBuf = buf(textBytes.length);
        for (int k = 0; k < textBytes.length; k++) textBuf.put(k, textBytes[k]);
        final ByteBuffer scrollBuf = buf(scroll.size() * NveRuntimeAbi.SIZEOF_SCROLL_OFFSET);
        for (int k = 0; k < scroll.size(); k++) {
            final float[] e = scroll.get(k);
            final int at = k * NveRuntimeAbi.SIZEOF_SCROLL_OFFSET;
            scrollBuf.putInt(at + NveRuntimeAbi.SCROLL_OFFSET_OFFSET_NODE_ID, Float.floatToRawIntBits(e[0]));
            scrollBuf.putFloat(at + NveRuntimeAbi.SCROLL_OFFSET_OFFSET_X, e[1]);
            scrollBuf.putFloat(at + NveRuntimeAbi.SCROLL_OFFSET_OFFSET_Y, e[2]);
        }
        s[S_ROWS] = rows;
        s[S_RECTS] = rectBuf;
        s[S_TEXT] = textBuf;
        s[S_CHILDREN] = ints(childSlab);
        s[S_DIRTY] = ints(dirtyIndexes());
        s[S_MOVED] = ints(moved);
        s[S_REMOVED] = ints(removed);
        s[S_DIRTY_RESOURCES] = ints(dirtyResources);
        s[S_SCROLL_OFFSETS] = scrollBuf;
        for (int t = 0; t < tables.size(); t++) {
            final Table tb = tables.get(t);
            final int at = S_TABLES + t * TABLE_SLAB_STRIDE;
            final int rowCount = tb.dense ? capacity : tb.rows.size();
            final int[] words = new int[rowCount * tb.words];
            final int[] owners = new int[tb.rows.size()];
            int r = 0;
            for (Map.Entry<Integer, int[]> e : tb.rows.entrySet()) {
                final int row = tb.dense ? e.getKey() : r;
                System.arraycopy(e.getValue(), 0, words, row * tb.words, tb.words);
                owners[r++] = nodeId(e.getKey());
            }
            s[at + TS_ROWS] = ints(words);
            s[at + TS_DIRTY_ROWS] = ints(tb.dirtyRows);
            if (!tb.dense) {
                final int[][] index = rowIndex(tb);
                s[at + TS_OWNERS] = ints(owners);
                s[at + TS_ROW_INDEX_DIR] = ints(index[0]);
                s[at + TS_ROW_INDEX_PAGES] = ints(index[1]);
            }
        }
        return s;
    }

    /** Attaches {@code reader} to a freshly built frame. */
    public RuntimeSceneReader attach(RuntimeSceneReader reader) {
        return reader.attach(header(), slabs());
    }

    private int[] dirtyIndexes() {
        if (dirty != null) return dirty;
        final List<Integer> all = new ArrayList<>();
        for (int i = 2; i < capacity; i++) {
            if (live[i]) all.add(i);
        }
        return all.stream().mapToInt(Integer::intValue).toArray();
    }

    private int[] childSlab() {
        final List<Integer> out = new ArrayList<>();
        for (int i = 1; i < capacity; i++) {
            if (!live[i]) continue;
            for (int c : children.get(i)) out.add(nodeId(c));
        }
        return out.stream().mapToInt(Integer::intValue).toArray();
    }

    /** {@code {dir, pages}}: the runtime's paged node-index → {@code row + 1} index of a sparse table. */
    private int[][] rowIndex(Table tb) {
        int maxIndex = -1;
        for (int i : tb.rows.keySet()) maxIndex = Math.max(maxIndex, i);
        final int dirLen = maxIndex < 0 ? 0 : (maxIndex >>> PAGE_BITS) + 1;
        final int[] dir = new int[dirLen];
        java.util.Arrays.fill(dir, NveAbi.ROW_INDEX_NO_PAGE);
        final List<int[]> pages = new ArrayList<>();
        int r = 0;
        for (int i : tb.rows.keySet()) {
            final int d = i >>> PAGE_BITS;
            if (dir[d] == NveAbi.ROW_INDEX_NO_PAGE) {
                dir[d] = pages.size();
                pages.add(new int[PAGE_LEN]);
            }
            pages.get(dir[d])[i & (PAGE_LEN - 1)] = ++r;
        }
        final int[] flat = new int[pages.size() * PAGE_LEN];
        for (int p = 0; p < pages.size(); p++) System.arraycopy(pages.get(p), 0, flat, p * PAGE_LEN, PAGE_LEN);
        return new int[][] {dir, flat};
    }

    private static ByteBuffer buf(int n) {
        return ByteBuffer.allocateDirect(n).order(ByteOrder.LITTLE_ENDIAN);
    }

    private static ByteBuffer ints(int[] v) {
        final ByteBuffer b = buf(v.length * 4);
        for (int i = 0; i < v.length; i++) b.putInt(i * 4, v[i]);
        return b;
    }
}
