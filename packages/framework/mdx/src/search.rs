//! Zero-server compact binary full-text search index (`NVS1`).

use std::collections::{BTreeMap, HashMap};

use crate::ast::{Block, CompiledPage, DirectiveBlock, Inline};
use crate::inline::append_inline_plain_text;
pub use crate::search_codec::{NVS1_MAGIC, NVS1_VERSION, SearchIndexError, fnv1a64};

/// Field weights stored in tenths (`weight * 10`).
const WEIGHT_TITLE_X10: u16 = 40; // 4.0
const WEIGHT_HEADING_X10: u16 = 25; // 2.5
const WEIGHT_CODE_X10: u16 = 20; // 2.0
const WEIGHT_BODY_X10: u16 = 10; // 1.0

/// Indexed document or heading section record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchDoc {
    pub route: String,
    pub title: String,
    pub section: String,
    pub anchor: String,
    pub excerpt: String,
}

/// Inverted index posting `(doc_id, field weight in tenths)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Posting {
    pub doc_id: u32,
    pub weight_x10: u16,
}

/// Ranked search result returned by [`SearchIndex::query`].
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub route: String,
    pub title: String,
    pub section: String,
    pub anchor: String,
    pub excerpt: String,
    pub score: f32,
}

/// In-memory & binary-serializable full-text search index (`NVS1`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchIndex {
    pub docs: Vec<SearchDoc>,
    /// Lexicographically sorted `(term, postings)` table for exact & prefix binary search.
    pub tokens: Vec<(String, Vec<Posting>)>,
    /// Lexicographically sorted `(trigram, postings)` table for substring matching.
    pub trigrams: Vec<(String, Vec<Posting>)>,
}

impl SearchIndex {
    /// Build a search index from compiled documentation pages.
    pub fn build(pages: &[CompiledPage]) -> Self {
        let mut docs: Vec<SearchDoc> = Vec::new();
        let mut token_map: BTreeMap<String, BTreeMap<u32, u16>> = BTreeMap::new();
        let mut trigram_map: BTreeMap<String, BTreeMap<u32, u16>> = BTreeMap::new();

        for page in pages {
            let root_id = docs.len() as u32;
            let mut root_body = String::new();
            let mut root_code = String::new();
            let mut sections: Vec<SectionAcc> = Vec::new();
            let mut current_sec: Option<SectionAcc> = None;

            for block in &page.blocks {
                if let Block::Heading {
                    id, text, level, ..
                } = block
                {
                    if *level == 1 && current_sec.is_none() {
                        add_tokens(&mut token_map, &mut trigram_map, root_id, text, WEIGHT_HEADING_X10);
                        continue;
                    }
                    if let Some(prev) = current_sec.take() {
                        sections.push(prev);
                    }
                    current_sec = Some(SectionAcc {
                        section: text.clone(),
                        anchor: id.clone(),
                        body: String::new(),
                        code: String::new(),
                    });
                } else if let Some(sec) = current_sec.as_mut() {
                    extract_block_fields(block, &mut sec.body, &mut sec.code);
                } else {
                    extract_block_fields(block, &mut root_body, &mut root_code);
                }
            }
            if let Some(last) = current_sec {
                sections.push(last);
            }

            let root_excerpt = make_excerpt(
                if !page.description.is_empty() {
                    &page.description
                } else if !root_body.is_empty() {
                    &root_body
                } else {
                    &page.plain_text
                },
                180,
            );
            docs.push(SearchDoc {
                route: page.route.clone(),
                title: page.title.clone(),
                section: page.title.clone(),
                anchor: String::new(),
                excerpt: root_excerpt,
            });

            add_tokens(&mut token_map, &mut trigram_map, root_id, &page.title, WEIGHT_TITLE_X10);
            if !page.description.is_empty() {
                add_tokens(
                    &mut token_map,
                    &mut trigram_map,
                    root_id,
                    &page.description,
                    WEIGHT_BODY_X10,
                );
            }
            add_tokens(&mut token_map, &mut trigram_map, root_id, &root_code, WEIGHT_CODE_X10);
            add_tokens(&mut token_map, &mut trigram_map, root_id, &root_body, WEIGHT_BODY_X10);

            for sec in sections {
                let sec_id = docs.len() as u32;
                let excerpt = make_excerpt(
                    if !sec.body.is_empty() {
                        &sec.body
                    } else if !sec.code.is_empty() {
                        &sec.code
                    } else {
                        &sec.section
                    },
                    180,
                );
                docs.push(SearchDoc {
                    route: page.route.clone(),
                    title: page.title.clone(),
                    section: sec.section.clone(),
                    anchor: sec.anchor,
                    excerpt,
                });
                add_tokens(
                    &mut token_map,
                    &mut trigram_map,
                    sec_id,
                    &sec.section,
                    WEIGHT_HEADING_X10,
                );
                add_tokens(&mut token_map, &mut trigram_map, sec_id, &sec.code, WEIGHT_CODE_X10);
                add_tokens(&mut token_map, &mut trigram_map, sec_id, &sec.body, WEIGHT_BODY_X10);
            }
        }

        let to_sorted_vec = |m: BTreeMap<String, BTreeMap<u32, u16>>| {
            m.into_iter()
                .map(|(k, map)| {
                    let postings = map
                        .into_iter()
                        .map(|(doc_id, weight_x10)| Posting { doc_id, weight_x10 })
                        .collect();
                    (k, postings)
                })
                .collect()
        };

        Self {
            docs,
            tokens: to_sorted_vec(token_map),
            trigrams: to_sorted_vec(trigram_map),
        }
    }

