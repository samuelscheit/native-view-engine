use nve_store::NveTable;

#[derive(Debug, Clone, Copy)]
struct NotAnEvent;

#[derive(NveTable)]
#[nve_table(name = "test.not_ev", events = NotAnEvent)]
struct BadEventsTable {
    #[effect(paint)]
    pub val: u32,
}

fn main() {}
