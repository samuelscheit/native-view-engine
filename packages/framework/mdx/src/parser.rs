//! Block-level CommonMark + GFM + Fenced Directive + Declarative MDX parser.

use crate::ast::{
    Block, CodeBlock, ComponentNode, Frontmatter, Inline, ListItem, LiveBlock, SlugGenerator,
    SourcePos, TableAlign, TocHeading,
};
use crate::directive::{parse_directive_block, parse_directive_open};
use crate::highlight::highlight_code;
use crate::inline::{inline_plain_text, parse_inlines};
use crate::mdx_tag::{parse_open_tag, starts_with_component_tag};
use crate::parser_blocks::{
    collect_blocks_plain_text, count_leading_spaces, is_thematic_break, looks_like_table_row,
    parse_atx_heading, parse_code_fence_open, parse_code_meta, parse_list_marker,
    parse_table_separator, split_table_cells,
};

const MAX_BLOCK_DEPTH: usize = 64;

/// Output of parsing a Markdown/MDX body.
#[derive(Debug, Clone)]
pub struct ParsedBody {
    pub blocks: Vec<Block>,
    pub toc: Vec<TocHeading>,
    pub live_blocks: Vec<LiveBlock>,
    pub plain_text: String,
}

/// Shared mutable parser context passed across recursive block parses.
pub(crate) struct ParseCtx<'a> {
    pub fm: &'a Frontmatter,
    pub slugs: SlugGenerator,
    pub toc: Vec<TocHeading>,
    pub live_blocks: Vec<LiveBlock>,
    pub route_prefix: String,
}

/// Parse a Markdown/MDX document body starting at `body_start_line` (1-based).
pub fn parse_markdown_body(
    body: &str,
    body_start_line: usize,
    fm: &Frontmatter,
    route_prefix: &str,
) -> ParsedBody {
    let lines: Vec<&str> = body.lines().collect();
    let mut ctx = ParseCtx {
        fm,
        slugs: SlugGenerator::default(),
        toc: Vec::new(),
        live_blocks: Vec::new(),
        route_prefix: route_prefix.to_string(),
    };
    let blocks = ctx.parse_lines(&lines, body_start_line, 0);
    let plain_text = collect_blocks_plain_text(&blocks);
    ParsedBody {
        blocks,
        toc: ctx.toc,
        live_blocks: ctx.live_blocks,
        plain_text,
    }
}

impl<'a> ParseCtx<'a> {
    pub(crate) fn parse_lines(
        &mut self,
        lines: &[&str],
        base_line: usize,
        depth: usize,
    ) -> Vec<Block> {
        if depth >= MAX_BLOCK_DEPTH {
            let text = lines.join("\n");
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return Vec::new();
            }
            return vec![Block::Paragraph {
                content: vec![Inline::Text(trimmed.to_string())],
                line: base_line,
            }];
        }

        let mut blocks = Vec::new();
        let mut i = 0usize;

