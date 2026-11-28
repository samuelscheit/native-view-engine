use nve_abi::ReprEnum;

#[repr(u8)]
#[derive(ReprEnum)]
enum TooBig {
    A = 256,
}

#[repr(u8)]
#[derive(ReprEnum)]
enum NegUnsigned {
    A = -1,
}

#[repr(u8)]
#[derive(ReprEnum)]
enum ImplicitOverflow {
    A = 255,
    B,
}

fn main() {}
