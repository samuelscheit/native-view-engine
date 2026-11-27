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

#[test]
fn view_only_slots_are_free_method_names_in_modules() {
    for name in ["apply_props", "setPropsDirect"] {
        assert_eq!(spec(name, "shape", "x").validate(), Ok(()), "{name}");
        let mut s = spec("run", "shape", "x");
        s.views.push(HybridViewSpec {
            name: "Panel".to_owned(),
            trait_name: "PanelSpec".to_owned(),
            methods: vec![method(name, vec![])],
            ..Default::default()
        });
        assert!(
            error_of(&s).contains("reserved hybrid vtable member"),
            "{name}"
        );
    }
}

#[test]
fn reserves_the_generated_item_prefix_in_every_spelling() {
    for name in [
        "NVE_HYBRID_ABI_VERSION",
        "NVE_HYBRID_FILE_SCHEMA_HASH",
        "nve_hybrid_alloc_bytes",
    ] {
        let mut s = spec("run", "shape", "x");
        s.consts.push(HybridConstSpec {
            name: name.to_owned(),
            ty: HybridType::U32,
            value: 1,
            hex: false,
            doc: None,
        });
        assert!(
            error_of(&s).contains("reserved by the hybrid runtime"),
            "{name}"
        );
    }
}

#[test]
fn names_that_collide_in_snake_case_are_duplicates() {
    let mut s = spec("getValue", "shape", "x");
    s.modules[0].methods.push(method("get_value", vec![]));
    assert!(error_of(&s).contains("duplicate method"));

    let mut s = spec("run", "shape", "x");
    s.modules[0].methods[0]
        .params
        .push(param("Shape", HybridType::I32));
    assert!(error_of(&s).contains("duplicate parameter"));

    let mut s = spec("run", "shape", "_x");
    s.structs[0].fields.push(HybridFieldSpec {
        name: "x".to_owned(),
        ty: HybridType::I32,
        doc: None,
        offset: None,
        offset_64: None,
    });
    assert!(error_of(&s).contains("duplicate field"));

    let mut s = spec("run", "shape", "x");
    s.modules.push(HybridModuleSpec {
        name: "probe".to_owned(),
        trait_name: "ProbeTwoSpec".to_owned(),
        ..Default::default()
    });
    assert!(error_of(&s).contains("duplicate hybrid module/view name"));
}

#[test]
fn enum_variants_that_collide_in_snake_case_are_duplicates() {
    let variant = |name: &str, value: u32| HybridEnumVariant {
        name: name.to_owned(),
        value,
        doc: None,
    };
    for clash in ["foo_bar", "FOO_BAR", "fooBar"] {
        let mut s = spec("run", "shape", "x");
        s.enums.push(HybridEnumSpec {
            name: "Mode".to_owned(),
            variants: vec![variant("FooBar", 0), variant(clash, 1)],
            ..Default::default()
        });
        assert!(error_of(&s).contains("duplicate variant"), "{clash}");
    }
    let mut s = spec("run", "shape", "x");
    s.enums.push(HybridEnumSpec {
        name: "Mode".to_owned(),
        variants: vec![variant("FooBar", 0), variant("FooBaz", 1)],
        ..Default::default()
    });
    assert_eq!(s.validate(), Ok(()));
}

fn with_struct(name: &str) -> HybridFileSpec {
    let mut s = spec("run", "shape", "x");
    s.modules[0].methods[0].params[0].ty =
        HybridType::Optional(Box::new(HybridType::Struct("Shape".to_owned())));
    let mut extra = s.structs[0].clone();
    extra.name = name.to_owned();
    s.structs.push(extra);
    s
}

#[test]
fn rejects_type_names_spelling_another_generated_type() {
    // `NveHybridStatus` (runtime), `NveHybridProbeVTable`, `NveHybridOptShape` (C option of
    // `Shape`), `HybridProbeSpec` / `ProbeClient` / `ProbeHybridClient` / `ProbeHybrid` (bindings
    // of module `Probe`), and `HybridSpec` file types of the default stem.
    for name in [
        "Status",
        "ProbeVTable",
        "OptShape",
        "HybridProbeSpec",
        "ProbeClient",
        "ProbeHybridClient",
        "ProbeHybrid",
    ] {
        assert!(
            error_of(&with_struct(name)).contains("collides with the one of"),
            "{name}"
        );
    }
    // `NveHybridRawShape` (Rust carrier of `Shape`) never meets `NveHybridRawShape` (C type of
    // `RawShape`): Rust spells `RawShape`'s carrier `NveHybridRawRawShape`.
    for name in ["Options", "RawShape", "RawData", "OptI64", "PanelVTable", "Statuses", "Value"] {
        assert_eq!(with_struct(name).validate(), Ok(()), "{name}");
    }
}

#[test]
fn rejects_generated_names_that_collide_with_each_other() {
    let event = |name: &str| HybridEventSpec {
        name: name.to_owned(),
        params: vec![],
        doc: None,
    };
    let view = |name: &str, events: Vec<HybridEventSpec>| HybridViewSpec {
        name: name.to_owned(),
        trait_name: format!("{name}Spec"),
        events,
        ..Default::default()
    };
    // `PanelOpenCloseEvent` from view `PanelOpen` event `close` and view `Panel` event `openClose`.
    let mut s = spec("run", "shape", "x");
    s.views = vec![view("PanelOpen", vec![event("close")]), view("Panel", vec![event("openClose")])];
    assert!(error_of(&s).contains("generated type name 'PanelOpenCloseEvent'"));
    // A struct spelling a view's props struct.
    let mut s = with_struct("PanelProps");
    s.views = vec![view("Panel", vec![])];
    assert!(error_of(&s).contains("generated type name 'PanelProps'"));
    // `NveHybridOptVTable` would be both module `Opt`'s vtable and the C option of struct `VTable`.
    let mut s = with_struct("VTable");
    s.modules[0].methods[0].params[0].ty =
        HybridType::Optional(Box::new(HybridType::Struct("VTable".to_owned())));
    s.modules.push(HybridModuleSpec {
        name: "Opt".to_owned(),
        trait_name: "OptSpec".to_owned(),
        ..Default::default()
    });
    assert!(error_of(&s).contains("generated type name 'NveHybridOptVTable'"));
}

