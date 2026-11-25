//! Deep unit, validation, determinism, cross-language Rust <-> C++20 real-compiler execution,
//! and adversarial security tests for NVE's zero-copy `RelocSlice` & `VerifyMode` codegen.

use std::path::PathBuf;
use std::process::Command;

use nve_codegen::{
    generate_reloc_all, generate_reloc_cpp, generate_reloc_rust, CodegenError, RelocEnum,
    RelocEnumRepr, RelocEnumVariant, RelocField, RelocSchema, RelocStruct, RelocType,
    RELOC_HEADER_SIZE,
};

fn temp_test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "nve_reloc_codegen_test_{tag}_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn tool_available(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Realistic multi-struct schema exercising:
/// - `#[repr(u8)]`, `#[repr(u16)]`, and `#[repr(u32)]` enums
/// - Pure zero-padding POD struct (`Vec2`, 8 bytes -> gets `as_bytes()`)
/// - Fixed-size POD array (`[f32; 4]`)
/// - `RelocStr` UTF-8 strings
/// - Nested `RelocSlice<LayerRecord>` containing `RelocSlice<Vec2>` + `RelocStr` + enums + `bool`
fn realistic_asset_pack_schema() -> RelocSchema {
    RelocSchema {
        name: "AssetPack".to_string(),
        magic: *b"NVE_PACK",
        root: "PackRoot".to_string(),
        enums: vec![
            RelocEnum {
                name: "BlendMode".to_string(),
                repr: RelocEnumRepr::U8,
                variants: vec![
                    RelocEnumVariant::new("Normal", 0),
                    RelocEnumVariant::new("Multiply", 1),
                    RelocEnumVariant::new("Screen", 2),
                ],
                doc: Some("Compositing blend mode.".to_string()),
            },
            RelocEnum {
                name: "LayerKind".to_string(),
                repr: RelocEnumRepr::U16,
                variants: vec![
                    RelocEnumVariant::new("Vector", 1),
                    RelocEnumVariant::new("Raster", 2),
                    RelocEnumVariant::new("Text", 10),
                ],
                doc: Some("Layer payload kind.".to_string()),
            },
            RelocEnum {
                name: "ColorSpace".to_string(),
                repr: RelocEnumRepr::U32,
                variants: vec![
                    RelocEnumVariant::new("Srgb", 0),
                    RelocEnumVariant::new("DisplayP3", 1),
                    RelocEnumVariant::new("LinearF16", 2),
                ],
                doc: Some("Surface color space.".to_string()),
            },
        ],
        structs: vec![
            RelocStruct {
                name: "Vec2".to_string(),
                fields: vec![
                    RelocField::new("x", RelocType::F32),
                    RelocField::new("y", RelocType::F32),
                ],
                align: None,
                doc: Some("Zero-padding 8-byte POD point.".to_string()),
            },
            RelocStruct {
                name: "LayerRecord".to_string(),
                fields: vec![
                    RelocField::new("id", RelocType::U32),
                    RelocField::new("kind", RelocType::Enum("LayerKind".to_string())),
                    RelocField::new("blend", RelocType::Enum("BlendMode".to_string())),
                    RelocField::new("visible", RelocType::Bool),
                    RelocField::new("bounds", RelocType::FixedArray {
                        elem: Box::new(RelocType::F32),
                        len: 4,
                    }),
                    RelocField::new("name", RelocType::RelocStr),
                    RelocField::new(
                        "points",
                        RelocType::RelocSlice(Box::new(RelocType::Struct("Vec2".to_string()))),
                    ),
                ],
                align: None,
                doc: Some("One compositing layer with nested vertex slice and UTF-8 label.".to_string()),
            },
            RelocStruct {
                name: "PackRoot".to_string(),
                fields: vec![
                    RelocField::new("version", RelocType::U32),
                    RelocField::new("color_space", RelocType::Enum("ColorSpace".to_string())),
                    RelocField::new("created_ns", RelocType::U64),
                    RelocField::new("title", RelocType::RelocStr),
                    RelocField::new(
                        "palette",
                        RelocType::RelocSlice(Box::new(RelocType::U32)),
                    ),
                    RelocField::new(
                        "layers",
                        RelocType::RelocSlice(Box::new(RelocType::Struct("LayerRecord".to_string()))),
                    ),
                ],
                align: None,
                doc: Some("Root table inside an `AssetPack` binary slab.".to_string()),
            },
        ],
    }
}

