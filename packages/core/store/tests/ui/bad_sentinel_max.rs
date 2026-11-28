use nve_store::NveTable;

#[derive(NveTable)]
#[nve_table(name = "ui.bad_sentinel_max", storage = "sparse")]
struct BadSentinelMax {
    #[effect(paint)]
    #[default(u32::MAX)]
    x: u32,
}

fn main() {}
