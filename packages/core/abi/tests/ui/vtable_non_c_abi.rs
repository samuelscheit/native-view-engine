use nve_abi::NveAbi;

#[repr(C)]
#[derive(NveAbi)]
#[nve_abi(vtable)]
struct BadBareFn {
    pub a: fn(x: u32),
}

#[repr(C)]
#[derive(NveAbi)]
#[nve_abi(vtable)]
struct BadRustAbi {
    pub b: extern "Rust" fn(x: u32),
}

fn main() {}