/// Self-referential tree schema (`RelocSlice<TreeNode>`) used to test depth-limit protection.
fn recursive_tree_schema() -> RelocSchema {
    RelocSchema {
        name: "SceneTree".to_string(),
        magic: *b"NVETREE1",
        root: "TreeNode".to_string(),
        enums: vec![],
        structs: vec![RelocStruct {
            name: "TreeNode".to_string(),
            fields: vec![
                RelocField::new("id", RelocType::U32),
                RelocField::new("_pad", RelocType::U32),
                RelocField::new("label", RelocType::RelocStr),
                RelocField::new(
                    "children",
                    RelocType::RelocSlice(Box::new(RelocType::Struct("TreeNode".to_string()))),
                ),
            ],
            align: None,
            doc: None,
        }],
    }
}

#[test]
fn computes_exact_repr_c_layouts_and_deterministic_schema_hash() {
    let schema = realistic_asset_pack_schema();
    let layouts = schema.compute_layouts().expect("valid schema");

    let vec2 = &layouts["Vec2"];
    assert_eq!(vec2.size, 8);
    assert_eq!(vec2.align, 4);
    assert!(vec2.is_pure_pod);
    assert!(!vec2.has_implicit_padding);
    assert!(!vec2.needs_value_verify);

    let layer = &layouts["LayerRecord"];
    assert_eq!(layer.size, 40);
    assert_eq!(layer.align, 4);
    assert!(!layer.is_pure_pod);
    assert!(layer.has_reloc_indirection);
    assert!(layer.needs_value_verify);
    assert_eq!(layer.fields[0].offset, 0); // id: u32
    assert_eq!(layer.fields[1].offset, 4); // kind: u16
    assert_eq!(layer.fields[2].offset, 6); // blend: u8
    assert_eq!(layer.fields[3].offset, 7); // visible: bool
    assert_eq!(layer.fields[4].offset, 8); // bounds: [f32; 4]
    assert_eq!(layer.fields[5].offset, 24); // name: RelocStr
    assert_eq!(layer.fields[6].offset, 32); // points: RelocSlice<Vec2>

    let root = &layouts["PackRoot"];
    assert_eq!(root.size, 40);
    assert_eq!(root.align, 8);
    assert_eq!(root.fields[0].offset, 0); // version: u32
    assert_eq!(root.fields[1].offset, 4); // color_space: u32
    assert_eq!(root.fields[2].offset, 8); // created_ns: u64
    assert_eq!(root.fields[3].offset, 16); // title: RelocStr
    assert_eq!(root.fields[4].offset, 24); // palette: RelocSlice<u32>
    assert_eq!(root.fields[5].offset, 32); // layers: RelocSlice<LayerRecord>

    // Schema hash is deterministic and mutates on any schema change.
    let h1 = schema.schema_hash().unwrap();
    let h2 = schema.schema_hash().unwrap();
    assert_eq!(h1, h2);
    assert_ne!(h1, 0);

    let mut mutated_variant = schema.clone();
    mutated_variant.enums[0].variants[1].value = 9;
    assert_ne!(mutated_variant.schema_hash().unwrap(), h1);

    let mut mutated_field = schema.clone();
    mutated_field.structs[0].fields[1].ty = RelocType::F64;
    assert_ne!(mutated_field.schema_hash().unwrap(), h1);

    let mut mutated_magic = schema.clone();
    mutated_magic.magic = *b"NVE_PAC2";
    assert_ne!(mutated_magic.schema_hash().unwrap(), h1);
}

#[test]
fn generated_files_are_deterministic_follow_naming_convention_and_have_zero_json() {
    let schema = realistic_asset_pack_schema();
    let first = generate_reloc_all(&schema).expect("generate all");
    let second = generate_reloc_all(&schema).expect("generate second pass");
    assert_eq!(first, second);
    assert_eq!(first.len(), 2);
    assert_eq!(first[0].relative_path, "asset_pack_reloc.generated.rs");
    assert_eq!(first[1].relative_path, "AssetPackReloc.generated.hpp");

    for f in &first {
        assert!(!f.content.contains("serde_json"));
        assert!(!f.content.contains("JSON.parse"));
    }
}

