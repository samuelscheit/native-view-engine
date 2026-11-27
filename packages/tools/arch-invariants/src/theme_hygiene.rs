//! Unified-theme source hygiene for every themed UI crate: colors come from `nve_ui::Theme`
//! tokens only, a tone's `solid` is only ever a fill, and themed hosts read the mounted theme mode
//! directly instead of scraping the rendered tree.

use crate::{is_test_or_bench_rs, RepoIndex, Violation};

/// Source roots whose colors must all come from `nve_ui::Theme`.
pub const THEMED_SOURCE_DIRS: &[&str] = &[
    "packages/framework/ui/src",
    "packages/framework/docs/src",
    "examples/nve-docs-site/src",
    "examples/studio-workspace/src",
    "examples/kitchen-sink/src",
    "examples/fullstack-issues/client/src",
];

/// Files that legitimately hold color literals: the token generator itself and the CSS color
/// parser (named-color table).
const LITERAL_EXEMPT: &[&str] = &["packages/framework/ui/src/theme/", "packages/framework/ui/src/style_buf/typed.rs"];

/// Files allowed to pair `solid` freely: the theme generator and the role table that defines
/// the pairings every component paints through.
const SOLID_EXEMPT: &[&str] = &["packages/framework/ui/src/theme/", "packages/framework/ui/src/components/roles.rs"];

const STALE_THEME_NAMES: &[&str] = &["DocsTheme", "UiTheme", "UiPalette"];

/// Production `.rs` sources under [`THEMED_SOURCE_DIRS`] as `(path, text)`.
fn themed_sources(idx: &RepoIndex) -> Vec<(String, String)> {
    THEMED_SOURCE_DIRS
        .iter()
        .flat_map(|dir| idx.rs_files_under(dir))
        .filter(|rel| !is_test_or_bench_rs(rel))
        .filter_map(|rel| idx.read_text(&rel).map(|text| (rel, text)))
        .collect()
}

/// Code lines (skipping `//` comment lines) with 1-based numbers.
fn code_lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.lines().enumerate().filter(|(_, l)| !l.trim_start().starts_with("//")).map(|(i, l)| (i + 1, l))
}

/// `#rgb` / `#rrggbb` / `#rrggbbaa` followed by a non-identifier character.
pub fn has_css_hex(line: &str) -> bool {
    let bytes = line.as_bytes();
    bytes.iter().enumerate().any(|(i, &b)| {
        if b != b'#' {
            return false;
        }
        let digits = bytes[i + 1..].iter().take_while(|c| c.is_ascii_hexdigit()).count();
        let end_ok = bytes.get(i + 1 + digits).is_none_or(|c| !c.is_ascii_alphanumeric() && *c != b'_');
        matches!(digits, 3 | 6 | 8) && end_ok
    })
}

/// A standalone CSS color function call (`rgb(`, `rgba(`, `hsl(`, `hsla(`, `oklch(`): the name
/// is not the tail of a longer identifier or path (`accent_rgba(`, `ColorRgba::rgb(`).
pub fn has_css_color_fn(line: &str) -> bool {
    ["rgb(", "rgba(", "hsl(", "hsla(", "oklch("].iter().any(|f| {
        line.match_indices(f).any(|(i, _)| {
            line.as_bytes()[..i].last().is_none_or(|c| !c.is_ascii_alphanumeric() && *c != b'_' && *c != b':')
        })
    })
}

/// What a line does wrong with colors, if anything.
pub fn color_literal_problem(line: &str) -> Option<&'static str> {
    if has_css_hex(line) || has_css_color_fn(line) {
        Some("CSS color literal; use a `Theme` token")
    } else if line.contains("ColorRgba::rgb(") || line.contains("ColorRgba::rgba(") {
        Some("hardcoded `ColorRgba`; use a `Theme` token")
    } else if line.contains(".hex().to_string()") {
        Some("per-render hex allocation; pass the `ColorRgba` or `Hex` itself")
    } else if STALE_THEME_NAMES.iter().any(|s| line.contains(s)) {
        Some("stale pre-unification theme type")
    } else {
        None
    }
}

/// Themed sources use `Theme` tokens only: no CSS or `ColorRgba` literals, no per-render hex
/// strings, no stale theme types.
pub fn check_theme_token_hygiene(idx: &RepoIndex) -> Vec<Violation> {
    let mut out = Vec::new();
    for (rel, text) in themed_sources(idx) {
        if LITERAL_EXEMPT.iter().any(|p| rel.starts_with(p)) {
            continue;
        }
        for (n, line) in code_lines(&text) {
            if let Some(why) = color_literal_problem(line) {
                out.push(Violation::new(&rel, n, line.trim(), why));
            }
        }
    }
    out
}

