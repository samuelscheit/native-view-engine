//! Runtime ABI signatures (`RUNTIME_CALLBACK_TYPEDEFS` / `RUNTIME_API_SLOTS`) are validated at model
//! construction: every C type spells a known scalar, record, handle, or callback typedef, so the
//! Dart bindings never fall back to an unresolved name.

use nve_abi::RuntimeFnSignature;
use nve_abi::schema::{AbiFileDescriptor, runtime_abi_descriptor};
use nve_codegen::CodegenError;
use nve_codegen::abi::model::RuntimeAbiModel;

static RUNTIME: AbiFileDescriptor = runtime_abi_descriptor();

const fn sig(name: &'static str, ret: &'static str, params: &'static str) -> RuntimeFnSignature {
    RuntimeFnSignature { name, ret, params, doc: "" }
}

fn model(slots: &'static [RuntimeFnSignature]) -> Result<RuntimeAbiModel<'static>, CodegenError> {
    let base = nve_codegen::abi::abi_model().expect("ABI model");
    RuntimeAbiModel::new(&RUNTIME, &base, nve_abi::RUNTIME_CALLBACK_TYPEDEFS, slots)
}

#[test]
fn every_supported_c_spelling_maps_to_dart_ffi() {
    static SLOTS: [RuntimeFnSignature; 3] = [
        sig(
            "scalars",
            "bool",
            "char c, int8_t a, int16_t b, const char* const* names, NveRuntimeHandle h, const struct NveMapEntry* e",
        ),
        sig("no_params", "void", "void"),
        sig("callback", "int32_t", "NveRuntimeWakeFn cb, NveGetInterfaceFn get, double* out"),
    ];
    let m = model(&SLOTS).expect("valid signatures");
    assert_eq!(
        m.slots[0].dart_native,
        "ffi.Bool Function(ffi.Char c, ffi.Int8 a, ffi.Int16 b, ffi.Pointer<ffi.Pointer<ffi.Char>> names, \
         NveRuntimeHandle h, ffi.Pointer<NveMapEntry> e)"
    );
    assert_eq!(
        m.slots[0].dart,
        "bool Function(int c, int a, int b, ffi.Pointer<ffi.Pointer<ffi.Char>> names, NveRuntimeHandle h, \
         ffi.Pointer<NveMapEntry> e)"
    );
    assert_eq!(m.slots[1].dart_native, "ffi.Void Function()");
    assert_eq!(m.slots[1].dart, "void Function()");
    assert_eq!(
        m.slots[2].dart,
        "int Function(ffi.Pointer<ffi.NativeFunction<NveRuntimeWakeNative>> cb, \
         ffi.Pointer<ffi.NativeFunction<NveGetInterfaceNative>> get, ffi.Pointer<ffi.Double> out)"
    );
}

#[test]
fn unknown_or_malformed_signatures_are_refused() {
    static UNKNOWN_PARAM: [RuntimeFnSignature; 1] = [sig("a", "int32_t", "Mystery m")];
    static UNKNOWN_RET: [RuntimeFnSignature; 1] = [sig("b", "Mystery", "void")];
    static UNKNOWN_CALLBACK: [RuntimeFnSignature; 1] = [sig("c", "void", "NveUnknownFn f")];
    static UNNAMED: [RuntimeFnSignature; 1] = [sig("d", "void", "uint32_t")];
    static VOID_PARAM: [RuntimeFnSignature; 1] = [sig("e", "void", "void x")];
    static CONST_VOID_PARAM: [RuntimeFnSignature; 1] = [sig("g", "void", "const void x")];
    static UNKNOWN_POINTEE: [RuntimeFnSignature; 1] = [sig("f", "void", "const Mystery* const* m")];
    for (slots, why) in [
        (&UNKNOWN_PARAM, "unknown C type `Mystery`"),
        (&UNKNOWN_RET, "unknown C type `Mystery`"),
        (&UNKNOWN_CALLBACK, "unknown C type `NveUnknownFn`"),
        (&UNNAMED, "needs a type and a name"),
        (&VOID_PARAM, "is `void`"),
        (&CONST_VOID_PARAM, "is `void`"),
        (&UNKNOWN_POINTEE, "unknown C type `Mystery`"),
    ] {
        match model(slots) {
            Err(CodegenError::Validation(m)) => {
                assert!(m.contains(why) && m.contains(&format!("`{}`", slots[0].name)), "{m}")
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
    let check = dart.split("bool runtimeApiSlotsComplete(").nth(1).expect("slots helper");
    for s in nve_abi::RUNTIME_API_SLOTS {
        let field = nve_codegen::abi::spell::camel(s.name);
        assert!(check.contains(&format!("a.{field} != ffi.nullptr")), "{field}");
    }
}