#[test]
fn rejects_invalid_reloc_schemas() {
    // 1. All-zero magic
    let mut s = realistic_asset_pack_schema();
    s.magic = [0u8; 8];
    assert!(matches!(s.validate(), Err(CodegenError::Validation(_))));

    // 2. Missing root struct
    let mut s = realistic_asset_pack_schema();
    s.root = "DoesNotExist".to_string();
    assert!(matches!(s.validate(), Err(CodegenError::Validation(_))));

    // 3. Enum discriminant out of u8 range
    let mut s = realistic_asset_pack_schema();
    s.enums[0].variants.push(RelocEnumVariant::new("TooBig", 300));
    assert!(matches!(s.validate(), Err(CodegenError::Validation(_))));

    // 4. Direct infinite-size by-value struct cycle (A -> B -> A)
    let cycle = RelocSchema {
        name: "BadCycle".to_string(),
        magic: *b"BADCYCLE",
        root: "NodeA".to_string(),
        enums: vec![],
        structs: vec![
            RelocStruct {
                name: "NodeA".to_string(),
                fields: vec![RelocField::new("b", RelocType::Struct("NodeB".to_string()))],
                align: None,
                doc: None,
            },
            RelocStruct {
                name: "NodeB".to_string(),
                fields: vec![RelocField::new("a", RelocType::Struct("NodeA".to_string()))],
                align: None,
                doc: None,
            },
        ],
    };
    assert!(matches!(
        cycle.validate(),
        Err(CodegenError::Validation(msg)) if msg.contains("cycle")
    ));

    // 5. Recursive RelocSlice<Self> IS valid (8-byte offset+len breaks value-size cycle)
    assert!(recursive_tree_schema().validate().is_ok());
}

