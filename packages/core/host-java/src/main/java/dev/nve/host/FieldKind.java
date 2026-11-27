package dev.nve.host;

/** How a table field's bits are interpreted (Rust {@code nve_abi::table::FieldKind}, by ordinal). */
public enum FieldKind {
    U32, I32, F32, BOOL, ENUM, COLOR, LENGTH, STR, RESOURCE, NODE
}
