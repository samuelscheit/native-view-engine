package dev.nve.host;

/** One field of a table row: {@code bits} wide at {@code shift} inside {@code word} ({@code STR} spans two words). */
public final class FieldSpec {
    public final int word;
    public final int shift;
    public final int bits;
    public final FieldKind kind;

    /**
     * Throws {@link IllegalArgumentException} unless {@code bits} is in {@code 1..32},
     * {@code shift + bits <= 32}, and the field fits a 32-word row, so extraction never mis-shifts.
     */
    public FieldSpec(int word, int shift, int bits, FieldKind kind) {
        final int span = kind == FieldKind.STR ? 2 : 1;
        if (kind == null || bits < 1 || bits > 32 || shift < 0 || shift + bits > 32 || word < 0
                || word + span > TableReader.MAX_WORDS) {
            throw new IllegalArgumentException("invalid field spec word=" + word + " shift=" + shift + " bits=" + bits);
        }
        this.word = word;
        this.shift = shift;
        this.bits = bits;
        this.kind = kind;
    }

    /** Whole-word field. */
    public static FieldSpec word(int word, FieldKind kind) {
        return new FieldSpec(word, 0, 32, kind);
    }

    /** {@code true} when the field lies inside a row of {@code words} words. */
    public boolean fits(int words) {
        return word + (kind == FieldKind.STR ? 2 : 1) <= words;
    }

    /** Field bits of {@code value} (unsigned, as raw int bits). */
    public int extract(int value) {
        return bits >= 32 ? value >>> shift : (value >>> shift) & ((1 << bits) - 1);
    }

    /** Field bits of {@code value}, sign-extended from {@code bits}. */
    public int extractSigned(int value) {
        return bits >= 32 ? value : (extract(value) << (32 - bits)) >> (32 - bits);
    }
}
