use nve_store::NveTable;

#[derive(NveTable)]
#[nve_table(name = "ui.bad_inherited_events", storage = "sparse", inherited, events = DummyEv)]
struct BadInheritedEvents {
    #[effect(paint)]
    x: u32,
}

fn main() {}
