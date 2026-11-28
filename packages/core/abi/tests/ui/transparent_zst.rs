use nve_abi::NveAbi;

#[repr(transparent)]
#[derive(NveAbi)]
struct EmptyTransparent(pub ());

fn main() {}
