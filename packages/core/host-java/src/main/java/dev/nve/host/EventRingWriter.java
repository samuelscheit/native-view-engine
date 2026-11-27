package dev.nve.host;

import java.nio.Buffer;
import java.nio.BufferOverflowException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;

/**
 * Allocation-free writer of the binary {@code EventRing} ({@code nve_abi::encode_events}, byte-checked
 * against the Rust-generated {@code EventRingVectors}) into a reusable buffer, dispatched with
 * {@link NativeRuntime#dispatchEvents}.
 *
 * <p>Layout: {@code u32 count}, then per event {@code u32 surface, u32 node, u64 timestamp_us,
 * u8 kind, u8 payload_tag} and the payload. {@code kind} is an {@code NativeEventKind} code
 * ({@code NveAbi.EVENT_*}). A write that does not fit throws {@link BufferOverflowException},
 * a lone surrogate {@link BinCodecException}; both leave the ring as it was.
 */
public final class EventRingWriter {
    // `NveEventPayloadTag`: the ring's own payload discriminants.
    static final int PAYLOAD_EMPTY = 0;
    static final int PAYLOAD_POINT = 1;
    static final int PAYLOAD_SCROLL = 2;
    static final int PAYLOAD_TEXT = 3;
    static final int PAYLOAD_LAYOUT = 4;
    static final int PAYLOAD_CUSTOM = 5;
    static final int PAYLOAD_KEY = 6;
    static final int PAYLOAD_POINTER = 7;
    static final int PAYLOAD_WHEEL = 8;
    static final int PAYLOAD_TABLE_EVENT = 9;

    private static final int HEADER = 18;

    private final ByteBuffer buf;
    private final int surfaceId;
    private int count;
    private byte[] scratch = new byte[64];

    /** Writes into {@code buf} (direct for zero-copy dispatch) from position 0. */
    public EventRingWriter(ByteBuffer buf, int surfaceId) {
        this.buf = buf.order(ByteOrder.LITTLE_ENDIAN);
        this.surfaceId = surfaceId;
        reset();
    }

    /**
     * A writer continuing a ring already in {@code buf} ({@code count} events ending at
     * {@code buf.position()}), e.g. after copying it into a larger buffer.
     */
    public static EventRingWriter resume(ByteBuffer buf, int surfaceId, int count) {
        final int end = buf.position();
        final EventRingWriter w = new EventRingWriter(buf, surfaceId);
        ((Buffer) w.buf).position(end);
        w.count = count;
        w.buf.putInt(0, count);
        return w;
    }

    /** Rolls back to an earlier {@code (count(), length())} snapshot (drops later events). */
    public EventRingWriter truncate(int count, int length) {
        if (count < 0 || count > this.count || length < 4 || length > buf.position()) {
            throw new IllegalArgumentException("truncate past the ring: " + count + " / " + length);
        }
        this.count = count;
        buf.putInt(0, count);
        ((Buffer) buf).position(length);
        return this;
    }

    /** Empties the ring. */
    public EventRingWriter reset() {
        ((Buffer) buf).clear();  // `Buffer` receivers: Java 9 covariant overrides break Java 8 / old Android
        buf.putInt(0, 0);
        ((Buffer) buf).position(4);
        count = 0;
        return this;
    }

    public int count() {
        return count;
    }

    /** Bytes written so far (the {@code length} to dispatch). */
    public int length() {
        return buf.position();
    }

    public ByteBuffer buffer() {
        return buf;
    }

    public EventRingWriter empty(int kind, int nodeId, long timestampUs) {
        begin(kind, nodeId, timestampUs, PAYLOAD_EMPTY, 0);
        return end();
    }

    public EventRingWriter point(int kind, int nodeId, long timestampUs, float x, float y) {
        begin(kind, nodeId, timestampUs, PAYLOAD_POINT, 8);
        buf.putFloat(x).putFloat(y);
        return end();
    }

    /** Scroll offset plus content size. */
    public EventRingWriter scroll(int kind, int nodeId, long timestampUs, float x, float y, float contentWidth, float contentHeight) {
        begin(kind, nodeId, timestampUs, PAYLOAD_SCROLL, 16);
        buf.putFloat(x).putFloat(y).putFloat(contentWidth).putFloat(contentHeight);
        return end();
    }

