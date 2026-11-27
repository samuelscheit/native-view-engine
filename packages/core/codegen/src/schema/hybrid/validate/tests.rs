use super::super::*;

fn param(name: &str, ty: HybridType) -> HybridParamSpec {
    HybridParamSpec {
        name: name.to_owned(),
        ty,
    }
}

fn method(name: &str, params: Vec<HybridParamSpec>) -> HybridMethodSpec {
    HybridMethodSpec {
        name: name.to_owned(),
        params,
        return_type: HybridType::Void,
        doc: None,
        has_self: true,
        optional: false,
    }
}

/// A module `Probe` with one method, plus a struct `Shape` with one `i32` field named `field`.
fn spec(method_name: &str, param_name: &str, field: &str) -> HybridFileSpec {
    HybridFileSpec {
        structs: vec![HybridStructSpec {
            name: "Shape".to_owned(),
            fields: vec![HybridFieldSpec {
                name: field.to_owned(),
                ty: HybridType::I32,
                doc: None,
                offset: None,
                offset_64: None,
            }],
            ..Default::default()
        }],
        modules: vec![HybridModuleSpec {
            name: "Probe".to_owned(),
            trait_name: "ProbeSpec".to_owned(),
            methods: vec![method(
                method_name,
                vec![param(param_name, HybridType::Struct("Shape".to_owned()))],
            )],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn error_of(spec: &HybridFileSpec) -> String {
    match spec.validate() {
        Err(CodegenError::Validation(message)) => message,
        other => panic!("expected a validation error, got {other:?}"),
    }
}

#[test]
fn rejects_identifiers_in_the_reserved_nve_binding_namespace() {
    for reserved in ["_nve_", "_nve_handle", "_NVE_out_err", "_Nve_x"] {
        let message = error_of(&spec(reserved, "shape", "x"));
        assert!(
            message.contains("'_nve_' prefix reserved"),
            "method {reserved}: {message}"
        );
        let message = error_of(&spec("run", reserved, "x"));
        assert!(
            message.contains(&format!("parameter identifier '{reserved}'")),
            "{message}"
        );
        let message = error_of(&spec("run", "shape", reserved));
        assert!(
            message.contains(&format!("struct field identifier '{reserved}'")),
            "{message}"
        );
    }
}

#[test]
fn allows_names_that_only_resemble_the_reserved_namespace() {
    for allowed in [
        "_foo",
        "nve_foo",
        "_nvelope",
        "nve_",
        "foo_nve_bar",
        "__nve_x",
    ] {
        assert_eq!(
            spec(allowed, allowed, allowed).validate(),
            Ok(()),
            "{allowed}"
        );
    }
}

#[test]
fn rejects_methods_named_like_vtable_slots() {
    for slot in [
        "create",
        "destroy",
        "freeBytes",
        "alloc_bytes",
        "abiVersion",
        "setPropsDirect",
        "apply_props",
    ] {
        let message = error_of(&spec(slot, "shape", "x"));
        assert!(
            message.contains("reserved hybrid vtable member"),
            "{slot}: {message}"
        );
    }
    // Props and parameters are not vtable slots.
    assert_eq!(spec("run", "create", "destroy").validate(), Ok(()));
}

#[test]
fn rejects_runtime_and_generated_type_names() {
    for name in [
        "HybridCodec",
        "NveValue",
        "DecodeError",
        "NveHybridOptI32",
        "NveHybridAnything",
    ] {
        let mut s = spec("run", "shape", "x");
        s.structs[0].name = name.to_owned();
        s.modules[0].methods[0].params[0].ty = HybridType::Struct(name.to_owned());
        assert!(
            error_of(&s).contains("reserved by the hybrid runtime"),
            "{name}"
        );
    }
}

#[test]
fn rejects_vec_u8_in_favor_of_bytes() {
    let mut s = spec("run", "shape", "x");
    s.modules[0].methods[0].params[0].ty = HybridType::Vec(Box::new(HybridType::U8));
    assert!(error_of(&s).contains("Vec<u8> must be declared as Bytes"));
    s.modules[0].methods[0].params[0].ty = HybridType::Vec(Box::new(HybridType::Bytes));
    assert_eq!(s.validate(), Ok(()));
}
