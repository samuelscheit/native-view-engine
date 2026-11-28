package dev.nve.host;

import dev.nve.host.generated.NveAbi;
import java.nio.Buffer;
import java.nio.BufferOverflowException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;

/**
 * Allocation-free writer of uniform 64-byte {@code NveEvent} batches in the {@code NVEV} v2 wire
 * format ({@code nve_abi::EventBatchBuf::to_wire_bytes}, byte-checked against the Rust-generated
 * {@code EventRingVectors}) into a reusable buffer, dispatched with {@link NativeRuntime#dispatchEvents}.
 *
 * <p>Wire layout: 14-byte header ({@code "NVEV"}, {@code u16 version = 2}, {@code u32 count},
 * {@code u32 bytes_len}), followed by {@code count} contiguous 64-byte {@code NveEvent} records,
 * followed by the {@code bytes_len}-byte shared arena. Events are written directly into the target
 * buffer at {@code HEADER_SIZE + i * 64} while arena bytes accumulate in a reused scratch array
 * and are flushed at {@link #buffer()}. A write that does not fit throws
 * {@link BufferOverflowException}, a lone surrogate {@link BinCodecException}; both leave the
 * batch intact.
 */
public final class EventRingWriter {
    /** Little-endian {@code "NVEV"} wire magic. */
    public static final int WIRE_MAGIC = 0x5645564E;
    /** {@code NVEV} wire format version for 64-byte {@code NveEvent} records. */
    public static final int WIRE_VERSION = 2;
    /** Header size: {@code magic[4] + version:u16 + count:u32 + bytes_len:u32}. */
    public static final int HEADER_SIZE = 14;
    /** Stride of one uniform {@code NveEvent} record (64 bytes). */
    public static final int EVENT_SIZE = NveAbi.SIZEOF_EVENT;

    private static final int INTERNAL_FRAMING_MASK = NveAbi.EVENT_FLAG_UTF8_BYTES
            | NveAbi.EVENT_FLAG_INLINE_KEY_UTF8
            | NveAbi.EVENT_FLAG_CUSTOM_NAMED
            | NveAbi.EVENT_FLAG_KEY_SPLIT_BYTES;

    private static final long TABLE_INTERACTION = NveAbi.TABLE_INTERACTION.value();
    private static final long TABLE_TEXT_INPUT = NveAbi.TABLE_TEXT_INPUT.value();
    private static final long TABLE_SCROLL = NveAbi.TABLE_SCROLL.value();
    private static final long TABLE_LAYOUT = NveAbi.TABLE_LAYOUT.value();
    private static final long TABLE_CUSTOM = NveAbi.TABLE_CUSTOM.value();

    private final ByteBuffer buf;
    private final int surfaceId;
    private int count;
    private byte[] arena;
    private int arenaLen;
    private boolean dirty;

    /** Writes an {@code NVEV} v2 frame into {@code buf} (direct for zero-copy dispatch) from offset 0. */
    public EventRingWriter(ByteBuffer buf, int surfaceId) {
        if (buf == null || buf.capacity() < HEADER_SIZE) {
            throw new IllegalArgumentException("buffer capacity must be >= " + HEADER_SIZE);
        }
        this.buf = buf.order(ByteOrder.LITTLE_ENDIAN);
        this.surfaceId = surfaceId;
        this.arena = new byte[256];
        reset();
    }

    /** Encodes pointer button index {@code 0..7} into {@code NveEvent::flags} bits {@code 8..10}. */
    public static int pointerButtonFlag(int button) {
        return (button & 0x7) << NveAbi.EVENT_FLAG_BUTTON_SHIFT;
    }

    /** Copies all pending events and arena bytes into {@code bigger}, returning a writer over it. */
    public EventRingWriter copyTo(ByteBuffer bigger) {
        final int total = length();
        if (bigger == null || bigger.capacity() < total) {
            throw new IllegalArgumentException("target buffer capacity too small for " + total + " bytes");
        }
        final EventRingWriter next = new EventRingWriter(bigger, surfaceId);
        if (count > 0) {
            final int eventBytes = count * EVENT_SIZE;
            final ByteBuffer src = buf.duplicate();
            ((Buffer) src).position(HEADER_SIZE);
            ((Buffer) src).limit(HEADER_SIZE + eventBytes);
            ((Buffer) next.buf).position(HEADER_SIZE);
            next.buf.put(src);
        }
        if (arenaLen > 0) {
            next.ensureArena(arenaLen);
            System.arraycopy(arena, 0, next.arena, 0, arenaLen);
        }
        next.count = count;
        next.arenaLen = arenaLen;
        next.dirty = true;
        return next;
    }

