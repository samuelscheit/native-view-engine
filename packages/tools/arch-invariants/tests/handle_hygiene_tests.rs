//! Unit tests for typed domain handle hygiene (`NodeId`/`SurfaceId` privacy & repr locks,
//! `StoreNodeId` removal, and bare/wrapped primitive handle parameter scanning), split out of
//! `unit_tests.rs` to stay under `[INV:max-650-lines]`.

use nve_arch_invariants::ast_layout;

#[test]
fn unit_handle_hygiene_and_bare_handle_params_positive_and_negative() {
    // 1. Privacy, struct-shape, repr(transparent), and cfg(test) shadowing locks in id.rs.
    let bad_id_rs = r#"
        #[repr(transparent)] pub struct NodeId(pub u32);
        #[repr(transparent)] pub struct SurfaceId(pub(crate) u32);
    "#;
    let ast = syn::parse_file(bad_id_rs).unwrap();
    let mut v = Vec::new();
    ast_layout::scan_handle_hygiene("packages/core/abi/src/id.rs", bad_id_rs, &ast, &mut v);
    assert_eq!(v.len(), 2, "must catch pub and pub(crate) tuple fields: {v:?}");
    assert!(v.iter().all(|x| x.reason.contains("private (`Visibility::Inherited`)")));

    let bad_shape_id_rs = r#"
        pub struct NodeId { pub raw: u32 }
        #[repr(C)] pub struct SurfaceId(u32, u8);
    "#;
    let ast_shape = syn::parse_file(bad_shape_id_rs).unwrap();
    let mut v_shape = Vec::new();
    ast_layout::scan_handle_hygiene(
        "packages/core/abi/src/id.rs",
        bad_shape_id_rs,
        &ast_shape,
        &mut v_shape,
    );
    assert_eq!(
        v_shape.len(),
        4,
        "must catch missing repr(transparent) + named/multi-field shapes for both handles: {v_shape:?}"
    );
    assert!(v_shape.iter().any(|x| x.reason.contains("repr(transparent)")));
    assert!(v_shape.iter().any(|x| x.reason.contains("single-field tuple struct")));

    let bad_inner_and_vis_id_rs = r#"
        #[repr(transparent)] struct NodeId(u32);
        #[repr(transparent)] pub struct SurfaceId(std::num::NonZeroU32);
    "#;
    let ast_iv = syn::parse_file(bad_inner_and_vis_id_rs).unwrap();
    let mut v_iv = Vec::new();
    ast_layout::scan_handle_hygiene(
        "packages/core/abi/src/id.rs",
        bad_inner_and_vis_id_rs,
        &ast_iv,
        &mut v_iv,
    );
    assert_eq!(
        v_iv.len(),
        2,
        "must catch non-pub NodeId and NonZeroU32 inner field on SurfaceId: {v_iv:?}"
    );
    assert!(
        v_iv.iter().any(|x| x.reason.contains("`pub struct`")),
        "expected visibility violation: {v_iv:?}"
    );
    assert!(
        v_iv.iter().any(|x| x.reason.contains("`u32`")),
        "expected u32 inner type violation: {v_iv:?}"
    );

    // Top-level `#[cfg(test)]` struct and nested submodule must NOT satisfy top-level presence!
    let cfg_test_shadow_id_rs = r#"
        #[cfg(test)]
        #[repr(transparent)] pub struct NodeId(u32);
        mod inner { #[repr(transparent)] pub struct SurfaceId(u32); }
    "#;
    let ast_miss = syn::parse_file(cfg_test_shadow_id_rs).unwrap();
    let mut v_miss = Vec::new();
    ast_layout::scan_handle_hygiene(
        "packages/core/abi/src/id.rs",
        cfg_test_shadow_id_rs,
        &ast_miss,
        &mut v_miss,
    );
    assert_eq!(
        v_miss.len(),
        2,
        "top-level #[cfg(test)] NodeId and nested SurfaceId must both fail presence check: {v_miss:?}"
    );
    assert!(v_miss.iter().any(|x| x.reason.contains("NodeId")));
    assert!(v_miss.iter().any(|x| x.reason.contains("SurfaceId")));

    let good_id_rs =
        "#[repr(transparent)] pub struct NodeId(u32); #[repr(transparent)] pub struct SurfaceId(u32);";
    let ast_ok = syn::parse_file(good_id_rs).unwrap();
    let mut v_ok = Vec::new();
    ast_layout::scan_handle_hygiene("packages/core/abi/src/id.rs", good_id_rs, &ast_ok, &mut v_ok);
    assert!(
        v_ok.is_empty(),
        "valid repr(transparent) private handles in id.rs must pass: {v_ok:?}"
    );

    // 2. StoreNodeId anywhere (including direct import, rename from/to, inside fn) + raw ctors.
    let bad_user = r#"
        use foo::StoreNodeId;
        use foo::StoreNodeId as LegacyId;
        type StoreNodeId = u32;
        enum LocalEnum { NodeId(u32), SurfaceId(u32) }
        fn f() {
            use nve_abi::NodeId as StoreNodeId;
            let _a = NodeId(1);
            let _b = nve_abi::SurfaceId(2);
            let _c = NodeId { 0: 3 };
            let _d = crate::id::SurfaceId { 0: 4 };
            let _ok1 = LocalEnum::NodeId(5);
            let _ok2 = other_mod::LocalEnum::SurfaceId(6);
        }
    "#;
    let ast_u = syn::parse_file(bad_user).unwrap();
    let mut vu = Vec::new();
    ast_layout::scan_handle_hygiene("packages/core/engine/src/foo.rs", bad_user, &ast_u, &mut vu);
    assert_eq!(
        vu.len(),
        8,
        "expected use StoreNodeId + rename from StoreNodeId + type StoreNodeId + nested rename to StoreNodeId + 4 raw ctors (0 enum FPs): {vu:?}"
    );
    assert_eq!(
        vu.iter().filter(|x| x.reason.contains("StoreNodeId")).count(),
        4
    );
    assert_eq!(
        vu.iter().filter(|x| x.reason.contains("tuple construction is banned")).count(),
        2
    );
    assert_eq!(
        vu.iter().filter(|x| x.reason.contains("struct construction is banned")).count(),
        2
    );

    // 3. Bare & wrapped primitive handle params across fn, impl/trait, extern, bare fn, tuples,
    //    closures, wrong integer widths, qualified primitives, NonZero*, and extended containers.
    let bad_params = r#"
        extern "C" {
            fn c_entry(surface_id: u32, node_id: *const u32);
        }
        type Callback = fn(frame_id: u64, interface_id: Option<u32>);
        trait Host {
            fn on_surface(&self, parent_surface_id: u64);
        }
        impl Host for () {
            fn on_surface(&self, parent_surface_id: core::primitive::u32) {}
        }
        fn outer(
            surface_id: Option<u32>,
            node_id: Box<u32>,
            frame_id: Arc<core::num::NonZeroU64>,
            (child_node_id, _ok): (VecDeque<u32>, bool),
            surface_ids: &[u32],
            child_node_ids: Vec<core::num::NonZero<u32>>,
        ) {
            fn inner(node_id: &[u32], surface_id: Vec<u32>, interface_id: *mut u32) {}
            let _cl = |target_surface_id: u32| { let _ = target_surface_id; };
        }
        #[wasm_bindgen]
        extern "C" {
            fn js_import(surface_id: u32, node_ids: &[u32]);
        }
        #[wasm_bindgen]
        pub fn js_boundary(surface_id: u32, node_id: u32) {}
        pub fn typed_ok(surface_id: SurfaceId, node_id: Option<NodeId>, frame_id: FrameId) {}
    "#;
    let ast_p = syn::parse_file(bad_params).unwrap();
    let mut vp = Vec::new();
    ast_layout::scan_bare_handle_params(
        "packages/core/engine/src/p.rs",
        bad_params,
        &ast_p,
        &mut vp,
    );
    assert_eq!(
        vp.len(),
        16,
        "expected 2 extern + 2 bare fn + 1 trait + 1 impl + 6 outer/tuple/plural/NonZero<u32> + 3 inner + 1 closure violations (0 for wasm_bindgen/typed): {vp:?}"
    );
    assert!(vp.iter().any(|x| x.reason.contains("fn `c_entry`")));
    assert!(vp.iter().any(|x| x.reason.contains("bare `fn`")));
    assert!(vp.iter().any(|x| x.reason.contains("closure")));
}
