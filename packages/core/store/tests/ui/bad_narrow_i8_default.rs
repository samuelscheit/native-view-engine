use nve_store::NveTable;

#[derive(NveTable)]
#[nve_table(name = "ui.bad_narrow_i8", storage = "sparse")]
struct BadNarrowI8 {
    #[effect(paint)]
    #[bits(4)]
    #[default(8i8)]
    small_i: i8,
}

fn main() {}
