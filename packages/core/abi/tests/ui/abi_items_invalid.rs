nve_abi_macros::abi_consts! {
    const BAD_CONSTS;

    #[cfg(unix)]
    pub const CFG_CONST: u32 = 1;
}

nve_abi_macros::abi_aliases! {
    const BAD_ALIASES;

    #[nve_abi(hex)]
    pub type BadAlias = u32;
}

nve_abi_macros::abi_callbacks! {
    const BAD_CBS;

    pub type BadCb = Option<extern "C" fn(x: u32)>;
}

nve_abi_macros::abi_callbacks! {
    const BAD_DUP_PARAM_CBS;

    pub type DupParamCb = extern "C" fn(x: u32, x: u32);
}

fn main() {}
