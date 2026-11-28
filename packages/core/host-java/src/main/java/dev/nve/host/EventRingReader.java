package dev.nve.host;

import dev.nve.host.generated.NveAbi;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;

/**
 * Single-pass ingress-validated zero-copy reader over an {@code NVEV} v2 event batch
 * ({@code packages/core/abi/src/event/{wire,validated}.rs}).
 *
 * <p>{@link #attach} validates the 14-byte header, exact total length, every 64-byte
 * {@code NveEvent}'s arena span, framing flag exclusivity, inline key bounds, split-key /
 * custom-name boundaries, and UTF-8 validity once in place with zero allocations. Later indexed
 * reads are O(1) absolute buffer accesses.
 */
public final class EventRingReader {
    private ByteBuffer buf;
    private int base;
    private int count;
    private int arenaBase;
    private int arenaLen;

    /** Validates and binds this reader to {@code buf[offset .. offset + length)}; throws {@link BinCodecException} on any error (leaving the reader detached). */
    public EventRingReader attach(ByteBuffer source, int offset, int length) {
        detach();
        if (source == null || offset < 0 || length < 0 || length > source.limit() - offset) {
            throw new BinCodecException(BinCodecException.Kind.TRUNCATED, "invalid NVEV slice");
        }
        final ByteBuffer le = source.order() == ByteOrder.LITTLE_ENDIAN ? source : source.duplicate().order(ByteOrder.LITTLE_ENDIAN);
        if (length < EventRingWriter.HEADER_SIZE) {
            throw new BinCodecException(BinCodecException.Kind.TRUNCATED, "NVEV header");
        }
        final int magic = le.getInt(offset);
        if (magic != EventRingWriter.WIRE_MAGIC) {
            throw new BinCodecException(BinCodecException.Kind.INVALID_TAG, "bad NVEV magic 0x" + Integer.toHexString(magic));
        }
        final int version = le.getShort(offset + 4) & 0xFFFF;
        if (version != EventRingWriter.WIRE_VERSION) {
            throw new BinCodecException(BinCodecException.Kind.INVALID_TAG, "unsupported NVEV version " + version);
        }
        final long cnt = Integer.toUnsignedLong(le.getInt(offset + 6));
        final long blen = Integer.toUnsignedLong(le.getInt(offset + 10));
        final long evBytes = cnt * EventRingWriter.EVENT_SIZE;
        final long expected = EventRingWriter.HEADER_SIZE + evBytes + blen;
        if (cnt > Integer.MAX_VALUE / EventRingWriter.EVENT_SIZE || expected > length) {
            throw new BinCodecException(BinCodecException.Kind.TRUNCATED, "NVEV frame shorter than declared");
        }
        if (expected < length) {
            throw new BinCodecException(BinCodecException.Kind.TRAILING_BYTES, (length - expected) + " trailing bytes");
        }
        final int aBase = offset + EventRingWriter.HEADER_SIZE + (int) evBytes;
        final int aLen = (int) blen;
        final int n = (int) cnt;
        for (int i = 0; i < n; i++) {
            validateEvent(le, offset + EventRingWriter.HEADER_SIZE + i * EventRingWriter.EVENT_SIZE, aBase, aLen, i);
        }
        this.buf = le;
        this.base = offset;
        this.count = n;
        this.arenaBase = aBase;
        this.arenaLen = aLen;
        return this;
    }

    /** Validates and binds to {@code source[0 .. length)}. */
    public EventRingReader attach(ByteBuffer source, int length) {
        return attach(source, 0, length);
    }

