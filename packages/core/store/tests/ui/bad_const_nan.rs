use nve_store::NveTable;

const BAD_SENTINEL: f32 = f32::NAN;

#[derive(NveTable)]
#[nve_table(name = "ui.bad_const_nan", storage = "sparse")]
struct BadConstNan {
    #[effect(paint)]
    #[default(BAD_SENTINEL)]
    x: f32,
}

fn main() {}
