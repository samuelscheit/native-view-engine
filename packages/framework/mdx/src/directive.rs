//! Fenced directive (`:::` container and `::` sub-item) parser for `nve-mdx`.

use crate::ast::{
    Block, CalloutKind, CardItem, DirectiveBlock, HydrateStrategy, StepItem, TabItem,
};
use crate::mdx_tag::parse_directive_attrs;
use crate::parser::ParseCtx;

/// Header parsed from a `:::name{...}` line.
#[derive(Debug, Clone)]
pub(crate) struct DirectiveHeader {
    pub colons: usize,
    pub name: String,
    pub attrs_raw: String,
}

/// Try to parse a container opening line (`:::name` or `:::name{...}`).
pub(crate) fn parse_directive_open(trimmed: &str, min_colons: usize) -> Option<DirectiveHeader> {
    let colons = trimmed.bytes().take_while(|&b| b == b':').count();
    if colons < min_colons {
        return None;
    }
    let rest = trimmed[colons..].trim();
    if rest.is_empty() {
        return None;
    }
    let name_end = rest
        .find(|c: char| c.is_whitespace() || c == '{')
        .unwrap_or(rest.len());
    let name = rest[..name_end].trim();
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return None;
    }
    let attrs_raw = rest[name_end..].trim().to_string();
    Some(DirectiveHeader {
        colons,
        name: name.to_ascii_lowercase(),
        attrs_raw,
    })
}

/// Check if a trimmed line is a closing directive fence with at least `min_colons` colons.
pub(crate) fn is_directive_close(trimmed: &str, min_colons: usize) -> bool {
    let colons = trimmed.bytes().take_while(|&b| b == b':').count();
    colons >= min_colons && trimmed[colons..].trim().is_empty()
}

/// Parse a `:::...` container directive starting at `lines[start]`.
/// Returns `(Block::Directive(...), next_index)`.
pub(crate) fn parse_directive_block(
    lines: &[&str],
    start: usize,
    base_line: usize,
    header: DirectiveHeader,
    ctx: &mut ParseCtx<'_>,
    depth: usize,
) -> (Block, usize) {
    let line_no = base_line + start;
    // Collect lines until the matching closing `:::` fence, respecting nested `:::` containers
    // and fenced code blocks inside the directive.
    let body_start = start + 1;
    let mut idx = body_start;
    let mut nest = 0usize;
    let mut in_code_fence: Option<(u8, usize)> = None;

    while idx < lines.len() {
        let trimmed = lines[idx].trim();
        if let Some((fch, flen)) = in_code_fence {
            let run = trimmed.bytes().take_while(|&b| b == fch).count();
            if run >= flen && trimmed[run..].trim().is_empty() {
                in_code_fence = None;
            }
            idx += 1;
            continue;
        }
        if let Some((fch, flen)) = detect_code_fence(trimmed) {
            in_code_fence = Some((fch, flen));
            idx += 1;
            continue;
        }

        if parse_directive_open(trimmed, 3).is_some() {
            nest += 1;
            idx += 1;
            continue;
        }
        if is_directive_close(trimmed, header.colons) {
            if nest == 0 {
                let body = &lines[body_start..idx];
                let block = build_directive(header, body, base_line + body_start, line_no, ctx, depth);
                return (Block::Directive(block), idx + 1);
            }
            nest -= 1;
        }
        idx += 1;
    }

    // Unclosed directive: parse remaining lines gracefully without panicking.
    let body = &lines[body_start..];
    let block = build_directive(header, body, base_line + body_start, line_no, ctx, depth);
    (Block::Directive(block), lines.len())
}

fn detect_code_fence(trimmed: &str) -> Option<(u8, usize)> {
    let first = *trimmed.as_bytes().first()?;
    if first != b'`' && first != b'~' {
        return None;
    }
    let count = trimmed.bytes().take_while(|&b| b == first).count();
    if count >= 3 {
        Some((first, count))
    } else {
        None
    }
}