        while i < lines.len() {
            let line = lines[i];
            let trimmed = line.trim();
            if trimmed.is_empty() {
                i += 1;
                continue;
            }

            if let Some((fence_ch, fence_len, info)) = parse_code_fence_open(trimmed) {
                let (block, next_i) =
                    self.parse_code_block(lines, i, base_line, fence_ch, fence_len, info);
                blocks.push(block);
                i = next_i;
                continue;
            }

            if let Some(header) = parse_directive_open(trimmed, 3) {
                let (block, next_i) =
                    parse_directive_block(lines, i, base_line, header, self, depth);
                blocks.push(block);
                i = next_i;
                continue;
            }

            if let Some((level, raw_text)) = parse_atx_heading(trimmed) {
                let line_no = base_line + i;
                let col = line.len() - line.trim_start().len() + level as usize + 2;
                let content = parse_inlines(raw_text, SourcePos::new(line_no, col), self.fm);
                let text = inline_plain_text(&content).trim().to_string();
                let id = self.slugs.next_slug(&text);
                self.toc.push(TocHeading {
                    level,
                    id: id.clone(),
                    text: text.clone(),
                    line: line_no,
                });
                blocks.push(Block::Heading {
                    level,
                    id,
                    content,
                    text,
                    line: line_no,
                });
                i += 1;
                continue;
            }

            if is_thematic_break(trimmed) {
                blocks.push(Block::ThematicBreak {
                    line: base_line + i,
                });
                i += 1;
                continue;
            }

            if trimmed.starts_with('>') {
                let (block, next_i) = self.parse_blockquote(lines, i, base_line, depth);
                blocks.push(block);
                i = next_i;
                continue;
            }

            if i + 1 < lines.len()
                && let Some(alignments) = parse_table_separator(lines[i + 1])
                && looks_like_table_row(line)
            {
                let (block, next_i) = self.parse_table(lines, i, base_line, alignments);
                blocks.push(block);
                i = next_i;
                continue;
            }

            if starts_with_component_tag(trimmed)
                && let Some((comp, next_i)) = self.try_parse_component(lines, i, base_line, depth)
            {
                blocks.push(Block::Component(comp));
                i = next_i;
                continue;
            }

            if parse_list_marker(line).is_some() {
                let (block, next_i) = self.parse_list(lines, i, base_line, depth);
                blocks.push(block);
                i = next_i;
                continue;
            }

            let (block, next_i) = self.parse_paragraph(lines, i, base_line);
            blocks.push(block);
            i = next_i;
        }