    /** Rolls back to an earlier {@code (count(), arenaLength())} snapshot (drops later events). */
    public EventRingWriter truncate(int count, int arenaLength) {
        if (count < 0 || count > this.count || arenaLength < 0 || arenaLength > this.arenaLen) {
            throw new IllegalArgumentException("truncate past batch: " + count + " / " + arenaLength);
        }
        this.count = count;
        this.arenaLen = arenaLength;
        this.dirty = true;
        return this;
    }

    /** Empties the batch (leaves a valid 14-byte empty {@code NVEV} v2 header). */
    public EventRingWriter reset() {
        count = 0;
        arenaLen = 0;
        dirty = false;
        ((Buffer) buf).clear();  // `Buffer` receivers: Java 9 covariant overrides break old Android
        writeHeader();
        ((Buffer) buf).position(HEADER_SIZE);
        return this;
    }

    public int count() {
        return count;
    }

    public int arenaLength() {
        return arenaLen;
    }

    public int capacity() {
        return buf.capacity();
    }

    /** Encoded {@code NVEV} v2 frame byte length (the {@code length} to pass to {@link NativeRuntime#dispatchEvents}). */
    public int length() {
        return HEADER_SIZE + count * EVENT_SIZE + arenaLen;
    }

    /**
     * Flushes the {@code NVEV} v2 header and trailing byte arena into the backing buffer and
     * returns it with {@code position() == length()}.
     */
    public ByteBuffer buffer() {
        if (dirty) {
            writeHeader();
            final int arenaAt = HEADER_SIZE + count * EVENT_SIZE;
            ((Buffer) buf).clear();
            ((Buffer) buf).position(arenaAt);
            if (arenaLen > 0) buf.put(arena, 0, arenaLen);
            dirty = false;
        }
        return buf;
    }

    public EventRingWriter press(int nodeId, long timestampUs, float x, float y) {
        return writeEvent(TABLE_INTERACTION, NveAbi.INTERACTION_EVENT_PRESS, nodeId, timestampUs, 0,
                Float.floatToRawIntBits(x), Float.floatToRawIntBits(y), 0, 0, 0, 0, 0, 0);
    }

    public EventRingWriter longPress(int nodeId, long timestampUs, float x, float y) {
        return writeEvent(TABLE_INTERACTION, NveAbi.INTERACTION_EVENT_LONG_PRESS, nodeId, timestampUs, 0,
                Float.floatToRawIntBits(x), Float.floatToRawIntBits(y), 0, 0, 0, 0, 0, 0);
    }

    public EventRingWriter focus(int nodeId, long timestampUs) {
        return writeEvent(TABLE_INTERACTION, NveAbi.INTERACTION_EVENT_FOCUS, nodeId, timestampUs, 0,
                0, 0, 0, 0, 0, 0, 0, 0);
    }

    public EventRingWriter blur(int nodeId, long timestampUs) {
        return writeEvent(TABLE_INTERACTION, NveAbi.INTERACTION_EVENT_BLUR, nodeId, timestampUs, 0,
                0, 0, 0, 0, 0, 0, 0, 0);
    }

