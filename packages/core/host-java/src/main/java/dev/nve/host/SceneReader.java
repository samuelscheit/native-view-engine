package dev.nve.host;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;

/**
 * Reusable validated zero-copy reader over one published {@code NveSceneView} frame whose slabs
 * arrive as direct buffers (JNI {@code NewDirectByteBuffer}).
 *
 * <p>Node records are read by byte offset; the generated ABI supplies {@code nodeStride} and the
 * field offsets, so this static runtime never hard-codes the record layout. {@link #attach}
 * validates the schema hash, every buffer against its count, and the text slab's UTF-8 once;
 * later reads are O(1) bounds checks.
 */
public final class SceneReader {
    private long frameId;
    private int nodeCount;
    private int nodeStride;
    private ByteBuffer nodes;
    private int nodesBase;
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
    private TableReader[] tables = new TableReader[0];
    private int tableCount;
    private byte[] textScratch = new byte[256];

    /**
     * Rebinds this reader to a new frame. {@code tables[0 .. tableCount)} must already be attached.
     *
     * @throws SceneException when the schema, a slab, the text UTF-8, or a table is invalid
     */
    public SceneReader attach(
            Key64 expectedSchema, long schemaHash, long frameId,
            ByteBuffer nodes, int nodeCount, int nodeStride,
            ByteBuffer textSlab, int textSlabLen,
            ByteBuffer childrenSlab, int childrenSlabLen,
            ByteBuffer dirtyIndices, int dirtyCount,
            ByteBuffer removedIds, int removedCount,
            TableReader[] tables, int tableCount) {
        if (schemaHash != expectedSchema.value()) {
            throw new SceneException(SceneException.Kind.SCHEMA_MISMATCH,
                    "expected " + expectedSchema + ", got " + Key64.of(schemaHash));
        }
        if (nodeStride <= 0) throw new SceneException(SceneException.Kind.MISSING_SLAB, "nodeStride " + nodeStride);
        requireBytes(nodes, (long) nodeCount * nodeStride, "nodes");
        requireBytes(textSlab, textSlabLen, "textSlab");
        requireBytes(childrenSlab, (long) childrenSlabLen * 4, "childrenSlab");
        requireBytes(dirtyIndices, (long) dirtyCount * 4, "dirtyIndices");
        requireBytes(removedIds, (long) removedCount * 4, "removedIds");
        if (tableCount < 0 || (tableCount > 0 && (tables == null || tables.length < tableCount))) {
            throw new SceneException(SceneException.Kind.INVALID_TABLE, "tableCount " + tableCount);
        }
        for (int i = 0; i < tableCount; i++) {
            if (tables[i] == null || tables[i].words() < 1) {
                throw new SceneException(SceneException.Kind.INVALID_TABLE, "table " + i + " is null or not attached");
            }
        }
        if (textSlabLen > 0 && !Utf8.isValid(textSlab, textSlab.position(), textSlabLen)) {
            throw new SceneException(SceneException.Kind.INVALID_UTF8, "textSlab");
        }
        this.frameId = frameId;
        this.nodeCount = nodeCount;
        this.nodeStride = nodeStride;
        this.nodes = le(nodes);
        this.nodesBase = base(nodes);
        this.text = le(textSlab);
        this.textBase = base(textSlab);
        this.textLen = textSlabLen;
        this.children = le(childrenSlab);
        this.childrenBase = base(childrenSlab);
        this.childrenLen = childrenSlabLen;
        this.dirty = le(dirtyIndices);
        this.dirtyBase = base(dirtyIndices);
        this.dirtyCount = dirtyCount;
        this.removed = le(removedIds);
        this.removedBase = base(removedIds);
        this.removedCount = removedCount;
        this.tables = tables == null ? new TableReader[0] : tables;
        this.tableCount = tableCount;
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

    public int nodeCount() {
        return nodeCount;
    }

    public int dirtyCount() {
        return dirtyCount;
    }

    public int removedCount() {
        return removedCount;
    }

    public int childrenSlabLen() {
        return childrenLen;
    }

    public int textSlabLen() {
        return textLen;
    }

    public int tableCount() {
        return tableCount;
    }

    // ---- node records (offsets from the generated ABI) -------------------------------------

    public int nodeU8(int index, int offset) {
        return nodes.get(nodeAt(index, offset, 1)) & 0xFF;
    }

    public int nodeU16(int index, int offset) {
        return nodes.getShort(nodeAt(index, offset, 2)) & 0xFFFF;
    }

    /** Raw 32-bit field ({@code u32} as int bits, or {@code i32}). */
    public int nodeI32(int index, int offset) {
        return nodes.getInt(nodeAt(index, offset, 4));
    }

    public float nodeF32(int index, int offset) {
        return nodes.getFloat(nodeAt(index, offset, 4));
    }

    private int nodeAt(int index, int offset, int size) {
        if (index < 0 || index >= nodeCount || offset < 0 || offset > nodeStride - size) {
            throw new IndexOutOfBoundsException("node " + index + " offset " + offset + " (+" + size + ") of "
                    + nodeCount + "x" + nodeStride);
        }
        return nodesBase + index * nodeStride + offset;
    }

    // ---- slabs ---------------------------------------------------------------------------------

    /**
     * {@code textSlab[offset .. offset + len)} as a String; throws {@link IndexOutOfBoundsException}
     * when out of range or not on UTF-8 boundaries.
     */
    public String text(int offset, int len) {
        if (offset < 0 || len < 0 || len > textLen - offset || !isBoundary(offset) || !isBoundary(offset + len)) {
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

    private boolean isBoundary(int at) {
        return at == textLen || (text.get(textBase + at) & 0xC0) != 0x80;
    }

    /** {@code childrenSlab[slabIndex]}. */
    public int childId(int slabIndex) {
        return u32At(children, childrenBase, slabIndex, childrenLen, "child");
    }

    public int dirtyIndex(int i) {
        return u32At(dirty, dirtyBase, i, dirtyCount, "dirty");
    }

    public int removedId(int i) {
        return u32At(removed, removedBase, i, removedCount, "removed");
    }

    private static int u32At(ByteBuffer buf, int base, int i, int count, String what) {
        if (i < 0 || i >= count) throw new IndexOutOfBoundsException(what + " " + i + " of " + count);
        return buf.getInt(base + i * 4);
    }

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
}
