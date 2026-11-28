package dev.nve.host;

import static dev.nve.host.generated.NveJniFrame.*;

import dev.nve.host.generated.NveAbi;
import dev.nve.host.generated.NveRuntimeAbi;
import java.nio.Buffer;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;

/**
 * Reusable validated zero-copy reader over one acquired {@code NveSceneView} frame, packed
 * by the JNI layer into the generated {@link dev.nve.host.generated.NveJniFrame} layout: a
 * {@code long[]} header and direct-buffer slabs over the runtime's own memory.
 *
 * <p>Slabs: {@code rows} ({@code node_capacity} 32-byte {@code NveNodeRow}s), {@code rects}
 * ({@code node_capacity} 16-byte {@code NveLayoutRect}s), the UTF-8 {@code text_slab}, the
 * {@code u32} {@code children_slab}, per-frame {@code dirty_indices} (content changed) /
 * {@code moved_indices} (geometry only) / {@code removed_ids} / {@code dirty_resources}, the
 * sorted {@code scroll_offsets}, and the published word tables. {@link #attach} validates the
 * schema hash, every slab against its count, every delta index, the sort order of scroll offsets
 * and dirty resources, and every table (slot range, unique slots) once; later reads are O(1)
 * bounds checks. Table readers are pooled: attaching and reading a frame allocates nothing except
 * decoded strings (and one bulk-read text view per frame on the first {@link #text}).
 *
 * <p>Node ids are raw {@code NodeId}s ({@code generation << 24 | index}); node indexes address
 * {@code rows} / {@code rects} / dense table rows. UTF-8 validity of the text slab is checked by
 * the native {@code nve::host::SceneReader::open} before the frame is handed out; Java
 * decoding is memory-safe regardless.
 */
public final class SceneReader {
    /** {@code NVE_RUNTIME_SCHEMA_HASH}: fingerprint of the runtime scene + vtable layout read here. */
    public static final Key64 SCHEMA = Key64.of(NveRuntimeAbi.RUNTIME_SCHEMA_HASH);
    public static final int INDEX_BITS = 24;
    public static final int INDEX_MASK = (1 << INDEX_BITS) - 1;

    /** {@link #relationTo}: the same frame again (nothing new to apply). */
    public static final int FRAME_SAME = 0;
    /** {@link #relationTo}: the frame right after the previous one in its epoch (apply the deltas). */
    public static final int FRAME_NEXT = 1;
    /** {@link #relationTo}: a new epoch, a skipped frame, or no previous frame (reconcile everything). */
    public static final int FRAME_RESET = 2;

    private static final int ROW_SIZE = NveRuntimeAbi.SIZEOF_NODE_ROW;
    private static final int RECT_SIZE = NveRuntimeAbi.SIZEOF_LAYOUT_RECT;
    private static final int SCROLL_SIZE = NveRuntimeAbi.SIZEOF_SCROLL_OFFSET;

    private long frameId;
    private int sceneEpoch;
    private int capacity;
    private ByteBuffer rows;
    private int rowsBase;
    private ByteBuffer rects;
    private int rectsBase;
    private ByteBuffer text;
    private ByteBuffer textView;
    private int textBase;
    private int textLen;
    private ByteBuffer children;
    private int childrenBase;
    private int childrenLen;
    private final IdSlab dirty = new IdSlab();
    private final IdSlab moved = new IdSlab();
    private final IdSlab removed = new IdSlab();
    private final IdSlab dirtyResources = new IdSlab();
    private ByteBuffer scroll;
    private int scrollBase;
    private int scrollCount;
    private final TableReader[] pool = new TableReader[MAX_TABLES];
    /** Pool readers attached at least once since the last {@link #detach} (all detached there). */
    private int poolUsed;
    private int tableCount;
    /** Table index by slot, {@code -1} when the slot is not published this frame. */
    private final int[] bySlot = new int[MAX_TABLES];
    private byte[] textScratch = new byte[256];

