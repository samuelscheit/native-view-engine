use nve_abi::NveAbi;

#[repr(C, packed)]
#[derive(NveAbi)]
struct PackedStruct {
    pub a: u8,
    pub b: u32,
}

fn main() {}
