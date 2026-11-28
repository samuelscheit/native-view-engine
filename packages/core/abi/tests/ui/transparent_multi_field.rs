use nve_abi::NveAbi;

#[repr(transparent)]
#[derive(NveAbi)]
struct BadTransparent(pub u32, pub ());

fn main() {}