    /** One validated {@code u32} slab with its count. */
    private static final class IdSlab {
        ByteBuffer buf;
        int base;
        int count;

        void bind(ByteBuffer b, int n) {
            buf = b == null ? null : b.order(ByteOrder.LITTLE_ENDIAN);
            base = b == null ? 0 : b.position();
            count = n;
        }

        int get(int i, String what) {
            if (i < 0 || i >= count) throw new IndexOutOfBoundsException(what + " " + i + " of " + count);
            return buf.getInt(base + i * 4);
        }
    }

    public SceneReader() {
        java.util.Arrays.fill(bySlot, -1);
    }

    /**
     * Rebinds this reader to a packed JNI frame ({@code NveJniFrame}). A rejected frame leaves the
     * reader {@link #detach detached}, never half old / half new.
     *
     * @throws SceneException when the header, a slab, a delta index, or a table is invalid
     */
    public SceneReader attach(long[] header, ByteBuffer[] slabs) {
        try {
            return attachPacked(header, slabs);
        } catch (SceneException e) {
            detach();
            throw e;
        }
    }

    private SceneReader attachPacked(long[] h, ByteBuffer[] s) {
        if (h == null || s == null || h.length < HEADER_LEN || s.length < SLABS_LEN) {
            throw new SceneException(SceneException.Kind.MISSING_SLAB, "packed frame arrays shorter than NveJniFrame");
        }
        if (h[H_SCHEMA] != SCHEMA.value()) {
            throw new SceneException(SceneException.Kind.SCHEMA_MISMATCH,
                    "expected " + SCHEMA + ", got " + Key64.of(h[H_SCHEMA]));
        }
        final int cap = count(h[H_NODE_CAPACITY], "nodeCapacity");
        if (cap > INDEX_MASK + 1) throw new SceneException(SceneException.Kind.MISSING_SLAB, "nodeCapacity " + cap);
        final int tables = count(h[H_TABLE_COUNT], "tableCount");
        if (tables > MAX_TABLES) throw new SceneException(SceneException.Kind.INVALID_TABLE, "tableCount " + tables);
        final int textN = count(h[H_TEXT_LEN], "textLen");
        final int childrenN = count(h[H_CHILDREN_LEN], "childrenLen");
        final int scrollN = count(h[H_SCROLL_COUNT], "scrollCount");
        requireBytes(s[S_ROWS], (long) cap * ROW_SIZE, "rows");
        requireBytes(s[S_RECTS], (long) cap * RECT_SIZE, "rects");
        requireBytes(s[S_TEXT], textN, "textSlab");
        requireBytes(s[S_CHILDREN], (long) childrenN * 4, "childrenSlab");
        requireBytes(s[S_SCROLL_OFFSETS], (long) scrollN * SCROLL_SIZE, "scrollOffsets");
        final int dirtyN = idSlab(s[S_DIRTY], h[H_DIRTY_COUNT], "dirtyIndices");
        final int movedN = idSlab(s[S_MOVED], h[H_MOVED_COUNT], "movedIndices");
        final int removedN = idSlab(s[S_REMOVED], h[H_REMOVED_COUNT], "removedIds");
        final int dirtyResN = idSlab(s[S_DIRTY_RESOURCES], h[H_DIRTY_RESOURCE_COUNT], "dirtyResources");
        requireNodeIndexes(s[S_DIRTY], dirtyN, cap, "dirty");
        requireNodeIndexes(s[S_MOVED], movedN, cap, "moved");
        requireAscending(s[S_DIRTY_RESOURCES], 0, 4, dirtyResN, "dirty resource ids");
        requireAscending(s[S_SCROLL_OFFSETS], NveRuntimeAbi.SCROLL_OFFSET_OFFSET_NODE_ID, SCROLL_SIZE, scrollN,
                "scroll offset node ids");

        long seenLo = 0L;
        long seenHi = 0L;
        java.util.Arrays.fill(bySlot, -1);
        for (int t = 0; t < tables; t++) {
            final int th = H_TABLES + t * TABLE_HEADER_STRIDE;
            final int ts = S_TABLES + t * TABLE_SLAB_STRIDE;
            if (pool[t] == null) pool[t] = new TableReader();
            poolUsed = Math.max(poolUsed, t + 1);
            final TableReader r = pool[t];
            final Key64 key = r.key().value() == h[th + TH_KEY] ? r.key() : Key64.of(h[th + TH_KEY]);
            final int slot = count(h[th + TH_SLOT], "slot");
            try {
                r.attach(key, slot, count(h[th + TH_WORDS], "words"), count(h[th + TH_ROW_COUNT], "rowCount"),
                        s[ts + TS_ROWS], s[ts + TS_OWNERS], s[ts + TS_DIRTY_ROWS], count(h[th + TH_DIRTY_COUNT], "dirtyCount"),
                        s[ts + TS_ROW_INDEX_DIR], count(h[th + TH_ROW_INDEX_DIR_LEN], "rowIndexDirLen"),
                        s[ts + TS_ROW_INDEX_PAGES], count(h[th + TH_ROW_INDEX_PAGE_COUNT], "rowIndexPageCount"));
            } catch (IllegalArgumentException e) {
                throw new SceneException(SceneException.Kind.INVALID_TABLE, "table " + t + ": " + e.getMessage());
            }
            final long bit = 1L << (slot & 63);
            if (slot >= MAX_TABLES || ((slot < 64 ? seenLo : seenHi) & bit) != 0) {
                throw new SceneException(SceneException.Kind.INVALID_TABLE, "table " + t + " slot " + slot);
            }
            if (slot < 64) {
                seenLo |= bit;
            } else {
                seenHi |= bit;
            }
            if (!r.isSparse() && r.rowCount() > cap) {
                throw new SceneException(SceneException.Kind.INVALID_TABLE,
                        "dense table " + t + " holds " + r.rowCount() + " rows for capacity " + cap);
            }
            bySlot[slot] = t;
        }
        for (int t = tables; t < poolUsed; t++) pool[t].detach();

        frameId = h[H_FRAME_ID];
        sceneEpoch = (int) h[H_SCENE_EPOCH];
        capacity = cap;
        rows = le(s[S_ROWS]);
        rowsBase = base(s[S_ROWS]);
        rects = le(s[S_RECTS]);
        rectsBase = base(s[S_RECTS]);
        text = le(s[S_TEXT]);
        textView = null;
        textBase = base(s[S_TEXT]);
        textLen = textN;
        children = le(s[S_CHILDREN]);
        childrenBase = base(s[S_CHILDREN]);
        childrenLen = childrenN;
        dirty.bind(s[S_DIRTY], dirtyN);
        moved.bind(s[S_MOVED], movedN);
        removed.bind(s[S_REMOVED], removedN);
        dirtyResources.bind(s[S_DIRTY_RESOURCES], dirtyResN);
        scroll = le(s[S_SCROLL_OFFSETS]);
        scrollBase = base(s[S_SCROLL_OFFSETS]);
        scrollCount = scrollN;
        tableCount = tables;
        return this;
    }

