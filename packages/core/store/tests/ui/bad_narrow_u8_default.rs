use nve_store::NveTable;

#[derive(NveTable)]
#[nve_table(name = "ui.bad_narrow_u8", storage = "sparse")]
struct BadNarrowU8 {
    #[effect(paint)]
    #[bits(4)]
    #[default(16u8)]
    small_u: u8,
}

fn main() {}
