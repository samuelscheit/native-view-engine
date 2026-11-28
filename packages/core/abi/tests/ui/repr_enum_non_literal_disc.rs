use nve_abi::ReprEnum;

#[repr(u8)]
#[derive(ReprEnum)]
enum ExprDisc {
    A = 1 << 0,
}

fn main() {}