    private static int count(long v, String what) {
        if (v < 0 || v > Integer.MAX_VALUE) throw new SceneException(SceneException.Kind.MISSING_SLAB, what + " " + v);
        return (int) v;
    }

    private static int idSlab(ByteBuffer b, long n, String what) {
        final int c = count(n, what);
        requireBytes(b, (long) c * 4, what);
        return c;
    }

    private static void requireNodeIndexes(ByteBuffer b, int n, int cap, String what) {
        final ByteBuffer d = le(b);
        final int at = base(b);
        for (int i = 0; i < n; i++) {
            final int idx = d.getInt(at + i * 4);
            if (idx <= 0 || idx >= cap) {
                throw new SceneException(SceneException.Kind.INVALID_NODE, what + " index " + Integer.toUnsignedString(idx));
            }
        }
    }

    /** Unsigned {@code u32}s at {@code offset} of each {@code stride}-byte record are strictly ascending. */
    private static void requireAscending(ByteBuffer b, int offset, int stride, int n, String what) {
        final ByteBuffer d = le(b);
        final int at = base(b) + offset;
        for (int i = 1; i < n; i++) {
            if (Integer.compareUnsigned(d.getInt(at + (i - 1) * stride), d.getInt(at + i * stride)) >= 0) {
                throw new SceneException(SceneException.Kind.INVALID_NODE, what + " are not strictly ascending at " + i);
            }
        }
    }