/// Byte offsets of `.solid` field reads (not `on_solid`, not a `solid(` call).
fn solid_reads(line: &str) -> impl Iterator<Item = usize> + '_ {
    line.match_indices(".solid").map(|(i, _)| i).filter(move |&i| {
        let after = line.as_bytes().get(i + ".solid".len());
        after.is_none_or(|c| !c.is_ascii_alphanumeric() && *c != b'_' && *c != b'(')
    })
}

/// `true` when the statement around `.solid` paints a fill (`background`, `bg`, `fill`).
pub fn solid_is_fill(context: &str) -> bool {
    ["background", "bg", "fill"].iter().any(|k| context.contains(k))
}

/// WCAG 1.4.11: a tone's `solid` may fall below 3:1 on surfaces, so it is only ever a fill (with
/// `on_solid` ink). Borders, rails, strokes, dots-as-state, and text use the tone's `text`
/// (`accent_indicator`) or `border`. The context is the line plus the two before it, which
/// covers `let bg = if active {` / `.bg(if … {` splits.
pub fn check_solid_only_as_fill(idx: &RepoIndex) -> Vec<Violation> {
    let mut out = Vec::new();
    for (rel, text) in themed_sources(idx) {
        if SOLID_EXEMPT.iter().any(|p| rel.starts_with(p)) {
            continue;
        }
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if line.trim_start().starts_with("//") || solid_reads(line).next().is_none() {
                continue;
            }
            let context = lines[i.saturating_sub(2)..=i].join("\n");
            if !solid_is_fill(&context) {
                out.push(Violation::new(
                    &rel,
                    i + 1,
                    line.trim(),
                    "`Tone::solid` outside a fill; use `accent_indicator` / tone `.text` / `.border` for non-text indicators",
                ));
            }
        }
    }
    out
}

/// A rendered-tree attribute read of a theme value (`data_attr_of(.., "data-theme-mode")`).
pub fn is_theme_tree_scrape(line: &str) -> bool {
    line.contains("data_attr_of(") && line.contains("theme")
}

/// Hosts and examples read the mounted docs theme mode from its shared state, never by walking
/// the rendered tree for a `data-theme*` attribute.
pub fn check_no_theme_tree_scraping(idx: &RepoIndex) -> Vec<Violation> {
    let mut out = Vec::new();
    let mut files = idx.rs_files_under("examples");
    files.extend(idx.rs_files_under("hosts"));
    files.extend(idx.rs_files_under("packages/framework"));
    for rel in files {
        let Some(text) = idx.read_text(&rel) else { continue };
        for (n, line) in code_lines(&text) {
            if is_theme_tree_scrape(line) {
                out.push(Violation::new(&rel, n, line.trim(), "theme mode scraped from the tree; read the mounted theme state"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_literals_are_classified_by_kind() {
        let css = Some("CSS color literal; use a `Theme` token");
        assert_eq!(color_literal_problem(r#"style: "color: #0f172a;""#), css);
        assert_eq!(color_literal_problem(r#"style: "color: rgba(0, 0, 0, 0.5);""#), css);
        assert_eq!(color_literal_problem(r#"style: "fill: oklch(0.7 0.1 200);""#), css);
        assert_eq!(color_literal_problem("let c = ColorRgba::rgb(1, 2, 3);"), Some("hardcoded `ColorRgba`; use a `Theme` token"));
        assert_eq!(
            color_literal_problem("let s = t.text.hex().to_string();"),
            Some("per-render hex allocation; pass the `ColorRgba` or `Hex` itself")
        );
        assert_eq!(color_literal_problem("struct DocsTheme;"), Some("stale pre-unification theme type"));
    }

    #[test]
    fn identifiers_ending_in_a_color_function_name_are_not_literals() {
        for line in [
            "accent_rgba: accent_rgba(AccentTheme::Cyan),",
            "let x = to_rgb(v);",
            "#[derive(Debug)]",
            "let id = \"#abcdefg\";",
            "let anchor = \"#install\";",
        ] {
            assert_eq!(color_literal_problem(line), None, "{line}");
        }
    }

    #[test]
    fn theme_tree_scrapes_are_detected() {
        assert!(is_theme_tree_scrape(r#"let mode = data_attr_of(store, root, "data-theme-mode");"#));
        assert!(!is_theme_tree_scrape(r#"let id = data_attr_of(store, root, "data-nve-id");"#));
        assert!(!is_theme_tree_scrape("let mode = site.theme_mode.get();"));
    }

    #[test]
    fn solid_reads_skip_on_solid_and_constructor_calls() {
        assert_eq!(solid_reads("bg: tone.solid,").count(), 1);
        assert_eq!(solid_reads("fg: tone.on_solid, ControlColors::solid(t, tone)").count(), 0);
        assert!(solid_is_fill("background-color: {}\n tone.solid"));
        assert!(!solid_is_fill("border-color: {}\n tone.solid"));
    }
}
