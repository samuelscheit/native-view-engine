package dev.nve.host;

import dev.nve.host.generated.NveDrawList;
import java.nio.ByteBuffer;

/**
 * Zero-copy {@code nve.res.draw_list} ({@code FlatResource}) vector draw-list codec facade over
 * the generated {@link NveDrawList} decoder, shared by Android ({@code NveCanvasView}) and
 * Jetpack Compose ({@code NveDrawListPainter}).
 *
 * <p>Reads are absolute and honour any {@link ByteBuffer#order}: neither the buffer's position nor
 * its order is touched, and no call allocates.
 */
public final class DrawList {
    private DrawList() {}

    /** Resource kind of vector draw lists ({@code nve.res.draw_list}). */
    public static final Key64 KIND = NveDrawList.KIND;

    public static final int HEADER_SIZE = NveDrawList.HEADER_SIZE;
    public static final int MAX_DRAW_OPS = NveDrawList.MAX_DRAW_OPS;
    public static final int MAX_POLYLINE_POINTS = NveDrawList.MAX_POLYLINE_POINTS;
    public static final int MAX_CLIP_DEPTH = NveDrawList.MAX_CLIP_DEPTH;

    public static final int OP_CLEAR = NveDrawList.OP_CLEAR;
    public static final int OP_FILL_RECT = NveDrawList.OP_FILL_RECT;
    public static final int OP_FILL_ROUNDED_RECT = NveDrawList.OP_FILL_ROUNDED_RECT;
    public static final int OP_FILL_CIRCLE = NveDrawList.OP_FILL_CIRCLE;
    public static final int OP_STROKE_LINE = NveDrawList.OP_STROKE_LINE;
    public static final int OP_STROKE_POLYLINE = NveDrawList.OP_STROKE_POLYLINE;
    public static final int OP_CLIP_RECT = NveDrawList.OP_CLIP_RECT;
    public static final int OP_RESTORE_CLIP = NveDrawList.OP_RESTORE_CLIP;

    /** Why {@link #validate} rejected a draw list. */
    public enum Error {
        TRUNCATED,
        OP_COUNT_EXCEEDS_LIMIT,
        POLYLINE_POINTS_EXCEEDS_LIMIT,
        UNKNOWN_OPCODE,
        NON_FINITE_FLOAT,
        NEGATIVE_FLOAT,
        TRAILING_BYTES,
    }

    /**
     * Receives decoded ops. Every float is finite (radii and widths non-negative); colors are
     * {@code ColorRgba::to_u32()} ({@code r | g << 8 | b << 16 | a << 24}). Clips arrive balanced
     * and capped at {@link #MAX_CLIP_DEPTH}: unmatched {@code RestoreClip}s are dropped, overflowed
     * clips are balanced before popping clip #64, and clips still open at the end are closed with
     * {@link #onRestoreClip}, also when a sink callback throws.
     */
    public interface Sink extends NveDrawList.Sink {}

    /** Validates {@code bytes} whole; {@code null} when it is a well-formed draw list. */
    public static Error validate(byte[] bytes) {
        return mapError(NveDrawList.validate(bytes));
    }

    /**
     * Strict validation of {@code buf[offset .. offset + length)}: bounds, 4-byte-aligned word
     * header and ops, op and polyline-point limits, known {@code u32} opcodes, finite floats
     * (non-negative radii/widths), and no trailing bytes. {@code null} when well-formed.
     */
    public static Error validate(ByteBuffer buf, int offset, int length) {
        return mapError(NveDrawList.validate(buf, offset, length));
    }

    /** {@link #validate}s {@code buf[offset .. offset + length)}, then streams it into {@code sink}. */
    public static Error decode(ByteBuffer buf, int offset, int length, Sink sink) {
        return mapError(NveDrawList.decode(buf, offset, length, sink));
    }

    /**
     * Streams the draw list at {@code buf[offset ..)} into {@code sink}. The caller has
     * {@link #validate}d these exact bytes (e.g. once when they were copied, then on every paint).
     */
    public static void streamValidated(ByteBuffer buf, int offset, Sink sink) {
        NveDrawList.streamValidated(buf, offset, sink);
    }

    private static Error mapError(NveDrawList.Error err) {
        if (err == null) return null;
        switch (err) {
            case TRUNCATED: return Error.TRUNCATED;
            case OP_COUNT_EXCEEDS_LIMIT: return Error.OP_COUNT_EXCEEDS_LIMIT;
            case SLICE_LEN_EXCEEDS_LIMIT: return Error.POLYLINE_POINTS_EXCEEDS_LIMIT;
            case UNKNOWN_OPCODE: return Error.UNKNOWN_OPCODE;
            case NON_FINITE_FLOAT: return Error.NON_FINITE_FLOAT;
            case NEGATIVE_FLOAT: return Error.NEGATIVE_FLOAT;
            case TRAILING_BYTES: return Error.TRAILING_BYTES;
            default: throw new AssertionError("unhandled error: " + err);
        }
    }
}
