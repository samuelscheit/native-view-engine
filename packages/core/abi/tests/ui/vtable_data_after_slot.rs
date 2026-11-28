use nve_abi::NveAbi;

#[repr(C)]
#[derive(NveAbi)]
#[nve_abi(vtable)]
struct BadDataAfterSlot {
    pub header: u32,
    pub ping: extern "C" fn(x: u32),
    pub trailing_data: u32,
}

fn main() {}
