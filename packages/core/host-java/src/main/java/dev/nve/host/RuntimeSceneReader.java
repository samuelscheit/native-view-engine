package dev.nve.host;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;

/**
 * Reusable validated zero-copy reader over one acquired {@code NveRuntimeSceneView} frame whose
 * slabs arrive as direct buffers (JNI {@code NewDirectByteBuffer} over the runtime's own memory).
 *
 * <p>Slabs: {@code rows} ({@code node_capacity} 32-byte {@code NveNodeRow}s), {@code rects}
 * ({@code node_capacity} 16-byte {@code NveLayoutRect}s), the UTF-8 {@code text_slab}, the
 * {@code u32} {@code children_slab}, per-frame {@code dirty_indices} / {@code removed_ids}, and
 * the published word tables. {@link #attach} validates the schema hash, every slab against its
 * count, every dirty index, and every table (slot range, unique slots) once; later reads are O(1)
 * bounds checks. Steady-state frames allocate nothing: table readers and sparse-row indexes are
 * pooled and grow only.
 *
 * <p>Node ids are raw {@code NodeId}s ({@code generation << 24 | index}); node indexes address
 * {@code rows} / {@code rects} / dense table rows. UTF-8 validity of the text slab is checked by
 * the native {@code nve::host::RuntimeSceneReader::open} before the frame is handed out; Java
 * decoding is memory-safe regardless.
 */
public final class RuntimeSceneReader {
    /**
     * {@code NVE_RUNTIME_SCHEMA_HASH}: fingerprint of the runtime scene + vtable layout read here.
     * {@code hosts/android-shell} locks it against the generated C header with a {@code static_assert}.
     */
    public static final Key64 SCHEMA = Key64.of(0xDB2CE618E24C0761L);
    public static final int NODE_ROW_SIZE = 32;
    public static final int LAYOUT_RECT_SIZE = 16;
    /** {@code NveNodeRow::flags} bit of an allocated live slot. */
    public static final int FLAG_LIVE = 1;
    /** {@code NveNodeRow::primary_slot} of a plain container. */
    public static final int PRIMARY_SLOT_NONE = 0xFF;
    /** Table slots per process (Rust {@code MAX_TABLE_SLOTS}). */
    public static final int MAX_TABLES = 128;
    public static final int INDEX_BITS = 24;
    public static final int INDEX_MASK = (1 << INDEX_BITS) - 1;

    // `NveNodeRow` byte offsets.
    static final int ROW_TABLE_SET = 0;
    static final int ROW_PARENT = 16;
    static final int ROW_CHILD_OFF = 20;
    static final int ROW_CHILD_LEN = 24;
    static final int ROW_GENERATION = 28;
    static final int ROW_PRIMARY_SLOT = 29;
    static final int ROW_EFFECTS = 30;
    static final int ROW_FLAGS = 31;

    // ---- packed JNI frame: `long[] header` + `ByteBuffer[] slabs` (one JNI crossing per frame) ----

    public static final int H_SCHEMA = 0;
    public static final int H_FRAME_ID = 1;
    public static final int H_NODE_CAPACITY = 2;
    public static final int H_DIRTY_COUNT = 3;
    public static final int H_REMOVED_COUNT = 4;
    public static final int H_TEXT_LEN = 5;
    public static final int H_CHILDREN_LEN = 6;
    public static final int H_TABLE_COUNT = 7;
    public static final int H_TABLES = 8;
    /** Per table {@code t} at {@code H_TABLES + t * H_TABLE_STRIDE}: key, slot, words, rowCount, dirtyCount. */
    public static final int H_TABLE_STRIDE = 5;
    public static final int HEADER_LEN = H_TABLES + MAX_TABLES * H_TABLE_STRIDE;

    public static final int S_ROWS = 0;
    public static final int S_RECTS = 1;
    public static final int S_TEXT = 2;
    public static final int S_CHILDREN = 3;
    public static final int S_DIRTY = 4;
    public static final int S_REMOVED = 5;
    public static final int S_TABLES = 6;
    /** Per table {@code t} at {@code S_TABLES + t * S_TABLE_STRIDE}: rows, owners (null = dense), dirtyRows. */
    public static final int S_TABLE_STRIDE = 3;
    public static final int SLABS_LEN = S_TABLES + MAX_TABLES * S_TABLE_STRIDE;