    public EventRingWriter text(int kind, int nodeId, long timestampUs, String text) {
        final int len = Utf8.encodedLength(text);
        begin(kind, nodeId, timestampUs, PAYLOAD_TEXT, 4L + len);
        buf.putInt(len);
        putUtf8(text, len);
        return end();
    }

    /** Custom event {@code name} with opaque {@code data} ({@code null} = none). */
    public EventRingWriter custom(int kind, int nodeId, long timestampUs, String name, byte[] data) {
        final int nameLen = shortLength(name);
        final int dataLen = data == null ? 0 : data.length;
        begin(kind, nodeId, timestampUs, PAYLOAD_CUSTOM, 6L + nameLen + dataLen);
        buf.putShort((short) nameLen);
        putUtf8(name, nameLen);
        buf.putInt(dataLen);
        if (dataLen > 0) buf.put(data);
        return end();
    }

    /** Keyboard event: modifier bits, IME composing, platform key code, W3C {@code key}, physical {@code code}. */
    public EventRingWriter key(int kind, int nodeId, long timestampUs, String key, String code, int modifiers, int keyCode, boolean composing) {
        final int keyLen = shortLength(key);
        final int codeLen = shortLength(code);
        begin(kind, nodeId, timestampUs, PAYLOAD_KEY, 10L + keyLen + codeLen);
        buf.put((byte) modifiers).put((byte) (composing ? 1 : 0)).putInt(keyCode);
        buf.putShort((short) keyLen);
        putUtf8(key, keyLen);
        buf.putShort((short) codeLen);
        putUtf8(code, codeLen);
        return end();
    }

    /** Layout change: the node's new frame. */
    public EventRingWriter layout(int kind, int nodeId, long timestampUs, float x, float y, float width, float height) {
        begin(kind, nodeId, timestampUs, PAYLOAD_LAYOUT, 16);
        buf.putFloat(x).putFloat(y).putFloat(width).putFloat(height);
        return end();
    }

    /** Module table event: {@code tableKey}, {@code eventId}, opaque {@code payload} ({@code null} = none). */
    public EventRingWriter tableEvent(int kind, int nodeId, long timestampUs, long tableKey, int eventId, byte[] payload) {
        final int len = payload == null ? 0 : payload.length;
        begin(kind, nodeId, timestampUs, PAYLOAD_TABLE_EVENT, 16L + len);
        buf.putLong(tableKey).putInt(eventId).putInt(len);
        if (len > 0) buf.put(payload);
        return end();
    }

    /** Pointer event: position, modifier bits, button / capture flags. */
    public EventRingWriter pointer(int kind, int nodeId, long timestampUs, float x, float y, int modifiers, int eventFlags) {
        begin(kind, nodeId, timestampUs, PAYLOAD_POINTER, 10);
        buf.putFloat(x).putFloat(y).put((byte) modifiers).put((byte) eventFlags);
        return end();
    }

    public EventRingWriter wheel(int kind, int nodeId, long timestampUs, float dx, float dy, int modifiers) {
        begin(kind, nodeId, timestampUs, PAYLOAD_WHEEL, 9);
        buf.putFloat(dx).putFloat(dy).put((byte) modifiers);
        return end();
    }

    /** Checks room for the whole event before writing any byte, so a failed write leaves no trace. */
    private void begin(int kind, int nodeId, long timestampUs, int tag, long payloadBytes) {
        if (buf.remaining() < HEADER + payloadBytes) throw new BufferOverflowException();
        buf.putInt(surfaceId).putInt(nodeId).putLong(timestampUs).put((byte) kind).put((byte) tag);
    }

    private EventRingWriter end() {
        count++;
        buf.putInt(0, count);
        return this;
    }

    private static int shortLength(String s) {
        final int len = Utf8.encodedLength(s);
        if (len > 0xFFFF) throw new IllegalArgumentException("event string longer than 65535 bytes");
        return len;
    }

    /** Validated UTF-8 of {@code s} ({@code len} bytes); ASCII is written directly, others via a reused scratch. */
    private void putUtf8(String s, int len) {
        if (len == s.length()) {
            for (int i = 0; i < len; i++) buf.put((byte) s.charAt(i));
            return;
        }
        if (scratch.length < len) scratch = new byte[Math.max(len, scratch.length * 2)];
        Utf8.encode(s, scratch, 0);
        buf.put(scratch, 0, len);
    }
}