    /**
     * Drops every reference to the frame's native buffers, in this reader and every pooled
     * {@link TableReader} a caller may have retained (call before {@code releaseScene}): any later
     * read throws instead of touching freed memory.
     */
    public void detach() {
        frameId = 0L;
        sceneEpoch = 0;
        capacity = 0;
        rows = rects = text = textView = children = scroll = null;
        rowsBase = rectsBase = textBase = childrenBase = scrollBase = 0;
        textLen = childrenLen = scrollCount = 0;
        dirty.bind(null, 0);
        moved.bind(null, 0);
        removed.bind(null, 0);
        dirtyResources.bind(null, 0);
        for (int t = 0; t < poolUsed; t++) pool[t].detach();
        tableCount = 0;
        java.util.Arrays.fill(bySlot, -1);
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

    /** {@code NveSceneView::scene_epoch}: changes on every runtime scene reset. */
    public int sceneEpoch() {
        return sceneEpoch;
    }

    /**
     * How this frame follows the last one a consumer applied ({@code prevFrameId == 0}: none):
     * {@link #FRAME_SAME}, {@link #FRAME_NEXT} (its deltas are exactly what changed since), or
     * {@link #FRAME_RESET} (another epoch or a skipped frame: reconcile everything).
     */
    public int relationTo(long prevFrameId, int prevEpoch) {
        if (prevFrameId == 0L || prevEpoch != sceneEpoch) return FRAME_RESET;
        if (frameId == prevFrameId) return FRAME_SAME;
        return frameId == prevFrameId + 1 ? FRAME_NEXT : FRAME_RESET;
    }

    /** Slot capacity of {@code rows} / {@code rects}; index 0 is the reserved null slot. */
    public int nodeCapacity() {
        return capacity;
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
        return index > 0 && index < capacity && (rowU8(index, NveRuntimeAbi.NODE_ROW_OFFSET_FLAGS) & NveRuntimeAbi.NODE_ROW_FLAG_LIVE) != 0;
    }

    public int flags(int index) {
        return rowU8(index, NveRuntimeAbi.NODE_ROW_OFFSET_FLAGS);
    }

    public int generation(int index) {
        return rowU8(index, NveRuntimeAbi.NODE_ROW_OFFSET_GENERATION);
    }

    public int primarySlot(int index) {
        return rowU8(index, NveRuntimeAbi.NODE_ROW_OFFSET_PRIMARY_SLOT);
    }

    /** Union of {@code effect::*} bits dirtied on this node during the latest frame. */
    public int effects(int index) {
        return rowU8(index, NveRuntimeAbi.NODE_ROW_OFFSET_EFFECTS);
    }

    /** Raw parent {@code NodeId} ({@code 0} = root / detached). */
    public int parent(int index) {
        return rows.getInt(rowAt(index, NveRuntimeAbi.NODE_ROW_OFFSET_PARENT));
    }

    public int childCount(int index) {
        return checkedChildSpan(index);
    }

    /** Raw {@code NodeId} of child {@code k} of node {@code index}. */
    public int childId(int index, int k) {
        final int len = checkedChildSpan(index);
        if (k < 0 || k >= len) throw new IndexOutOfBoundsException("child " + k + " of " + len);
        return children.getInt(childrenBase + (rows.getInt(rowAt(index, NveRuntimeAbi.NODE_ROW_OFFSET_CHILD_OFF)) + k) * 4);
    }

    private int checkedChildSpan(int index) {
        final int off = rows.getInt(rowAt(index, NveRuntimeAbi.NODE_ROW_OFFSET_CHILD_OFF));
        final int len = rows.getInt(rowAt(index, NveRuntimeAbi.NODE_ROW_OFFSET_CHILD_LEN));
        if (off < 0 || len < 0 || len > childrenLen - off) {
            throw new IndexOutOfBoundsException("children [" + Integer.toUnsignedString(off) + ", +"
                    + Integer.toUnsignedString(len) + ") of " + childrenLen);
        }
        return len;
    }

    public boolean hasTable(int index, int slot) {
        if (slot < 0 || slot >= MAX_TABLES) return false;
        final int word = rows.getInt(rowAt(index, NveRuntimeAbi.NODE_ROW_OFFSET_TABLE_SET + (slot >>> 5) * 4));
        return (word & (1 << (slot & 31))) != 0;
    }

    private int rowU8(int index, int offset) {
        return rows.get(rowAt(index, offset)) & 0xFF;
    }

    private int rowAt(int index, int offset) {
        if (index < 0 || index >= capacity) throw new IndexOutOfBoundsException("node " + index + " of " + capacity);
        return rowsBase + index * ROW_SIZE + offset;
    }

    // ---- layout rects --------------------------------------------------------------------------

    public float x(int index) {
        return rects.getFloat(rectAt(index, NveRuntimeAbi.LAYOUT_RECT_OFFSET_X));
    }

    public float y(int index) {
        return rects.getFloat(rectAt(index, NveRuntimeAbi.LAYOUT_RECT_OFFSET_Y));
    }

    public float width(int index) {
        return rects.getFloat(rectAt(index, NveRuntimeAbi.LAYOUT_RECT_OFFSET_WIDTH));
    }

    public float height(int index) {
        return rects.getFloat(rectAt(index, NveRuntimeAbi.LAYOUT_RECT_OFFSET_HEIGHT));
    }

    private int rectAt(int index, int offset) {
        if (index < 0 || index >= capacity) throw new IndexOutOfBoundsException("rect " + index + " of " + capacity);
        return rectsBase + index * RECT_SIZE + offset;
    }

    // ---- scroll offsets ------------------------------------------------------------------------

    /** Number of scrolled containers in {@code scroll_offsets} (sorted by node id). */
    public int scrollCount() {
        return scrollCount;
    }

    /** Raw {@code NodeId} of scroll offset {@code i}. */
    public int scrollNodeId(int i) {
        return scroll.getInt(scrollAt(i, NveRuntimeAbi.SCROLL_OFFSET_OFFSET_NODE_ID));
    }

    /** Index of {@code nodeId}'s scroll offset, or {@code -1} when it is not scrolled (O(log n)). */
    public int scrollIndexOf(int nodeId) {
        int lo = 0;
        int hi = scrollCount - 1;
        while (lo <= hi) {
            final int mid = (lo + hi) >>> 1;
            final int c = Integer.compareUnsigned(scrollNodeId(mid), nodeId);
            if (c == 0) return mid;
            if (c < 0) {
                lo = mid + 1;
            } else {
                hi = mid - 1;
            }
        }
        return -1;
    }

    /** Horizontal visual shift of scroll container {@code nodeId} ({@code 0} when not scrolled). */
    public float scrollX(int nodeId) {
        final int i = scrollIndexOf(nodeId);
        return i < 0 ? 0f : scroll.getFloat(scrollAt(i, NveRuntimeAbi.SCROLL_OFFSET_OFFSET_X));
    }

    /** Vertical visual shift of scroll container {@code nodeId} ({@code 0} when not scrolled). */
    public float scrollY(int nodeId) {
        final int i = scrollIndexOf(nodeId);
        return i < 0 ? 0f : scroll.getFloat(scrollAt(i, NveRuntimeAbi.SCROLL_OFFSET_OFFSET_Y));
    }

    private int scrollAt(int i, int offset) {
        if (i < 0 || i >= scrollCount) throw new IndexOutOfBoundsException("scroll offset " + i + " of " + scrollCount);
        return scrollBase + i * SCROLL_SIZE + offset;
    }

    // ---- per-frame deltas ----------------------------------------------------------------------

    /** Live nodes whose content changed this frame (see {@link #dirtyIndex}). */
    public int dirtyCount() {
        return dirty.count;
    }

    /**
     * Node index whose content changed this frame (created, row / table / child list changed, or
     * inherited typography changed); validated {@code 1 .. nodeCapacity)} at attach. Its rect may
     * have changed too.
     */
    public int dirtyIndex(int i) {
        return dirty.get(i, "dirty");
    }

    /** Live nodes (disjoint from the dirty ones) whose geometry alone changed this frame. */
    public int movedCount() {
        return moved.count;
    }

    /** Node index whose rect or parent-relative origin changed this frame; re-apply its frame only. */
    public int movedIndex(int i) {
        return moved.get(i, "moved");
    }

    public int removedCount() {
        return removed.count;
    }

    /** Raw {@code NodeId} removed this frame. */
    public int removedId(int i) {
        return removed.get(i, "removed");
    }

    /** Number of resource ids (re)defined on this frame (see {@link #dirtyResource}). */
    public int dirtyResourceCount() {
        return dirtyResources.count;
    }

    /**
     * {@code i}-th live resource id whose bytes changed on this frame (created, recycled, or
     * rewritten), strictly ascending: caches keyed by resource id must drop it. Ids not listed
     * kept their bytes.
     */
    public int dirtyResource(int i) {
        return dirtyResources.get(i, "dirty resource");
    }

    /** {@code true} when resource {@code id} is in {@link #dirtyResource} (O(log n)). */
    public boolean isResourceDirty(int id) {
        int lo = 0;
        int hi = dirtyResources.count - 1;
        while (lo <= hi) {
            final int mid = (lo + hi) >>> 1;
            final int c = Integer.compareUnsigned(dirtyResource(mid), id);
            if (c == 0) return true;
            if (c < 0) {
                lo = mid + 1;
            } else {
                hi = mid - 1;
            }
        }
        return false;
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
        if (textView == null) textView = text.duplicate();  // one bulk-read view per frame, on first use
        ((Buffer) textView).position(textBase + offset);
        textView.get(textScratch, 0, len);
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
        return pool[i];
    }

    /** Published table with {@code key}, or {@code null} (O(tables), allocation-free). */
    public TableReader findTable(Key64 key) {
        for (int i = 0; i < tableCount; i++) {
            if (pool[i].key().equals(key)) return pool[i];
        }
        return null;
    }

    /** Published table in {@code slot}, or {@code null}. */
    public TableReader tableAtSlot(int slot) {
        if (slot < 0 || slot >= MAX_TABLES) return null;
        final int i = bySlot[slot];
        return i < 0 ? null : pool[i];
    }

    /** Table realizing node {@code index} ({@code primary_slot}), or {@code null} for plain containers. */
    public TableReader primaryTable(int index) {
        final int slot = primarySlot(index);
        return slot == NveAbi.PRIMARY_SLOT_NONE ? null : tableAtSlot(slot);
    }

    /**
     * Row of live node {@code index} in {@code table}, or {@code -1} when the node has none; O(1)
     * for dense and sparse tables (the published row index), owner generation checked.
     */
    public int rowOf(TableReader table, int index) {
        if (table == null || !isLive(index) || !hasTable(index, table.slot())) return -1;
        final int r = table.rowOfNode(index);
        if (r < 0 || !table.isSparse()) return r;
        final int owner = table.owner(r);
        return indexOf(owner) == index && generationOf(owner) == generation(index) ? r : -1;
    }
}