    /** Keyboard event ({@code NveAbi.INTERACTION_EVENT_KEY_DOWN} / {@code _KEY_UP}). */
    public EventRingWriter key(int interactionEvent, int nodeId, long timestampUs, String key, String code, int flags, int keyCode) {
        final int keyLen = Utf8.encodedLength(key);
        final int codeLen = Utf8.encodedLength(code);
        final int baseFlags = flags & ~INTERNAL_FRAMING_MASK;
        if (keyLen <= NveAbi.EVENT_INLINE_KEY_CAP) {
            requireRoom(codeLen);
            final int off = codeLen == 0 ? 0 : appendUtf8(code, codeLen);
            final int f = baseFlags | NveAbi.EVENT_FLAG_INLINE_KEY_UTF8
                    | (codeLen > 0 ? NveAbi.EVENT_FLAG_UTF8_BYTES : 0);
            final int base = beginEvent(TABLE_INTERACTION, interactionEvent, nodeId, timestampUs, f, off, codeLen);
            buf.putInt(base + NveAbi.EVENT_OFFSET_PAYLOAD, keyCode);
            buf.putInt(base + NveAbi.EVENT_OFFSET_PAYLOAD + 4, keyLen);
            writeInlineKey(base + NveAbi.EVENT_OFFSET_PAYLOAD + 8, key, keyLen);
            return finishEvent();
        }
        final long pairLen = (long) keyLen + codeLen;
        requireRoom(pairLen);
        final int off = arenaLen;
        appendUtf8(key, keyLen);
        if (codeLen > 0) appendUtf8(code, codeLen);
        final int f = baseFlags | NveAbi.EVENT_FLAG_KEY_SPLIT_BYTES | NveAbi.EVENT_FLAG_UTF8_BYTES;
        return writeEvent(TABLE_INTERACTION, interactionEvent, nodeId, timestampUs, f,
                keyCode, keyLen, 0, 0, 0, 0, off, (int) pairLen);
    }

    public EventRingWriter keyDown(int nodeId, long timestampUs, String key, String code, int flags, int keyCode) {
        return key(NveAbi.INTERACTION_EVENT_KEY_DOWN, nodeId, timestampUs, key, code, flags, keyCode);
    }

    public EventRingWriter keyUp(int nodeId, long timestampUs, String key, String code, int flags, int keyCode) {
        return key(NveAbi.INTERACTION_EVENT_KEY_UP, nodeId, timestampUs, key, code, flags, keyCode);
    }

    /** Pointer event ({@code NveAbi.INTERACTION_EVENT_POINTER_*}). */
    public EventRingWriter pointer(int interactionEvent, int nodeId, long timestampUs, float x, float y, int pointerId, int flags) {
        return pointer(interactionEvent, nodeId, timestampUs, x, y, pointerId, 0, 0, flags);
    }

    /** Pointer event with pressed-button bitmask ({@code payload[3]} low 16 bits) and pointer type ({@code 0=mouse, 1=pen, 2=touch}). */
    public EventRingWriter pointer(
            int interactionEvent, int nodeId, long timestampUs, float x, float y,
            int pointerId, int buttons, int pointerType, int flags) {
        final int p3 = (buttons & 0xFFFF) | ((pointerType & 0xFF) << 16);
        return writeEvent(TABLE_INTERACTION, interactionEvent, nodeId, timestampUs, flags & ~INTERNAL_FRAMING_MASK,
                Float.floatToRawIntBits(x), Float.floatToRawIntBits(y), pointerId, p3, 0, 0, 0, 0);
    }

    public EventRingWriter wheel(int nodeId, long timestampUs, float deltaX, float deltaY, int flags) {
        return writeEvent(TABLE_INTERACTION, NveAbi.INTERACTION_EVENT_WHEEL, nodeId, timestampUs, flags & ~INTERNAL_FRAMING_MASK,
                Float.floatToRawIntBits(deltaX), Float.floatToRawIntBits(deltaY), 0, 0, 0, 0, 0, 0);
    }

    public EventRingWriter textChange(int nodeId, long timestampUs, String text) {
        return tableEventText(TABLE_TEXT_INPUT, NveAbi.TEXT_INPUT_EVENT_CHANGE, nodeId, timestampUs, 0, 0, 0, 0, 0, 0, 0, text);
    }

    public EventRingWriter submit(int nodeId, long timestampUs, String text) {
        return tableEventText(TABLE_TEXT_INPUT, NveAbi.TEXT_INPUT_EVENT_SUBMIT, nodeId, timestampUs, 0, 0, 0, 0, 0, 0, 0, text);
    }

    /** IME composition event ({@code NveAbi.TEXT_INPUT_EVENT_COMPOSITION_*}). */
    public EventRingWriter composition(int textInputEvent, int nodeId, long timestampUs, String text) {
        return tableEventText(TABLE_TEXT_INPUT, textInputEvent, nodeId, timestampUs, 0, 0, 0, 0, 0, 0, 0, text);
    }

