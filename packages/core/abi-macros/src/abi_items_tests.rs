use super::{expand_aliases_tokens, expand_callbacks_tokens, expand_consts_tokens};
use crate::tok::lex;

#[test]
fn abi_consts_emits_free_and_associated_consts_with_doc_trimming() {
    let free = expand_consts_tokens(&lex(r#"
        pub(crate) const DEMO_CONSTS;

        #[doc = " Line one."]
        #[doc = " Line two."]
        #[nve_abi(hex)]
        pub const NVE_MAGIC: u32 = u32::from_le_bytes(*b"NVE2");

        #[doc = " Max depth."]
        pub const NVE_MAX_DEPTH: usize = 64;

        #[nve_abi(name = "NVE_ROOT_NODE_ID")]
        pub const ROOT_NODE_ID: NodeId = NodeId::ROOT;

        #[nve_abi(hex)]
        pub const NVE_TABLE_X: TableKey = Key64::of("x");
    "#))
    .expect("free consts");

    assert!(free.contains("pub const NVE_MAGIC: u32 ="));
    assert!(free.contains("doc: ::core::option::Option::Some(\"Line one.\\nLine two.\")"));
    assert!(free.contains("hex: true"));
    assert!(free.contains("name: \"NVE_ROOT_NODE_ID\", ty: \"u32\", value: (ROOT_NODE_ID).raw() as u64"));
    assert!(free.contains("name: \"NVE_TABLE_X\", ty: \"u64\", value: (NVE_TABLE_X).0"));
    assert!(free.contains("(NVE_MAX_DEPTH as u64) <= (::core::primitive::u32::MAX as u64)"));

    let assoc = expand_consts_tokens(&lex(r#"
        pub(crate) const FLAG_CONSTS (impl EventFlags, prefix = "NVE_EVENT_FLAG_");

        #[doc = " Meta."]
        #[nve_abi(hex)]
        pub const MOD_META: Self = Self(1 << 0);

        #[nve_abi(skip)]
        pub const MOD_MASK: Self = Self(0x1F);

        pub const BUTTON_SHIFT: u32 = 8;
    "#))
    .expect("associated consts");

    assert!(assoc.contains("impl EventFlags {"));
    assert!(assoc.contains("pub const MOD_MASK: Self ="));
    assert!(!assoc.contains("NVE_EVENT_FLAG_MOD_MASK"));
    assert!(assoc.contains("name: \"NVE_EVENT_FLAG_MOD_META\", ty: \"u32\", value: (EventFlags::MOD_META).raw() as u64, hex: true"));
    assert!(assoc.contains("name: \"NVE_EVENT_FLAG_BUTTON_SHIFT\", ty: \"u32\", value: (EventFlags::BUTTON_SHIFT) as u64, hex: false"));
}

#[test]
fn abi_aliases_and_callbacks_emit_descriptors_and_reject_bad_sigs() {
    let aliases = expand_aliases_tokens(&lex(r#"
        pub(crate) const ALIASES;

        #[doc = " Opaque handle."]
        pub type NveAppHandle = *mut c_void;

        #[doc = " Table key."]
        #[nve_abi(name = "NveTableKey", target = "u64")]
        pub type TableKey = Key64;
    "#))
    .expect("aliases");

    assert!(aliases.contains("name: \"NveAppHandle\", rust_name: ::core::option::Option::None, target: \"*mut c_void\""));
    assert!(aliases.contains("name: \"NveTableKey\", rust_name: ::core::option::Option::Some(\"TableKey\"), target: \"u64\""));

    let cbs = expand_callbacks_tokens(&lex(r#"
        pub(crate) const CBS;

        #[doc = " Wake callback."]
        pub type NveRuntimeWakeFn = unsafe extern "C" fn(ctx: *mut c_void);

        pub type NveGetInterfaceFn = unsafe extern "C" fn(interface_id: InterfaceId) -> *const c_void;
    "#))
    .expect("callbacks");

    assert!(cbs.contains("name: \"NveRuntimeWakeFn\""));
    assert!(cbs.contains("name: \"ctx\", ty: \"*mut c_void\""));
    assert!(cbs.contains("return_type: \"()\""));
    assert!(cbs.contains("name: \"interface_id\", ty: \"InterfaceId\""));
    assert!(cbs.contains("return_type: \"*const c_void\""));

    let bad_opt = expand_callbacks_tokens(&lex(r#"
        pub(crate) const BAD;
        pub type BadFn = Option<unsafe extern "C" fn(x: u32)>;
    "#))
    .unwrap_err();
    assert!(bad_opt.contains("Option"), "{bad_opt}");

    let bad_unnamed = expand_callbacks_tokens(&lex(r#"
        pub(crate) const BAD;
        pub type BadFn = unsafe extern "C" fn(u32);
    "#))
    .unwrap_err();
    assert!(bad_unnamed.contains("named"), "{bad_unnamed}");
}
