use nve_abi::NveAbi;

#[repr(C)]
#[derive(NveAbi)]
#[nve_abi(vtable)]
struct BadUnnamed {
    pub ping: extern "C" fn(u32),
}

fn main() {}