    /// Encode this index into the compact little-endian `NVS1` binary format with FNV-1a64 trailer.
    pub fn encode(&self) -> Vec<u8> {
        crate::search_codec::encode_index(self)
    }

    /// Decode an `NVS1` binary buffer with strict bounds & checksum verification (zero panics).
    pub fn decode(bytes: &[u8]) -> Result<Self, SearchIndexError> {
        crate::search_codec::decode_index(bytes)
    }

    /// Execute a ranked prefix + trigram full-text query returning up to `limit` hits.
    pub fn query(&self, text: &str, limit: usize) -> Vec<SearchHit> {
        if limit == 0 || self.docs.is_empty() {
            return Vec::new();
        }
        let q_terms = tokenize_query(text);
        if q_terms.is_empty() {
            return Vec::new();
        }

        let mut scores: HashMap<u32, (f32, usize)> = HashMap::new();

        for term in &q_terms {
            let mut matched_docs: HashMap<u32, f32> = HashMap::new();

            let start = self
                .tokens
                .partition_point(|(k, _)| k.as_str() < term.as_str());
            for (cand, postings) in &self.tokens[start..] {
                if !cand.starts_with(term.as_str()) {
                    break;
                }
                let mult = if cand == term {
                    2.0f32
                } else {
                    1.2 + 0.3 * (term.len() as f32 / cand.len().max(1) as f32)
                };
                for p in postings {
                    let s = (p.weight_x10 as f32 / 10.0) * mult;
                    matched_docs
                        .entry(p.doc_id)
                        .and_modify(|e| {
                            if s > *e {
                                *e = s;
                            }
                        })
                        .or_insert(s);
                }
            }

            let tris = extract_trigrams(term);
            if !tris.is_empty() {
                let mut tri_hits: HashMap<u32, (usize, f32)> = HashMap::new();
                for tri in &tris {
                    if let Ok(pos) = self.trigrams.binary_search_by(|(k, _)| k.cmp(tri)) {
                        for p in &self.trigrams[pos].1 {
                            let w = p.weight_x10 as f32 / 10.0;
                            tri_hits
                                .entry(p.doc_id)
                                .and_modify(|(cnt, best_w)| {
                                    *cnt += 1;
                                    if w > *best_w {
                                        *best_w = w;
                                    }
                                })
                                .or_insert((1, w));
                        }
                    }
                }
                let min_needed = (tris.len() / 2).max(1);
                for (doc_id, (cnt, best_w)) in tri_hits {
                    if cnt >= min_needed {
                        let coverage = cnt as f32 / tris.len() as f32;
                        let s = best_w * 0.45 * coverage;
                        matched_docs
                            .entry(doc_id)
                            .and_modify(|e| {
                                if s > *e {
                                    *e = s;
                                }
                            })
                            .or_insert(s);
                    }
                }
            }

            for (doc_id, term_score) in matched_docs {
                scores
                    .entry(doc_id)
                    .and_modify(|(total, hits)| {
                        *total += term_score;
                        *hits += 1;
                    })
                    .or_insert((term_score, 1));
            }
        }

        let num_terms = q_terms.len();
        let mut ranked: Vec<(u32, f32)> = scores
            .into_iter()
            .map(|(doc_id, (base, matched))| {
                let bonus = if num_terms > 1 && matched == num_terms {
                    1.25
                } else {
                    1.0
                };
                (doc_id, base * bonus)
            })
            .collect();

        ranked.sort_by(|a, b| {
            b.1.total_cmp(&a.1)
                .then_with(|| {
                    let da = &self.docs[a.0 as usize];
                    let db = &self.docs[b.0 as usize];
                    da.anchor.is_empty().cmp(&db.anchor.is_empty()).reverse()
                })
                .then_with(|| a.0.cmp(&b.0))
        });

        ranked
            .into_iter()
            .take(limit)
            .filter_map(|(doc_id, score)| {
                let doc = self.docs.get(doc_id as usize)?;
                Some(SearchHit {
                    route: doc.route.clone(),
                    title: doc.title.clone(),
                    section: doc.section.clone(),
                    anchor: doc.anchor.clone(),
                    excerpt: doc.excerpt.clone(),
                    score,
                })
            })
            .collect()
    }
}