#[test]
fn real_compiler_rust_and_cpp20_round_trip_and_adversarial_security() {
    assert!(tool_available("rustc"), "rustc must be available");
    assert!(tool_available("clang++"), "clang++ must be available");

    let dir = temp_test_dir("roundtrip");
    let schema = realistic_asset_pack_schema();
    let rust_file = generate_reloc_rust(&schema).expect("generate rust");
    let cpp_file = generate_reloc_cpp(&schema).expect("generate cpp");

    let tree_schema = recursive_tree_schema();
    let tree_rust_file = generate_reloc_rust(&tree_schema).expect("generate tree rust");
    let tree_cpp_file = generate_reloc_cpp(&tree_schema).expect("generate tree cpp");

    let rust_mod_path = dir.join(&rust_file.relative_path);
    let cpp_hdr_path = dir.join(&cpp_file.relative_path);
    let tree_rust_path = dir.join(&tree_rust_file.relative_path);
    let tree_cpp_path = dir.join(&tree_cpp_file.relative_path);
    let slab_bin_path = dir.join("asset_pack.bin");

    std::fs::write(&rust_mod_path, &rust_file.content).unwrap();
    std::fs::write(&cpp_hdr_path, &cpp_file.content).unwrap();
    std::fs::write(&tree_rust_path, &tree_rust_file.content).unwrap();
    std::fs::write(&tree_cpp_path, &tree_cpp_file.content).unwrap();

    // 1. Rust 2024 driver: builds slab, verifies Full/HeaderOnly/Unchecked, tests pure POD
    //    `as_bytes()`, runs full adversarial suite (including cyclic depth overflow), and
    //    writes `asset_pack.bin` for C++20 zero-copy consumption.
    let rust_driver = dir.join("driver.rs");
    std::fs::write(
        &rust_driver,
        format!(
            r#"
#[path = "{rust_mod}"]
mod asset_pack;
#[path = "{tree_mod}"]
mod scene_tree;

use asset_pack::{{
    AlignedSlab, BlendMode, ColorSpace, LayerKind, LayerRecord, PackRoot, RelocError, RelocHeader,
    RelocSlice, RelocStr, SlabBuilder, Vec2, VerifyMode, HEADER_SIZE, SCHEMA_HASH, SCHEMA_MAGIC,
}};

fn main() {{
    // Pure POD zero-copy `as_bytes()` check
    assert!(RelocStr::EMPTY.is_empty());
    let pt = Vec2 {{ x: 1.5, y: -2.25 }};
    let pt_bytes = pt.as_bytes();
    assert_eq!(pt_bytes.len(), 8);
    assert_eq!(&pt_bytes[0..4], &1.5f32.to_le_bytes());
    assert_eq!(&pt_bytes[4..8], &(-2.25f32).to_le_bytes());

    // Build realistic multi-struct binary slab with nested RelocSlices + RelocStr + enums
    let mut b = SlabBuilder::new();
    let title = b.push_str("NVE Zero-Copy Pack 🚀");
    let palette = b.push_slice(&[0xFF11_2233u32, 0xFF44_5566u32, 0xFF77_8899u32]);

    let l0_name = b.push_str("hero-vector");
    let l0_pts = b.push_slice(&[
        Vec2 {{ x: 0.0, y: 0.0 }},
        Vec2 {{ x: 120.5, y: 42.25 }},
        Vec2 {{ x: 240.0, y: 99.0 }},
    ]);
    let l1_name = b.push_str("badge-text");
    let l1_pts = RelocSlice::<Vec2>::EMPTY;

    let layers = b.push_slice(&[
        LayerRecord {{
            id: 101,
            kind: LayerKind::Vector,
            blend: BlendMode::Multiply,
            visible: true,
            bounds: [1.0, 2.0, 240.0, 99.0],
            name: l0_name,
            points: l0_pts,
        }},
        LayerRecord {{
            id: 202,
            kind: LayerKind::Text,
            blend: BlendMode::Screen,
            visible: false,
            bounds: [8.0, 16.0, 80.0, 24.0],
            name: l1_name,
            points: l1_pts,
        }},
    ]);

    let root_val = PackRoot {{
        version: 7,
        color_space: ColorSpace::DisplayP3,
        created_ns: 1_735_689_600_123_456_789u64,
        title,
        palette,
        layers,
    }};
    let slab = b.finish(&root_val, 0xA5A5_0001u32);
    let bytes = slab.as_slice();
    assert_eq!(&bytes[0..8], &SCHEMA_MAGIC);

    // Read zero-copy under Full, HeaderOnly, and Unchecked modes
    let r_full = asset_pack::view_full(bytes).expect("Full verify must succeed");
    let r_hdr = unsafe {{ asset_pack::view(bytes, VerifyMode::HeaderOnly) }}.expect("HeaderOnly");
    let r_un = unsafe {{ asset_pack::view_unchecked(bytes) }};
    assert_eq!(r_full, r_hdr);
    assert_eq!(r_full, r_un);

    assert_eq!(r_full.version, 7);
    assert_eq!(r_full.color_space, ColorSpace::DisplayP3);
    assert_eq!(r_full.created_ns, 1_735_689_600_123_456_789u64);
    assert_eq!(r_full.title.as_str(bytes).unwrap(), "NVE Zero-Copy Pack 🚀");
    assert_eq!(unsafe {{ r_full.title.as_str_unchecked(bytes) }}, "NVE Zero-Copy Pack 🚀");
    assert_eq!(r_full.palette.resolve(bytes).unwrap(), &[0xFF11_2233u32, 0xFF44_5566u32, 0xFF77_8899u32]);

    let layer_slice = r_full.layers.resolve(bytes).unwrap();
    assert_eq!(layer_slice.len(), 2);
    assert_eq!(layer_slice[0].id, 101);
    assert_eq!(layer_slice[0].kind, LayerKind::Vector);
    assert_eq!(layer_slice[0].blend, BlendMode::Multiply);
    assert!(layer_slice[0].visible);
    assert_eq!(layer_slice[0].name.as_str(bytes).unwrap(), "hero-vector");
    assert_eq!(layer_slice[0].points.resolve(bytes).unwrap().len(), 3);
    assert_eq!(layer_slice[1].points.resolve(bytes).unwrap().len(), 0);

    // Persist valid binary slab for cross-language C++20 round-trip
    std::fs::write("{slab_bin}", bytes).expect("write slab bin");

    // --- Adversarial security tests for VerifyMode::Full ---
    let root_off = u32::from_le_bytes(bytes[24..28].try_into().unwrap()) as usize;
    let layers_off = u32::from_le_bytes(bytes[root_off + 32..root_off + 36].try_into().unwrap()) as usize;

    // 1. Truncated buffer
    assert_eq!(asset_pack::view_full(&bytes[..0]), Err(RelocError::Truncated));
    assert_eq!(asset_pack::view_full(&bytes[..HEADER_SIZE - 1]), Err(RelocError::Truncated));

    // 2. Misaligned buffer (offset by 1 byte inside an aligned slab)
    let mut padded = vec![0u8; bytes.len() + 16];
    let base_misalign = ((padded.as_ptr() as usize) % 8 == 0) as usize;
    let mis_start = if base_misalign == 1 {{ 1 }} else {{ 0 }};
    padded[mis_start..mis_start + bytes.len()].copy_from_slice(bytes);
    assert_eq!(
        asset_pack::view_full(&padded[mis_start..mis_start + bytes.len()]),
        Err(RelocError::Misaligned)
    );

    // 3. Wrong magic
    let mut bad_magic = slab.clone();
    bad_magic.as_mut_slice()[0] ^= 0xFF;
    assert_eq!(asset_pack::view_full(bad_magic.as_slice()), Err(RelocError::InvalidMagic));

    // 4. Wrong schema_hash
    let mut bad_hash = slab.clone();
    let wrong_hash = (SCHEMA_HASH ^ 1).to_le_bytes();
    bad_hash.as_mut_slice()[8..16].copy_from_slice(&wrong_hash);
    assert_eq!(asset_pack::view_full(bad_hash.as_slice()), Err(RelocError::SchemaHashMismatch));

    // 5. Out-of-bounds root_offset (aligned) and misaligned root_offset
    let mut bad_root = slab.clone();
    bad_root.as_mut_slice()[24..28].copy_from_slice(&999_992u32.to_le_bytes());
    assert_eq!(asset_pack::view_full(bad_root.as_slice()), Err(RelocError::OutOfBounds));
    let mut mis_root = slab.clone();
    mis_root.as_mut_slice()[24..28].copy_from_slice(&36u32.to_le_bytes());
    assert_eq!(asset_pack::view_full(mis_root.as_slice()), Err(RelocError::Misaligned));

    // 6. Out-of-bounds RelocSlice offset / len
    let mut bad_slice_oob = slab.clone();
    bad_slice_oob.as_mut_slice()[root_off + 24..root_off + 28].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
    assert_eq!(asset_pack::view_full(bad_slice_oob.as_slice()), Err(RelocError::OutOfBounds));

    // 7. u32::MAX overflow on RelocSlice len * size_of::<u32>()
    let mut bad_overflow = slab.clone();
    bad_overflow.as_mut_slice()[root_off + 28..root_off + 32].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(asset_pack::view_full(bad_overflow.as_slice()), Err(RelocError::LengthOverflow));

    // 8. Misaligned RelocSlice offset (not multiple of 4 for u32)
    let mut bad_slice_align = slab.clone();
    bad_slice_align.as_mut_slice()[root_off + 24..root_off + 28].copy_from_slice(&33u32.to_le_bytes());
    bad_slice_align.as_mut_slice()[root_off + 28..root_off + 32].copy_from_slice(&1u32.to_le_bytes());
    assert_eq!(asset_pack::view_full(bad_slice_align.as_slice()), Err(RelocError::Misaligned));

    // 9. Invalid UTF-8 in RelocStr (corrupt title byte with 0xFF)
    let title_off = u32::from_le_bytes(bytes[root_off + 16..root_off + 20].try_into().unwrap()) as usize;
    let mut bad_utf8 = slab.clone();
    bad_utf8.as_mut_slice()[title_off] = 0xFF;
    assert_eq!(asset_pack::view_full(bad_utf8.as_slice()), Err(RelocError::InvalidUtf8));

    // 10. Invalid bool byte (2 instead of 0/1) inside LayerRecord[0]
    let mut bad_bool = slab.clone();
    bad_bool.as_mut_slice()[layers_off + 7] = 2;
    assert_eq!(asset_pack::view_full(bad_bool.as_slice()), Err(RelocError::InvalidBool));

    // 11. Invalid enum discriminants (u8 BlendMode, u16 LayerKind, u32 ColorSpace)
    let mut bad_u8_enum = slab.clone();
    bad_u8_enum.as_mut_slice()[layers_off + 6] = 99;
    assert_eq!(asset_pack::view_full(bad_u8_enum.as_slice()), Err(RelocError::InvalidEnumDiscriminant));

    let mut bad_u16_enum = slab.clone();
    bad_u16_enum.as_mut_slice()[layers_off + 4..layers_off + 6].copy_from_slice(&777u16.to_le_bytes());
    assert_eq!(asset_pack::view_full(bad_u16_enum.as_slice()), Err(RelocError::InvalidEnumDiscriminant));

    let mut bad_u32_enum = slab.clone();
    bad_u32_enum.as_mut_slice()[root_off + 4..root_off + 8].copy_from_slice(&42u32.to_le_bytes());
    assert_eq!(asset_pack::view_full(bad_u32_enum.as_slice()), Err(RelocError::InvalidEnumDiscriminant));

    // 12. Cyclic self-referential RelocSlice depth overflow
    let mut tb = scene_tree::SlabBuilder::new();
    let root_node = scene_tree::TreeNode {{
        id: 1,
        _pad: 0,
        label: tb.push_str("cycle"),
        children: scene_tree::RelocSlice::EMPTY,
    }};
    let mut tree_slab = tb.finish(&root_node, 0);
    let t_root_off = u32::from_le_bytes(tree_slab.as_slice()[24..28].try_into().unwrap());
    // Point root.children right back at root_offset with len = 1 (hostile cycle!)
    let off = t_root_off as usize;
    tree_slab.as_mut_slice()[off + 16..off + 20].copy_from_slice(&t_root_off.to_le_bytes());
    tree_slab.as_mut_slice()[off + 20..off + 24].copy_from_slice(&1u32.to_le_bytes());
    assert_eq!(
        scene_tree::view_full(tree_slab.as_slice()),
        Err(scene_tree::RelocError::DepthExceeded)
    );

    // Ensure RelocHeader size constant stays consistent
    assert_eq!(core::mem::size_of::<RelocHeader>(), HEADER_SIZE);
    let _ = AlignedSlab::from_bytes(bytes);
}}
"#,
            rust_mod = rust_mod_path.display().to_string().replace('\\', "/"),
            tree_mod = tree_rust_path.display().to_string().replace('\\', "/"),
            slab_bin = slab_bin_path.display().to_string().replace('\\', "/"),
        ),
    )
    .unwrap();

    let rust_exe = dir.join("rust_reloc_driver");
    let rustc_out = Command::new("rustc")
        .args(["--edition=2024", "-D", "warnings"])
        .arg(&rust_driver)
        .arg("-o")
        .arg(&rust_exe)
        .output()
        .expect("run rustc");
    assert!(
        rustc_out.status.success(),
        "rustc --edition=2024 -D warnings failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&rustc_out.stdout),
        String::from_utf8_lossy(&rustc_out.stderr)
    );

    let rust_run = Command::new(&rust_exe).output().expect("execute rust driver");
    assert!(
        rust_run.status.success(),
        "rust reloc driver failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&rust_run.stdout),
        String::from_utf8_lossy(&rust_run.stderr)
    );
    assert!(slab_bin_path.exists(), "rust driver must produce asset_pack.bin");

    // 2. C++20 driver: reads the exact Rust-written `asset_pack.bin` zero-copy under Full,
    //    HeaderOnly, and Unchecked modes, AND verifies the complete adversarial suite in C++20!
    let cpp_driver = dir.join("driver.cpp");
    std::fs::write(
        &cpp_driver,
        format!(
            r#"
#include "AssetPackReloc.generated.hpp"
#include "SceneTreeReloc.generated.hpp"

#include <cassert>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <fstream>
#include <limits>
#include <vector>

namespace ap = nve::reloc::asset_pack;
namespace st = nve::reloc::scene_tree;

static void write_le32(uint8_t* dst, uint32_t v) {{
  dst[0] = static_cast<uint8_t>(v & 0xFFu);
  dst[1] = static_cast<uint8_t>((v >> 8u) & 0xFFu);
  dst[2] = static_cast<uint8_t>((v >> 16u) & 0xFFu);
  dst[3] = static_cast<uint8_t>((v >> 24u) & 0xFFu);
}}

static void write_le16(uint8_t* dst, uint16_t v) {{
  dst[0] = static_cast<uint8_t>(v & 0xFFu);
  dst[1] = static_cast<uint8_t>((v >> 8u) & 0xFFu);
}}

int main() {{
  std::ifstream ifs("{slab_bin}", std::ios::binary);
  assert(ifs.good());
  std::vector<uint8_t> raw((std::istreambuf_iterator<char>(ifs)),
                            std::istreambuf_iterator<char>());
  assert(raw.size() >= ap::kHeaderSize);

  // Copy into guaranteed 8-byte aligned buffer
  std::vector<uint64_t> aligned_words((raw.size() + 7u) / 8u, 0u);
  auto* buf = reinterpret_cast<uint8_t*>(aligned_words.data());
  std::memcpy(buf, raw.data(), raw.size());
  std::span<const uint8_t> slab(buf, raw.size());

  // Verify Full, HeaderOnly, and Unchecked all yield the exact same root pointer & fields
  const auto res_full = ap::view(slab, ap::VerifyMode::Full);
  assert(res_full.ok());
  const auto res_hdr = ap::view(slab, ap::VerifyMode::HeaderOnly);
  assert(res_hdr.ok());
  const ap::PackRoot* r_un = ap::view_unchecked(buf);
  assert(res_full.value == res_hdr.value);
  assert(res_full.value == r_un);

  const ap::PackRoot& root = *res_full.value;
  assert(root.version == 7u);
  assert(root.color_space == ap::ColorSpace::DisplayP3);
  assert(root.created_ns == UINT64_C(1735689600123456789));

  const auto title = root.title.as_str(slab);
  assert(title.ok());
  assert(title.value == "NVE Zero-Copy Pack \xF0\x9F\x9A\x80");
  assert(root.title.as_str_unchecked(buf) == title.value);

  const auto palette = root.palette.resolve(slab);
  assert(palette.ok());
  assert(palette.value.size() == 3u);
  assert(palette.value[0] == 0xFF112233u);
  assert(palette.value[1] == 0xFF445566u);
  assert(palette.value[2] == 0xFF778899u);

  const auto layers = root.layers.resolve(slab);
  assert(layers.ok());
  assert(layers.value.size() == 2u);
  assert(layers.value[0].id == 101u);
  assert(layers.value[0].kind == ap::LayerKind::Vector);
  assert(layers.value[0].blend == ap::BlendMode::Multiply);
  assert(layers.value[0].visible == true);
  assert(layers.value[0].name.as_str_unchecked(buf) == "hero-vector");

  const auto pts = layers.value[0].points.resolve(slab);
  assert(pts.ok());
  assert(pts.value.size() == 3u);
  assert(std::fabs(pts.value[1].x - 120.5f) < 1e-6f);
  assert(std::fabs(pts.value[1].y - 42.25f) < 1e-6f);
  assert(pts.value[1].as_bytes().size() == 8u);

  assert(layers.value[1].id == 202u);
  assert(layers.value[1].kind == ap::LayerKind::Text);
  assert(layers.value[1].blend == ap::BlendMode::Screen);
  assert(layers.value[1].visible == false);
  assert(layers.value[1].points.as_span_unchecked(buf).empty());

  // --- Adversarial C++20 security tests under VerifyMode::Full ---
  const uint32_t root_off = ap::detail::read_u32_le(buf + 24u);
  const uint32_t layers_off = ap::detail::read_u32_le(buf + root_off + 32u);
  const uint32_t title_off = ap::detail::read_u32_le(buf + root_off + 16u);

  // 1. Truncated
  assert(ap::view_full(buf, 0u).error == ap::RelocError::Truncated);
  assert(ap::view_full(buf, ap::kHeaderSize - 1u).error == ap::RelocError::Truncated);

  // 2. Misaligned base pointer
  std::vector<uint8_t> mis(raw.size() + 16u, 0u);
  const size_t mis_off = (reinterpret_cast<uintptr_t>(mis.data()) % 8u == 0u) ? 1u : 0u;
  std::memcpy(mis.data() + mis_off, raw.data(), raw.size());
  assert(ap::view_full(mis.data() + mis_off, raw.size()).error == ap::RelocError::Misaligned);

  auto clone_words = [&]() {{ return aligned_words; }};

  // 3. Wrong magic
  {{
    auto w = clone_words();
    auto* p = reinterpret_cast<uint8_t*>(w.data());
    p[0] ^= 0xFFu;
    assert(ap::view_full(p, raw.size()).error == ap::RelocError::InvalidMagic);
  }}

  // 4. Wrong schema hash
  {{
    auto w = clone_words();
    auto* p = reinterpret_cast<uint8_t*>(w.data());
    p[8] ^= 0x01u;
    assert(ap::view_full(p, raw.size()).error == ap::RelocError::SchemaHashMismatch);
  }}

  // 5. Out-of-bounds RelocSlice
  {{
    auto w = clone_words();
    auto* p = reinterpret_cast<uint8_t*>(w.data());
    write_le32(p + root_off + 24u, static_cast<uint32_t>(raw.size()));
    assert(ap::view_full(p, raw.size()).error == ap::RelocError::OutOfBounds);
  }}

  // 6. u32::MAX overflow on RelocSlice
  {{
    auto w = clone_words();
    auto* p = reinterpret_cast<uint8_t*>(w.data());
    write_le32(p + root_off + 28u, std::numeric_limits<uint32_t>::max());
    assert(ap::view_full(p, raw.size()).error == ap::RelocError::LengthOverflow);
  }}

  // 7. Invalid UTF-8 (overlong 0xC0 0x80 and UTF-16 surrogate 0xED 0xA0 0x80)
  {{
    auto w = clone_words();
    auto* p = reinterpret_cast<uint8_t*>(w.data());
    p[title_off] = 0xC0u;
    p[title_off + 1u] = 0x80u;
    assert(ap::view_full(p, raw.size()).error == ap::RelocError::InvalidUtf8);
  }}
  {{
    auto w = clone_words();
    auto* p = reinterpret_cast<uint8_t*>(w.data());
    p[title_off] = 0xEDu;
    p[title_off + 1u] = 0xA0u;
    p[title_off + 2u] = 0x80u;
    assert(ap::view_full(p, raw.size()).error == ap::RelocError::InvalidUtf8);
  }}

  // 8. Invalid bool
  {{
    auto w = clone_words();
    auto* p = reinterpret_cast<uint8_t*>(w.data());
    p[layers_off + 7u] = 2u;
    assert(ap::view_full(p, raw.size()).error == ap::RelocError::InvalidBool);
  }}

  // 9. Invalid enum discriminants (u8, u16, u32)
  {{
    auto w = clone_words();
    auto* p = reinterpret_cast<uint8_t*>(w.data());
    p[layers_off + 6u] = 99u;
    assert(ap::view_full(p, raw.size()).error == ap::RelocError::InvalidEnumDiscriminant);
  }}
  {{
    auto w = clone_words();
    auto* p = reinterpret_cast<uint8_t*>(w.data());
    write_le16(p + layers_off + 4u, 777u);
    assert(ap::view_full(p, raw.size()).error == ap::RelocError::InvalidEnumDiscriminant);
  }}
  {{
    auto w = clone_words();
    auto* p = reinterpret_cast<uint8_t*>(w.data());
    write_le32(p + root_off + 4u, 42u);
    assert(ap::view_full(p, raw.size()).error == ap::RelocError::InvalidEnumDiscriminant);
  }}

  // 10. Depth limit on cyclic SceneTree header
  {{
    alignas(8) uint8_t tree_buf[64] = {{}};
    std::memcpy(tree_buf, st::kSchemaMagic.data(), 8u);
    const uint64_t th = st::kSchemaHash;
    std::memcpy(tree_buf + 8u, &th, 8u);
    write_le32(tree_buf + 16u, 64u);
    write_le32(tree_buf + 24u, 32u); // root_offset = 32
    // TreeNode at offset 32: children at +16 points back to 32 with len 1
    write_le32(tree_buf + 32u + 16u, 32u);
    write_le32(tree_buf + 32u + 20u, 1u);
    assert(st::view_full(tree_buf, sizeof(tree_buf)).error == st::RelocError::DepthExceeded);
  }}

  return 0;
}}
"#,
            slab_bin = slab_bin_path.display().to_string().replace('\\', "/"),
        ),
    )
    .unwrap();

    let cpp_exe = dir.join("cpp_reloc_driver");
    let clang_out = Command::new("clang++")
        .args(["-std=c++20", "-Wall", "-Wextra", "-Werror"])
        .arg(format!("-I{}", dir.display()))
        .arg(&cpp_driver)
        .arg("-o")
        .arg(&cpp_exe)
        .output()
        .expect("run clang++");
    assert!(
        clang_out.status.success(),
        "clang++ -std=c++20 -Wall -Wextra -Werror failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&clang_out.stdout),
        String::from_utf8_lossy(&clang_out.stderr)
    );

    let cpp_run = Command::new(&cpp_exe).output().expect("execute cpp driver");
    assert!(
        cpp_run.status.success(),
        "cpp reloc driver failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&cpp_run.stdout),
        String::from_utf8_lossy(&cpp_run.stderr)
    );

    assert_eq!(RELOC_HEADER_SIZE, 32);
    let _ = std::fs::remove_dir_all(&dir);
}
