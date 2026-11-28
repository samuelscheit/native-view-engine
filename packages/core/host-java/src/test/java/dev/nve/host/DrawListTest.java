package dev.nve.host;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;

/** {@link DrawList}: strict validation, zero-copy streaming, and balanced clips. */
final class DrawListTest {
    private DrawListTest() {}

    static void run(Check c) {
        streamsEveryOpcodeFromAnOffsetSlice(c);
        rejectsMalformedLists(c);
        balancesClips(c);
    }

    private static void streamsEveryOpcodeFromAnOffsetSlice(Check c) {
        final byte[] list = all8();
        for (ByteOrder order : new ByteOrder[] {ByteOrder.LITTLE_ENDIAN, ByteOrder.BIG_ENDIAN}) {
            final ByteBuffer padded = ByteBuffer.allocateDirect(list.length + 19).order(order);
            for (int i = 0; i < 11; i++) padded.put((byte) 0x55);
            padded.put(list);
            for (int i = 0; i < 8; i++) padded.put((byte) 0xAA);
            padded.position(3);
            c.that(DrawList.hasMagic(padded, 11, list.length), "magic at an offset (" + order + ")");
            final Log log = new Log();
            c.that(DrawList.decode(padded, 11, list.length, log) == null, "decodes (" + order + ")");
            c.eq(padded.position(), 3, "position untouched");
            c.that(padded.order() == order, "order untouched");
            c.eq(String.join("|", log.ops),
                    "clear:-1|rect:4.0,8.0,40.0,20.0,255|rrect:10.0,12.0,32.0,16.0,6.0,255|circle:50.0,30.0,8.0,255"
                            + "|line:2.0,4.0,60.0,44.0,2.5,255|clip:1.0,2.0,3.0,4.0|poly:3,3.0,255"
                            + "|pt0:6.0,40.0|pt1:30.0,15.0|pt2:60.0,35.0|polyEnd|restore",
                    "every op in order, little-endian whatever the buffer's order (" + order + ")");
        }
        c.that(DrawList.validate(header(0)) == null, "an empty list is valid");
    }

    private static void rejectsMalformedLists(Check c) {
        final ByteBuffer small = ByteBuffer.allocate(32);
        c.that(!DrawList.hasMagic(small, Integer.MAX_VALUE - 4, 16), "offset + length overflow is out of bounds");
        c.that(DrawList.validate(small, Integer.MAX_VALUE - 4, 16) == DrawList.Error.TRUNCATED, "overflowing slice");
        c.that(DrawList.validate(small, -1, 16) == DrawList.Error.TRUNCATED, "negative offset");
        c.that(DrawList.validate(small, 0, -1) == DrawList.Error.TRUNCATED, "negative length");
        c.that(!DrawList.hasMagic(null, 0, 15), "null buffer");
        c.that(DrawList.validate(new byte[0]) == DrawList.Error.TRUNCATED, "empty");
        c.that(DrawList.validate(new byte[14]) == DrawList.Error.TRUNCATED, "short header");
        final byte[] bad = header(0);
        bad[0] = 'X';
        c.that(DrawList.validate(bad) == DrawList.Error.BAD_MAGIC, "bad magic");
        final byte[] v2 = header(0);
        v2[9] = 2;
        c.that(DrawList.validate(v2) == DrawList.Error.UNSUPPORTED_VERSION, "version 2");
        c.that(DrawList.validate(header(DrawList.MAX_DRAW_OPS + 1)) == DrawList.Error.OP_COUNT_EXCEEDS_LIMIT, "op limit");
        c.that(DrawList.validate(header(-1)) == DrawList.Error.OP_COUNT_EXCEEDS_LIMIT, "u32 op count is unsigned");
        c.that(DrawList.validate(op(header(1), 99)) == DrawList.Error.UNKNOWN_OPCODE, "unknown opcode");
        c.that(DrawList.validate(op(header(1), 0)) == DrawList.Error.UNKNOWN_OPCODE, "opcode 0");
        c.that(DrawList.validate(f(op(header(1), DrawList.OP_FILL_RECT), Float.NaN, 0, 10, 10, 0))
                == DrawList.Error.NON_FINITE_FLOAT, "NaN rect");
        c.that(DrawList.validate(f(op(header(1), DrawList.OP_FILL_CIRCLE), 1, Float.POSITIVE_INFINITY, 5, 0))
                == DrawList.Error.NON_FINITE_FLOAT, "infinite circle");
        c.that(DrawList.validate(f(op(header(1), DrawList.OP_STROKE_POLYLINE), Float.NaN, 0, 0))
                == DrawList.Error.NON_FINITE_FLOAT, "NaN polyline width");
        c.that(DrawList.validate(f(op(header(1), DrawList.OP_STROKE_POLYLINE), 1, 0, Float.intBitsToFloat(1), 0, Float.NaN))
                == DrawList.Error.NON_FINITE_FLOAT, "NaN polyline point");
        c.that(DrawList.validate(f(op(header(1), DrawList.OP_STROKE_POLYLINE), 2, 0,
                Float.intBitsToFloat(DrawList.MAX_POLYLINE_POINTS + 1))) == DrawList.Error.POLYLINE_POINTS_EXCEEDS_LIMIT,
                "polyline point limit");
        c.that(DrawList.validate(f(op(header(1), DrawList.OP_STROKE_POLYLINE), 2, 0, Float.intBitsToFloat(-1)))
                == DrawList.Error.POLYLINE_POINTS_EXCEEDS_LIMIT, "u32 point count is unsigned");
        final byte[] valid = all8();
        int truncated = 0;
        for (int cut = 1; cut < valid.length; cut++) {
            if (DrawList.validate(Arrays.copyOf(valid, cut)) == DrawList.Error.TRUNCATED) truncated++;
        }
        c.eq(truncated, valid.length - 1, "every proper prefix is TRUNCATED");
        c.that(DrawList.validate(Arrays.copyOf(valid, valid.length + 1)) == DrawList.Error.TRAILING_BYTES, "trailing byte");
        boolean threw = false;
        try {
            DrawList.streamValidated(ByteBuffer.wrap(op(header(1), 99)), 0, new Log());
        } catch (IllegalArgumentException e) {
            threw = true;
        }
        c.that(threw, "streaming an unvalidated unknown opcode throws instead of misreading");
    }

