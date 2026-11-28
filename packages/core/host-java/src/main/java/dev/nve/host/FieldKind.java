package dev.nve.host;

/** How a table field's bits are interpreted (Rust {@code nve_abi::table::FieldKind}, by ordinal). */
public enum FieldKind {
    U32, I32, F32, BOOL, ENUM, COLOR, LENGTH, STR, RESOURCE, NODE, U64, I64, F64, GRID_LINE;

    /** {@code true} when a field of this kind spans two consecutive 32-bit row words. */
    public boolean isTwoWord() {
        return this == STR || this == U64 || this == I64 || this == F64;
    }
}