    /** Scroll offset plus content size. */
    public EventRingWriter scroll(int nodeId, long timestampUs, float x, float y, float contentWidth, float contentHeight) {
        return writeEvent(TABLE_SCROLL, NveAbi.SCROLL_EVENT_SCROLL, nodeId, timestampUs, 0,
                Float.floatToRawIntBits(x), Float.floatToRawIntBits(y),
                Float.floatToRawIntBits(contentWidth), Float.floatToRawIntBits(contentHeight), 0, 0, 0, 0);
    }

    /** Layout change: the node's new frame. */
    public EventRingWriter layout(int nodeId, long timestampUs, float x, float y, float width, float height) {
        return writeEvent(TABLE_LAYOUT, NveAbi.LAYOUT_EVENT_LAYOUT, nodeId, timestampUs, 0,
                Float.floatToRawIntBits(x), Float.floatToRawIntBits(y),
                Float.floatToRawIntBits(width), Float.floatToRawIntBits(height), 0, 0, 0, 0);
    }

    /** Named custom event with opaque {@code data} ({@code null} = none). */
    public EventRingWriter custom(int nodeId, long timestampUs, String name, byte[] data) {
        return custom(nodeId, timestampUs, name, data, 0);
    }

    /** Named custom event with opaque {@code data} ({@code null} = none) and caller {@code flags} (e.g. {@code NveAbi.EVENT_FLAG_NO_BUBBLE}). */
    public EventRingWriter custom(int nodeId, long timestampUs, String name, byte[] data, int flags) {
        final int nameLen = Utf8.encodedLength(name);
        final int dataLen = data == null ? 0 : data.length;
        final long total = (long) nameLen + dataLen;
        requireRoom(total);
        final int off = total == 0 ? 0 : arenaLen;
        if (nameLen > 0) appendUtf8(name, nameLen);
        if (dataLen > 0) appendBytes(data, dataLen);
        final int f = (flags & ~INTERNAL_FRAMING_MASK) | NveAbi.EVENT_FLAG_CUSTOM_NAMED;
        return writeEvent(TABLE_CUSTOM, 0, nodeId, timestampUs, f,
                nameLen, 0, 0, 0, 0, 0, off, (int) total);
    }

    /** Arbitrary table event with 6 inline payload words and optional raw {@code data} ({@code null} = none). */
    public EventRingWriter tableEvent(
            long tableKey, int eventId, int nodeId, long timestampUs, int flags,
            int p0, int p1, int p2, int p3, int p4, int p5, byte[] data) {
        final int dataLen = data == null ? 0 : data.length;
        requireRoom(dataLen);
        final int off = dataLen == 0 ? 0 : appendBytes(data, dataLen);
        return writeEvent(tableKey, eventId, nodeId, timestampUs, flags & ~INTERNAL_FRAMING_MASK,
                p0, p1, p2, p3, p4, p5, off, dataLen);
    }

    public EventRingWriter tableEvent(
            Key64 tableKey, int eventId, int nodeId, long timestampUs, int flags,
            int p0, int p1, int p2, int p3, int p4, int p5, byte[] data) {
        return tableEvent(tableKey.value(), eventId, nodeId, timestampUs, flags, p0, p1, p2, p3, p4, p5, data);
    }

    /** Arbitrary table event with 6 inline payload words and validated UTF-8 {@code text}. */
    public EventRingWriter tableEventText(
            long tableKey, int eventId, int nodeId, long timestampUs, int flags,
            int p0, int p1, int p2, int p3, int p4, int p5, String text) {
        final int len = Utf8.encodedLength(text);
        requireRoom(len);
        final int off = len == 0 ? 0 : appendUtf8(text, len);
        final int f = (flags & ~INTERNAL_FRAMING_MASK) | NveAbi.EVENT_FLAG_UTF8_BYTES;
        return writeEvent(tableKey, eventId, nodeId, timestampUs, f, p0, p1, p2, p3, p4, p5, off, len);
    }

    public EventRingWriter tableEventText(
            Key64 tableKey, int eventId, int nodeId, long timestampUs, int flags,
            int p0, int p1, int p2, int p3, int p4, int p5, String text) {
        return tableEventText(tableKey.value(), eventId, nodeId, timestampUs, flags, p0, p1, p2, p3, p4, p5, text);
    }

