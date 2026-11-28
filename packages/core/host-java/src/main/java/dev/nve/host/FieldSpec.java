package dev.nve.host;

/**
 * One field of a table row: {@code bits} wide at {@code shift} inside {@code word} (two-word kinds
 * {@code STR}/{@code U64}/{@code I64}/{@code F64} span {@code word} and {@code word + 1};
 * {@link OptEncoding#WIDE_BIT} puts a 1-bit presence flag at {@code (word, shift)} and the full
 * 32-bit value in {@code valWord}).
 */
public final class FieldSpec {
    /** Sentinel for {@link #valWord} / {@link #explicitBit} when not applicable (Rust {@code NO_VAL_WORD}). */
    public static final int NO_WORD = 0xFF;

    public final int word;
    public final int shift;
    public final int bits;
    public final FieldKind kind;
    public final OptEncoding optEncoding;
    public final int valWord;
    public final int explicitBit;
    public final boolean inherits;

    /**
     * Full 8-argument descriptor matching generated {@code NveTables} / {@code NveModuleTables}.
     * Throws {@link IllegalArgumentException} unless every index, bit range, optionality encoding,
     * and inheritance flag is well-formed within a 32-word row.
     */
    public FieldSpec(
            int word, int shift, int bits, FieldKind kind,
            OptEncoding optEncoding, int valWord, int explicitBit, boolean inherits) {
        if (kind == null || optEncoding == null) {
            throw new IllegalArgumentException("null kind or optEncoding");
        }
        final int span = kind.isTwoWord() ? 2 : 1;
        // Range-check each operand first, then compare by subtraction so `shift + bits` / `word + span`
        // cannot overflow `int` and let a hostile spec through.
        if (bits < 1 || bits > 32 || shift < 0 || shift > 32 - bits || word < 0
                || word > TableReader.MAX_WORDS - span) {
            throw new IllegalArgumentException("invalid field spec word=" + word + " shift=" + shift + " bits=" + bits);
        }
        if (kind.isTwoWord() && (shift != 0 || bits != 32)) {
            throw new IllegalArgumentException("two-word field must be shift=0 bits=32");
        }
        if (optEncoding == OptEncoding.WIDE_BIT) {
            if (kind.isTwoWord() || bits != 1 || valWord < 0 || valWord >= TableReader.MAX_WORDS || valWord == word) {
                throw new IllegalArgumentException("invalid WIDE_BIT spec word=" + word + " bits=" + bits + " valWord=" + valWord);
            }
        } else if (valWord != NO_WORD) {
            throw new IllegalArgumentException("non-WIDE_BIT field must use valWord=0xFF, got " + valWord);
        }
        if (optEncoding == OptEncoding.BIT0_PRESENT && (kind.isTwoWord() || bits < 2)) {
            throw new IllegalArgumentException("BIT0_PRESENT requires bits >= 2, got " + bits);
        }
        if (optEncoding == OptEncoding.TAG_XOR) {
            final boolean ok = (kind == FieldKind.LENGTH && bits == 32) || (kind == FieldKind.GRID_LINE && bits == 16);
            if (!ok) throw new IllegalArgumentException("TAG_XOR requires 32-bit LENGTH or 16-bit GRID_LINE");
        }
        if (explicitBit != NO_WORD) {
            if (explicitBit < 0 || explicitBit >= 32 || word == 0 || valWord == 0) {
                throw new IllegalArgumentException("invalid explicitBit=" + explicitBit + " for word=" + word);
            }
        } else if (inherits) {
            throw new IllegalArgumentException("inherits requires an explicitBit");
        }
        this.word = word;
        this.shift = shift;
        this.bits = bits;
        this.kind = kind;
        this.optEncoding = optEncoding;
        this.valWord = valWord;
        this.explicitBit = explicitBit;
        this.inherits = inherits;
    }

    /** Required non-inherited field shorthand. */
    public FieldSpec(int word, int shift, int bits, FieldKind kind) {
        this(word, shift, bits, kind, OptEncoding.REQUIRED, NO_WORD, NO_WORD, false);
    }

    /** Whole-word required non-inherited field. */
    public static FieldSpec word(int word, FieldKind kind) {
        return new FieldSpec(word, 0, 32, kind);
    }

    public boolean isOptional() {
        return optEncoding != OptEncoding.REQUIRED;
    }

    public boolean hasExplicitBit() {
        return explicitBit != NO_WORD;
    }

    /** Bit in hidden word 0 ({@code __explicit}) for this field, or {@code 0} when not inherited. */
    public int explicitMask() {
        return explicitBit == NO_WORD ? 0 : 1 << explicitBit;
    }

    /** Niche mask for {@link OptEncoding#TAG_XOR} ({@code 0x3} for {@code LENGTH}, {@code 0xC000} for {@code GRID_LINE}). */
    int tagNicheMask() {
        return kind == FieldKind.GRID_LINE ? GridLine.TAG_NICHE : 0x3;
    }

    /** {@code true} when the field (and its {@code valWord} for {@code WIDE_BIT}) lies inside a row of {@code words} words. */
    public boolean fits(int words) {
        final int span = kind.isTwoWord() ? 2 : 1;
        if (word + span > words) return false;
        return optEncoding != OptEncoding.WIDE_BIT || valWord < words;
    }

    /** Raw field bits of {@code value} (unsigned, as raw int bits). */
    public int extract(int value) {
        return bits >= 32 ? value >>> shift : (value >>> shift) & ((1 << bits) - 1);
    }

    /** Raw field bits of {@code value}, sign-extended from {@code bits}. */
    public int extractSigned(int value) {
        return bits >= 32 ? value : (extract(value) << (32 - bits)) >> (32 - bits);
    }
}
