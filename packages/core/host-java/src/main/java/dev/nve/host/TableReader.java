package dev.nve.host;

import dev.nve.host.generated.NveAbi;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;

/**
 * Reusable zero-copy reader over one published word table ({@code NveTableView}) in direct buffers.
 *
 * <p>Dense tables: row {@code i} belongs to node index {@code i} ({@code owners == null}). Sparse
 * tables: row {@code r} belongs to raw {@code NodeId} {@code owners[r]} (unsorted), and the store's
 * paged node-index → row index ({@code rowIndexDir} / {@code rowIndexPages}) resolves a node's row
 * in O(1). Reads never allocate; out-of-range access throws {@link IndexOutOfBoundsException}.
 * {@link #attach} sets the buffers' byte order to little-endian and reads relative to their
 * current position. {@link #detach} drops every buffer: any later read throws instead of touching
 * released native memory.
 */
public final class TableReader {
    /** Maximum words per table row (Rust {@code MAX_TABLE_WORDS}). */
    public static final int MAX_WORDS = 32;
    private static final int PAGE_BITS = NveAbi.ROW_INDEX_PAGE_BITS;
    private static final int PAGE_LEN = NveAbi.ROW_INDEX_PAGE_LEN;

    private Key64 key = Key64.ZERO;
    private int slot;
    private int words;
    private int rowCount;
    private int dirtyCount;
    private ByteBuffer rows;
    private int rowsBase;
    private ByteBuffer owners;
    private int ownersBase;
    private ByteBuffer dirtyRows;
    private int dirtyBase;
    private ByteBuffer indexDir;
    private int indexDirBase;
    private int indexDirLen;
    private ByteBuffer indexPages;
    private int indexPagesBase;
    private int indexPageCount;

    /**
     * Rebinds this reader to one {@code NveTableView}. {@code rowIndexDir} / {@code rowIndexPages}
     * are the sparse row index ({@code null} / {@code 0} for dense tables). Throws
     * {@link IllegalArgumentException} when {@code words} is outside {@code 1..32}, a count is
     * negative, or a buffer is missing / shorter than its count; the reader is then detached.
     */
    public TableReader attach(
            Key64 key, int slot, int words, int rowCount,
            ByteBuffer rows, ByteBuffer owners, ByteBuffer dirtyRows, int dirtyCount,
            ByteBuffer rowIndexDir, int rowIndexDirLen, ByteBuffer rowIndexPages, int rowIndexPageCount) {
        try {
            if (words < 1 || words > MAX_WORDS) throw new IllegalArgumentException("words " + words);
            if (rowCount < 0 || dirtyCount < 0 || rowIndexDirLen < 0 || rowIndexPageCount < 0) {
                throw new IllegalArgumentException("negative count");
            }
            requireWords(rows, (long) rowCount * words, "rows");
            if (owners != null) requireWords(owners, rowCount, "owners");
            requireWords(dirtyRows, dirtyCount, "dirtyRows");
            requireWords(rowIndexDir, rowIndexDirLen, "rowIndexDir");
            requireWords(rowIndexPages, (long) rowIndexPageCount * PAGE_LEN, "rowIndexPages");
        } catch (IllegalArgumentException e) {
            detach();
            throw e;
        }
        this.key = key;
        this.slot = slot;
        this.words = words;
        this.rowCount = rowCount;
        this.dirtyCount = dirtyCount;
        this.rows = le(rows);
        this.rowsBase = base(rows);
        // An empty table is dense whatever its owners buffer (Rust publishes a dangling pointer
        // for an empty `Vec`), like C++ `is_sparse()`.
        this.owners = rowCount > 0 ? le(owners) : null;
        this.ownersBase = rowCount > 0 ? base(owners) : 0;
        this.dirtyRows = le(dirtyRows);
        this.dirtyBase = base(dirtyRows);
        this.indexDir = le(rowIndexDir);
        this.indexDirBase = base(rowIndexDir);
        this.indexDirLen = rowIndexDirLen;
        this.indexPages = le(rowIndexPages);
        this.indexPagesBase = base(rowIndexPages);
        this.indexPageCount = rowIndexPageCount;
        return this;
    }

    /**
     * Drops every buffer and count: later reads throw ({@link #isAttached} is {@code false}). The
     * key stays (it references no native memory) so a pooled reader re-attaching to the same table
     * reuses it without allocating.
     */
    public void detach() {
        slot = words = rowCount = dirtyCount = indexDirLen = indexPageCount = 0;
        rows = owners = dirtyRows = indexDir = indexPages = null;
        rowsBase = ownersBase = dirtyBase = indexDirBase = indexPagesBase = 0;
    }

    public boolean isAttached() {
        return words > 0;
    }

