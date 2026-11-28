use nve_abi::{NveAbi, ReprEnum};

#[repr(C)]
#[derive(NveAbi)]
struct CfgField {
    #[cfg(unix)]
    pub x: u32,
}

#[repr(u8)]
#[derive(NveAbi)]
enum CfgAbiVariant {
    #[cfg(unix)]
    A = 0,
}

#[repr(u8)]
#[derive(ReprEnum)]
enum CfgReprVariant {
    #[cfg(unix)]
    A = 0,
}

fn main() {}
