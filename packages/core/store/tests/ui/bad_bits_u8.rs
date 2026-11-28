use nve_store::NveTable;

#[derive(NveTable)]
#[nve_table(name = "ui.bad_bits_u8", storage = "sparse")]
struct BadBitsU8 {
    #[effect(paint)]
    #[bits(8)]
    x: u8,
}

fn main() {}