#[test]
fn c_function_options_get_their_c_option_type() {
    let mut s = with_struct("Other");
    s.modules[0].methods[0].params[0].ty = HybridType::Struct("Shape".to_owned());
    s.c_functions.push(HybridMethodSpec {
        has_self: false,
        ..method(
            "probe_other",
            vec![param("other", HybridType::Optional(Box::new(HybridType::Struct("Other".to_owned()))))],
        )
    });
    let suffixes: Vec<_> = s.collect_pod_option_types().into_iter().map(|(suffix, _)| suffix).collect();
    assert_eq!(suffixes, ["Other"]);
    let mut clash = s.clone();
    clash.structs[0].name = "OptOther".to_owned();
    clash.modules[0].methods[0].params[0].ty = HybridType::Struct("OptOther".to_owned());
    assert!(error_of(&clash).contains("generated type name 'NveHybridOptOther'"));
}

fn fixed(elem: HybridType, len: usize) -> HybridType {
    HybridType::FixedArray { elem: Box::new(elem), len }
}

#[test]
fn fixed_arrays_hold_at_most_4096_elements_across_all_dimensions() {
    let cases = [
        (fixed(HybridType::U8, 1), true),
        (fixed(HybridType::U8, 4_096), true),
        (fixed(fixed(HybridType::U8, 64), 64), true),
        (fixed(HybridType::U8, 0), false),
        (fixed(fixed(HybridType::U8, 0), 4), false),
        (fixed(HybridType::U8, 4_097), false),
        (fixed(fixed(HybridType::U8, 65), 65), false),
        (fixed(fixed(HybridType::U64, 65_536), 65_536), false),
        (fixed(fixed(HybridType::U8, usize::MAX), 2), false),
    ];
    for (ty, ok) in cases {
        let signature = ty.canonical_signature();
        // As a method parameter and as a POD struct field.
        let mut param = spec("run", "shape", "x");
        param.modules[0].methods[0].params[0].ty = ty.clone();
        let mut field = spec("run", "shape", "x");
        field.structs[0].fields[0].ty = ty;
        for s in [param, field] {
            match s.validate() {
                Ok(()) => assert!(ok, "{signature} must be rejected"),
                Err(e) => {
                    assert!(!ok, "{signature}: {e}");
                    let message = e.to_string();
                    assert!(
                        message.contains("at least one element") || message.contains("more than 4096 elements"),
                        "{message}"
                    );
                }
            }
        }
    }
}

/// Struct fields may hold opaque pointers to anything (the struct then has no C form); a static C
/// function can only point to `void` or a type with a C value form (`*const [T; N]` would need C's
/// `T (*)[N]`).
#[test]
fn c_functions_point_only_to_void_or_c_value_types() {
    let ptr = |inner: HybridType| HybridType::ConstPtr(Box::new(inner));
    for (pointee, c_ok) in [
        (HybridType::Void, true),
        (HybridType::U32, true),
        (HybridType::Struct("Shape".to_owned()), true),
        (fixed(HybridType::U32, 4), false),
        (HybridType::String, false),
        (HybridType::Vec(Box::new(HybridType::U32)), false),
    ] {
        let mut field = spec("run", "shape", "x");
        field.structs[0].fields[0].ty = HybridType::MutPtr(Box::new(pointee.clone()));
        assert_eq!(field.validate(), Ok(()), "{}", pointee.canonical_signature());
        let mut c_fn = spec("run", "shape", "x");
        c_fn.c_functions.push(HybridMethodSpec {
            has_self: false,
            return_type: ptr(pointee.clone()),
            ..method("probe_ptr", vec![])
        });
        match c_fn.validate() {
            Ok(()) => assert!(c_ok, "{}", pointee.canonical_signature()),
            Err(e) => assert!(!c_ok && e.to_string().contains("must point to void, a scalar, an enum, or a struct"), "{e}"),
        }
    }
    // C functions take opaque handles to non-POD structs too.
    let mut s = with_struct("Named");
    s.structs[1].fields[0].ty = HybridType::String;
    s.c_functions.push(HybridMethodSpec {
        has_self: false,
        ..method("probe_named", vec![param("named", ptr(HybridType::Struct("Named".to_owned())))])
    });
    assert_eq!(s.validate(), Ok(()));
}

#[test]
fn cyclic_structs_through_fixed_arrays_are_not_pod() {
    let mut s = spec("run", "shape", "x");
    s.structs[0].fields[0].ty = fixed(fixed(HybridType::Struct("Shape".to_owned()), 1), 2);
    assert!(!s.structs[0].is_pod(&s));
    assert!(!HybridType::Struct("Shape".to_owned()).is_pod(&s));
    s.structs[0].fields[0].ty = fixed(fixed(HybridType::U32, 1), 2);
    assert!(s.structs[0].is_pod(&s));
}
