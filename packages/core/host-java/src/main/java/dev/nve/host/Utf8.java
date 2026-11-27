package dev.nve.host;

import java.nio.ByteBuffer;

/**
 * Allocation-free strict UTF-8 (Rust {@code core::str::from_utf8} acceptance): validation over
 * {@code byte[]} / {@link ByteBuffer}, and encoding of Java strings that rejects lone surrogates
 * (a Rust {@code &str} can never hold one, so silently writing {@code ?} or U+FFFD would diverge).
 */
public final class Utf8 {
    private Utf8() {}

    /** Validates {@code b[offset .. offset + length)}; throws {@link IndexOutOfBoundsException} for a bad range. */
    public static boolean isValid(byte[] b, int offset, int length) {
        checkRange(offset, length, b.length);
        return valid(b, null, offset, offset + length);
    }

    /** Validates {@code buf[offset .. offset + length)} with absolute reads (position untouched). */
    public static boolean isValid(ByteBuffer buf, int offset, int length) {
        checkRange(offset, length, buf.limit());
        return valid(null, buf, offset, offset + length);
    }

    private static void checkRange(int offset, int length, int limit) {
        if (offset < 0 || length < 0 || length > limit - offset) {
            throw new IndexOutOfBoundsException("range [" + offset + ", +" + length + ") of " + limit);
        }
    }

    /** One loop for both sources: exactly one of {@code arr} / {@code buf} is non-null. */
    private static boolean valid(byte[] arr, ByteBuffer buf, int start, int end) {
        int i = start;
        while (i < end) {
            final int b0 = (arr != null ? arr[i] : buf.get(i)) & 0xFF;
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
            final int b1 = (arr != null ? arr[i + 1] : buf.get(i + 1)) & 0xFF;
            if (b1 < lo || b1 > hi) return false;
            for (int k = 2; k <= need; k++) {
                if (((arr != null ? arr[i + k] : buf.get(i + k)) & 0xC0) != 0x80) return false;
            }
            i += need + 1;
        }
        return true;
    }

    /** UTF-8 byte length of {@code s}; throws {@link BinCodecException.Kind#INVALID_UTF8} on a lone surrogate. */
    public static int encodedLength(CharSequence s) {
        long len = 0;
        for (int i = 0; i < s.length(); i++) {
            final char c = s.charAt(i);
            if (c < 0x80) {
                len += 1;
            } else if (c < 0x800) {
                len += 2;
            } else if (Character.isSurrogate(c)) {
                requirePair(s, i);
                len += 4;
                i++;
            } else {
                len += 3;
            }
            if (len > Integer.MAX_VALUE) throw new BinCodecException(BinCodecException.Kind.TOO_LARGE, "over " + len + " bytes");
        }
        return (int) len;
    }

    /**
     * Encodes {@code s} into {@code out[at ..)} (room for {@link #encodedLength} bytes required) and
     * returns the end offset; throws on a lone surrogate.
     */
    public static int encode(CharSequence s, byte[] out, int at) {
        int o = at;
        for (int i = 0; i < s.length(); i += Character.isHighSurrogate(s.charAt(i)) ? 2 : 1) {
            o = encodeAt(s, i, out, o);
        }
        return o;
    }

    /**
     * Encodes the code point starting at {@code s[i]} (a surrogate pair consumes {@code s[i + 1]} too)
     * into {@code out[o ..)} and returns the end offset; throws on a lone surrogate.
     */
    static int encodeAt(CharSequence s, int i, byte[] out, int o) {
        final char c = s.charAt(i);
        if (c < 0x80) {
            out[o] = (byte) c;
            return o + 1;
        }
        if (c < 0x800) {
            out[o] = (byte) (0xC0 | c >>> 6);
            out[o + 1] = (byte) (0x80 | c & 0x3F);
            return o + 2;
        }
        if (Character.isSurrogate(c)) {
            requirePair(s, i);
            final int cp = Character.toCodePoint(c, s.charAt(i + 1));
            out[o] = (byte) (0xF0 | cp >>> 18);
            out[o + 1] = (byte) (0x80 | (cp >>> 12) & 0x3F);
            out[o + 2] = (byte) (0x80 | (cp >>> 6) & 0x3F);
            out[o + 3] = (byte) (0x80 | cp & 0x3F);
            return o + 4;
        }
        out[o] = (byte) (0xE0 | c >>> 12);
        out[o + 1] = (byte) (0x80 | (c >>> 6) & 0x3F);
        out[o + 2] = (byte) (0x80 | c & 0x3F);
        return o + 3;
    }

    /**
     * {@code s} as a well-formed UTF-16 string: every lone surrogate becomes U+FFFD (what the
     * runtime's UTF-8 boundary would do). Returns {@code s.toString()} itself when it is already
     * well-formed, so text fields can compare the result to their content to see whether a
     * replacement happened. Platform text widgets can hold lone surrogates (IME / paste); call this
     * before {@link EventRingWriter#text} and keep the widget in sync with the result.
     */
    public static String wellFormed(CharSequence s) {
        final int n = s.length();
        int i = 0;
        while (i < n && !loneSurrogateAt(s, i)) i += pairAt(s, i) ? 2 : 1;
        if (i == n) return s.toString();
        final StringBuilder out = new StringBuilder(n).append(s, 0, i);
        while (i < n) {
            if (loneSurrogateAt(s, i)) {
                out.append('\uFFFD');
                i++;
            } else {
                final int step = pairAt(s, i) ? 2 : 1;
                out.append(s, i, i + step);
                i += step;
            }
        }
        return out.toString();
    }

    private static boolean pairAt(CharSequence s, int i) {
        return Character.isHighSurrogate(s.charAt(i)) && i + 1 < s.length() && Character.isLowSurrogate(s.charAt(i + 1));
    }

    private static boolean loneSurrogateAt(CharSequence s, int i) {
        return Character.isSurrogate(s.charAt(i)) && !pairAt(s, i);
    }

    /** Throws unless {@code s[i]} is a high surrogate followed by a low surrogate. */
    static void requirePair(CharSequence s, int i) {
        if (!Character.isHighSurrogate(s.charAt(i)) || i + 1 >= s.length() || !Character.isLowSurrogate(s.charAt(i + 1))) {
            throw new BinCodecException(BinCodecException.Kind.INVALID_UTF8, "lone surrogate at index " + i);
        }
    }
}
