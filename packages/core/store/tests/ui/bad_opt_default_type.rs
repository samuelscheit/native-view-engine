use nve_store::NveTable;

#[derive(NveTable)]
#[nve_table(name = "ui.bad_opt_def_ty", storage = "sparse")]
struct BadOptDefTy {
    #[effect(paint)]
    #[default(Some(2.5))]
    x: Option<u32>,
}

fn main() {}