    private long frameId;
    private int capacity;
    private ByteBuffer rows;
    private int rowsBase;
    private ByteBuffer rects;
    private int rectsBase;
    private ByteBuffer text;
    private int textBase;
    private int textLen;
    private ByteBuffer children;
    private int childrenBase;
    private int childrenLen;
    private ByteBuffer dirty;
    private int dirtyBase;
    private int dirtyCount;
    private ByteBuffer removed;
    private int removedBase;
    private int removedCount;
    private final TableReader[] pool = new TableReader[MAX_TABLES];
    private TableReader[] tables = new TableReader[0];
    private int tableCount;
    /** Table index by slot, {@code -1} when the slot is not published this frame. */
    private final int[] bySlot = new int[MAX_TABLES];
    /** Lazily built node-index → row maps of sparse tables, valid while {@code sparseFrame[slot] == attachEpoch}. */
    private final int[][] sparseRows = new int[MAX_TABLES][];
    private final long[] sparseEpoch = new long[MAX_TABLES];
    private long attachEpoch;
    private byte[] textScratch = new byte[256];

    /**
     * Rebinds this reader to a packed JNI frame (see {@code H_*} / {@code S_*}).
     *
     * @throws SceneException when the header, a slab, a dirty index, or a table is invalid
     */
    public RuntimeSceneReader attach(long[] header, ByteBuffer[] slabs) {
        if (header == null || slabs == null || header.length < H_TABLES || slabs.length < S_TABLES) {
            throw new SceneException(SceneException.Kind.MISSING_SLAB, "packed frame arrays too short");
        }
        final int count = count(header[H_TABLE_COUNT], "tableCount");
        if (count > MAX_TABLES || header.length < H_TABLES + count * H_TABLE_STRIDE
                || slabs.length < S_TABLES + count * S_TABLE_STRIDE) {
            throw new SceneException(SceneException.Kind.INVALID_TABLE, "tableCount " + count);
        }
        for (int t = 0; t < count; t++) {
            final int h = H_TABLES + t * H_TABLE_STRIDE;
            final int s = S_TABLES + t * S_TABLE_STRIDE;
            if (pool[t] == null) pool[t] = new TableReader();
            final Key64 key = pool[t].key().value() == header[h] ? pool[t].key() : Key64.of(header[h]);
            try {
                pool[t].attach(key, count(header[h + 1], "slot"), count(header[h + 2], "words"),
                        count(header[h + 3], "rowCount"), slabs[s], slabs[s + 1], slabs[s + 2],
                        count(header[h + 4], "dirtyCount"));
            } catch (IllegalArgumentException e) {
                throw new SceneException(SceneException.Kind.INVALID_TABLE, "table " + t + ": " + e.getMessage());
            }
        }
        return attach(header[H_SCHEMA], header[H_FRAME_ID], count(header[H_NODE_CAPACITY], "nodeCapacity"),
                slabs[S_ROWS], slabs[S_RECTS],
                slabs[S_TEXT], count(header[H_TEXT_LEN], "textLen"),
                slabs[S_CHILDREN], count(header[H_CHILDREN_LEN], "childrenLen"),
                slabs[S_DIRTY], count(header[H_DIRTY_COUNT], "dirtyCount"),
                slabs[S_REMOVED], count(header[H_REMOVED_COUNT], "removedCount"),
                pool, count);
    }

    private static int count(long v, String what) {
        if (v < 0 || v > Integer.MAX_VALUE) throw new SceneException(SceneException.Kind.MISSING_SLAB, what + " " + v);
        return (int) v;
    }

