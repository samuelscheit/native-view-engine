use nve_store::NveTable;

#[derive(NveTable)]
#[nve_table(name = "ui.bad_too_wide", storage = "sparse")]
struct BadTooWide {
    #[effect(paint)] w00: u64, #[effect(paint)] w01: u64, #[effect(paint)] w02: u64,
    #[effect(paint)] w03: u64, #[effect(paint)] w04: u64, #[effect(paint)] w05: u64,
    #[effect(paint)] w06: u64, #[effect(paint)] w07: u64, #[effect(paint)] w08: u64,
    #[effect(paint)] w09: u64, #[effect(paint)] w10: u64, #[effect(paint)] w11: u64,
    #[effect(paint)] w12: u64, #[effect(paint)] w13: u64, #[effect(paint)] w14: u64,
    #[effect(paint)] w15: u64, #[effect(paint)] w16: u64,
}

fn main() {}
