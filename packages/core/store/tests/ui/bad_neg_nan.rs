use nve_store::NveTable;

#[derive(NveTable)]
#[nve_table(name = "ui.bad_neg_nan", storage = "sparse")]
struct BadNegNan {
    #[effect(paint)]
    #[default(-f32::NAN)]
    x: f32,
}

fn main() {}