    /**
     * Rebinds this reader. {@code tables[0 .. tableCount)} must already be attached.
     *
     * @throws SceneException when the schema, a slab, a dirty index, or a table is invalid
     */
    public RuntimeSceneReader attach(
            long schemaHash, long frameId, int nodeCapacity,
            ByteBuffer rows, ByteBuffer rects,
            ByteBuffer textSlab, int textSlabLen,
            ByteBuffer childrenSlab, int childrenSlabLen,
            ByteBuffer dirtyIndices, int dirtyCount,
            ByteBuffer removedIds, int removedCount,
            TableReader[] tables, int tableCount) {
        if (schemaHash != SCHEMA.value()) {
            throw new SceneException(SceneException.Kind.SCHEMA_MISMATCH,
                    "expected " + SCHEMA + ", got " + Key64.of(schemaHash));
        }
        if (nodeCapacity > INDEX_MASK + 1) {
            throw new SceneException(SceneException.Kind.MISSING_SLAB, "nodeCapacity " + nodeCapacity);
        }
        requireBytes(rows, (long) nodeCapacity * NODE_ROW_SIZE, "rows");
        requireBytes(rects, (long) nodeCapacity * LAYOUT_RECT_SIZE, "rects");
        requireBytes(textSlab, textSlabLen, "textSlab");
        requireBytes(childrenSlab, (long) childrenSlabLen * 4, "childrenSlab");
        requireBytes(dirtyIndices, (long) dirtyCount * 4, "dirtyIndices");
        requireBytes(removedIds, (long) removedCount * 4, "removedIds");
        if (tableCount < 0 || tableCount > MAX_TABLES || (tableCount > 0 && (tables == null || tables.length < tableCount))) {
            throw new SceneException(SceneException.Kind.INVALID_TABLE, "tableCount " + tableCount);
        }
        java.util.Arrays.fill(bySlot, -1);
        for (int i = 0; i < tableCount; i++) {
            final TableReader t = tables[i];
            if (t == null || t.words() < 1) {
                throw new SceneException(SceneException.Kind.INVALID_TABLE, "table " + i + " is null or not attached");
            }
            if (t.slot() < 0 || t.slot() >= MAX_TABLES || bySlot[t.slot()] >= 0) {
                throw new SceneException(SceneException.Kind.INVALID_TABLE, "table " + i + " slot " + t.slot());
            }
            if (!t.isSparse() && t.rowCount() > nodeCapacity) {
                throw new SceneException(SceneException.Kind.INVALID_TABLE,
                        "dense table " + i + " holds " + t.rowCount() + " rows for capacity " + nodeCapacity);
            }
            bySlot[t.slot()] = i;
        }
        final ByteBuffer d = le(dirtyIndices);
        final int dBase = base(dirtyIndices);
        for (int i = 0; i < dirtyCount; i++) {
            final int idx = d.getInt(dBase + i * 4);
            if (idx <= 0 || idx >= nodeCapacity) {
                for (int k = 0; k < tableCount; k++) bySlot[tables[k].slot()] = -1;
                throw new SceneException(SceneException.Kind.INVALID_NODE, "dirty index " + Integer.toUnsignedString(idx));
            }
        }
        this.frameId = frameId;
        this.capacity = nodeCapacity;
        this.rows = le(rows);
        this.rowsBase = base(rows);
        this.rects = le(rects);
        this.rectsBase = base(rects);
        this.text = le(textSlab);
        this.textBase = base(textSlab);
        this.textLen = textSlabLen;
        this.children = le(childrenSlab);
        this.childrenBase = base(childrenSlab);
        this.childrenLen = childrenSlabLen;
        this.dirty = d;
        this.dirtyBase = dBase;
        this.dirtyCount = dirtyCount;
        this.removed = le(removedIds);
        this.removedBase = base(removedIds);
        this.removedCount = removedCount;
        this.tables = tables == null ? new TableReader[0] : tables;
        this.tableCount = tableCount;
        this.attachEpoch++;
        return this;
    }

    private static void requireBytes(ByteBuffer buf, long count, String what) {
        if (count < 0) throw new SceneException(SceneException.Kind.MISSING_SLAB, what + " count " + count);
        if (count == 0) return;
        if (buf == null || buf.remaining() < count) {
            throw new SceneException(SceneException.Kind.MISSING_SLAB,
                    what + " holds " + (buf == null ? "null" : buf.remaining()) + " bytes, need " + count);
        }
    }

    private static ByteBuffer le(ByteBuffer b) {
        return b == null ? null : b.order(ByteOrder.LITTLE_ENDIAN);
    }

    private static int base(ByteBuffer b) {
        return b == null ? 0 : b.position();
    }

    public long frameId() {
        return frameId;
    }

