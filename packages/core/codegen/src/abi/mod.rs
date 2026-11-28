//! Host ABI data generated with Askama templates (`packages/core/codegen/templates/`): the Core
//! and runtime scene ABIs, the word-table field specs, and the golden XXH64 vectors. Every file lands
//! next to the static runtime it plugs into (`packages/core/host-{cxx,swift,java,dart,ts}`).
//!
//! Inputs are borrowed: `nve-abi`'s `const` [`AbiFileDescriptor`]s and the `&'static TableDesc`s of
//! whichever table crate the caller links (`nve-cli` passes `nve_tables::CORE_TABLE_DESCS`), so
//! this crate never depends on a table crate. Templates are checked by `rustc` at compile time.

pub mod flat_op;
pub mod java_host;
pub mod jni_frame;
pub mod layout;
pub mod model;
pub mod spell;
pub mod tables;
mod validate;

pub use flat_op::generate_flat_op_codec_files;

use askama::Template;
use nve_abi::schema::{AbiFileDescriptor, core_abi_descriptor, runtime_abi_descriptor};
use nve_abi::table::{TableDesc, ValueSetDesc};

use crate::error::CodegenError;
use crate::schema::export::GeneratedFile;
use model::{AbiModel, HashVector, RuntimeAbiModel, hash_vectors};
use tables::{TableModel, tables_model, value_sets};
use jni_frame::JniFrameModel;
use spell::{c, dart, swift, ts};

const CORE: AbiFileDescriptor = core_abi_descriptor();
const RUNTIME: AbiFileDescriptor = runtime_abi_descriptor();

const CXX: &str = "packages/core/host-cxx";
const SWIFT: &str = "packages/core/host-swift";
const JAVA: &str = "packages/core/host-java/src";
const DART: &str = "packages/core/host-dart";
const TS: &str = "packages/core/host-ts";
const C_ABI_HEADER: &str = "packages/core/host-cxx/include/nve/generated/nve_abi.generated.h"; // under `CXX`
const C_RUNTIME_ABI_HEADER: &str =
    "packages/core/host-cxx/include/nve/generated/nve_runtime_abi.generated.h"; // under `CXX`

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
    CRuntimeAbi: "c/nve_runtime_abi.h" => m: RuntimeAbiModel<'a>;
    CVectors: "c/hash_vectors.h" => vectors: [HashVector];
    SwiftAbiConsts: "swift/NveAbiConsts.swift" => m: AbiModel<'a>;
    SwiftVectors: "swift/HashVectors.swift" => vectors: [HashVector];
    JavaAbi: "java/NveAbi.java" => m: AbiModel<'a>;
    JavaRuntimeAbi: "java/NveRuntimeAbi.java" => m: RuntimeAbiModel<'a>;
    JavaJniFrame: "java/NveJniFrame.java" => m: JniFrameModel;
    CxxJniFrame: "c/nve_jni_frame.hpp" => m: JniFrameModel;
    JavaVectors: "java/HashVectors.java" => vectors: [HashVector];
    DartAbiConst: "dart/nve_abi_const.dart" => m: AbiModel<'a>;
    DartAbiFfi: "dart/nve_abi_ffi.dart" => m: AbiModel<'a>;
    DartRuntimeAbi: "dart/nve_runtime_abi.dart" => m: RuntimeAbiModel<'a>;
    DartVectors: "dart/hash_vectors.dart" => vectors: [HashVector];
    TsTables: "ts/nve_tables.ts" => tables: [TableModel<'a>];
    TsVectors: "ts/hash_vectors.ts" => vectors: [HashVector];
}

/// `nve_abi.generated.ts`: the Core constants and records plus the runtime scene layout
/// (`NVE_SCENE_LAYOUT_WASM32`) the TypeScript `SceneReader` reads.
#[derive(Template)]
#[template(path = "ts/nve_abi.ts", escape = "none")]
struct TsAbi<'a> {
    m: &'a AbiModel<'a>,
    rt: &'a RuntimeAbiModel<'a>,
}

