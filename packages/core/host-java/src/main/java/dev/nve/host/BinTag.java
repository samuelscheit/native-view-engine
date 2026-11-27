package dev.nve.host;

/** {@code NveBinValue} tag (Rust {@code nve_abi::NveValueTag}). */
public enum BinTag {
    NULL, BOOL, I64, F64, STR, BYTES, ARRAY, MAP;

    /** Maximum container nesting depth (Rust {@code NVE_BIN_VALUE_MAX_DEPTH}). */
    public static final int MAX_DEPTH = 64;

    private static final BinTag[] BY_CODE = values();

    /** Wire byte of this tag. */
    public int code() {
        return ordinal();
    }

    /** Tag for wire byte {@code code}; throws {@link BinCodecException.Kind#INVALID_TAG} otherwise. */
    public static BinTag fromCode(int code) {
        if (code < 0 || code >= BY_CODE.length) {
            throw new BinCodecException(BinCodecException.Kind.INVALID_TAG, "tag " + code);
        }
        return BY_CODE[code];
    }
}
