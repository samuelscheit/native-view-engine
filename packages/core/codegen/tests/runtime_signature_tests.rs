//! Runtime ABI vtable & callback signatures (`AbiVtableDescriptor` / `AbiCallbackDescriptor`)
//! are validated at model construction through the unified `Ty::parse` + `validate::abi` pipeline,
//! ensuring every C and Dart binding resolves cleanly.

use nve_abi::schema::{
    AbiFileDescriptor, AbiMethodDescriptor, AbiParamDescriptor, AbiVtableDescriptor,
    runtime_abi_descriptor,
};
use nve_codegen::CodegenError;
use nve_codegen::abi::model::RuntimeAbiModel;
use nve_codegen::abi::spell::dart;

const fn method(
    name: &'static str,
    params: &'static [AbiParamDescriptor],
    return_type: &'static str,
) -> AbiMethodDescriptor {
    AbiMethodDescriptor {
        name,
        params,
        return_type,
        doc: None,
        optional: false,
    }
}

const fn param(name: &'static str, ty: &'static str) -> AbiParamDescriptor {
    AbiParamDescriptor { name, ty }
}

fn with_methods(
    methods: &'static [AbiMethodDescriptor],
) -> Result<RuntimeAbiModel<'static>, CodegenError> {
    let base = nve_codegen::abi::abi_model().expect("ABI model");
    let rt = runtime_abi_descriptor();
    let vtables: &'static [AbiVtableDescriptor] = Box::leak(Box::new([AbiVtableDescriptor {
        methods,
        ..rt.vtables[0]
    }]));
    let file: &'static AbiFileDescriptor = Box::leak(Box::new(AbiFileDescriptor {
        vtables,
        ..rt
    }));
    RuntimeAbiModel::new(file, &base)
}

#[test]
fn every_supported_abi_spelling_maps_to_dart_ffi() {
    static SLOTS: [AbiMethodDescriptor; 3] = [
        method(
            "scalars",
            &[
                param("b", "u8"),
                param("s", "u16"),
                param("h", "NveRuntimeHandle"),
                param("e", "*const NveMapEntry"),
                param("node", "NodeId"),
                param("req", "HostRequestId"),
            ],
            "bool",
        ),
        method("no_params", &[], "()"),
        method(
            "callback",
            &[
                param("cb", "Option<NveRuntimeWakeFn>"),
                param("get", "Option<NveGetInterfaceFn>"),
                param("out", "*mut f64"),
            ],
            "i32",
        ),
    ];
    let m = with_methods(&SLOTS).expect("valid signatures");
    assert_eq!(
        dart::fn_sig(
            m.all_aliases(),
            m.file.aliases,
            m.all_callbacks(),
            m.slots()[0].params,
            m.slots()[0].return_type,
            false,
        ),
        "ffi.Bool Function(ffi.Uint8 b, ffi.Uint16 s, NveRuntimeHandle h, ffi.Pointer<NveMapEntry> e, ffi.Uint32 node, ffi.Uint64 req)"
    );
    assert_eq!(
        dart::fn_sig(
            m.all_aliases(),
            m.file.aliases,
            m.all_callbacks(),
            m.slots()[0].params,
            m.slots()[0].return_type,
            true,
        ),
        "bool Function(int b, int s, NveRuntimeHandle h, ffi.Pointer<NveMapEntry> e, int node, int req)"
    );
    assert_eq!(
        dart::fn_sig(
            m.all_aliases(),
            m.file.aliases,
            m.all_callbacks(),
            m.slots()[1].params,
            m.slots()[1].return_type,
            false,
        ),
        "ffi.Void Function()"
    );
    assert_eq!(
        dart::fn_sig(
            m.all_aliases(),
            m.file.aliases,
            m.all_callbacks(),
            m.slots()[1].params,
            m.slots()[1].return_type,
            true,
        ),
        "void Function()"
    );
    assert_eq!(
        dart::fn_sig(
            m.all_aliases(),
            m.file.aliases,
            m.all_callbacks(),
            m.slots()[2].params,
            m.slots()[2].return_type,
            true,
        ),
        "int Function(ffi.Pointer<ffi.NativeFunction<NveRuntimeWakeNative>> cb, ffi.Pointer<ffi.NativeFunction<NveGetInterfaceNative>> get, ffi.Pointer<ffi.Double> out)"
    );
}

#[test]
fn unknown_or_malformed_signatures_are_refused() {
    static UNKNOWN_PARAM: [AbiMethodDescriptor; 1] = [method("a", &[param("m", "Mystery")], "i32")];
    static UNKNOWN_RET: [AbiMethodDescriptor; 1] = [method("b", &[], "Mystery")];
    static UNKNOWN_CALLBACK: [AbiMethodDescriptor; 1] =
        [method("c", &[param("f", "Option<NveUnknownFn>")], "()")];
    static OPT_SCALAR: [AbiMethodDescriptor; 1] =
        [method("d", &[param("x", "Option<u32>")], "()")];
    static UNIT_PARAM: [AbiMethodDescriptor; 1] = [method("e", &[param("x", "()")], "()")];
    static VOID_PARAM: [AbiMethodDescriptor; 1] = [method("g", &[param("x", "c_void")], "()")];
    static UNKNOWN_POINTEE: [AbiMethodDescriptor; 1] =
        [method("f", &[param("m", "*const *const Mystery")], "()")];
    static EMPTY_NAME: [AbiMethodDescriptor; 1] = [method("h", &[param("", "u32")], "()")];
    for (slots, why) in [
        (&UNKNOWN_PARAM, "uses `Mystery`"),
        (&UNKNOWN_RET, "uses `Mystery`"),
        (&UNKNOWN_CALLBACK, "only valid for declared callback typedefs"),
        (&OPT_SCALAR, "only valid for declared callback typedefs"),
        (&UNIT_PARAM, "only valid as a return type"),
        (&VOID_PARAM, "only valid behind a pointer"),
        (&UNKNOWN_POINTEE, "uses `Mystery`"),
        (&EMPTY_NAME, "must be a valid identifier"),
    ] {
        match with_methods(slots) {
            Err(CodegenError::Validation(m)) => {
                assert!(
                    m.contains(why) && m.contains(slots[0].name),
                    "{m}"
                )
            }
            other => panic!("{}: expected `{why}`, got {other:?}", slots[0].name),
        }
    }
}

#[test]
fn dart_runtime_bindings_check_every_slot() {
    let files = nve_codegen::generate_abi_files().expect("ABI files");
    let dart = &files
        .iter()
        .find(|f| f.relative_path.ends_with("nve_runtime_abi.generated.dart"))
        .expect("Dart runtime ABI")
        .content;
    let check = dart
        .split("bool runtimeApiSlotsComplete(")
        .nth(1)
        .expect("slots helper");
    let rt = runtime_abi_descriptor();
    assert_eq!(rt.vtables[0].methods.len(), 37);
    for s in rt.vtables[0].methods {
        let field = nve_codegen::abi::spell::camel(s.name);
        assert!(check.contains(&format!("a.{field} != ffi.nullptr")), "{field}");
    }
}
