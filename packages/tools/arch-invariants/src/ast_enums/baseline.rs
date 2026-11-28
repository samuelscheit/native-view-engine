//! Shrink-only monotonic ratchet allowlist for pre-existing duplicate enum match/assignment tables.
//!
//! - Any **new** duplicate enum match/assignment anywhere in the workspace fails `cargo test -p nve-arch-invariants` immediately.
//! - Any **stale** entry in `ALLOWLISTED_DUPLICATES` whose actual hit count drops below `expected_count` fails immediately,
//!   forcing the author who deduplicated it to shrink or delete the entry.
//! - Under `NVE_STRICT_INVARIANTS=1b1d`, `allowlisted_violations` are also reported and enforced.

use std::collections::BTreeMap;

use crate::Violation;

use super::fingerprint::{DuplicateKind, MatchDuplicate};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllowlistedDuplicate {
    pub kind: DuplicateKind,
    pub file_a: &'static str,
    pub symbol_a: &'static str,
    pub file_b: &'static str,
    pub symbol_b: &'static str,
    pub enum_name: &'static str,
    pub expected_count: usize,
    pub tracking: &'static str,
    pub reason: &'static str,
}

impl AllowlistedDuplicate {
    fn sort_key(&self) -> (DuplicateKind, &str, &str, &str, &str, &str) {
        (
            self.kind,
            self.file_a,
            self.symbol_a,
            self.file_b,
            self.symbol_b,
            self.enum_name,
        )
    }
}

const fn entry(
    kind: DuplicateKind,
    a: (&'static str, &'static str),
    b: (&'static str, &'static str),
    enum_name: &'static str,
    expected_count: usize,
    tracking: &'static str,
    reason: &'static str,
) -> AllowlistedDuplicate {
    AllowlistedDuplicate {
        kind,
        file_a: a.0,
        symbol_a: a.1,
        file_b: b.0,
        symbol_b: b.1,
        enum_name,
        expected_count,
        tracking,
        reason,
    }
}

#[derive(Debug, Default)]
pub struct DuplicateMatchReport {
    /// New un-allowlisted duplicates AND stale/malformed allowlist entries (always enforced).
    pub active_violations: Vec<Violation>,
    /// Pre-existing allowlisted duplicates (reported/enforced under `Stage::Post1B1D`).
    pub allowlisted_violations: Vec<Violation>,
}

const SAME: DuplicateKind = DuplicateKind::SameDirection;
const MIRR: DuplicateKind = DuplicateKind::Mirrored;

