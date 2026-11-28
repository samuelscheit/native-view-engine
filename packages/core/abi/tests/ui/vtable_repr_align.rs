use nve_abi::NveAbi;

#[repr(C, align(16))]
#[derive(NveAbi)]
#[nve_abi(vtable)]
struct AlignedVtable {
    pub ping: extern "C" fn(x: u32),
}

#[repr(u8, align(4))]
#[derive(NveAbi)]
enum AlignedEnum {
    A = 0,
}

fn main() {}
