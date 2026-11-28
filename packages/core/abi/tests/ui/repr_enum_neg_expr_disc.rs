use nve_abi::ReprEnum;

#[repr(i32)]
#[derive(ReprEnum)]
enum NegParen {
    A = -(1),
}

#[repr(i32)]
#[derive(ReprEnum)]
enum NegBinOp {
    A = -1 + 2,
}

#[repr(i32)]
#[derive(ReprEnum)]
enum NegUnsignedSuffix {
    A = -1u8,
}

fn main() {}