fn build_directive(
    header: DirectiveHeader,
    body: &[&str],
    body_base_line: usize,
    line_no: usize,
    ctx: &mut ParseCtx<'_>,
    depth: usize,
) -> DirectiveBlock {
    let attrs = parse_directive_attrs(&header.attrs_raw);
    match header.name.as_str() {
        "callout" => {
            let kind = attrs
                .get("kind")
                .or_else(|| attrs.get("type"))
                .map_or(CalloutKind::Info, |s| CalloutKind::parse(s));
            let title = attrs.get("title").cloned();
            let blocks = ctx.parse_lines(body, body_base_line, depth + 1);
            DirectiveBlock::Callout {
                kind,
                title,
                blocks,
                line: line_no,
            }
        }
        "tabs" => {
            let sections = split_sub_directives(body, body_base_line, "tab");
            let mut items = Vec::with_capacity(sections.len());
            for sec in sections {
                let label = sec
                    .attrs
                    .get("label")
                    .or_else(|| sec.attrs.get("title"))
                    .cloned()
                    .unwrap_or_else(|| "Tab".to_string());
                let blocks = ctx.parse_lines(sec.lines, sec.body_base_line, depth + 1);
                items.push(TabItem {
                    label,
                    blocks,
                    line: sec.line,
                });
            }
            DirectiveBlock::Tabs {
                items,
                line: line_no,
            }
        }
        "steps" => {
            let sections = split_sub_directives(body, body_base_line, "step");
            let mut items = Vec::with_capacity(sections.len());
            for sec in sections {
                let title = sec
                    .attrs
                    .get("title")
                    .or_else(|| sec.attrs.get("label"))
                    .cloned()
                    .unwrap_or_else(|| "Step".to_string());
                let blocks = ctx.parse_lines(sec.lines, sec.body_base_line, depth + 1);
                items.push(StepItem {
                    title,
                    blocks,
                    line: sec.line,
                });
            }
            DirectiveBlock::Steps {
                items,
                line: line_no,
            }
        }
        "cards" => {
            let sections = split_sub_directives(body, body_base_line, "card");
            let mut items = Vec::with_capacity(sections.len());
            for sec in sections {
                let title = sec
                    .attrs
                    .get("title")
                    .cloned()
                    .unwrap_or_else(|| "Card".to_string());
                let href = sec.attrs.get("href").cloned();
                let description = sec.attrs.get("description").cloned();
                let icon = sec.attrs.get("icon").cloned();
                let blocks = ctx.parse_lines(sec.lines, sec.body_base_line, depth + 1);
                items.push(CardItem {
                    title,
                    href,
                    description,
                    icon,
                    blocks,
                    line: sec.line,
                });
            }
            DirectiveBlock::Cards {
                items,
                line: line_no,
            }
        }
        "demo" => {
            let id = attrs
                .get("id")
                .cloned()
                .unwrap_or_else(|| format!("demo-{line_no}"));
            let hydrate = attrs
                .get("hydrate")
                .map_or(HydrateStrategy::Visible, |s| HydrateStrategy::parse(s));
            let height = attrs.get("height").and_then(|s| s.parse::<u32>().ok());
            let title = attrs.get("title").cloned();
            let blocks = ctx.parse_lines(body, body_base_line, depth + 1);
            DirectiveBlock::Demo {
                id,
                hydrate,
                height,
                title,
                blocks,
                line: line_no,
            }
        }
        _ => {
            let blocks = ctx.parse_lines(body, body_base_line, depth + 1);
            DirectiveBlock::Custom {
                name: header.name,
                attrs,
                blocks,
                line: line_no,
            }
        }
    }
}

struct SubSection<'a> {
    attrs: std::collections::BTreeMap<String, String>,
    lines: &'a [&'a str],
    line: usize,
    body_base_line: usize,
}

/// Split `::tab{...}` / `::step{...}` / `::card{...}` sub-directives inside a parent container.
///
/// Supports both explicit `::` closing lines and implicit closing on the next sibling `::<name>`.
fn split_sub_directives<'a>(
    body: &'a [&'a str],
    body_base_line: usize,
    expected_name: &str,
) -> Vec<SubSection<'a>> {
    let mut out = Vec::new();
    let mut idx = 0usize;
    let mut in_code_fence: Option<(u8, usize)> = None;

    while idx < body.len() {
        let trimmed = body[idx].trim();
        if let Some((fch, flen)) = in_code_fence {
            let run = trimmed.bytes().take_while(|&b| b == fch).count();
            if run >= flen && trimmed[run..].trim().is_empty() {
                in_code_fence = None;
            }
            idx += 1;
            continue;
        }
        if let Some((fch, flen)) = detect_code_fence(trimmed) {
            in_code_fence = Some((fch, flen));
            idx += 1;
            continue;
        }

        if let Some(sub) = parse_directive_open(trimmed, 2)
            && sub.colons == 2
            && sub.name == expected_name
        {
            let open_line = body_base_line + idx;
            let attrs = parse_directive_attrs(&sub.attrs_raw);
            let sub_body_start = idx + 1;
            let mut end = sub_body_start;
            let mut inner_code: Option<(u8, usize)> = None;
            let mut inner_nest = 0usize;

            let mut sec_end = body.len();
            let mut next_idx = body.len();

            while end < body.len() {
                let t = body[end].trim();
                if let Some((fch, flen)) = inner_code {
                    let run = t.bytes().take_while(|&b| b == fch).count();
                    if run >= flen && t[run..].trim().is_empty() {
                        inner_code = None;
                    }
                    end += 1;
                    continue;
                }
                if let Some((fch, flen)) = detect_code_fence(t) {
                    inner_code = Some((fch, flen));
                    end += 1;
                    continue;
                }
                if parse_directive_open(t, 3).is_some() {
                    inner_nest += 1;
                    end += 1;
                    continue;
                }
                if inner_nest > 0 && is_directive_close(t, 3) {
                    inner_nest -= 1;
                    end += 1;
                    continue;
                }
                if inner_nest == 0 {
                    if is_directive_close(t, 2) && t.bytes().take_while(|&b| b == b':').count() == 2
                    {
                        sec_end = end;
                        next_idx = end + 1;
                        break;
                    }
                    if let Some(next_sub) = parse_directive_open(t, 2)
                        && next_sub.colons == 2
                        && next_sub.name == expected_name
                    {
                        sec_end = end;
                        next_idx = end;
                        break;
                    }
                }
                end += 1;
            }

            out.push(SubSection {
                attrs,
                lines: &body[sub_body_start..sec_end],
                line: open_line,
                body_base_line: body_base_line + sub_body_start,
            });
            idx = next_idx;
            continue;
        }
        idx += 1;
    }

    out
}