    private void requireRoom(long addedArena) {
        final long need = HEADER_SIZE + (long) (count + 1) * EVENT_SIZE + arenaLen + addedArena;
        if (addedArena < 0 || need > buf.capacity()) throw new BufferOverflowException();
    }

    private EventRingWriter writeEvent(
            long tableKey, int eventId, int nodeId, long timestampUs, int flags,
            int p0, int p1, int p2, int p3, int p4, int p5, int bytesOff, int bytesLen) {
        requireRoom(0);
        final int base = beginEvent(tableKey, eventId, nodeId, timestampUs, flags, bytesOff, bytesLen);
        buf.putInt(base + NveAbi.EVENT_OFFSET_PAYLOAD, p0);
        buf.putInt(base + NveAbi.EVENT_OFFSET_PAYLOAD + 4, p1);
        buf.putInt(base + NveAbi.EVENT_OFFSET_PAYLOAD + 8, p2);
        buf.putInt(base + NveAbi.EVENT_OFFSET_PAYLOAD + 12, p3);
        buf.putInt(base + NveAbi.EVENT_OFFSET_PAYLOAD + 16, p4);
        buf.putInt(base + NveAbi.EVENT_OFFSET_PAYLOAD + 20, p5);
        return finishEvent();
    }

    private int beginEvent(long tableKey, int eventId, int nodeId, long timestampUs, int flags, int bytesOff, int bytesLen) {
        final int base = HEADER_SIZE + count * EVENT_SIZE;
        buf.putLong(base + NveAbi.EVENT_OFFSET_TIMESTAMP, timestampUs);
        buf.putLong(base + NveAbi.EVENT_OFFSET_TABLE, tableKey);
        buf.putInt(base + NveAbi.EVENT_OFFSET_SURFACE, (surfaceId & SceneReader.INDEX_MASK) == 0 ? 0 : surfaceId);
        buf.putInt(base + NveAbi.EVENT_OFFSET_NODE, (nodeId & SceneReader.INDEX_MASK) == 0 ? 0 : nodeId);
        buf.putInt(base + NveAbi.EVENT_OFFSET_EVENT, eventId);
        buf.putInt(base + NveAbi.EVENT_OFFSET_FLAGS, flags);
        buf.putInt(base + NveAbi.EVENT_OFFSET_BYTES + NveAbi.RAW_EVENT_BYTES_OFFSET_OFFSET, bytesOff);
        buf.putInt(base + NveAbi.EVENT_OFFSET_BYTES + NveAbi.RAW_EVENT_BYTES_OFFSET_LEN, bytesLen);
        return base;
    }

    private EventRingWriter finishEvent() {
        count++;
        dirty = true;
        return this;
    }

    private void writeHeader() {
        buf.putInt(0, WIRE_MAGIC);
        buf.putShort(4, (short) WIRE_VERSION);
        buf.putInt(6, count);
        buf.putInt(10, arenaLen);
    }

    private void writeInlineKey(int at, String key, int keyLen) {
        // Zero-pad all 16 inline key bytes first, then encode UTF-8 directly into the first keyLen bytes.
        buf.putLong(at, 0L);
        buf.putLong(at + 8, 0L);
        if (keyLen == 0) return;
        if (keyLen == key.length()) {
            for (int i = 0; i < keyLen; i++) buf.put(at + i, (byte) key.charAt(i));
            return;
        }
        // Multi-byte UTF-8 inline key (<= 16 bytes): encode into tail of arena scratch (above arenaLen) without growing arenaLen.
        ensureArena(arenaLen + keyLen);
        Utf8.encode(key, arena, arenaLen);
        for (int i = 0; i < keyLen; i++) buf.put(at + i, arena[arenaLen + i]);
    }

    private int appendUtf8(String s, int len) {
        final int off = arenaLen;
        ensureArena(off + len);
        Utf8.encode(s, arena, off);
        arenaLen = off + len;
        return off;
    }

    private int appendBytes(byte[] data, int len) {
        final int off = arenaLen;
        ensureArena(off + len);
        System.arraycopy(data, 0, arena, off, len);
        arenaLen = off + len;
        return off;
    }

    private void ensureArena(int needed) {
        if (arena.length >= needed) return;
        int cap = arena.length;
        while (cap < needed) cap = Math.max(needed, cap * 2);
        arena = java.util.Arrays.copyOf(arena, cap);
    }
}
