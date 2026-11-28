//! Reusable zero-allocation full-text query execution for [`SearchIndex`].
//!
//! Uses generation-stamped dense document score tables and reusable token/trigram/ranking buffers
//! so steady-state queries perform zero heap allocations and zero `O(N)` table clears.

use crate::search::{SearchHit, SearchIndex};

/// Zero-copy borrowed search hit returned by [`SearchIndex::query_refs_into`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SearchHitRef<'a> {
    pub doc_id: u32,
    pub route: &'a str,
    pub title: &'a str,
    pub section: &'a str,
    pub anchor: &'a str,
    pub excerpt: &'a str,
    pub score: f32,
}

impl SearchHitRef<'_> {
    /// Converts this borrowed search hit into an owned [`SearchHit`].
    #[inline]
    pub fn to_owned_hit(&self) -> SearchHit {
        SearchHit {
            route: self.route.to_owned(),
            title: self.title.to_owned(),
            section: self.section.to_owned(),
            anchor: self.anchor.to_owned(),
            excerpt: self.excerpt.to_owned(),
            score: self.score,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct QueryDocSlot {
    stamp: u32,
    hits: u32,
    total: f32,
}

#[derive(Debug, Clone, Copy, Default)]
struct TermDocSlot {
    stamp: u32,
    best: f32,
}

#[derive(Debug, Clone, Copy, Default)]
struct TriDocSlot {
    stamp: u32,
    count: u32,
    best_w: f32,
}

/// Reusable scratch buffers for [`SearchIndex::query_refs_into`].
///
/// Once warmed (or constructed with [`Self::for_index`]), repeated calls to
/// [`SearchIndex::query_refs_into`] perform zero heap allocations.
#[derive(Debug, Clone, Default)]
pub struct SearchQueryScratch {
    query_stamp: u32,
    term_stamp: u32,
    tri_stamp: u32,
    term_buf: String,
    term_spans: Vec<(usize, usize)>,
    char_offsets: Vec<usize>,
    trigram_spans: Vec<(usize, usize)>,
    query_slots: Vec<QueryDocSlot>,
    touched_query_docs: Vec<u32>,
    term_slots: Vec<TermDocSlot>,
    touched_term_docs: Vec<u32>,
    tri_slots: Vec<TriDocSlot>,
    touched_tri_docs: Vec<u32>,
    ranked: Vec<(u32, f32)>,
}

impl SearchQueryScratch {
    /// Creates an empty scratch workspace that grows lazily on first use.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a scratch workspace pre-sized for `index` so steady-state queries allocate nothing.
    pub fn for_index(index: &SearchIndex) -> Self {
        let doc_count = index.docs.len();
        Self {
            query_stamp: 0,
            term_stamp: 0,
            tri_stamp: 0,
            term_buf: String::with_capacity(64),
            term_spans: Vec::with_capacity(8),
            char_offsets: Vec::with_capacity(32),
            trigram_spans: Vec::with_capacity(32),
            query_slots: vec![QueryDocSlot::default(); doc_count],
            touched_query_docs: Vec::with_capacity(doc_count),
            term_slots: vec![TermDocSlot::default(); doc_count],
            touched_term_docs: Vec::with_capacity(doc_count),
            tri_slots: vec![TriDocSlot::default(); doc_count],
            touched_tri_docs: Vec::with_capacity(doc_count),
            ranked: Vec::with_capacity(doc_count),
        }
    }

    #[inline]
    fn ensure_doc_capacity(&mut self, doc_count: usize) {
        if self.query_slots.len() < doc_count {
            self.query_slots.resize(doc_count, QueryDocSlot::default());
            self.term_slots.resize(doc_count, TermDocSlot::default());
            self.tri_slots.resize(doc_count, TriDocSlot::default());
        }
    }

    #[inline]
    fn next_query_stamp(&mut self) -> u32 {
        self.query_stamp = self.query_stamp.wrapping_add(1);
        if self.query_stamp == 0 {
            for slot in &mut self.query_slots {
                slot.stamp = 0;
            }
            self.query_stamp = 1;
        }
        self.query_stamp
    }

    #[inline]
    fn next_term_stamp(&mut self) -> u32 {
        self.term_stamp = self.term_stamp.wrapping_add(1);
        if self.term_stamp == 0 {
            for slot in &mut self.term_slots {
                slot.stamp = 0;
            }
            self.term_stamp = 1;
        }
        self.term_stamp
    }

    #[inline]
    fn next_tri_stamp(&mut self) -> u32 {
        self.tri_stamp = self.tri_stamp.wrapping_add(1);
        if self.tri_stamp == 0 {
            for slot in &mut self.tri_slots {
                slot.stamp = 0;
            }
            self.tri_stamp = 1;
        }
        self.tri_stamp
    }
}

pub(crate) fn query_refs_into<'a>(
    index: &'a SearchIndex,
    text: &str,
    limit: usize,
    scratch: &mut SearchQueryScratch,
    out: &mut Vec<SearchHitRef<'a>>,
) {
    out.clear();
    if limit == 0 || index.docs.is_empty() {
        return;
    }

    tokenize_query_into(text, &mut scratch.term_buf, &mut scratch.term_spans);
    let num_terms = scratch.term_spans.len();
    if num_terms == 0 {
        return;
    }

    let doc_count = index.docs.len();
    scratch.ensure_doc_capacity(doc_count);

    let q_stamp = scratch.next_query_stamp();
    scratch.touched_query_docs.clear();

    for term_idx in 0..num_terms {
        let (t_start, t_end) = scratch.term_spans[term_idx];
        let term_stamp = scratch.next_term_stamp();
        scratch.touched_term_docs.clear();

        // 1. Exact & prefix search over the sorted `tokens` table.
        {
            let term = &scratch.term_buf[t_start..t_end];
            let start = index.tokens.partition_point(|(k, _)| k.as_str() < term);
            for (cand, postings) in &index.tokens[start..] {
                if !cand.starts_with(term) {
                    break;
                }
                let mult = if cand.as_str() == term {
                    2.0f32
                } else {
                    1.2 + 0.3 * (term.len() as f32 / cand.len().max(1) as f32)
                };
                for p in postings {
                    let idx = p.doc_id as usize;
                    if idx >= doc_count {
                        continue;
                    }
                    let s = (p.weight_x10 as f32 / 10.0) * mult;
                    record_term_score(
                        &mut scratch.term_slots,
                        &mut scratch.touched_term_docs,
                        term_stamp,
                        p.doc_id,
                        s,
                    );
                }
            }
        }

        // 2. Deduplicated trigram substring search over the sorted `trigrams` table.
        {
            let term = &scratch.term_buf[t_start..t_end];
            collect_trigram_spans_into(
                term,
                &mut scratch.char_offsets,
                &mut scratch.trigram_spans,
            );
        }
        let tri_count = scratch.trigram_spans.len();
        if tri_count > 0 {
            let tri_stamp = scratch.next_tri_stamp();
            scratch.touched_tri_docs.clear();

            let term = &scratch.term_buf[t_start..t_end];
            for &(s, e) in &scratch.trigram_spans {
                let tri = &term[s..e];
                if let Ok(pos) = index.trigrams.binary_search_by(|(k, _)| k.as_str().cmp(tri)) {
                    for p in &index.trigrams[pos].1 {
                        let idx = p.doc_id as usize;
                        if idx >= doc_count {
                            continue;
                        }
                        let w = p.weight_x10 as f32 / 10.0;
                        let slot = &mut scratch.tri_slots[idx];
                        if slot.stamp != tri_stamp {
                            slot.stamp = tri_stamp;
                            slot.count = 1;
                            slot.best_w = w;
                            scratch.touched_tri_docs.push(p.doc_id);
                        } else {
                            slot.count += 1;
                            if w > slot.best_w {
                                slot.best_w = w;
                            }
                        }
                    }
                }
            }

            let min_needed = (tri_count / 2).max(1) as u32;
            for &doc_id in &scratch.touched_tri_docs {
                let slot = scratch.tri_slots[doc_id as usize];
                if slot.count >= min_needed {
                    let coverage = slot.count as f32 / tri_count as f32;
                    let s = slot.best_w * 0.45 * coverage;
                    record_term_score(
                        &mut scratch.term_slots,
                        &mut scratch.touched_term_docs,
                        term_stamp,
                        doc_id,
                        s,
                    );
                }
            }
        }

        // 3. Fold per-term best scores into query totals in exact left-to-right term order.
        for &doc_id in &scratch.touched_term_docs {
            let term_score = scratch.term_slots[doc_id as usize].best;
            let q_slot = &mut scratch.query_slots[doc_id as usize];
            if q_slot.stamp != q_stamp {
                q_slot.stamp = q_stamp;
                q_slot.total = term_score;
                q_slot.hits = 1;
                scratch.touched_query_docs.push(doc_id);
            } else {
                q_slot.total += term_score;
                q_slot.hits += 1;
            }
        }
    }

    scratch.ranked.clear();
    for &doc_id in &scratch.touched_query_docs {
        let q_slot = scratch.query_slots[doc_id as usize];
        let bonus = if num_terms > 1 && (q_slot.hits as usize) == num_terms {
            1.25
        } else {
            1.0
        };
        scratch.ranked.push((doc_id, q_slot.total * bonus));
    }

    // Strict total order (unique `doc_id` final tie-breaker), so in-place `sort_unstable_by`
    // matches stable sorting identically while performing zero heap allocations.
    scratch.ranked.sort_unstable_by(|a, b| {
        b.1.total_cmp(&a.1)
            .then_with(|| {
                let da = &index.docs[a.0 as usize];
                let db = &index.docs[b.0 as usize];
                da.anchor.is_empty().cmp(&db.anchor.is_empty()).reverse()
            })
            .then_with(|| a.0.cmp(&b.0))
    });

    for &(doc_id, score) in scratch.ranked.iter().take(limit) {
        if let Some(doc) = index.docs.get(doc_id as usize) {
            out.push(SearchHitRef {
                doc_id,
                route: &doc.route,
                title: &doc.title,
                section: &doc.section,
                anchor: &doc.anchor,
                excerpt: &doc.excerpt,
                score,
            });
        }
    }
}

#[inline]
fn record_term_score(
    term_slots: &mut [TermDocSlot],
    touched: &mut Vec<u32>,
    stamp: u32,
    doc_id: u32,
    score: f32,
) {
    let slot = &mut term_slots[doc_id as usize];
    if slot.stamp != stamp {
        slot.stamp = stamp;
        slot.best = score;
        touched.push(doc_id);
    } else if score > slot.best {
        slot.best = score;
    }
}

fn tokenize_query_into(text: &str, buf: &mut String, spans: &mut Vec<(usize, usize)>) {
    buf.clear();
    spans.clear();
    let mut cur_start = 0usize;
    for ch in text.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            for lc in ch.to_lowercase() {
                buf.push(lc);
            }
        } else if buf.len() > cur_start {
            let cur_len = buf.len() - cur_start;
            if cur_len >= 2 || spans.is_empty() {
                spans.push((cur_start, buf.len()));
                cur_start = buf.len();
            } else {
                buf.truncate(cur_start);
            }
        }
    }
    if buf.len() > cur_start {
        spans.push((cur_start, buf.len()));
    }
}

fn collect_trigram_spans_into(
    term: &str,
    char_offsets: &mut Vec<usize>,
    trigram_spans: &mut Vec<(usize, usize)>,
) {
    char_offsets.clear();
    trigram_spans.clear();
    for (byte_idx, _) in term.char_indices() {
        char_offsets.push(byte_idx);
    }
    let char_len = char_offsets.len();
    if char_len < 3 {
        return;
    }
    char_offsets.push(term.len());
    for i in 0..=(char_len - 3) {
        let start = char_offsets[i];
        let end = char_offsets[i + 3];
        let cand = &term[start..end];
        if !trigram_spans.iter().any(|&(ps, pe)| &term[ps..pe] == cand) {
            trigram_spans.push((start, end));
        }
    }
}