/// The Core ABI model of the linked `nve-abi`.
pub fn abi_model() -> Result<AbiModel<'static>, CodegenError> {
    AbiModel::new(&CORE)
}

/// The runtime ABI model on top of `base` (`abi_model()`).
pub fn runtime_abi_model(base: &AbiModel<'static>) -> Result<RuntimeAbiModel<'static>, CodegenError> {
    RuntimeAbiModel::new(&RUNTIME, base)
}

/// Renders one template into a pre-sized buffer, ending in exactly one `\n` (Askama drops a
/// template's final newline). Output is LF-only by construction (asserted).
pub(crate) fn render(
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

/// Renders the C runtime ABI header for an arbitrary validated [`RuntimeAbiModel`].
pub fn render_c_runtime_abi(m: &RuntimeAbiModel<'_>) -> Result<String, CodegenError> {
    Ok(render(&CRuntimeAbi { m }, C_RUNTIME_ABI_HEADER.to_owned())?.content)
}

/// Renders the Dart runtime ABI bindings for an arbitrary validated [`RuntimeAbiModel`].
pub fn render_dart_runtime_abi(m: &RuntimeAbiModel<'_>) -> Result<String, CodegenError> {
    Ok(render(
        &DartRuntimeAbi { m },
        format!("{DART}/lib/src/generated/nve_runtime_abi.generated.dart"),
    )?
    .content)
}

/// Every Core ABI file (C, Swift constants, Java, Dart const + `dart:ffi`, TypeScript)
/// plus the C runtime ABI header.
pub fn generate_abi_files() -> Result<Vec<GeneratedFile>, CodegenError> {
    let m = abi_model()?;
    let runtime = runtime_abi_model(&m)?;
    let jni = JniFrameModel::new();
    Ok(vec![
        render(&CAbi { m: &m }, C_ABI_HEADER.to_owned())?,
        render(&CRuntimeAbi { m: &runtime }, C_RUNTIME_ABI_HEADER.to_owned())?,
        render(
            &SwiftAbiConsts { m: &m },
            format!("{SWIFT}/Sources/NveHost/NveAbiConsts.generated.swift"),
        )?,
        render(
            &JavaAbi { m: &m },
            format!("{JAVA}/main/java/dev/nve/host/generated/NveAbi.java"),
        )?,
        render(
            &JavaRuntimeAbi { m: &runtime },
            format!("{JAVA}/main/java/dev/nve/host/generated/NveRuntimeAbi.java"),
        )?,
        render(
            &JavaJniFrame { m: &jni },
            format!("{JAVA}/main/java/dev/nve/host/generated/NveJniFrame.java"),
        )?,
        render(
            &CxxJniFrame { m: &jni },
            format!("{CXX}/include/nve/generated/nve_jni_frame.generated.hpp"),
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
            &DartRuntimeAbi { m: &runtime },
            format!("{DART}/lib/src/generated/nve_runtime_abi.generated.dart"),
        )?,
        render(
            &TsAbi { m: &m, rt: &runtime },
            format!("{TS}/src/generated/nve_abi.generated.ts"),
        )?,
    ])
}

/// The C11 / C++20 table header: the core one (`module: None`, adds `NVE_TABLE_COUNT` /
/// `NVE_TABLE_LIST`) or a capability module's (`NVE_<MODULE>_TABLES_GENERATED_H`).
#[derive(Template)]
#[template(path = "c/nve_tables.h", escape = "none")]
struct CTables<'a> {
    tables: &'a [TableModel<'a>],
    value_sets: &'a [&'static ValueSetDesc],
    guard: String,
    table_word: &'a str,
    core: bool,
}

impl<'a> CTables<'a> {
    fn new(
        tables: &'a [TableModel<'a>],
        value_sets: &'a [&'static ValueSetDesc],
        module: Option<&str>,
    ) -> Self {
        let guard = match module {
            Some(m) => format!("NVE_{}_TABLES_GENERATED_H", spell::screaming(m)),
            None => "NVE_TABLES_GENERATED_H".to_owned(),
        };
        let table_word = if module.is_some() { "<nve/host/table_word.h>" } else { "\"../host/table_word.h\"" };
        Self { tables, value_sets, guard, table_word, core: module.is_none() }
    }
}

