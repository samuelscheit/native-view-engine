nve_abi_macros::abi_consts! {
    const BAD_CONSTS;

    #[cfg(all())]
    pub const CFG_CONST: u32 = 1;
}

nve_abi_macros::abi_consts! {
    const BAD_CFG_ATTR_CONSTS;

    #[cfg_attr(any(), nve_abi(hex))]
    pub const CFG_ATTR_CONST: u32 = 2;
}

nve_abi_macros::abi_aliases! {
    const BAD_ALIASES;

    #[nve_abi(hex)]
    pub type BadAlias = u32;
}

nve_abi_macros::abi_aliases! {
    const BAD_CFG_ATTR_ALIASES;

    #[cfg_attr(any(), nve_abi(doc = "conditional"))]
    pub type CfgAttrAlias = u32;
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
