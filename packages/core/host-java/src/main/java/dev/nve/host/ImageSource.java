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
}
