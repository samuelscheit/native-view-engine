use nve_abi::EventEnum;
use nve_store::NveTable;

#[derive(Debug, Clone, Copy, PartialEq, Eq, EventEnum)]
#[repr(u8)]
enum MyEv {
    Tap = 0,
}

#[derive(NveTable)]
#[nve_table(name = "test.ev_collision", events = MyEv)]
struct CollidingTable {
    #[effect(paint)]
    pub listeners: u32,
}

fn main() {}
