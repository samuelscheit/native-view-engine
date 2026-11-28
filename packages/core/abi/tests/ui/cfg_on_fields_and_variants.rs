use nve_abi::{NveAbi, ReprEnum};

#[repr(C)]
#[derive(NveAbi)]
struct CfgField {
    #[cfg(all())]
    pub x: u32,
}

#[repr(u8)]
#[derive(NveAbi)]
enum CfgAbiVariant {
    #[cfg(all())]
    A = 0,
}

#[repr(u8)]
#[derive(ReprEnum)]
enum CfgReprVariant {
    #[cfg(all())]
    A = 0,
}

fn main() {}
