package dev.nve.host;

/** Malformed or truncated binary input (or an oversized value on write). */
public final class BinCodecException extends RuntimeException {
    private static final long serialVersionUID = 1L;

    public enum Kind {
        TRUNCATED, UNEXPECTED_TAG, INVALID_TAG, INVALID_BOOL, INVALID_UTF8, TOO_LARGE, MAX_DEPTH, TRAILING_BYTES,
        /**
         * {@link BinWriter#end} got a mark that is not an open container in the buffer, or
         * {@link BinReader#end} one that is not the reader's innermost open container.
         */
        INVALID_MARK
    }

    private final Kind kind;

    public BinCodecException(Kind kind, String detail) {
        super(kind + ": " + detail);
        this.kind = kind;
    }

    public Kind kind() {
        return kind;
    }
}
