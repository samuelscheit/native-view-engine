use nve_store::NveTable;

#[derive(NveTable)]
#[nve_table(name = "ui.bad_bits_bool", storage = "sparse")]
struct BadBitsBool {
    #[effect(paint)]
    #[bits(1)]
    x: bool,
}

fn main() {}
