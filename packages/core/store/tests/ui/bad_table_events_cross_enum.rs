use nve_abi::EventEnum;
use nve_store::{NodeId, NodeStore, NveTable};

#[derive(Debug, Clone, Copy, PartialEq, Eq, EventEnum)]
#[repr(u8)]
enum EventA {
    Click = 0,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, EventEnum)]
#[repr(u8)]
enum EventB {
    Hover = 0,
}

#[derive(NveTable)]
#[nve_table(name = "test.cross_ev", events = EventA)]
struct TableA {
    #[effect(paint)]
    pub val: u32,
}

fn main() {
    let mut store = NodeStore::new();
    let mut tx = store.tx();
    tx.edit::<TableA>(NodeId::ROOT).listen(EventB::Hover);
}
