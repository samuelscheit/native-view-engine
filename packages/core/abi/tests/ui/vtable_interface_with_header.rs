use nve_abi::NveAbi;

#[repr(C)]
#[derive(NveAbi)]
#[nve_abi(interface = nve_abi::NVE_IFACE_SURFACE)]
struct NveBadApi {
    pub header: u32,
    pub ping: extern "C" fn(x: u32),
}

fn main() {}