    /** Slot capacity of {@code rows} / {@code rects}; index 0 is the reserved null slot. */
    public int nodeCapacity() {
        return capacity;
    }

    public int dirtyCount() {
        return dirtyCount;
    }

    public int removedCount() {
        return removedCount;
    }

    public int textSlabLen() {
        return textLen;
    }

    public int childrenSlabLen() {
        return childrenLen;
    }

    public int tableCount() {
        return tableCount;
    }

    // ---- node ids ------------------------------------------------------------------------------

    public static int indexOf(int nodeId) {
        return nodeId & INDEX_MASK;
    }

    public static int generationOf(int nodeId) {
        return nodeId >>> INDEX_BITS;
    }

    /** Raw {@code NodeId} of live slot {@code index}, or {@code 0} when the slot is free. */
    public int nodeId(int index) {
        return isLive(index) ? (generation(index) << INDEX_BITS) | index : 0;
    }

    /** Index of live {@code nodeId} whose generation matches, or {@code -1} (stale / free / out of range). */
    public int resolve(int nodeId) {
        final int index = indexOf(nodeId);
        return index > 0 && index < capacity && isLive(index) && generation(index) == generationOf(nodeId) ? index : -1;
    }

    // ---- node rows -----------------------------------------------------------------------------

    public boolean isLive(int index) {
        return index > 0 && index < capacity && (rowU8(index, ROW_FLAGS) & FLAG_LIVE) != 0;
    }

    public int flags(int index) {
        return rowU8(index, ROW_FLAGS);
    }

    public int generation(int index) {
        return rowU8(index, ROW_GENERATION);
    }

    public int primarySlot(int index) {
        return rowU8(index, ROW_PRIMARY_SLOT);
    }

    /** Union of {@code effect::*} bits dirtied on this node during the latest frame. */
    public int effects(int index) {
        return rowU8(index, ROW_EFFECTS);
    }

    /** Raw parent {@code NodeId} ({@code 0} = root / detached). */
    public int parent(int index) {
        return rows.getInt(rowAt(index, ROW_PARENT));
    }

    public int childCount(int index) {
        return checkedChildSpan(index);
    }

    /** Raw {@code NodeId} of child {@code k} of node {@code index}. */
    public int childId(int index, int k) {
        final int len = checkedChildSpan(index);
        if (k < 0 || k >= len) throw new IndexOutOfBoundsException("child " + k + " of " + len);
        return children.getInt(childrenBase + (rows.getInt(rowAt(index, ROW_CHILD_OFF)) + k) * 4);
    }

    private int checkedChildSpan(int index) {
        final int off = rows.getInt(rowAt(index, ROW_CHILD_OFF));
        final int len = rows.getInt(rowAt(index, ROW_CHILD_LEN));
        if (off < 0 || len < 0 || len > childrenLen - off) {
            throw new IndexOutOfBoundsException("children [" + Integer.toUnsignedString(off) + ", +"
                    + Integer.toUnsignedString(len) + ") of " + childrenLen);
        }
        return len;
    }

    public boolean hasTable(int index, int slot) {
        if (slot < 0 || slot >= MAX_TABLES) return false;
        return (rows.getInt(rowAt(index, ROW_TABLE_SET + (slot >>> 5) * 4)) & (1 << (slot & 31))) != 0;
    }

    private int rowU8(int index, int offset) {
        return rows.get(rowAt(index, offset)) & 0xFF;
    }

    private int rowAt(int index, int offset) {
        if (index < 0 || index >= capacity) throw new IndexOutOfBoundsException("node " + index + " of " + capacity);
        return rowsBase + index * NODE_ROW_SIZE + offset;
    }

    // ---- layout rects --------------------------------------------------------------------------

    public float x(int index) {
        return rects.getFloat(rectAt(index, 0));
    }

    public float y(int index) {
        return rects.getFloat(rectAt(index, 4));
    }

    public float width(int index) {
        return rects.getFloat(rectAt(index, 8));
    }

    public float height(int index) {
        return rects.getFloat(rectAt(index, 12));
    }

    private int rectAt(int index, int offset) {
        if (index < 0 || index >= capacity) throw new IndexOutOfBoundsException("rect " + index + " of " + capacity);
        return rectsBase + index * LAYOUT_RECT_SIZE + offset;
    }

