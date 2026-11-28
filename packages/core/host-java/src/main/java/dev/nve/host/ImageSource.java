package dev.nve.host;

import java.nio.Buffer;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;

/**
 * Decoded {@code nve.res.image_src} resource ({@code nve_tables::ImageSrc} {@code FlatResource}
 * 4-byte-aligned word buffer: {@code u32 tag | u32 byte_len | payload zero-padded to 4 B}): a URL /
 * URI, or inline encoded image bytes (PNG / WebP / JPEG). Bytes are a zero-copy slice of the
 * resource buffer, valid while the scene that published it is acquired.
 */
public final class ImageSource {
    /** Resource kind of image sources. */
    public static final Key64 KIND = Key64.of("nve.res.image_src");

    private static final int TAG_URL = 0;
    private static final int TAG_ENCODED = 1;
    private static final int MAX_URL_BYTES = 256 * 1024;
    private static final int MAX_IMAGE_BYTES = 32 * 1024 * 1024;

    /** URL / URI, or {@code null} for inline bytes. */
    public final String url;
    /** Inline encoded image bytes, or {@code null} for a URL. */
    public final ByteBuffer encoded;

    private ImageSource(String url, ByteBuffer encoded) {
        this.url = url;
        this.encoded = encoded;
    }

    /**
     * Decodes {@code bytes[position .. limit)}; throws {@link BinCodecException} unless it is exactly
     * one well-formed 4-byte-aligned {@code FlatResource} image source.
     */
    public static ImageSource decode(ByteBuffer bytes) {
        final ByteBuffer b = bytes.duplicate().order(ByteOrder.LITTLE_ENDIAN);
        if (b.remaining() < 8) {
            throw new BinCodecException(BinCodecException.Kind.TRUNCATED, "image source header");
        }
        final int tag = b.getInt();
        final long len = b.getInt() & 0xFFFFFFFFL;
        final long padded = (len + 3L) & ~3L;
        if (padded > b.remaining()) {
            throw new BinCodecException(BinCodecException.Kind.TRUNCATED, "image source payload " + len + " > " + b.remaining());
        }
        if (padded < b.remaining()) {
            throw new BinCodecException(BinCodecException.Kind.TRAILING_BYTES, (b.remaining() - padded) + " bytes after image source");
        }
        final int n = (int) len;
        final int base = b.position();
        for (int i = n; i < (int) padded; i++) {
            if (b.get(base + i) != 0) {
                throw new BinCodecException(BinCodecException.Kind.TRAILING_BYTES, "non-zero image source padding byte");
            }
        }
        ((Buffer) b).limit(base + n);
        final ByteBuffer payload = b.slice();
        if (tag == TAG_ENCODED) {
            if (n > MAX_IMAGE_BYTES) {
                throw new BinCodecException(BinCodecException.Kind.TOO_LARGE, "image source encoded bytes " + n);
            }
            return new ImageSource(null, payload);
        }
        if (tag != TAG_URL) {
            throw new BinCodecException(BinCodecException.Kind.INVALID_TAG, "image source tag " + tag);
        }
        if (n > MAX_URL_BYTES) {
            throw new BinCodecException(BinCodecException.Kind.TOO_LARGE, "image source url bytes " + n);
        }
        if (!Utf8.isValid(payload, payload.position(), payload.remaining())) {
            throw new BinCodecException(BinCodecException.Kind.INVALID_UTF8, "image source url");
        }
        final byte[] utf8 = new byte[n];
        payload.get(utf8);
        return new ImageSource(new String(utf8, StandardCharsets.UTF_8), null);
    }

    /**
     * The encoded image bytes this source carries itself: a copy of the inline bytes, or the
     * payload of a {@code data:image/*;base64,} URL (strict RFC 4648; whitespace ignored).
     * {@code null} for URLs the host must load ({@code asset://}, {@code file://}, network) and
     * for a malformed data URL. Shared by every Java / Kotlin host (no platform Base64: Android
     * API 24 has no {@code java.util.Base64}).
     */
    public byte[] bytes() {
        if (encoded != null) {
            final byte[] out = new byte[encoded.remaining()];
            encoded.duplicate().get(out);
            return out;
        }
        // RFC 2397: the scheme, media type, and `;base64` token are case-insensitive.
        final int comma = url.indexOf(',');
        final String base64 = ";base64";
        if (comma < base64.length() || !url.regionMatches(true, 0, "data:image/", 0, "data:image/".length())
                || !url.regionMatches(true, comma - base64.length(), base64, 0, base64.length())) {
            return null;
        }
        return base64(url, comma + 1);
    }

    private static boolean isSpace(char c) {
        return c == ' ' || c == '\n' || c == '\r' || c == '\t';
    }

    /**
     * Strict RFC 4648 base64 of {@code s[from ..)} (whitespace ignored), or {@code null}: padding
     * required and only at the end, and the unused low bits of the last quantum must be zero
     * (canonical encoding). The output array is sized exactly up front (no copy).
     */
    static byte[] base64(String s, int from) {
        int chars = 0;
        int padded = 0;
        for (int i = from; i < s.length(); i++) {
            final char c = s.charAt(i);
            if (isSpace(c)) continue;
            chars++;
            if (c == '=') padded++;
        }
        if (chars % 4 != 0 || padded > 2) return null;
        final byte[] out = new byte[chars / 4 * 3 - padded];
        int n = 0;
        int quad = 0;
        int bits = 0;
        int pad = 0;
        for (int i = from; i < s.length(); i++) {
            final char c = s.charAt(i);
            if (isSpace(c)) continue;
            final int v;
            if (c >= 'A' && c <= 'Z') v = c - 'A';
            else if (c >= 'a' && c <= 'z') v = c - 'a' + 26;
            else if (c >= '0' && c <= '9') v = c - '0' + 52;
            else if (c == '+') v = 62;
            else if (c == '/') v = 63;
            else if (c == '=' && quad >= 2) v = -1;
            else return null;
            if (v < 0) {
                pad++;
            } else if (pad > 0) {
                return null;  // data after padding
            }
            bits = bits << 6 | Math.max(v, 0);
            if (++quad == 4) {
                // Non-canonical: bits the padding drops must be zero.
                if ((pad == 1 && (bits & 0xFF) != 0) || (pad == 2 && (bits & 0xFFFF) != 0)) return null;
                out[n++] = (byte) (bits >> 16);
                if (pad < 2) out[n++] = (byte) (bits >> 8);
                if (pad < 1) out[n++] = (byte) bits;
                quad = 0;
                bits = 0;
            }
        }
        return n == out.length ? out : null;
    }
}