struct SectionAcc {
    section: String,
    anchor: String,
    body: String,
    code: String,
}

fn extract_block_fields(block: &Block, body: &mut String, code: &mut String) {
    match block {
        Block::Heading { text, .. } => {
            body.push_str(text);
            body.push(' ');
        }
        Block::Paragraph { content, .. } => {
            extract_inline_fields(content, body, code);
            body.push(' ');
        }
        Block::CodeBlock(cb) => {
            code.push_str(&cb.code);
            code.push(' ');
        }
        Block::List { items, .. } => {
            for item in items {
                for b in &item.blocks {
                    extract_block_fields(b, body, code);
                }
            }
        }
        Block::Table { headers, rows, .. } => {
            for h in headers {
                extract_inline_fields(h, body, code);
                body.push(' ');
            }
            for row in rows {
                for cell in row {
                    extract_inline_fields(cell, body, code);
                    body.push(' ');
                }
            }
        }
        Block::Blockquote { blocks, .. } => {
            for b in blocks {
                extract_block_fields(b, body, code);
            }
        }
        Block::Directive(dir) => match dir {
            DirectiveBlock::Callout { title, blocks, .. }
            | DirectiveBlock::Demo { title, blocks, .. } => {
                if let Some(t) = title {
                    body.push_str(t);
                    body.push(' ');
                }
                for b in blocks {
                    extract_block_fields(b, body, code);
                }
            }
            DirectiveBlock::Tabs { items, .. } => {
                for it in items {
                    body.push_str(&it.label);
                    body.push(' ');
                    for b in &it.blocks {
                        extract_block_fields(b, body, code);
                    }
                }
            }
            DirectiveBlock::Steps { items, .. } => {
                for it in items {
                    body.push_str(&it.title);
                    body.push(' ');
                    for b in &it.blocks {
                        extract_block_fields(b, body, code);
                    }
                }
            }
            DirectiveBlock::Cards { items, .. } => {
                for it in items {
                    body.push_str(&it.title);
                    body.push(' ');
                    if let Some(d) = &it.description {
                        body.push_str(d);
                        body.push(' ');
                    }
                    for b in &it.blocks {
                        extract_block_fields(b, body, code);
                    }
                }
            }
            DirectiveBlock::Custom { blocks, .. } => {
                for b in blocks {
                    extract_block_fields(b, body, code);
                }
            }
        },
        Block::Component(comp) => {
            for b in &comp.children {
                extract_block_fields(b, body, code);
            }
        }
        Block::ThematicBreak { .. } => {}
    }
}