    private static void balancesClips(Check c) {
        final byte[] list = clip(clip(op(op(header(4), DrawList.OP_RESTORE_CLIP), DrawList.OP_RESTORE_CLIP)));
        final Log log = new Log();
        c.that(DrawList.decode(ByteBuffer.wrap(list), 0, list.length, log) == null, "unbalanced clips are valid");
        c.eq(String.join("|", log.ops), "clip:1.0,2.0,3.0,4.0|clip:1.0,2.0,3.0,4.0|restore|restore",
                "unmatched restores are dropped and open clips closed at the end");

        final Log throwing = new Log() {
            @Override
            public void onClipRect(float x, float y, float w, float h) {
                super.onClipRect(x, y, w, h);
                if (ops.size() == 2) throw new IllegalStateException("paint failed");
            }
        };
        try {
            DrawList.decode(ByteBuffer.wrap(list), 0, list.length, throwing);
        } catch (IllegalStateException expected) {
            // the sink's own failure propagates
        }
        c.eq(String.join("|", throwing.ops), "clip:1.0,2.0,3.0,4.0|clip:1.0,2.0,3.0,4.0|restore|restore",
                "clips opened before a throwing sink callback are still closed");
    }

    // ---- builders ------------------------------------------------------------------------------

    private static byte[] header(int ops) {
        final ByteBuffer b = ByteBuffer.allocate(DrawList.HEADER_SIZE).order(ByteOrder.LITTLE_ENDIAN);
        b.put("NVE_DRAW1".getBytes(StandardCharsets.US_ASCII)).putShort((short) DrawList.FORMAT_VERSION).putInt(ops);
        return b.array();
    }

    private static byte[] op(byte[] head, int opcode) {
        final byte[] out = Arrays.copyOf(head, head.length + 1);
        out[head.length] = (byte) opcode;
        return out;
    }

    /** Appends little-endian f32s (ints go through {@link Float#intBitsToFloat}). */
    private static byte[] f(byte[] head, float... values) {
        final ByteBuffer b = ByteBuffer.allocate(head.length + values.length * 4).order(ByteOrder.LITTLE_ENDIAN);
        b.put(head);
        for (float v : values) b.putInt(Float.floatToRawIntBits(v));
        return b.array();
    }

    private static byte[] clip(byte[] head) {
        return f(op(head, DrawList.OP_CLIP_RECT), 1, 2, 3, 4);
    }

    private static float rgba(int v) {
        return Float.intBitsToFloat(v);
    }

    private static byte[] all8() {
        byte[] b = header(8);
        b = f(op(b, DrawList.OP_CLEAR), rgba(-1));
        b = f(op(b, DrawList.OP_FILL_RECT), 4, 8, 40, 20, rgba(255));
        b = f(op(b, DrawList.OP_FILL_ROUNDED_RECT), 10, 12, 32, 16, 6, rgba(255));
        b = f(op(b, DrawList.OP_FILL_CIRCLE), 50, 30, 8, rgba(255));
        b = f(op(b, DrawList.OP_STROKE_LINE), 2, 4, 60, 44, 2.5f, rgba(255));
        b = clip(b);
        b = f(op(b, DrawList.OP_STROKE_POLYLINE), 3, rgba(255), rgba(3), 6, 40, 30, 15, 60, 35);
        return op(b, DrawList.OP_RESTORE_CLIP);
    }

    private static class Log implements DrawList.Sink {
        final List<String> ops = new ArrayList<>();

        @Override public void onClear(int rgba) { ops.add("clear:" + rgba); }

        @Override public void onFillRect(float x, float y, float w, float h, int rgba) {
            ops.add("rect:" + x + "," + y + "," + w + "," + h + "," + rgba);
        }

        @Override public void onFillRoundedRect(float x, float y, float w, float h, float r, int rgba) {
            ops.add("rrect:" + x + "," + y + "," + w + "," + h + "," + r + "," + rgba);
        }

        @Override public void onFillCircle(float cx, float cy, float r, int rgba) {
            ops.add("circle:" + cx + "," + cy + "," + r + "," + rgba);
        }

        @Override public void onStrokeLine(float x0, float y0, float x1, float y1, float w, int rgba) {
            ops.add("line:" + x0 + "," + y0 + "," + x1 + "," + y1 + "," + w + "," + rgba);
        }

        @Override public void onStrokePolylineStart(int n, float w, int rgba) { ops.add("poly:" + n + "," + w + "," + rgba); }

        @Override public void onStrokePolylinePoint(int i, float x, float y) { ops.add("pt" + i + ":" + x + "," + y); }

        @Override public void onStrokePolylineEnd() { ops.add("polyEnd"); }

        @Override public void onClipRect(float x, float y, float w, float h) {
            ops.add("clip:" + x + "," + y + "," + w + "," + h);
        }

        @Override public void onRestoreClip() { ops.add("restore"); }
    }
}