        blocks
    }

    fn parse_code_block(
        &mut self,
        lines: &[&str],
        start: usize,
        base_line: usize,
        fence_ch: u8,
        fence_len: usize,
        info: &str,
    ) -> (Block, usize) {
        let line_no = base_line + start;
        let mut idx = start + 1;
        let mut code_lines = Vec::new();
        while idx < lines.len() {
            let t = lines[idx].trim();
            let run = t.bytes().take_while(|&b| b == fence_ch).count();
            if run >= fence_len && t[run..].trim().is_empty() {
                idx += 1;
                break;
            }
            code_lines.push(lines[idx]);
            idx += 1;
        }
        let code = code_lines.join("\n");
        let meta = parse_code_meta(info);
        let highlighted = highlight_code(meta.lang.as_deref(), &code, &meta.highlight_lines);

        if meta.live {
            let live_idx = self.live_blocks.len() + 1;
            let stem = crate::ast::slugify(&self.route_prefix);
            self.live_blocks.push(LiveBlock {
                id: format!("{stem}-live-{live_idx}"),
                lang: meta.lang.clone().unwrap_or_else(|| "rsx".to_string()),
                title: meta.title.clone(),
                code: code.clone(),
                line: line_no,
            });
        }

        (
            Block::CodeBlock(CodeBlock {
                lang: meta.lang,
                title: meta.title,
                highlight_lines: meta.highlight_lines,
                show_line_numbers: meta.show_line_numbers,
                live: meta.live,
                meta: info.trim().to_string(),
                code,
                highlighted,
                line: line_no,
            }),
            idx,
        )
    }

    fn parse_blockquote(
        &mut self,
        lines: &[&str],
        start: usize,
        base_line: usize,
        depth: usize,
    ) -> (Block, usize) {
        let mut inner = Vec::new();
        let mut idx = start;
        while idx < lines.len() {
            let trimmed = lines[idx].trim_start();
            if let Some(rest) = trimmed.strip_prefix('>') {
                inner.push(rest.strip_prefix(' ').unwrap_or(rest));
                idx += 1;
            } else {
                break;
            }
        }
        let blocks = self.parse_lines(&inner, base_line + start, depth + 1);
        (
            Block::Blockquote {
                blocks,
                line: base_line + start,
            },
            idx,
        )
    }

    fn parse_table(
        &mut self,
        lines: &[&str],
        start: usize,
        base_line: usize,
        alignments: Vec<TableAlign>,
    ) -> (Block, usize) {
        let header_cells = split_table_cells(lines[start]);
        let col_count = alignments.len().max(header_cells.len());
        let headers: Vec<Vec<Inline>> = header_cells
            .into_iter()
            .map(|c| parse_inlines(c.trim(), SourcePos::new(base_line + start, 1), self.fm))
            .collect();

        let mut rows = Vec::new();
        let mut idx = start + 2;
        while idx < lines.len() {
            if lines[idx].trim().is_empty() || !looks_like_table_row(lines[idx]) {
                break;
            }
            let raw_cells = split_table_cells(lines[idx]);
            let mut row = Vec::with_capacity(col_count);
            for c_idx in 0..col_count {
                let cell = raw_cells.get(c_idx).map_or("", |s| s.trim());
                row.push(parse_inlines(
                    cell,
                    SourcePos::new(base_line + idx, 1),
                    self.fm,
                ));
            }
            rows.push(row);
            idx += 1;
        }

        (
            Block::Table {
                alignments,
                headers,
                rows,
                line: base_line + start,
            },
            idx,
        )
    }

    fn try_parse_component(
        &mut self,
        lines: &[&str],
        start: usize,
        base_line: usize,
        depth: usize,
    ) -> Option<(ComponentNode, usize)> {
        let leading_ws = lines[start].len() - lines[start].trim_start().len();
        let start_pos = SourcePos::new(base_line + start, leading_ws + 1);

        let window_end = (start + 32).min(lines.len());
        let mut joined = String::new();
        let mut line_offsets = Vec::with_capacity(window_end - start);
        for (rel, l) in lines[start..window_end].iter().enumerate() {
            line_offsets.push(joined.len());
            if rel == 0 {
                joined.push_str(&l[leading_ws..]);
            } else {
                joined.push_str(l);
            }
            joined.push('\n');
        }

        let tag = parse_open_tag(&joined, start_pos, self.fm)?;
        let end_rel = line_offsets
            .iter()
            .rposition(|&off| off < tag.end_offset)
            .unwrap_or(0);
        let open_end_line_idx = start + end_rel;
        let rest_on_open_line = joined[tag.end_offset..].lines().next().unwrap_or("").trim();

        if tag.self_closing {
            return Some((
                ComponentNode {
                    name: tag.name,
                    props: tag.props,
                    children: Vec::new(),
                    self_closing: true,
                    pos: tag.pos,
                },
                open_end_line_idx + 1,
            ));
        }

        let close_needle = format!("</{}>", tag.name);
        if let Some(close_pos) = rest_on_open_line.find(&close_needle) {
            let inline_body = rest_on_open_line[..close_pos].trim();
            let children = if inline_body.is_empty() {
                Vec::new()
            } else {
                self.parse_lines(&[inline_body], base_line + open_end_line_idx, depth + 1)
            };
            return Some((
                ComponentNode {
                    name: tag.name,
                    props: tag.props,
                    children,
                    self_closing: false,
                    pos: tag.pos,
                },
                open_end_line_idx + 1,
            ));
        }

        let mut body_lines: Vec<&str> = Vec::new();
        let body_base_line = if !rest_on_open_line.is_empty() {
            body_lines.push(rest_on_open_line);
            base_line + open_end_line_idx
        } else {
            base_line + open_end_line_idx + 1
        };

        let mut idx = open_end_line_idx + 1;
        let mut nest = 0usize;
        let mut in_code: Option<(u8, usize)> = None;
        let open_prefix = format!("<{}", tag.name);

        while idx < lines.len() {
            let raw = lines[idx];
            let t = raw.trim();
            if let Some((fch, flen)) = in_code {
                let run = t.bytes().take_while(|&b| b == fch).count();
                if run >= flen && t[run..].trim().is_empty() {
                    in_code = None;
                }
                body_lines.push(raw);
                idx += 1;
                continue;
            }
            if let Some((fch, flen, _)) = parse_code_fence_open(t) {
                in_code = Some((fch, flen));
                body_lines.push(raw);
                idx += 1;
                continue;
            }
            if t.starts_with(&open_prefix)
                && !t.ends_with("/>")
                && !t.contains(&close_needle)
                && t.as_bytes()
                    .get(open_prefix.len())
                    .is_some_and(|&b| b == b' ' || b == b'>' || b == b'\t')
            {
                nest += 1;
                body_lines.push(raw);
                idx += 1;
                continue;
            }
            if t == close_needle || t.ends_with(&close_needle) {
                if nest == 0 {
                    if let Some(before) = t.strip_suffix(&close_needle)
                        && !before.trim().is_empty()
                    {
                        body_lines.push(before.trim());
                    }
                    idx += 1;
                    break;
                }
                nest -= 1;
            }
            body_lines.push(raw);
            idx += 1;
        }

        let children = self.parse_lines(&body_lines, body_base_line, depth + 1);
        Some((
            ComponentNode {
                name: tag.name,
                props: tag.props,
                children,
                self_closing: false,
                pos: tag.pos,
            },
            idx,
        ))
    }

    fn parse_list(
        &mut self,
        lines: &[&str],
        start: usize,
        base_line: usize,
        depth: usize,
    ) -> (Block, usize) {
        let first = parse_list_marker(lines[start])
            .expect("parse_list is only called when lines[start] has a list marker");
        let ordered = first.ordered;
        let start_num = first.number.unwrap_or(1);
        let base_indent = first.indent;
        let mut items = Vec::new();
        let mut idx = start;

        while idx < lines.len() {
            let Some(marker) = parse_list_marker(lines[idx]) else {
                break;
            };
            if marker.indent != base_indent || marker.ordered != ordered {
                break;
            }

            let item_line = base_line + idx;
            let content_indent = marker.content_indent;
            let mut item_lines = vec![marker.rest];
            idx += 1;

            while idx < lines.len() {
                let raw = lines[idx];
                if raw.trim().is_empty() {
                    if let Some(next_nb) = lines[idx + 1..].iter().position(|l| !l.trim().is_empty())
                        && count_leading_spaces(lines[idx + 1 + next_nb]) >= content_indent
                    {
                        item_lines.push("");
                        idx += 1;
                        continue;
                    }
                    break;
                }
                let ind = count_leading_spaces(raw);
                if ind > base_indent {
                    let strip = ind.min(content_indent);
                    item_lines.push(&raw[strip..]);
                    idx += 1;
                } else if parse_list_marker(raw).is_some() || is_block_starter(raw) {
                    break;
                } else {
                    item_lines.push(raw.trim_start());
                    idx += 1;
                }
            }

            let blocks = self.parse_lines(&item_lines, item_line, depth + 1);
            items.push(ListItem {
                checked: marker.checked,
                blocks,
                line: item_line,
            });

            while idx < lines.len() && lines[idx].trim().is_empty() {
                if let Some(next_m) = lines.get(idx + 1).and_then(|l| parse_list_marker(l))
                    && next_m.indent == base_indent
                    && next_m.ordered == ordered
                {
                    idx += 1;
                } else {
                    break;
                }
            }
        }

        (
            Block::List {
                ordered,
                start: start_num,
                items,
                line: base_line + start,
            },
            idx,
        )
    }

    fn parse_paragraph(&mut self, lines: &[&str], start: usize, base_line: usize) -> (Block, usize) {
        let mut para_lines = vec![lines[start].trim_start()];
        let mut idx = start + 1;
        while idx < lines.len() {
            let raw = lines[idx];
            if raw.trim().is_empty() || is_block_starter(raw) {
                break;
            }
            if idx + 1 < lines.len()
                && looks_like_table_row(raw)
                && parse_table_separator(lines[idx + 1]).is_some()
            {
                break;
            }
            para_lines.push(raw.trim_start());
            idx += 1;
        }
        let joined = para_lines.join("\n");
        let col = lines[start].len() - lines[start].trim_start().len() + 1;
        let content = parse_inlines(&joined, SourcePos::new(base_line + start, col), self.fm);
        (
            Block::Paragraph {
                content,
                line: base_line + start,
            },
            idx,
        )
    }
}

fn is_block_starter(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.is_empty()
        || parse_code_fence_open(trimmed).is_some()
        || parse_directive_open(trimmed, 3).is_some()
        || parse_atx_heading(trimmed).is_some()
        || is_thematic_break(trimmed)
        || trimmed.starts_with('>')
        || starts_with_component_tag(trimmed)
        || parse_list_marker(line).is_some()
}
