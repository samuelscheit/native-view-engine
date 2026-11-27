package dev.nve.host;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;

/**
 * Decoded {@code nve.res.image_src} resource ({@code nve_components::ImageSrc} wire bytes): a URL /
 * URI, or inline encoded image bytes (PNG / WebP / JPEG). Bytes are a zero-copy slice of the
 * resource buffer, valid while the scene that published it is acquired.
 */
public final class ImageSource {
    /** Resource kind of image sources. */
    public static final Key64 KIND = Key64.of("nve.res.image_src");

    private static final int TAG_URL = 0;
    private static final int TAG_ENCODED = 1;

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
     * one well-formed image source.
     */
    public static ImageSource decode(ByteBuffer bytes) {
        final ByteBuffer b = bytes.duplicate().order(ByteOrder.LITTLE_ENDIAN);
        if (b.remaining() < 5) throw new BinCodecException(BinCodecException.Kind.TRUNCATED, "image source header");
        final int tag = b.get() & 0xFF;
        final long len = b.getInt() & 0xFFFFFFFFL;
        if (len > b.remaining()) {
            throw new BinCodecException(BinCodecException.Kind.TRUNCATED, "image source payload " + len + " > " + b.remaining());
        }
        if (len < b.remaining()) {
            throw new BinCodecException(BinCodecException.Kind.TRAILING_BYTES, (b.remaining() - len) + " bytes after image source");
        }
        final ByteBuffer payload = b.slice();
        if (tag == TAG_ENCODED) return new ImageSource(null, payload);
        if (tag != TAG_URL) throw new BinCodecException(BinCodecException.Kind.INVALID_TAG, "image source tag " + tag);
        if (!Utf8.isValid(payload, payload.position(), payload.remaining())) {
            throw new BinCodecException(BinCodecException.Kind.INVALID_UTF8, "image source url");
        }
        final byte[] utf8 = new byte[payload.remaining()];
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
        final int comma = url.indexOf(',');
        if (!url.startsWith("data:image/") || comma < 0 || !url.substring(0, comma).endsWith(";base64")) return null;
        return base64(url, comma + 1);
    }

    /** Strict RFC 4648 base64 of {@code s[from ..)}, or {@code null}. */
    static byte[] base64(String s, int from) {
        final byte[] out = new byte[(s.length() - from) / 4 * 3];
        int n = 0;
        int quad = 0;
        int bits = 0;
        int pad = 0;
        for (int i = from; i < s.length(); i++) {
            final char c = s.charAt(i);
            if (c == ' ' || c == '\n' || c == '\r' || c == '\t') continue;
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
                if (n + 3 - pad > out.length) return null;
                out[n++] = (byte) (bits >> 16);
                if (pad < 2) out[n++] = (byte) (bits >> 8);
                if (pad < 1) out[n++] = (byte) bits;
                if (pad > 0 && i + 1 < s.length() && s.substring(i + 1).trim().length() > 0) return null;
                quad = 0;
                bits = 0;
            }
        }
        return quad == 0 ? java.util.Arrays.copyOf(out, n) : null;
    }
}
