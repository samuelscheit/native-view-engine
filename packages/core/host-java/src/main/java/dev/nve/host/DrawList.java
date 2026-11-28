package dev.nve.host;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;

/**
 * Zero-copy {@code NVE_DRAW1} vector draw-list codec, byte-for-byte with
 * {@code packages/core/components/src/canvas_ir.rs}: one validator and one streaming decoder that
 * every JVM renderer (Android {@code NveCanvasView}, Compose {@code NveDrawListPainter}) paints
 * through a {@link Sink}.
 *
 * <p>Wire layout (integers and IEEE-754 floats little-endian):
 * <ul>
 *   <li>{@code [0..9)} ASCII magic {@code "NVE_DRAW1"}, {@code [9..11)} {@code u16} version
 *       ({@code 1}), {@code [11..15)} {@code u32} op count ({@code <= MAX_DRAW_OPS})
 *   <li>{@code [15..)} tagged ops: {@code 1 Clear{rgba}}, {@code 2 FillRect{x,y,w,h,rgba}},
 *       {@code 3 FillRoundedRect{x,y,w,h,radius,rgba}}, {@code 4 FillCircle{cx,cy,radius,rgba}},
 *       {@code 5 StrokeLine{x0,y0,x1,y1,width,rgba}},
 *       {@code 6 StrokePolyline{width,rgba,count,(x,y)[count]}}, {@code 7 ClipRect{x,y,w,h}},
 *       {@code 8 RestoreClip}
 * </ul>
 *
 * <p>Reads are absolute and honour any {@link ByteBuffer#order}: neither the buffer's position nor
 * its order is touched, and no call allocates.
 */
public final class DrawList {
    private DrawList() {}

    public static final int HEADER_SIZE = 15;
    public static final int FORMAT_VERSION = 1;
    public static final int MAX_DRAW_OPS = 65_536;
    public static final int MAX_POLYLINE_POINTS = 16_384;

    public static final int OP_CLEAR = 1;
    public static final int OP_FILL_RECT = 2;
    public static final int OP_FILL_ROUNDED_RECT = 3;
    public static final int OP_FILL_CIRCLE = 4;
    public static final int OP_STROKE_LINE = 5;
    public static final int OP_STROKE_POLYLINE = 6;
    public static final int OP_CLIP_RECT = 7;
    public static final int OP_RESTORE_CLIP = 8;

    private static final byte[] MAGIC = {'N', 'V', 'E', '_', 'D', 'R', 'A', 'W', '1'};

    /** Why {@link #validate} rejected a draw list. */
    public enum Error {
        TRUNCATED,
        BAD_MAGIC,
        UNSUPPORTED_VERSION,
        OP_COUNT_EXCEEDS_LIMIT,
        POLYLINE_POINTS_EXCEEDS_LIMIT,
        UNKNOWN_OPCODE,
        NON_FINITE_FLOAT,
        TRAILING_BYTES,
    }

    /**
     * Receives decoded ops. Every float is finite; colors are {@code ColorRgba::to_u32()}
     * ({@code r | g << 8 | b << 16 | a << 24}). Clips arrive balanced: an unmatched
     * {@code RestoreClip} is dropped and clips still open at the end are closed with
     * {@link #onRestoreClip}, also when a sink callback throws.
     */
    public interface Sink {
        void onClear(int rgba);

        void onFillRect(float x, float y, float w, float h, int rgba);

        void onFillRoundedRect(float x, float y, float w, float h, float radius, int rgba);

        void onFillCircle(float cx, float cy, float radius, int rgba);

        void onStrokeLine(float x0, float y0, float x1, float y1, float width, int rgba);

        /** Starts a polyline of {@code pointCount} points ({@link #onStrokePolylinePoint}), then {@link #onStrokePolylineEnd}. */
        void onStrokePolylineStart(int pointCount, float width, int rgba);

        void onStrokePolylinePoint(int index, float x, float y);

        void onStrokePolylineEnd();

        void onClipRect(float x, float y, float w, float h);

        void onRestoreClip();
    }

    /**
     * {@code true} when {@code buf[offset .. offset + length)} lies inside {@code buf}'s limit,
     * holds a full header, and starts with the magic (overflow-safe for any int arguments).
     */
    public static boolean hasMagic(ByteBuffer buf, int offset, int length) {
        if (!inBounds(buf, offset, length) || length < HEADER_SIZE) return false;
        for (int i = 0; i < MAGIC.length; i++) {
            if (buf.get(offset + i) != MAGIC[i]) return false;
        }
        return true;
    }

    /** Validates {@code bytes} whole; {@code null} when it is a well-formed draw list. */
    public static Error validate(byte[] bytes) {
        return validate(ByteBuffer.wrap(bytes), 0, bytes.length);
    }