/// Strictly sorted shrink-only baseline of verified legacy duplicate enum match/assignment tables
/// audited on `integration` (`521f7f95`). Shrink or delete entries whenever duplicates are collapsed.
#[rustfmt::skip]
pub const ALLOWLISTED_DUPLICATES: &[AllowlistedDuplicate] = &[
    // --- Same-direction duplicates (11 hits across 11 unique symbol pairs) ---
    entry(SAME, ("examples/studio-workspace/src/theme.rs", "polarity_index"), ("packages/framework/ui/src/theme/presets.rs", "Theme::preset"), "Polarity", 1, "track-theme", "Duplicate Polarity->0..3 index match"),
    entry(SAME, ("packages/core/codegen/src/frontend/rn_spec/calls.rs", "Parser::parse_component_options_object"), ("packages/core/codegen/src/frontend/rn_spec/mod.rs", "Parser::skip_to_statement_end"), "TokenKind", 1, "track-codegen", "Duplicate TokenKind balanced-delimiter skipper"),
    entry(SAME, ("packages/core/codegen/src/frontend/rn_spec/mod.rs", "Parser::parse_export_default"), ("packages/core/codegen/src/frontend/rn_spec/mod.rs", "Parser::parse_var_statement"), "TokenKind", 1, "track-codegen", "Duplicate TokenKind balanced-delimiter loop"),
    entry(SAME, ("packages/core/codegen/src/targets/dart/hybrid/client_emitter.rs", "emit_dart_direct_prop_write"), ("packages/core/codegen/src/targets/dart/hybrid/client_emitter.rs", "emit_dart_direct_prop_write"), "HybridType", 1, "track-codegen", "Duplicate Dart direct-prop dispatch in single function"),
    entry(SAME, ("packages/core/codegen/src/targets/dart/hybrid/type_helpers.rs", "dart_ffi_call_param_type"), ("packages/core/codegen/src/targets/dart/hybrid/type_helpers.rs", "dart_ffi_call_return_type"), "HybridType", 1, "track-codegen", "Duplicate Dart FFI call scalar type table"),
    entry(SAME, ("packages/core/codegen/src/targets/dart/hybrid/type_helpers.rs", "dart_ffi_call_param_type"), ("packages/core/codegen/src/targets/dart/hybrid/type_helpers.rs", "dart_type"), "HybridType", 1, "track-codegen", "Duplicate Dart scalar type table"),
    entry(SAME, ("packages/core/codegen/src/targets/dart/hybrid/type_helpers.rs", "dart_ffi_call_return_type"), ("packages/core/codegen/src/targets/dart/hybrid/type_helpers.rs", "dart_type"), "HybridType", 1, "track-codegen", "Duplicate Dart scalar type table"),
    entry(SAME, ("packages/core/codegen/src/targets/kotlin_java/kotlin_hybrid/type_helpers.rs", "jni_cpp_param_type"), ("packages/core/codegen/src/targets/kotlin_java/kotlin_hybrid/type_helpers.rs", "jni_cpp_return_type"), "HybridType", 1, "track-codegen", "Duplicate JNI C++ param/return scalar type table"),
    entry(SAME, ("packages/core/codegen/src/targets/kotlin_java/kotlin_hybrid/type_helpers.rs", "kotlin_jni_param_type"), ("packages/core/codegen/src/targets/kotlin_java/kotlin_hybrid/type_helpers.rs", "kotlin_jni_return_type"), "HybridType", 1, "track-codegen", "Duplicate Kotlin JNI param/return scalar type table"),
    entry(SAME, ("packages/framework/router/src/channel.rs", "apply_command_to_router"), ("packages/framework/router/src/channel.rs", "apply_command_to_tabs"), "NavigationAction", 1, "track-router", "Duplicate NavigationAction dispatch across router/tabs channel"),
    entry(SAME, ("packages/framework/router/src/tabs.rs", "TabRouterState::open_modal"), ("packages/framework/router/src/tabs.rs", "TabRouterState::select_tab_index"), "GuardOutcome", 1, "track-router", "Duplicate GuardOutcome match in TabRouterState"),

    // --- Mirrored / bidirectional duplicates (11 hits across 11 unique symbol pairs) ---
    entry(MIRR, ("examples/studio-workspace/src/routes.rs", "StudioRoute::active_view"), ("examples/studio-workspace/src/routes.rs", "StudioRoute::from_view"), "StudioRoute", 1, "track-studio", "Bidirectional StudioRoute<->ActiveView table"),
    entry(MIRR, ("examples/studio-workspace/src/state/codec.rs", "WorkspaceState::from_bytes"), ("examples/studio-workspace/src/state/codec.rs", "WorkspaceState::to_bytes"), "SyncTransportMode", 1, "track-studio", "Bidirectional SyncTransportMode<->u8 codec"),
    entry(MIRR, ("examples/studio-workspace/src/state/codec.rs", "decode_snapshot"), ("examples/studio-workspace/src/state/codec.rs", "encode_snapshot"), "AccentTheme", 1, "track-studio", "Bidirectional AccentTheme<->u8 codec"),
    entry(MIRR, ("examples/studio-workspace/src/state/codec.rs", "decode_snapshot"), ("examples/studio-workspace/src/state/codec.rs", "encode_snapshot"), "ActiveView", 1, "track-studio", "Bidirectional ActiveView<->u8 codec"),
    entry(MIRR, ("examples/studio-workspace/src/state/codec_io.rs", "decode_islands"), ("examples/studio-workspace/src/state/codec_io.rs", "encode_islands"), "IslandHostKind", 1, "track-studio", "Bidirectional IslandHostKind<->u8 codec"),
    entry(MIRR, ("examples/studio-workspace/src/state/codec_io.rs", "decode_workbench"), ("examples/studio-workspace/src/state/codec_io.rs", "encode_workbench"), "WorkbenchPreset", 1, "track-studio", "Bidirectional WorkbenchPreset<->u8 codec"),
    entry(MIRR, ("packages/framework/i18n/src/plural.rs", "PluralCategory::as_str"), ("packages/framework/i18n/src/plural.rs", "PluralCategory::parse"), "PluralCategory", 1, "track-i18n", "Bidirectional PluralCategory<->CLDR str table"),
    entry(MIRR, ("packages/modules/tv/src/action.rs", "RemoteAction::from_key_name"), ("packages/modules/tv/src/action.rs", "RemoteAction::key_name"), "RemoteAction", 1, "track-tv", "Bidirectional RemoteAction<->W3C key table (20 arms)"),
    entry(MIRR, ("packages/modules/tv/src/action.rs", "RemoteAction::key_name"), ("packages/modules/tv/src/web_keys.rs", "from_code_name"), "RemoteAction", 1, "track-tv", "Cross-file bidirectional RemoteAction<->W3C code table"),
];

