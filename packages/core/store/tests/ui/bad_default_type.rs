use nve_store::NveTable;

#[derive(NveTable)]
#[nve_table(name = "ui.bad_def_ty", storage = "sparse")]
struct BadDefTy {
    #[effect(paint)]
    #[default(true)]
    x: u8,
}

fn main() {}
