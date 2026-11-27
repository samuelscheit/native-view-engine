//! Host ABI data generated with Askama templates (`packages/core/codegen/templates/`): the merged
//! Core + Scene ABI, the word-table field specs, and the golden XXH64 vectors. Every file lands
//! next to the static runtime it plugs into (`packages/core/host-{cxx,swift,java,dart,ts}`).
//!
//! Inputs are borrowed: `nve-abi`'s `const` [`AbiFileDescriptor`]s and the `&'static TableDesc`s of
//! whichever table crate the caller links (`nve-cli` passes `nve_components::CORE_TABLE_DESCS`), so
//! this crate never depends on a table crate. Templates are checked by `rustc` at compile time.

pub mod layout;
pub mod model;
pub mod spell;
mod validate;

use askama::Template;
use nve_abi::NveHostStatus;
use nve_abi::schema::{
    AbiEnumDescriptor, AbiFileDescriptor, NveEnumSchema, core_abi_descriptor, scene_abi_descriptor,
};
use nve_abi::table::TableDesc;

use crate::error::CodegenError;
use crate::schema::export::GeneratedFile;
use model::{AbiModel, HashVector, TableModel, hash_vectors, tables_model};
use spell::{c, dart, ts};

const CORE: AbiFileDescriptor = core_abi_descriptor();
const SCENE: AbiFileDescriptor = scene_abi_descriptor();
const HOST_STATUS: AbiEnumDescriptor = <NveHostStatus as NveEnumSchema>::SCHEMA;

const CXX: &str = "packages/core/host-cxx";
const SWIFT: &str = "packages/core/host-swift";
const JAVA: &str = "packages/core/host-java/src";
const DART: &str = "packages/core/host-dart";
const TS: &str = "packages/core/host-ts";
const C_ABI_HEADER: &str = "packages/core/host-cxx/include/nve/generated/nve_abi.generated.h"; // under `CXX`

macro_rules! templates {
    ($($name:ident: $path:literal => $field:ident: $ty:ty;)*) => {$(
        #[derive(Template)]
        #[template(path = $path, escape = "none")]
        struct $name<'a> {
            $field: &'a $ty,
        }
    )*};
}

