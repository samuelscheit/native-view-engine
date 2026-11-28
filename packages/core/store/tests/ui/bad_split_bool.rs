use nve_store::NveTable;

#[derive(NveTable)]
#[nve_table(name = "ui.bad_split_bool", storage = "sparse")]
struct BadSplitBool {
    #[effect(paint)]
    has_color: bool,
    #[effect(paint)]
    color: u32,
}

fn main() {}