    /** Validates and binds to {@code bytes}. */
    public EventRingReader attach(byte[] bytes) {
        return attach(ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN), 0, bytes.length);
    }

    /** Clears the attached buffer and counts. */
    public void detach() {
        buf = null;
        base = count = arenaBase = arenaLen = 0;
    }

    private static void validateEvent(ByteBuffer le, int evAt, int arenaBase, int arenaLen, int index) {
        final long off = Integer.toUnsignedLong(le.getInt(evAt + NveAbi.EVENT_OFFSET_BYTES + NveAbi.RAW_EVENT_BYTES_OFFSET_OFFSET));
        final long len = Integer.toUnsignedLong(le.getInt(evAt + NveAbi.EVENT_OFFSET_BYTES + NveAbi.RAW_EVENT_BYTES_OFFSET_LEN));
        if (off > arenaLen || len > arenaLen - off) {
            throw new BinCodecException(BinCodecException.Kind.TRUNCATED, "event " + index + " arena span out of bounds");
        }
        final int sliceStart = arenaBase + (int) off;
        final int sliceLen = (int) len;
        final int flags = le.getInt(evAt + NveAbi.EVENT_OFFSET_FLAGS);
        final boolean hasUtf8 = (flags & NveAbi.EVENT_FLAG_UTF8_BYTES) != 0;
        final boolean hasInlineKey = (flags & NveAbi.EVENT_FLAG_INLINE_KEY_UTF8) != 0;
        final boolean hasSplitKey = (flags & NveAbi.EVENT_FLAG_KEY_SPLIT_BYTES) != 0;
        final boolean hasCustom = (flags & NveAbi.EVENT_FLAG_CUSTOM_NAMED) != 0;
        if ((hasInlineKey && hasSplitKey) || (hasCustom && (hasInlineKey || hasSplitKey))) {
            throw new BinCodecException(BinCodecException.Kind.INVALID_TAG, "event " + index + " conflicting framing flags");
        }
        if (hasUtf8 && !Utf8.isValid(le, sliceStart, sliceLen)) {
            throw new BinCodecException(BinCodecException.Kind.INVALID_UTF8, "event " + index + " utf8 bytes");
        }
        if (hasInlineKey) {
            final long klen = Integer.toUnsignedLong(le.getInt(evAt + NveAbi.EVENT_OFFSET_PAYLOAD + 4));
            if (klen > NveAbi.EVENT_INLINE_KEY_CAP) {
                throw new BinCodecException(BinCodecException.Kind.TOO_LARGE, "event " + index + " inline key len " + klen);
            }
            if (!Utf8.isValid(le, evAt + NveAbi.EVENT_OFFSET_PAYLOAD + 8, (int) klen)) {
                throw new BinCodecException(BinCodecException.Kind.INVALID_UTF8, "event " + index + " inline key");
            }
        }
        if (hasSplitKey) {
            final long klen = Integer.toUnsignedLong(le.getInt(evAt + NveAbi.EVENT_OFFSET_PAYLOAD + 4));
            if (klen > sliceLen) {
                throw new BinCodecException(BinCodecException.Kind.TRUNCATED, "event " + index + " split key len " + klen);
            }
            final int kl = (int) klen;
            if (hasUtf8) {
                if (kl < sliceLen && (le.get(sliceStart + kl) & 0xC0) == 0x80) {
                    throw new BinCodecException(BinCodecException.Kind.INVALID_UTF8, "event " + index + " split key boundary");
                }
            } else if (!Utf8.isValid(le, sliceStart, kl) || !Utf8.isValid(le, sliceStart + kl, sliceLen - kl)) {
                throw new BinCodecException(BinCodecException.Kind.INVALID_UTF8, "event " + index + " split key utf8");
            }
        }
        if (hasCustom) {
            final long nlen = Integer.toUnsignedLong(le.getInt(evAt + NveAbi.EVENT_OFFSET_PAYLOAD));
            if (nlen > sliceLen) {
                throw new BinCodecException(BinCodecException.Kind.TRUNCATED, "event " + index + " custom name len " + nlen);
            }
            if (!Utf8.isValid(le, sliceStart, (int) nlen)) {
                throw new BinCodecException(BinCodecException.Kind.INVALID_UTF8, "event " + index + " custom name utf8");
            }
        }
    }

    public int count() {
        return count;
    }

    public int arenaLength() {
        return arenaLen;
    }

    public long timestamp(int i) {
        return buf.getLong(eventAt(i) + NveAbi.EVENT_OFFSET_TIMESTAMP);
    }

    public long tableBits(int i) {
        return buf.getLong(eventAt(i) + NveAbi.EVENT_OFFSET_TABLE);
    }

    public Key64 table(int i) {
        return Key64.of(tableBits(i));
    }

    public int surface(int i) {
        return buf.getInt(eventAt(i) + NveAbi.EVENT_OFFSET_SURFACE);
    }

    public int node(int i) {
        return buf.getInt(eventAt(i) + NveAbi.EVENT_OFFSET_NODE);
    }

    public int event(int i) {
        return buf.getInt(eventAt(i) + NveAbi.EVENT_OFFSET_EVENT);
    }

    public int flags(int i) {
        return buf.getInt(eventAt(i) + NveAbi.EVENT_OFFSET_FLAGS);
    }

    public int payloadU32(int i, int slot) {
        if (slot < 0 || slot >= 6) throw new IndexOutOfBoundsException("payload slot " + slot);
        return buf.getInt(eventAt(i) + NveAbi.EVENT_OFFSET_PAYLOAD + slot * 4);
    }

    public float payloadF32(int i, int slot) {
        return Float.intBitsToFloat(payloadU32(i, slot));
    }

    public int bytesOffset(int i) {
        return buf.getInt(eventAt(i) + NveAbi.EVENT_OFFSET_BYTES + NveAbi.RAW_EVENT_BYTES_OFFSET_OFFSET);
    }

    public int bytesLen(int i) {
        return buf.getInt(eventAt(i) + NveAbi.EVENT_OFFSET_BYTES + NveAbi.RAW_EVENT_BYTES_OFFSET_LEN);
    }

    public byte[] bytes(int i) {
        return copySlice(arenaBase + bytesOffset(i), bytesLen(i));
    }

    public String text(int i) {
        final int f = flags(i);
        if ((f & NveAbi.EVENT_FLAG_UTF8_BYTES) != 0 && (f & NveAbi.EVENT_FLAG_KEY_SPLIT_BYTES) == 0) {
            return utf8(arenaBase + bytesOffset(i), bytesLen(i));
        }
        return "";
    }

    public String key(int i) {
        final int at = eventAt(i);
        final int f = buf.getInt(at + NveAbi.EVENT_OFFSET_FLAGS);
        if ((f & NveAbi.EVENT_FLAG_INLINE_KEY_UTF8) != 0) {
            return utf8(at + NveAbi.EVENT_OFFSET_PAYLOAD + 8, buf.getInt(at + NveAbi.EVENT_OFFSET_PAYLOAD + 4));
        }
        if ((f & NveAbi.EVENT_FLAG_KEY_SPLIT_BYTES) != 0) {
            return utf8(arenaBase + bytesOffset(i), buf.getInt(at + NveAbi.EVENT_OFFSET_PAYLOAD + 4));
        }
        return "";
    }

    public String code(int i) {
        final int at = eventAt(i);
        final int f = buf.getInt(at + NveAbi.EVENT_OFFSET_FLAGS);
        if ((f & NveAbi.EVENT_FLAG_INLINE_KEY_UTF8) != 0 && (f & NveAbi.EVENT_FLAG_UTF8_BYTES) != 0) {
            return utf8(arenaBase + bytesOffset(i), bytesLen(i));
        }
        if ((f & NveAbi.EVENT_FLAG_KEY_SPLIT_BYTES) != 0) {
            final int klen = buf.getInt(at + NveAbi.EVENT_OFFSET_PAYLOAD + 4);
            return utf8(arenaBase + bytesOffset(i) + klen, bytesLen(i) - klen);
        }
        return "";
    }

    public String customName(int i) {
        final int at = eventAt(i);
        if ((buf.getInt(at + NveAbi.EVENT_OFFSET_FLAGS) & NveAbi.EVENT_FLAG_CUSTOM_NAMED) == 0) return "";
        return utf8(arenaBase + bytesOffset(i), buf.getInt(at + NveAbi.EVENT_OFFSET_PAYLOAD));
    }

    public byte[] customData(int i) {
        final int at = eventAt(i);
        final int off = bytesOffset(i);
        final int len = bytesLen(i);
        if ((buf.getInt(at + NveAbi.EVENT_OFFSET_FLAGS) & NveAbi.EVENT_FLAG_CUSTOM_NAMED) == 0) {
            return copySlice(arenaBase + off, len);
        }
        final int nlen = buf.getInt(at + NveAbi.EVENT_OFFSET_PAYLOAD);
        return copySlice(arenaBase + off + nlen, len - nlen);
    }

    private int eventAt(int i) {
        if (buf == null || i < 0 || i >= count) throw new IndexOutOfBoundsException("event " + i + " of " + count);
        return base + EventRingWriter.HEADER_SIZE + i * EventRingWriter.EVENT_SIZE;
    }

    private byte[] copySlice(int at, int len) {
        final byte[] out = new byte[len];
        for (int k = 0; k < len; k++) out[k] = buf.get(at + k);
        return out;
    }

    private String utf8(int at, int len) {
        return len == 0 ? "" : new String(copySlice(at, len), StandardCharsets.UTF_8);
    }
}
