use nve_store::NveTable;

#[derive(NveTable)]
#[nve_table(name = "ui.bad_explicit_collision", storage = "sparse", inherited)]
struct BadExplicitCollision {
    #[effect(paint)]
    explicit_mask: u32,
}

fn main() {}
