use nve_store::{NveTable, ResourceId};

#[derive(NveTable)]
#[nve_table(name = "ui.bad_opt_res", storage = "sparse")]
struct BadOptRes {
    #[effect(paint)]
    raw: Option<ResourceId>,
}

fn main() {}
