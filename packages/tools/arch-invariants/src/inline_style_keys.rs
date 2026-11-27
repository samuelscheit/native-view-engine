//! Every key in an rsx `style: "..."` literal must be a property the inline style parser knows
//! (`nve_components::css_key_bits`). The parser skips unknown keys without a trace, so an
//! unsupported key ships as a declaration that silently never applies (e.g. `flex-basis`,
//! `padding-bottom` and `font-family` were dropped before they were added to the parser).

use nve_components::css_key_bits;

use crate::{is_test_or_bench_rs, RepoIndex, Violation};

/// Roots holding rsx UI code.
const UI_SOURCE_ROOTS: &[&str] = &["packages", "examples", "hosts"];

/// Keys of the declarations in `literal`, the body of one `style: "..."` string. An interpolated
/// fragment (`{BASE_STYLE} padding: 4px`) is skipped: its own keys are checked where it is defined.
pub fn style_literal_keys(literal: &str) -> impl Iterator<Item = &str> {
    literal
        .split(';')
        .filter_map(|decl| decl.split_once(':'))
        .map(|(key, _)| key.rsplit_once('}').map_or(key, |(_, tail)| tail).trim())
        .filter(|key| !key.is_empty())
}

/// Bodies of every `style: "..."` literal on `line`.
pub fn style_literals(line: &str) -> impl Iterator<Item = &str> {
    line.match_indices("style: \"").filter_map(|(i, m)| {
        let rest = &line[i + m.len()..];
        rest.find('"').map(|end| &rest[..end])
    })
}

/// Unknown inline-style keys in production rsx sources.
pub fn check_inline_style_keys_known(idx: &RepoIndex) -> Vec<Violation> {
    let mut out = Vec::new();
    for rel in UI_SOURCE_ROOTS.iter().flat_map(|root| idx.rs_files_under(root)) {
        if is_test_or_bench_rs(&rel) || rel.starts_with("packages/tools/arch-invariants/") {
            continue;
        }
        let Some(text) = idx.read_text(&rel) else { continue };
        for (i, line) in text.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            for key in style_literals(line).flat_map(style_literal_keys) {
                if css_key_bits(key).is_none() {
                    out.push(Violation {
                        path: rel.clone(),
                        line: i + 1,
                        snippet: line.trim().chars().take(160).collect(),
                        reason: format!("inline style key `{key}` is not parsed by nve_components::css_key_bits and silently never applies"),
                    });
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{style_literal_keys, style_literals};

    #[test]
    fn extracts_keys_from_every_literal_including_interpolated_values() {
        let line = r#"view { style: "{BASE} width: 100%; color: {theme.text.primary.hex()};", text { style: "flex-basis: 0%" } }"#;
        let keys: Vec<&str> = style_literals(line).flat_map(style_literal_keys).collect();
        assert_eq!(keys, ["width", "color", "flex-basis"]);
    }
}
