use nve_store::NveTable;

#[derive(NveTable)]
#[nve_table(name = "ui.bad_bits_opt", storage = "sparse")]
struct BadBitsOpt {
    #[effect(paint)]
    #[bits(4)]
    x: Option<u8>,
}

fn main() {}
