use nve_store::{NveTable, ResourceId};

#[derive(NveTable)]
#[nve_table(name = "ui.bad_res", storage = "sparse")]
struct BadRes {
    #[effect(paint)]
    raw: ResourceId,
}

fn main() {}
