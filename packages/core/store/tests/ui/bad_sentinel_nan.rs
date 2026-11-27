use nve_store::NveTable;

#[derive(NveTable)]
#[nve_table(name = "ui.bad_sentinel_nan", storage = "sparse")]
struct BadSentinelNan {
    #[effect(paint)]
    #[default(f32::NAN)]
    x: f32,
}

fn main() {}
