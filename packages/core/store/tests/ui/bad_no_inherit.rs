use nve_store::NveTable;

#[derive(NveTable)]
#[nve_table(name = "ui.bad_no_inherit", storage = "sparse")]
struct BadNoInherit {
    #[effect(paint)]
    #[no_inherit]
    x: u32,
}

fn main() {}
