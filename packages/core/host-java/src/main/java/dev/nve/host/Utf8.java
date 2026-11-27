package dev.nve.host;

import java.nio.ByteBuffer;

/** Allocation-free strict UTF-8 validation (Rust {@code core::str::from_utf8} acceptance). */
public final class Utf8 {
    private Utf8() {}

    public static boolean isValid(byte[] b, int offset, int length) {
        return isValid(ByteBuffer.wrap(b), offset, length);
    }

    /** Validates {@code buf[offset .. offset + length)} using absolute reads (position untouched). */
    public static boolean isValid(ByteBuffer buf, int offset, int length) {
        int i = offset;
        final int end = offset + length;
        while (i < end) {
            final int b0 = buf.get(i) & 0xFF;
            if (b0 < 0x80) {
                i++;
                continue;
            }
            final int need;
            int lo = 0x80;
            int hi = 0xBF;
            if (b0 >= 0xC2 && b0 <= 0xDF) {
                need = 1;
            } else if (b0 >= 0xE0 && b0 <= 0xEF) {
                need = 2;
                if (b0 == 0xE0) lo = 0xA0;
                if (b0 == 0xED) hi = 0x9F;
            } else if (b0 >= 0xF0 && b0 <= 0xF4) {
                need = 3;
                if (b0 == 0xF0) lo = 0x90;
                if (b0 == 0xF4) hi = 0x8F;
            } else {
                return false;
            }
            if (need >= end - i) return false;
            final int b1 = buf.get(i + 1) & 0xFF;
            if (b1 < lo || b1 > hi) return false;
            for (int k = 2; k <= need; k++) {
                if ((buf.get(i + k) & 0xC0) != 0x80) return false;
            }
            i += need + 1;
        }
        return true;
    }
}