fn extract_inline_fields(inlines: &[Inline], body: &mut String, code: &mut String) {
    for node in inlines {
        match node {
            Inline::Code(s) => {
                code.push_str(s);
                code.push(' ');
            }
            Inline::Strong(c)
            | Inline::Emphasis(c)
            | Inline::Strikethrough(c)
            | Inline::Link { text: c, .. } => extract_inline_fields(c, body, code),
            _ => append_inline_plain_text(std::slice::from_ref(node), body),
        }
    }
}

fn add_tokens(
    token_map: &mut BTreeMap<String, BTreeMap<u32, u16>>,
    trigram_map: &mut BTreeMap<String, BTreeMap<u32, u16>>,
    doc_id: u32,
    text: &str,
    weight_x10: u16,
) {
    for tok in tokenize_index(text) {
        token_map
            .entry(tok.clone())
            .or_default()
            .entry(doc_id)
            .and_modify(|w| {
                if weight_x10 > *w {
                    *w = weight_x10;
                }
            })
            .or_insert(weight_x10);

        for tri in extract_trigrams(&tok) {
            trigram_map
                .entry(tri)
                .or_default()
                .entry(doc_id)
                .and_modify(|w| {
                    if weight_x10 > *w {
                        *w = weight_x10;
                    }
                })
                .or_insert(weight_x10);
        }
    }
}

fn tokenize_index(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            cur.push(ch);
        } else if !cur.is_empty() {
            push_word_variants(&cur, &mut out);
            cur.clear();
        }
    }
    if !cur.is_empty() {
        push_word_variants(&cur, &mut out);
    }
    out
}

fn push_word_variants(raw: &str, out: &mut Vec<String>) {
    let lower = raw.to_lowercase();
    if lower.len() >= 2 {
        out.push(lower.clone());
    }
    for snake_part in raw.split('_') {
        if snake_part.is_empty() {
            continue;
        }
        let sp_lower = snake_part.to_lowercase();
        if sp_lower.len() >= 2 && sp_lower != lower {
            out.push(sp_lower);
        }
        let mut part = String::new();
        for ch in snake_part.chars() {
            if ch.is_uppercase()
                && !part.is_empty()
                && part.chars().last().is_some_and(char::is_lowercase)
            {
                let pl = part.to_lowercase();
                if pl.len() >= 2 {
                    out.push(pl);
                }
                part.clear();
            }
            part.push(ch);
        }
        let pl = part.to_lowercase();
        if pl.len() >= 2 && pl != lower {
            out.push(pl);
        }
    }
}

fn tokenize_query(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            for lc in ch.to_lowercase() {
                cur.push(lc);
            }
        } else if !cur.is_empty() {
            if cur.len() >= 2 || out.is_empty() {
                out.push(std::mem::take(&mut cur));
            } else {
                cur.clear();
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn extract_trigrams(term: &str) -> Vec<String> {
    let chars: Vec<char> = term.chars().collect();
    if chars.len() < 3 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(chars.len() - 2);
    for win in chars.windows(3) {
        let mut s = String::with_capacity(6);
        s.push(win[0]);
        s.push(win[1]);
        s.push(win[2]);
        if !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

fn make_excerpt(raw: &str, max_chars: usize) -> String {
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max_chars {
        return collapsed;
    }
    let mut out: String = collapsed.chars().take(max_chars.saturating_sub(1)).collect();
    if let Some(last_sp) = out.rfind(' ')
        && last_sp > max_chars / 2
    {
        out.truncate(last_sp);
    }
    out.push('…');
    out
}
