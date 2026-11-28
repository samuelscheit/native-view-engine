use nve_abi::NveAbi;

#[repr(C)]
#[derive(NveAbi)]
#[nve_abi(vtable)]
struct InlineFnParam {
    pub register: extern "C" fn(cb: extern "C" fn(x: u32)),
}

#[repr(C)]
#[derive(NveAbi)]
#[nve_abi(vtable)]
struct DupParamSlot {
    pub ping: extern "C" fn(x: u32, x: u32),
}

#[repr(C)]
#[derive(NveAbi)]
#[nve_abi(vtable)]
struct WildcardParamSlot {
    pub ping: extern "C" fn(_: u32),
}

fn main() {}
