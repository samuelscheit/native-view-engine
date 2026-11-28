use nve_abi::NveAbi;

#[repr(C)]
#[derive(NveAbi)]
#[nve_abi(vtable)]
struct BadVariadic {
    pub log: unsafe extern "C" fn(x: u32, ...),
}

fn main() {}
