use nve_abi::NveAbi;

#[repr(C)]
#[derive(NveAbi)]
#[nve_abi(vtable)]
struct BadNestedOption {
    pub ping: Option<Option<extern "C" fn(x: u32)>>,
}

fn main() {}