/// Swift table mirror: field specs plus one `Nve<Set>` enum (variants) or namespace (flags) per
/// value set.
#[derive(Template)]
#[template(path = "swift/NveTables.swift", escape = "none")]
struct SwiftTables<'a> {
    namespace: &'a str,
    core: bool,
    tables: &'a [TableModel<'a>],
    value_sets: &'a [&'static ValueSetDesc],
}

/// Java word-table mirror (`NveTables`, and `NveModuleTables` for capability modules): field
/// specs, default rows, `Info` / `all()`, and one `Values.<Set>` class per value set.
#[derive(Template)]
#[template(path = "java/NveTables.java", escape = "none")]
pub(crate) struct JavaTables<'a> {
    pub(crate) class: &'a str,
    pub(crate) doc: &'a str,
    pub(crate) tables: &'a [TableModel<'a>],
    pub(crate) value_sets: &'a [&'static ValueSetDesc],
}

/// Dart word-table mirror: field specs plus one `Nve<Set>` constant class per value set.
#[derive(Template)]
#[template(path = "dart/nve_tables.dart", escape = "none")]
struct DartTables<'a> {
    tables: &'a [TableModel<'a>],
    value_sets: &'a [&'static ValueSetDesc],
}

/// Field specs, keys, schema hashes, and named field values of `tables` for every host runtime.
pub fn generate_table_files(tables: &[&TableDesc]) -> Result<Vec<GeneratedFile>, CodegenError> {
    let t = tables_model(tables)?;
    let sets = value_sets(&t)?;
    let tables = t.as_slice();
    Ok(vec![
        render(
            &CTables::new(tables, &sets, None),
            format!("{CXX}/include/nve/generated/nve_tables.generated.h"),
        )?,
        render(
            &SwiftTables { namespace: "NveTables", core: true, tables, value_sets: &sets },
            format!("{SWIFT}/Sources/NveHost/NveTables.generated.swift"),
        )?,
        render(
            &JavaTables {
                class: "NveTables",
                doc: "Word-table keys, schema hashes, default rows, field specs, and value sets for {@code TableReader}.",
                tables,
                value_sets: &sets,
            },
            format!("{JAVA}/main/java/dev/nve/host/generated/NveTables.java"),
        )?,
        render(
            &DartTables { tables, value_sets: &sets },
            format!("{DART}/lib/src/generated/nve_tables.generated.dart"),
        )?,
        render(
            &TsTables { tables },
            format!("{TS}/src/generated/nve_tables.generated.ts"),
        )?,
    ])
}

/// The C11 / C++20 mirror of a capability module's tables (`module` = `canvas` ->
/// `NVE_CANVAS_TABLES_GENERATED_H`, includes `<nve/host/table_word.h>` from `nve::host_cxx`).
/// The caller links the module and passes its descriptors, so this crate stays module-free.
pub fn generate_module_table_header(
    module: &str,
    tables: &[&TableDesc],
    relative_path: String,
) -> Result<GeneratedFile, CodegenError> {
    let t = tables_model(tables)?;
    let sets = value_sets(&t)?;
    render(&CTables::new(&t, &sets, Some(module)), relative_path)
}

/// Swift mirror (`NveModuleTables.generated.swift`) of capability-module tables (`nve.canvas`, ...).
pub fn generate_swift_module_tables(tables: &[&TableDesc]) -> Result<GeneratedFile, CodegenError> {
    let t = tables_model(tables)?;
    let sets = value_sets(&t)?;
    render(
        &SwiftTables {
            namespace: "NveModuleTables",
            core: false,
            tables: &t,
            value_sets: &sets,
        },
        format!("{SWIFT}/Sources/NveHost/NveModuleTables.generated.swift"),
    )
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
