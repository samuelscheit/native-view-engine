use nve_abi::ReprEnum;

#[repr(u8)]
#[derive(ReprEnum)]
enum DupDisc {
    A = 1,
    B,
    C = 2,
}

fn main() {}