    // ---- per-frame deltas ----------------------------------------------------------------------

    /** Node index modified this frame (validated live-range {@code 1 .. nodeCapacity)} at attach). */
    public int dirtyIndex(int i) {
        if (i < 0 || i >= dirtyCount) throw new IndexOutOfBoundsException("dirty " + i + " of " + dirtyCount);
        return dirty.getInt(dirtyBase + i * 4);
    }

    /** Raw {@code NodeId} removed this frame. */
    public int removedId(int i) {
        if (i < 0 || i >= removedCount) throw new IndexOutOfBoundsException("removed " + i + " of " + removedCount);
        return removed.getInt(removedBase + i * 4);
    }

    // ---- text ----------------------------------------------------------------------------------

    /**
     * {@code textSlab[offset .. offset + len)} as a String; throws {@link IndexOutOfBoundsException}
     * when out of range or not on UTF-8 boundaries.
     */
    public String text(int offset, int len) {
        if (offset < 0 || len < 0 || offset > textLen || len > textLen - offset || !isBoundary(offset)
                || !isBoundary(offset + len)) {
            throw new IndexOutOfBoundsException("text [" + offset + ", +" + len + ") of " + textLen);
        }
        if (len == 0) return "";
        if (text.hasArray()) {
            return new String(text.array(), text.arrayOffset() + textBase + offset, len, StandardCharsets.UTF_8);
        }
        if (textScratch.length < len) textScratch = new byte[Math.max(len, textScratch.length * 2)];
        for (int i = 0; i < len; i++) textScratch[i] = text.get(textBase + offset + i);
        return new String(textScratch, 0, len, StandardCharsets.UTF_8);
    }

    /** String of {@code STR} field {@code f} in row {@code row} of {@code table}. */
    public String text(TableReader table, int row, FieldSpec f) {
        return text(table.strOffset(row, f), table.strLen(row, f));
    }

    private boolean isBoundary(int at) {
        return at == textLen || (text.get(textBase + at) & 0xC0) != 0x80;
    }

    // ---- tables --------------------------------------------------------------------------------

    public TableReader table(int i) {
        if (i < 0 || i >= tableCount) throw new IndexOutOfBoundsException("table " + i + " of " + tableCount);
        return tables[i];
    }

    /** Published table with {@code key}, or {@code null} (O(tables), allocation-free). */
    public TableReader findTable(Key64 key) {
        for (int i = 0; i < tableCount; i++) {
            if (tables[i].key().equals(key)) return tables[i];
        }
        return null;
    }

    /** Published table in {@code slot}, or {@code null}. */
    public TableReader tableAtSlot(int slot) {
        if (slot < 0 || slot >= MAX_TABLES) return null;
        final int i = bySlot[slot];
        return i < 0 ? null : tables[i];
    }

    /** Table realizing node {@code index} ({@code primary_slot}), or {@code null} for plain containers. */
    public TableReader primaryTable(int index) {
        final int slot = primarySlot(index);
        return slot == PRIMARY_SLOT_NONE ? null : tableAtSlot(slot);
    }

    /**
     * Row of live node {@code index} in {@code table}, or {@code -1} when the node has none. Dense:
     * O(1). Sparse: O(rows) once per table per frame (pooled index), then O(1).
     */
    public int rowOf(TableReader table, int index) {
        if (table == null || !isLive(index) || !hasTable(index, table.slot())) return -1;
        if (!table.isSparse()) return index < table.rowCount() ? index : -1;
        final int slot = table.slot();
        int[] map = sparseRows[slot];
        if (sparseEpoch[slot] != attachEpoch) {
            if (map == null || map.length < capacity) {
                map = new int[Math.max(capacity, map == null ? 0 : map.length * 2)];
                sparseRows[slot] = map;
            }
            java.util.Arrays.fill(map, 0, capacity, -1);
            for (int r = 0; r < table.rowCount(); r++) {
                final int owner = table.owner(r);
                final int i = indexOf(owner);
                if (i > 0 && i < capacity && isLive(i) && generation(i) == generationOf(owner)) map[i] = r;
            }
            sparseEpoch[slot] = attachEpoch;
        }
        return map[index];
    }
}
