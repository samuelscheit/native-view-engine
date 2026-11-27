//! One registry of every type name the hybrid targets generate, so no two items of a spec ever
//! share one (user-declared or generated) in any target language.

use std::collections::BTreeMap;

use crate::error::CodegenError;
use crate::naming::to_pascal_case;
use crate::schema::hybrid::{HybridFileSpec, PodOptionScope};

/// The type namespaces generated names live in. C, C++, Swift (which imports the C header), and
/// Dart share the C spellings; Rust never spells `NveHybrid<Name>` but has its own carriers.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Scope {
    CFamily,
    Rust,
}

const EVERY_SCOPE: &[Scope] = &[Scope::CFamily, Scope::Rust];

/// `(scope, generated type name)` → the spec item it is generated for.
struct Registry(BTreeMap<(Scope, String), String>);

impl Registry {
    /// Claims `name` in `scopes` for `origin`; one item may claim a name more than once.
    fn claim(&mut self, scopes: &[Scope], name: &str, origin: &str) -> Result<(), CodegenError> {
        for &scope in scopes {
            match self.0.get(&(scope, name.to_owned())) {
                Some(owner) if owner != origin => {
                    return Err(CodegenError::validation(format!(
                        "generated type name '{name}' of {origin} collides with the one of {owner}"
                    )));
                }
                Some(_) => {}
                None => {
                    self.0.insert((scope, name.to_owned()), origin.to_owned());
                }
            }
        }
        Ok(())
    }
}

/// Checks every generated type name of every target against every other:
/// - user aliases, enums, and structs: verbatim `Name` everywhere; enums and structs also the C
///   type `NveHybrid<Name>` (C, Swift, Dart FFI), and structs the Rust carrier `NveHybridRaw<Name>`;
/// - modules / views: `Hybrid<P>Spec`, `<P>Client` (C++), `<P>HybridClient` (Swift, Kotlin, Dart,
///   TS), `<P>Hybrid` (Rust trait), `<P>WasmExports` (TS), `NveHybrid<P>VTable`; views also
///   `<P>Props` and `<P><Event>Event`;
/// - C options `NveHybridOpt<Suffix>` (one inner type per suffix), the file's `<Stem>HybridFile` (Swift) and
///   `<Stem>HybridSchema` (Kotlin), and the runtime's `NveHybridStatus` / `NveHybridOwnedBytes`
///   (everywhere) / `NveHybridValue` (Rust).
pub(super) fn validate_generated_type_names(spec: &HybridFileSpec) -> Result<(), CodegenError> {
    let mut names = Registry(BTreeMap::new());
    let runtime = "the hybrid runtime";
    names.claim(EVERY_SCOPE, "NveHybridStatus", runtime)?;
    names.claim(EVERY_SCOPE, "NveHybridOwnedBytes", runtime)?;
    names.claim(&[Scope::Rust], "NveHybridValue", runtime)?;
    let stem = spec.stem_pascal();
    names.claim(EVERY_SCOPE, &format!("{stem}HybridFile"), "the spec file")?;
    names.claim(EVERY_SCOPE, &format!("{stem}HybridSchema"), "the spec file")?;
    for alias in &spec.aliases {
        names.claim(EVERY_SCOPE, &alias.name, &format!("alias '{}'", alias.name))?;
    }
    for e in &spec.enums {
        let origin = format!("enum '{}'", e.name);
        names.claim(EVERY_SCOPE, &e.name, &origin)?;
        names.claim(&[Scope::CFamily], &format!("NveHybrid{}", e.name), &origin)?;
    }
    for s in &spec.structs {
        let origin = format!("struct '{}'", s.name);
        names.claim(EVERY_SCOPE, &s.name, &origin)?;
        names.claim(&[Scope::CFamily], &format!("NveHybrid{}", s.name), &origin)?;
        names.claim(&[Scope::Rust], &format!("NveHybridRaw{}", s.name), &origin)?;
    }
    let modules = spec.modules.iter().map(|m| ("module", &m.name, None));
    for (kind, name, view) in modules.chain(spec.views.iter().map(|v| ("view", &v.name, Some(v)))) {
        let origin = format!("{kind} '{name}'");
        let pascal = to_pascal_case(name);
        for pattern in [
            "Hybrid{}Spec",
            "{}Client",
            "{}HybridClient",
            "{}Hybrid",
            "{}WasmExports",
            "NveHybrid{}VTable",
        ] {
            names.claim(EVERY_SCOPE, &pattern.replace("{}", &pascal), &origin)?;
        }
        let Some(view) = view else { continue };
        names.claim(EVERY_SCOPE, &format!("{pascal}Props"), &origin)?;
        for ev in &view.events {
            let event = format!("{pascal}{}Event", to_pascal_case(&ev.name));
            names.claim(
                EVERY_SCOPE,
                &event,
                &format!("{origin} event '{}'", ev.name),
            )?;
        }
    }
    // Every use, not one per suffix: `Option<u32>` and `Option<U32>` (struct `U32`) would share
    // `NveHybridOpt<U32>` with two layouts.
    for (suffix, inner) in spec.pod_option_uses(PodOptionScope::WithCFunctions) {
        let origin = format!("the C option of `{}`", inner.canonical_signature());
        names.claim(EVERY_SCOPE, &format!("NveHybridOpt{suffix}"), &origin)?;
    }
    Ok(())
}
