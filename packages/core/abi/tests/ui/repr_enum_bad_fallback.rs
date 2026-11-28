use nve_abi::ReprEnum;

#[repr(u8)]
#[derive(ReprEnum)]
#[repr_enum(fallback = Missing)]
enum BadFallback {
    A = 0,
}

fn main() {}