    /**
     * Strict validation of {@code buf[offset .. offset + length)}: bounds, magic, version, op and
     * polyline-point limits, known opcodes, finite floats, and no trailing bytes. {@code null}
     * when well-formed.
     */
    public static Error validate(ByteBuffer buf, int offset, int length) {
        if (!inBounds(buf, offset, length) || length < HEADER_SIZE) return Error.TRUNCATED;
        if (!hasMagic(buf, offset, length)) return Error.BAD_MAGIC;
        if (u16(buf, offset + 9) != FORMAT_VERSION) return Error.UNSUPPORTED_VERSION;
        final long opCount = u32(buf, offset + 11);
        if (opCount > MAX_DRAW_OPS) return Error.OP_COUNT_EXCEEDS_LIMIT;
        final int end = offset + length;
        int pos = offset + HEADER_SIZE;
        if (end - pos < opCount) return Error.TRUNCATED;  // every op is at least its tag byte
        for (long i = 0; i < opCount; i++) {
            if (pos >= end) return Error.TRUNCATED;
            final int op = buf.get(pos++) & 0xFF;
            final int floats;
            final int size;
            switch (op) {
                case OP_CLEAR: floats = 0; size = 4; break;
                case OP_FILL_RECT: floats = 4; size = 20; break;
                case OP_FILL_ROUNDED_RECT: floats = 5; size = 24; break;
                case OP_FILL_CIRCLE: floats = 3; size = 16; break;
                case OP_STROKE_LINE: floats = 5; size = 24; break;
                case OP_CLIP_RECT: floats = 4; size = 16; break;
                case OP_RESTORE_CLIP: floats = 0; size = 0; break;
                case OP_STROKE_POLYLINE: {
                    if (end - pos < 12) return Error.TRUNCATED;
                    if (!Float.isFinite(f32(buf, pos))) return Error.NON_FINITE_FLOAT;
                    final long points = u32(buf, pos + 8);
                    pos += 12;
                    if (points > MAX_POLYLINE_POINTS) return Error.POLYLINE_POINTS_EXCEEDS_LIMIT;
                    if (points * 8 > end - pos) return Error.TRUNCATED;
                    floats = (int) points * 2;
                    size = floats * 4;
                    break;
                }
                default:
                    return Error.UNKNOWN_OPCODE;
            }
            if (end - pos < size) return Error.TRUNCATED;
            for (int f = 0; f < floats; f++) {
                if (!Float.isFinite(f32(buf, pos + f * 4))) return Error.NON_FINITE_FLOAT;
            }
            pos += size;
        }
        return pos == end ? null : Error.TRAILING_BYTES;
    }

    /** {@link #validate}s {@code buf[offset .. offset + length)}, then streams it into {@code sink}. */
    public static Error decode(ByteBuffer buf, int offset, int length, Sink sink) {
        final Error error = validate(buf, offset, length);
        if (error == null) streamValidated(buf, offset, sink);
        return error;
    }

    /**
     * Streams the draw list at {@code buf[offset ..)} into {@code sink}. The caller has
     * {@link #validate}d these exact bytes (e.g. once when they were copied, then on every paint).
     */
    public static void streamValidated(ByteBuffer buf, int offset, Sink sink) {
        final int opCount = (int) u32(buf, offset + 11);
        int pos = offset + HEADER_SIZE;
        int clips = 0;
        try {
            for (int i = 0; i < opCount; i++) {
                final int op = buf.get(pos++) & 0xFF;
                switch (op) {
                    case OP_CLEAR:
                        sink.onClear(i32(buf, pos));
                        pos += 4;
                        break;
                    case OP_FILL_RECT:
                        sink.onFillRect(f32(buf, pos), f32(buf, pos + 4), f32(buf, pos + 8), f32(buf, pos + 12), i32(buf, pos + 16));
                        pos += 20;
                        break;
                    case OP_FILL_ROUNDED_RECT:
                        sink.onFillRoundedRect(f32(buf, pos), f32(buf, pos + 4), f32(buf, pos + 8), f32(buf, pos + 12),
                                f32(buf, pos + 16), i32(buf, pos + 20));
                        pos += 24;
                        break;
                    case OP_FILL_CIRCLE:
                        sink.onFillCircle(f32(buf, pos), f32(buf, pos + 4), f32(buf, pos + 8), i32(buf, pos + 12));
                        pos += 16;
                        break;
                    case OP_STROKE_LINE:
                        sink.onStrokeLine(f32(buf, pos), f32(buf, pos + 4), f32(buf, pos + 8), f32(buf, pos + 12),
                                f32(buf, pos + 16), i32(buf, pos + 20));
                        pos += 24;
                        break;
                    case OP_STROKE_POLYLINE: {
                        final int points = (int) u32(buf, pos + 8);
                        sink.onStrokePolylineStart(points, f32(buf, pos), i32(buf, pos + 4));
                        pos += 12;
                        for (int p = 0; p < points; p++, pos += 8) {
                            sink.onStrokePolylinePoint(p, f32(buf, pos), f32(buf, pos + 4));
                        }
                        sink.onStrokePolylineEnd();
                        break;
                    }
                    case OP_CLIP_RECT:
                        clips++;
                        sink.onClipRect(f32(buf, pos), f32(buf, pos + 4), f32(buf, pos + 8), f32(buf, pos + 12));
                        pos += 16;
                        break;
                    case OP_RESTORE_CLIP:
                        if (clips > 0) {
                            clips--;
                            sink.onRestoreClip();
                        }
                        break;
                    default:
                        throw new IllegalArgumentException("draw list was not validated: opcode " + op);
                }
            }
        } finally {
            for (; clips > 0; clips--) sink.onRestoreClip();
        }
    }

    private static boolean inBounds(ByteBuffer buf, int offset, int length) {
        return buf != null && offset >= 0 && length >= 0 && length <= buf.limit() - offset;
    }

    private static int u16(ByteBuffer b, int p) {
        final short v = b.getShort(p);
        return (b.order() == ByteOrder.LITTLE_ENDIAN ? v : Short.reverseBytes(v)) & 0xFFFF;
    }

    private static int i32(ByteBuffer b, int p) {
        final int v = b.getInt(p);
        return b.order() == ByteOrder.LITTLE_ENDIAN ? v : Integer.reverseBytes(v);
    }

    private static long u32(ByteBuffer b, int p) {
        return i32(b, p) & 0xFFFF_FFFFL;
    }

    private static float f32(ByteBuffer b, int p) {
        return Float.intBitsToFloat(i32(b, p));
    }
}