type PairKey<'a> = (DuplicateKind, &'a str, &'a str, &'a str, &'a str, &'a str);

pub fn evaluate_duplicates_with_ratchet(
    duplicates: &[MatchDuplicate],
    allowlist: &[AllowlistedDuplicate],
) -> DuplicateMatchReport {
    let mut report = DuplicateMatchReport::default();

    for w in allowlist.windows(2) {
        if w[0].sort_key() >= w[1].sort_key() {
            report.active_violations.push(Violation::new(
                "packages/tools/arch-invariants/src/ast_enums/baseline.rs",
                0,
                format!(
                    "{}::{} vs {}::{}",
                    w[1].file_a, w[1].symbol_a, w[1].file_b, w[1].symbol_b
                ),
                "ALLOWLISTED_DUPLICATES must remain strictly sorted with unique keys",
            ));
        }
    }

    let mut allowed_map: BTreeMap<PairKey<'_>, &AllowlistedDuplicate> = BTreeMap::new();
    for item in allowlist {
        if item.expected_count == 0 {
            report.active_violations.push(Violation::new(
                item.file_a,
                0,
                format!("{}::{}", item.file_a, item.symbol_a),
                "ALLOWLISTED_DUPLICATES entry has expected_count == 0; delete it",
            ));
            continue;
        }
        allowed_map.insert(item.sort_key(), item);
    }

    let mut seen_counts: BTreeMap<PairKey<'_>, usize> = BTreeMap::new();

    for dup in duplicates {
        let key: PairKey<'_> = (
            dup.kind,
            dup.file_a.as_str(),
            dup.symbol_a.as_str(),
            dup.file_b.as_str(),
            dup.symbol_b.as_str(),
            dup.enum_name.as_str(),
        );
        let violation = format_duplicate_violation(dup, allowed_map.get(&key).copied());

        if let Some(&allowed) = allowed_map.get(&key) {
            let count = seen_counts.entry(key).or_insert(0);
            *count += 1;
            if *count > allowed.expected_count {
                report.active_violations.push(Violation::new(
                    &dup.file_a,
                    dup.line_a,
                    &dup.snippet_a,
                    format!(
                        "Duplicate enum match count grew to {count} (allowlist ceiling {}): {}",
                        allowed.expected_count, violation.reason
                    ),
                ));
            } else {
                report.allowlisted_violations.push(violation);
            }
        } else {
            report.active_violations.push(violation);
        }
    }

    for item in allowlist {
        if item.expected_count == 0 {
            continue;
        }
        let key = item.sort_key();
        let actual = seen_counts.get(&key).copied().unwrap_or(0);
        if actual < item.expected_count {
            report.active_violations.push(Violation::new(
                item.file_a,
                0,
                format!(
                    "[{}] {}::{} <-> {}::{} (enum {}, actual={actual}, allowlist={})",
                    item.kind.label(),
                    item.file_a,
                    item.symbol_a,
                    item.file_b,
                    item.symbol_b,
                    item.enum_name,
                    item.expected_count
                ),
                "Stale ALLOWLISTED_DUPLICATES entry (duplicate was fixed or reduced) — lower `expected_count` or delete it from baseline.rs",
            ));
        }
    }

    report
}

fn format_duplicate_violation(
    dup: &MatchDuplicate,
    allowed: Option<&AllowlistedDuplicate>,
) -> Violation {
    let tag = allowed
        .map(|e| format!(" [allowlisted: {} — {}]", e.tracking, e.reason))
        .unwrap_or_default();
    let reason = match dup.kind {
        DuplicateKind::SameDirection => format!(
            "Duplicated enum match/assignment dispatch on `{}` ({} identical arms, {} distinct bodies) between `{}` ({}:{}) and `{}` ({}:{}) — extract one shared helper/table{tag}",
            dup.enum_name,
            dup.overlap_arms,
            dup.distinct_bodies,
            dup.symbol_a,
            dup.file_a,
            dup.line_a,
            dup.symbol_b,
            dup.file_b,
            dup.line_b,
        ),
        DuplicateKind::Mirrored => format!(
            "Duplicated bidirectional/mirrored enum mapping on `{}` ({} mirrored arms) between `{}` ({}:{}) and `{}` ({}:{}) — generate from one source of truth{tag}",
            dup.enum_name,
            dup.overlap_arms,
            dup.symbol_a,
            dup.file_a,
            dup.line_a,
            dup.symbol_b,
            dup.file_b,
            dup.line_b,
        ),
    };
    Violation::new(&dup.file_a, dup.line_a, &dup.snippet_a, reason)
}