templates! {
    CAbi: "c/nve_abi.h" => m: AbiModel<'a>;
    CTables: "c/nve_tables.h" => tables: [TableModel<'a>];
    CVectors: "c/hash_vectors.h" => vectors: [HashVector];
    SwiftTables: "swift/NveTables.swift" => tables: [TableModel<'a>];
    SwiftVectors: "swift/HashVectors.swift" => vectors: [HashVector];
    JavaAbi: "java/NveAbi.java" => m: AbiModel<'a>;
    JavaTables: "java/NveTables.java" => tables: [TableModel<'a>];
    JavaVectors: "java/HashVectors.java" => vectors: [HashVector];
    DartAbiConst: "dart/nve_abi_const.dart" => m: AbiModel<'a>;
    DartAbiFfi: "dart/nve_abi_ffi.dart" => m: AbiModel<'a>;
    DartTables: "dart/nve_tables.dart" => tables: [TableModel<'a>];
    DartVectors: "dart/hash_vectors.dart" => vectors: [HashVector];
    TsAbi: "ts/nve_abi.ts" => m: AbiModel<'a>;
    TsTables: "ts/nve_tables.ts" => tables: [TableModel<'a>];
    TsVectors: "ts/hash_vectors.ts" => vectors: [HashVector];
}

/// The Core + Scene ABI model of the linked `nve-abi`.
pub fn abi_model() -> Result<AbiModel<'static>, CodegenError> {
    AbiModel::new(&CORE, &SCENE, &HOST_STATUS)
}

/// Renders one template into a pre-sized buffer, ending in exactly one `\n` (Askama drops a
/// template's final newline). Output is LF-only by construction (asserted).
fn render(
    template: &dyn askama::DynTemplate,
    relative_path: String,
) -> Result<GeneratedFile, CodegenError> {
    // Askama's hint covers only the literal text; every file is larger, so start at 16 KiB.
    let mut content = String::with_capacity(template.size_hint().max(16 * 1024));
    template
        .dyn_render_into(&mut content)
        .map_err(|e| CodegenError::validation(format!("rendering {relative_path}: {e}")))?;
    content.push('\n');
    if content.contains('\r') {
        return Err(CodegenError::validation(format!(
            "{relative_path} contains a carriage return"
        )));
    }
    Ok(GeneratedFile {
        relative_path,
        content,
    })
}

/// `nve_abi.generated.h`: the merged C ABI header every C / C++ / Objective-C / Swift host imports.
pub fn generate_c_abi_header() -> Result<String, CodegenError> {
    let m = abi_model()?;
    Ok(render(&CAbi { m: &m }, C_ABI_HEADER.to_owned())?.content)
}

/// Every Core + Scene ABI file (C, Java, Dart const + `dart:ffi`, TypeScript).
pub fn generate_abi_files() -> Result<Vec<GeneratedFile>, CodegenError> {
    let m = abi_model()?;
    Ok(vec![
        render(&CAbi { m: &m }, C_ABI_HEADER.to_owned())?,
        render(
            &JavaAbi { m: &m },
            format!("{JAVA}/main/java/dev/nve/host/generated/NveAbi.java"),
        )?,
        render(
            &DartAbiConst { m: &m },
            format!("{DART}/lib/src/generated/nve_abi_const.generated.dart"),
        )?,
        render(
            &DartAbiFfi { m: &m },
            format!("{DART}/lib/src/generated/nve_abi_ffi.generated.dart"),
        )?,
        render(
            &TsAbi { m: &m },
            format!("{TS}/src/generated/nve_abi.generated.ts"),
        )?,
    ])
}

/// Field specs, keys, and schema hashes of `tables` for every host runtime.
pub fn generate_table_files(tables: &[&TableDesc]) -> Result<Vec<GeneratedFile>, CodegenError> {
    let t = tables_model(tables)?;
    let tables = t.as_slice();
    Ok(vec![
        render(
            &CTables { tables },
            format!("{CXX}/include/nve/generated/nve_tables.generated.h"),
        )?,
        render(
            &SwiftTables { tables },
            format!("{SWIFT}/Sources/NveHost/NveTables.generated.swift"),
        )?,
        render(
            &JavaTables { tables },
            format!("{JAVA}/main/java/dev/nve/host/generated/NveTables.java"),
        )?,
        render(
            &DartTables { tables },
            format!("{DART}/lib/src/generated/nve_tables.generated.dart"),
        )?,
        render(
            &TsTables { tables },
            format!("{TS}/src/generated/nve_tables.generated.ts"),
        )?,
    ])
}

/// The golden XXH64 vectors for every host runtime's test suite.
pub fn generate_hash_vector_files() -> Result<Vec<GeneratedFile>, CodegenError> {
    let v = hash_vectors();
    let vectors = v.as_slice();
    Ok(vec![
        render(
            &CVectors { vectors },
            format!("{CXX}/tests/hash_vectors.generated.h"),
        )?,
        render(
            &SwiftVectors { vectors },
            format!("{SWIFT}/Tests/NveHostTests/HashVectors.generated.swift"),
        )?,
        render(
            &JavaVectors { vectors },
            format!("{JAVA}/test/java/dev/nve/host/generated/HashVectors.java"),
        )?,
        render(
            &DartVectors { vectors },
            format!("{DART}/test/src/hash_vectors.generated.dart"),
        )?,
        render(
            &TsVectors { vectors },
            format!("{TS}/test/hash_vectors.generated.ts"),
        )?,
    ])
}

/// Every generated host file: ABI, `tables`, and the golden XXH64 vectors.
pub fn generate_all_abi_files(tables: &[&TableDesc]) -> Result<Vec<GeneratedFile>, CodegenError> {
    let mut files = generate_abi_files()?;
    files.extend(generate_table_files(tables)?);
    files.extend(generate_hash_vector_files()?);
    Ok(files)
}
