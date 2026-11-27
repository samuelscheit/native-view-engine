package dev.nve.host;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;

/**
 * Reusable zero-copy reader over one published word table ({@code NveTableView}) in direct buffers.
 *
 * <p>Dense tables: row {@code i} belongs to node index {@code i} ({@code owners == null}). Sparse
 * tables: row {@code r} belongs to raw {@code NodeId} {@code owners[r]} (unsorted). Reads never
 * allocate; out-of-range access throws {@link IndexOutOfBoundsException}. {@link #attach} sets the
 * buffers' byte order to little-endian and reads relative to their current position.
 */
public final class TableReader {
    /** Maximum words per table row (Rust {@code MAX_TABLE_WORDS}). */
    public static final int MAX_WORDS = 32;

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

    /**
     * Rebinds this reader. Throws {@link IllegalArgumentException} when {@code words} is outside
     * {@code 1..32} or a buffer is missing / shorter than its count.
     */
    public TableReader attach(
            Key64 key, int slot, int words, int rowCount,
            ByteBuffer rows, ByteBuffer owners, ByteBuffer dirtyRows, int dirtyCount) {
        if (words < 1 || words > MAX_WORDS) throw new IllegalArgumentException("words " + words);
        if (rowCount < 0 || dirtyCount < 0) throw new IllegalArgumentException("negative count");
        requireWords(rows, (long) rowCount * words, "rows");
        if (owners != null) requireWords(owners, rowCount, "owners");
        requireWords(dirtyRows, dirtyCount, "dirtyRows");
        this.key = key;
        this.slot = slot;
        this.words = words;
        this.rowCount = rowCount;
        this.dirtyCount = dirtyCount;
        this.rows = rows == null ? null : rows.order(ByteOrder.LITTLE_ENDIAN);
        this.rowsBase = rows == null ? 0 : rows.position();
        this.owners = owners == null ? null : owners.order(ByteOrder.LITTLE_ENDIAN);
        this.ownersBase = owners == null ? 0 : owners.position();
        this.dirtyRows = dirtyRows == null ? null : dirtyRows.order(ByteOrder.LITTLE_ENDIAN);
        this.dirtyBase = dirtyRows == null ? 0 : dirtyRows.position();
        return this;
    }

    static void requireWords(ByteBuffer buf, long count, String what) {
        if (count == 0) return;
        if (buf == null) throw new IllegalArgumentException(what + " is null with count " + count);
        if (buf.remaining() / 4 < count) {
            throw new IllegalArgumentException(what + " holds " + buf.remaining() / 4 + " words, need " + count);
        }
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

    /** Text-slab offset of a {@code STR} field. */
    public int strOffset(int r, FieldSpec f) {
        return word(r, f.word);
    }

    /** Text-slab byte length of a {@code STR} field. */
    public int strLen(int r, FieldSpec f) {
        return word(r, f.word + 1);
    }

    /** Raw {@code NodeId} owning sparse row {@code r}, or {@code r} itself for dense tables. */
    public int owner(int r) {
        if (r < 0 || r >= rowCount) throw new IndexOutOfBoundsException("row " + r + " of " + rowCount);
        return owners == null ? r : owners.getInt(ownersBase + r * 4);
    }

    /** Row of {@code ownerId} (raw {@code NodeId} sparse, node index dense), or {@code -1}; O(rows) sparse. */
    public int rowOf(int ownerId) {
        if (owners == null) return ownerId >= 0 && ownerId < rowCount ? ownerId : -1;
        for (int r = 0; r < rowCount; r++) {
            if (owners.getInt(ownersBase + r * 4) == ownerId) return r;
        }
        return -1;
    }

    public int dirtyRow(int i) {
        if (i < 0 || i >= dirtyCount) throw new IndexOutOfBoundsException("dirty " + i + " of " + dirtyCount);
        return dirtyRows.getInt(dirtyBase + i * 4);
    }
}
