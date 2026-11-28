package dev.nve.host;

/** Optionality and wire-bit encoding strategy of a table field (Rust {@code nve_abi::table::OptEncoding}, by ordinal). */
public enum OptEncoding {
    REQUIRED, ZERO_NICHE, BIT0_PRESENT, PLUS_ONE, TAG_XOR, WIDE_BIT
}