    static void requireWords(ByteBuffer buf, long count, String what) {
        if (count == 0) return;
        if (buf == null) throw new IllegalArgumentException(what + " is null with count " + count);
        if (buf.remaining() / 4 < count) {
            throw new IllegalArgumentException(what + " holds " + buf.remaining() / 4 + " words, need " + count);
        }
    }

    private static ByteBuffer le(ByteBuffer b) {
        return b == null ? null : b.order(ByteOrder.LITTLE_ENDIAN);
    }

    private static int base(ByteBuffer b) {
        return b == null ? 0 : b.position();
    }

    public Key64 key() {
        return key;
    }

    public int slot() {
        return slot;
    }

    public int words() {
        return words;
    }

    public int rowCount() {
        return rowCount;
    }

    public int dirtyCount() {
        return dirtyCount;
    }

    public boolean isSparse() {
        return owners != null;
    }

    /** Raw word {@code w} of row {@code r}. */
    public int word(int r, int w) {
        if (r < 0 || r >= rowCount || w < 0 || w >= words) {
            throw new IndexOutOfBoundsException("row " + r + " word " + w + " of " + rowCount + "x" + words);
        }
        return rows.getInt(rowsBase + (r * words + w) * 4);
    }

    /** Field bits of row {@code r}; throws {@link IndexOutOfBoundsException} when the field is past this row. */
    public int bits(int r, FieldSpec f) {
        return f.extract(fieldWord(r, f));
    }

    private int fieldWord(int r, FieldSpec f) {
        if (!f.fits(words)) throw new IndexOutOfBoundsException("field word " + f.word + " past " + words + "-word row");
        return word(r, f.word);
    }

    /** Unsigned field bits as raw int bits ({@code U32}, {@code ENUM}, {@code RESOURCE}, {@code NODE}). */
    public int u32(int r, FieldSpec f) {
        return bits(r, f);
    }

    public int i32(int r, FieldSpec f) {
        return f.extractSigned(fieldWord(r, f));
    }

    public float f32(int r, FieldSpec f) {
        return Float.intBitsToFloat(bits(r, f));
    }

    public boolean bool(int r, FieldSpec f) {
        return (bits(r, f) & 1) != 0;
    }

    /** Packed RGBA word (decode channels with {@link Rgba}). */
    public int color(int r, FieldSpec f) {
        return bits(r, f);
    }

    /** Packed length word (decode with {@link Length}). */
    public int length(int r, FieldSpec f) {
        return bits(r, f);
    }

    /** Text-slab offset of a {@code STR} field (throws for a non-{@code STR} spec or one past the row). */
    public int strOffset(int r, FieldSpec f) {
        return word(r, strWord(f));
    }

    /** Text-slab byte length of a {@code STR} field. */
    public int strLen(int r, FieldSpec f) {
        return word(r, strWord(f) + 1);
    }

    private int strWord(FieldSpec f) {
        if (f.kind != FieldKind.STR || !f.fits(words)) {
            throw new IndexOutOfBoundsException(f.kind + " field at word " + f.word + " is not a string in a " + words + "-word row");
        }
        return f.word;
    }

    /** Raw {@code NodeId} owning sparse row {@code r}, or {@code r} itself for dense tables. */
    public int owner(int r) {
        if (r < 0 || r >= rowCount) throw new IndexOutOfBoundsException("row " + r + " of " + rowCount);
        return owners == null ? r : owners.getInt(ownersBase + r * 4);
    }

    /**
     * Row of node slot {@code index}, or {@code -1}; O(1) for both storages (sparse: the published
     * row index). The row may belong to a stale generation: {@link #rowOf} also checks the owner.
     */
    public int rowOfNode(int index) {
        if (index < 0) return -1;
        if (owners == null) return index < rowCount ? index : -1;
        final int dirSlot = index >>> PAGE_BITS;
        if (dirSlot >= indexDirLen) return -1;
        final int page = indexDir.getInt(indexDirBase + dirSlot * 4);  // NO_PAGE (-1) fails the bound
        if (page < 0 || page >= indexPageCount) return -1;
        final int rowPlusOne = indexPages.getInt(indexPagesBase + (page * PAGE_LEN + (index & (PAGE_LEN - 1))) * 4);
        return rowPlusOne <= 0 || rowPlusOne > rowCount ? -1 : rowPlusOne - 1;
    }

    /** Row of {@code ownerId} (raw {@code NodeId} sparse, node index dense), or {@code -1}; O(1). */
    public int rowOf(int ownerId) {
        if (owners == null) return rowOfNode(ownerId);
        final int r = rowOfNode(SceneReader.indexOf(ownerId));
        return r >= 0 && owners.getInt(ownersBase + r * 4) == ownerId ? r : -1;
    }

    public int dirtyRow(int i) {
        if (i < 0 || i >= dirtyCount) throw new IndexOutOfBoundsException("dirty " + i + " of " + dirtyCount);
        return dirtyRows.getInt(dirtyBase + i * 4);
    }
}
